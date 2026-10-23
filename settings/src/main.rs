//! herdr toolkit settings panel: status icons and colours, viewer and shortcuts.
//!
//!   herdr-settings         the panel (launched by herdr as a popup)
//!   herdr-settings open    the shortcut's action: opens the panel

mod apply;
mod herdr_config;
mod shortcuts;
mod ui;

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use serde_json::Value;

type Error = Box<dyn std::error::Error>;

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        Some("open") => open(),
        _ => panel(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("herdr-settings: {e}");
            ExitCode::FAILURE
        }
    }
}

fn herdr_bin() -> String {
    env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())
}

fn plugin() -> Result<String, Error> {
    env::var("HERDR_PLUGIN_ID").map_err(|_| "HERDR_PLUGIN_ID is missing: herdr launches this".into())
}

fn open() -> Result<(), Error> {
    let output = Command::new(herdr_bin())
        .args(["plugin", "pane", "open", "--plugin", &plugin()?, "--entrypoint", "settings"])
        .args(["--placement", "popup", "--width", "85%", "--height", "85%"])
        .output()?;
    if !output.status.success() {
        return Err(format!("herdr did not open the panel: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(())
}

fn panel() -> Result<(), Error> {
    let settings_dir = env::var_os("HERDR_PLUGIN_CONFIG_DIR")
        .map(PathBuf::from)
        .ok_or("HERDR_PLUGIN_CONFIG_DIR is missing: herdr launches this")?;
    // Where herdr reads its configuration from, per its documentation: HERDR_CONFIG_PATH or the usual one.
    let config = match env::var_os("HERDR_CONFIG_PATH") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(env::var_os("APPDATA").ok_or("APPDATA is missing")?).join("herdr").join("config.toml"),
    };
    let bin = herdr_bin();
    let herdr = |args: &[&str]| -> Result<Value, String> {
        let output = Command::new(&bin).args(args).output().map_err(|e| format!("could not run herdr: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        serde_json::from_slice(&output.stdout).map_err(|e| format!("unreadable herdr reply: {e}"))
    };
    let publish = |s: &common::Settings| common::publish::publish(&bin, s).map_err(|e| e.to_string());
    let defaults = Command::new(&bin).arg("--default-config").output()?;
    let env = apply::Env { settings_dir, config, plugin: plugin()?, herdr: &herdr, publish: &publish };
    ui::run(&env, &String::from_utf8_lossy(&defaults.stdout))
}
