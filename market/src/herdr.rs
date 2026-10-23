//! Calls to herdr that the marketplace makes besides installing: opening its popups.

use std::process::Command;

use serde_json::Value;

pub enum OpenError {
    /// Another popup or one of herdr's own modals is open (`ui_busy`): worth trying again later.
    Busy,
    Other(String),
}

/// Opens one of the plugin's popup panes (`updates`, `marketplace`) at the size of the settings panel.
pub fn open_popup(herdr: &str, plugin: &str, entrypoint: &str) -> Result<(), OpenError> {
    let output = Command::new(herdr)
        .args(["plugin", "pane", "open", "--plugin", plugin, "--entrypoint", entrypoint])
        .args(["--placement", "popup", "--width", "85%", "--height", "85%"])
        .output()
        .map_err(|e| OpenError::Other(e.to_string()))?;
    classify(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )
}

/// herdr reports a refusal as JSON, {"error":{"code":"ui_busy","message":"…"}}, on stderr (stdout is
/// also looked at, in case that changes); the exit status alone is not trusted.
fn classify(success: bool, stdout: &str, stderr: &str) -> Result<(), OpenError> {
    let error = [stdout, stderr]
        .iter()
        .filter_map(|s| serde_json::from_str::<Value>(s.trim()).ok())
        .find_map(|v| v.get("error").cloned());
    match error {
        Some(e) if e["code"] == "ui_busy" => Err(OpenError::Busy),
        Some(e) => {
            Err(OpenError::Other(e["message"].as_str().or(e["code"].as_str()).unwrap_or("herdr refused").to_string()))
        }
        None if success => Ok(()),
        None => Err(OpenError::Other(stderr.trim().to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUSY: &str = r#"{"error":{"code":"ui_busy","message":"a popup pane is already open"},"id":"cli:plugin"}"#;

    #[test]
    fn busy_on_stderr_or_stdout() {
        assert!(matches!(classify(false, "", BUSY), Err(OpenError::Busy)));
        assert!(matches!(classify(false, BUSY, ""), Err(OpenError::Busy)));
    }

    #[test]
    fn other_errors_keep_the_message() {
        let no_workspace = r#"{"error":{"code":"plugin_pane_open_failed","message":"no active workspace"}}"#;
        assert!(matches!(classify(false, "", no_workspace), Err(OpenError::Other(m)) if m == "no active workspace"));
        assert!(matches!(classify(false, "", "plain failure"), Err(OpenError::Other(m)) if m == "plain failure"));
        assert!(classify(true, r#"{"result":{"type":"ok"}}"#, "").is_ok());
    }
}
