//! The shortcut actions. They open the viewer on the active pane's folder (the repo root if it is
//! inside one), in one of two ways:
//!
//! - `side`: a pane to the right of the active one. If the tab already has one, it closes it.
//! - `popup`: a popup above everything. It takes all keyboard input, so it is closed from inside.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

use crate::AppResult;

pub fn run(mode: Option<&str>) -> AppResult {
    let popup = match mode {
        Some("side") => false,
        Some("popup") => true,
        other => return Err(format!("unknown mode {other:?}: use “open side” or “open popup”").into()),
    };
    let herdr = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let plugin = env::var("HERDR_PLUGIN_ID").map_err(|_| "HERDR_PLUGIN_ID is missing: herdr launches this")?;
    let pane_id = env::var("HERDR_PANE_ID").map_err(|_| "herdr did not pass the active pane (HERDR_PANE_ID)")?;

    let pane = &herdr_json(&herdr, &["pane", "get", &pane_id])?["result"]["pane"];

    // Side viewer toggle: if the tab already has one (or the shortcut was pressed from it), close it.
    if !popup && let Some(viewer) = viewer_in_tab(&herdr, pane)? {
        herdr_json(&herdr, &["plugin", "pane", "close", &viewer])?;
        return Ok(());
    }
    // foreground_cwd follows the `cd` of the foreground program; cwd is the one the pane started with.
    let folder = pane["foreground_cwd"]
        .as_str()
        .or(pane["cwd"].as_str())
        .ok_or("herdr does not report the active pane's folder")?;

    let root = Command::new("git")
        .args(["-C", folder, "rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|s| s.status.success())
        .map(|s| PathBuf::from(String::from_utf8_lossy(&s.stdout).trim()))
        .unwrap_or_else(|| PathBuf::from(folder));

    let mut open = Command::new(&herdr);
    open.args(["plugin", "pane", "open", "--plugin", &plugin, "--entrypoint", "viewer"]);
    if popup {
        let dir = env::var_os("HERDR_PLUGIN_CONFIG_DIR").map(PathBuf::from).unwrap_or_default();
        let (settings, warning) = common::Settings::load(&dir);
        if let Some(warning) = warning {
            eprintln!("herdr-viewer: {warning}; using the default size");
        }
        let (width, height) =
            (format!("{}%", settings.viewer.popup_width), format!("{}%", settings.viewer.popup_height));
        open.args(["--placement", "popup", "--width", &width, "--height", &height, "--env", "VIEWER_POPUP=1"]);
    } else {
        open.args(["--placement", "split", "--direction", "right", "--target-pane", &pane_id, "--focus"]);
    }
    let output = open.arg("--env").arg(format!("VIEWER_DIR={}", root.display())).output()?;
    if !output.status.success() {
        return Err(format!("herdr did not open the pane: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(())
}

/// herdr does not say which plugin opened each pane, and anyone can change a title; so the viewer is
/// recognised by the executable running in it, which is this very one.
fn viewer_in_tab(herdr: &str, pane: &Value) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let this = fs::canonicalize(env::current_exe()?)?;
    let tab = pane["tab_id"].as_str().ok_or("herdr does not report the active pane's tab")?;
    let workspace = pane["workspace_id"].as_str().ok_or("herdr does not report the active pane's workspace")?;
    let list = herdr_json(herdr, &["pane", "list", "--workspace", workspace])?;
    let panes = list["result"]["panes"].as_array().ok_or("herdr pane list has no “panes”")?;
    for p in panes.iter().filter(|p| p["tab_id"].as_str() == Some(tab)) {
        let Some(id) = p["pane_id"].as_str() else { continue };
        let info = herdr_json(herdr, &["pane", "process-info", "--pane", id])?;
        let processes = info["result"]["process_info"]["foreground_processes"].as_array().cloned().unwrap_or_default();
        let is_viewer = processes
            .iter()
            .any(|proc_| proc_["argv0"].as_str().and_then(|r| fs::canonicalize(r).ok()).is_some_and(|r| r == this));
        if is_viewer {
            return Ok(Some(id.to_string()));
        }
    }
    Ok(None)
}

fn herdr_json(herdr: &str, args: &[&str]) -> Result<Value, Box<dyn std::error::Error>> {
    let output = Command::new(herdr).args(args).output()?;
    if !output.status.success() {
        return Err(format!("herdr {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}
