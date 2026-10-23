//! Publishes the `$status` token («icon + workspace name») on every pane with an agent and on every
//! workspace; herdr's sidebar draws it instead of its own `state_icon`.

use std::error::Error;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::{Settings, State};

const SOURCE: &str = "jeg-herdr-toolkit:status";
pub const TOKEN: &str = "status";

/// Icon and workspace name in a single token. herdr separates each token from the next with « · »
/// and only spares its own `state_icon` from that; putting them together, the row reads «󰁪 Dev».
/// The price is that the name takes the state's colour, because colour rules apply to a whole token.
pub fn label(settings: &Settings, state: State, workspace: Option<&str>) -> String {
    let glyph = settings.glyph(state);
    match workspace {
        Some(name) => format!("{glyph} {name}"),
        None => glyph.to_string(),
    }
}

/// Goes over every pane and workspace instead of trusting the event that triggered it: events can
/// arrive out of order, and what counts is the state herdr has now. Each report carries the read
/// time as `--seq`, so herdr discards one from an older read that arrives late.
pub fn publish(herdr: &str, settings: &Settings) -> Result<(), Box<dyn Error>> {
    let seq = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().to_string();

    let workspaces = json(herdr, &["workspace", "list"])?;
    let workspaces = workspaces["result"]["workspaces"].as_array().ok_or("herdr workspace list has no “workspaces”")?;
    let name_of =
        |id: &str| workspaces.iter().find(|w| w["workspace_id"].as_str() == Some(id)).and_then(|w| w["label"].as_str());

    let panes = json(herdr, &["pane", "list"])?;
    let panes = panes["result"]["panes"].as_array().ok_or("herdr pane list has no “panes”")?;
    let mut workspaces_with_agent = Vec::new();
    for p in panes {
        let (Some(id), Some(workspace)) = (p["pane_id"].as_str(), p["workspace_id"].as_str()) else { continue };
        if p.get("agent").is_some() {
            workspaces_with_agent.push(workspace);
            let state = State::from_herdr(p["agent_status"].as_str().unwrap_or(""));
            report(herdr, "pane", id, Some(label(settings, state, name_of(workspace))), &seq)?;
        } else if p["tokens"].get(TOKEN).is_some() {
            // The agent exited and the pane is a plain terminal again.
            report(herdr, "pane", id, None, &seq)?;
        }
    }

    for w in workspaces {
        let Some(id) = w["workspace_id"].as_str() else { continue };
        let name = w["label"].as_str();
        let value = if workspaces_with_agent.contains(&id) {
            // The workspace's agent_status is already herdr's roll-up of all its agents.
            let state = State::from_herdr(w["agent_status"].as_str().unwrap_or(""));
            Some(label(settings, state, name))
        } else {
            // The token is the only thing on the space's first row: without an agent it still has
            // to carry the name, or the row comes out empty and only the branch shows.
            name.map(str::to_string)
        };
        if value.as_deref() != w["tokens"].get(TOKEN).and_then(Value::as_str) {
            report(herdr, "workspace", id, value, &seq)?;
        }
    }
    Ok(())
}

/// `kind`: «pane» or «workspace». `None` clears the token.
fn report(herdr: &str, kind: &str, id: &str, value: Option<String>, seq: &str) -> Result<(), Box<dyn Error>> {
    let token = match value {
        Some(v) => ["--token".to_string(), format!("{TOKEN}={v}")],
        None => ["--clear-token".to_string(), TOKEN.to_string()],
    };
    // report-metadata prints nothing on success: its exit code is enough.
    herdr_cli(herdr, &[kind, "report-metadata", id, "--source", SOURCE, &token[0], &token[1], "--seq", seq])?;
    Ok(())
}

fn json(herdr: &str, args: &[&str]) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&herdr_cli(herdr, args)?)?)
}

fn herdr_cli(herdr: &str, args: &[&str]) -> Result<Vec<u8>, Box<dyn Error>> {
    let output = Command::new(herdr).args(args).output()?;
    if !output.status.success() {
        return Err(format!("herdr {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_working() {
        assert_eq!(label(&Settings::default(), State::Working, Some("Dev")), "\u{f006a} Dev");
    }

    #[test]
    fn text_set_done() {
        let mut s = Settings::default();
        s.icons.set = "text".into();
        assert_eq!(label(&s, State::Done, Some("Dev")), "✔ Dev");
    }

    #[test]
    fn without_workspace_only_the_icon() {
        assert_eq!(label(&Settings::default(), State::Idle, None), "\u{f05e1}");
    }
}
