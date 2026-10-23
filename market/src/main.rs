//! herdr toolkit plugin marketplace and updates.
//!
//!   herdr-market check [--startup]   the hook: checks for updates if due and opens the updates window
//!   herdr-market check --dry-run     checks now and prints what it found, without opening anything
//!   herdr-market updates             the updates window (launched by herdr as a popup)
//!   herdr-market open                the shortcut's action: opens the marketplace popup
//!   herdr-market                     the marketplace explorer (launched by herdr as a popup)

mod cache;
mod herdr;
mod http;
mod index;
mod install;
mod installed;
mod process;
mod state;
mod text;
mod theme;
mod ui_browse;
mod ui_updates;
mod updates;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use common::Settings;
use common::fs_util::write_atomic;
use common::settings::Marketplace;

use crate::herdr::OpenError;
use crate::updates::Update;

type AppResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// While herdr has another popup open, the updates window waits this long between tries, up to the limit.
const BUSY_RETRY: Duration = Duration::from_secs(5);
const BUSY_LIMIT: Duration = Duration::from_secs(120);

struct Env {
    herdr: String,
    plugin: String,
    settings: Settings,
    /// The plugin's config folder, where `settings.toml` is.
    config: PathBuf,
    /// The marketplace's folder inside the plugin's state folder.
    state: PathBuf,
}

fn env() -> Env {
    let herdr = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let plugin = env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| "jeg-herdr-toolkit".into());
    let config = env::var_os("HERDR_PLUGIN_CONFIG_DIR").map(PathBuf::from).unwrap_or_default();
    let (settings, warning) = Settings::load(&config);
    if let Some(warning) = warning {
        eprintln!("herdr-market: {warning}; using the defaults");
    }
    let state = env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("herdr-toolkit"))
        .join("market");
    Env { herdr, plugin, settings, config, state }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let env = env();
    let result = match args.first().map(String::as_str) {
        Some("check") if flag("--dry-run") => dry_run(&env),
        // A hook that fails has nowhere to show it: the reason goes to the plugin log and herdr is told
        // nothing went wrong, so a network outage never looks like a broken plugin.
        Some("check") => {
            if let Err(e) = check(&env, flag("--startup")) {
                eprintln!("herdr-market: {e}");
            }
            Ok(())
        }
        Some("updates") => ui_updates::run(&env.herdr, &env.state, env.settings.marketplace.mark_all),
        Some("open") => herdr::open_popup(&env.herdr, &env.plugin, "marketplace").map_err(|e| match e {
            OpenError::Busy => "herdr has another window open".into(),
            OpenError::Other(m) => m.into(),
        }),
        _ => ui_browse::run(&env.herdr, &env.state, env.settings.marketplace.clone()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("herdr-market: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Nothing,
    /// The check at startup is off: the interval starts now, so the first focus event does not do it.
    Postpone,
    Check,
    /// The last check found updates but could not open the window: try again.
    ShowPending,
}

fn next_step(startup: bool, m: &Marketplace, due: bool, pending: bool) -> Step {
    if startup {
        return if m.check_at_startup { Step::Check } else { Step::Postpone };
    }
    match (due, pending) {
        (true, _) => Step::Check,
        (false, true) => Step::ShowPending,
        (false, false) => Step::Nothing,
    }
}

fn check(env: &Env, startup: bool) -> AppResult {
    // Only `[marketplace]` is read: one bad shortcut elsewhere must not switch on checks that were
    // turned off. If that table itself is wrong, nothing is checked.
    let m = Settings::load_marketplace(&env.config).map_err(|e| format!("{e}; updates are not checked"))?;
    let now = SystemTime::now();
    if startup {
        // A new herdr server offers again what was turned down in the previous one, and forgets what
        // the previous one could not show (it is looked for again now).
        for result in [state::forget_dismissed(&env.state), state::forget_pending(&env.state)] {
            if let Err(e) = result {
                eprintln!("herdr-market: {e}");
            }
        }
    }
    let due = state::due(&env.state, m.check_every_hours, now);
    let step = next_step(startup, &m, due, state::has_pending(&env.state));
    match step {
        Step::Nothing => return Ok(()),
        Step::Postpone => return Ok(state::mark_checked(&env.state, now)?),
        Step::Check | Step::ShowPending => {}
    }
    let Some(_lock) = state::try_lock(&env.state, now)? else {
        return Ok(()); // another focus event's check is already running
    };
    if step == Step::Check {
        // Another check may have finished between `due` and taking the lock.
        if !startup && !state::due(&env.state, m.check_every_hours, now) {
            return Ok(());
        }
        // Marked before looking, whatever the result: offline, every focus event would otherwise retry.
        state::mark_checked(&env.state, now)?;
        let dismissed = state::dismissed(&env.state);
        let found: Vec<Update> = updates::find(&env.herdr)?
            .into_iter()
            .filter(|u| !dismissed.contains(&(u.id.clone(), u.new_commit.clone())))
            .collect();
        if found.is_empty() {
            return Ok(());
        }
        write_atomic(&env.state.join(state::PENDING), serde_json::to_string(&found)?.as_bytes())?;
    }
    let started = Instant::now();
    // Until the window opens, or until it has been opened and closed by someone else (the file is gone).
    while state::has_pending(&env.state) {
        match herdr::open_popup(&env.herdr, &env.plugin, "updates") {
            Ok(()) => return Ok(()),
            Err(OpenError::Busy) if started.elapsed() < BUSY_LIMIT => thread::sleep(BUSY_RETRY),
            Err(OpenError::Busy) => {
                return Err("herdr kept another window open; the updates will be offered on a later focus".into());
            }
            Err(OpenError::Other(e)) => return Err(format!("could not open the updates window: {e}").into()),
        }
    }
    Ok(())
}

fn dry_run(env: &Env) -> AppResult {
    let found = updates::find(&env.herdr)?;
    if found.is_empty() {
        println!("No updates.");
    }
    for u in &found {
        let flag = if u.runs_changed { "  (runs something different)" } else { "" };
        println!("{}  {} -> {}  {}{flag}", u.name, u.from, u.to, u.repo);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_hook_does() {
        let on = Marketplace::default();
        let off = Marketplace { check_at_startup: false, check_every_hours: 0, ..Marketplace::default() };
        assert_eq!(next_step(true, &on, false, false), Step::Check);
        // Turned off at startup: the interval starts now, so the first focus event does not check
        // as if it were a startup check.
        assert_eq!(next_step(true, &off, true, true), Step::Postpone);
        assert_eq!(next_step(false, &on, true, false), Step::Check);
        assert_eq!(next_step(false, &on, false, false), Step::Nothing);
        // The window could not be opened last time (no workspace yet, another window open): try again
        // on the next focus event instead of waiting for the next check.
        assert_eq!(next_step(false, &on, false, true), Step::ShowPending);
        assert_eq!(next_step(false, &off, false, true), Step::ShowPending);
    }
}
