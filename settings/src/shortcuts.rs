//! Keyboard shortcuts: turning a pressed key into herdr's format and checking whether it clashes
//! with another shortcut.
//!
//! herdr accepts a repeated shortcut without warning (verified with `prefix+s`, its `settings`
//! binding, which was accepted with no diagnostic), so the panel has to check before saving.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use toml_edit::{DocumentMut, Item};

/// herdr's format: `prefix+` when it uses the prefix, modifiers `ctrl+alt+shift+` in that order,
/// lower-case letters (upper case is written `shift+`), `+` as `plus`, space as `space` and `-` as
/// `minus`. `None` if the key cannot be a shortcut: Esc, or a character without prefix nor Ctrl/Alt,
/// which could then no longer be typed in the terminal.
pub fn key_to_herdr(key: KeyEvent, with_prefix: bool) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let name = match key.code {
        KeyCode::Char(c) => {
            if !with_prefix && !ctrl && !alt {
                return None;
            }
            if c.is_ascii_alphabetic() {
                shift |= c.is_ascii_uppercase();
                c.to_ascii_lowercase().to_string()
            } else {
                // For a symbol, Shift is already part of the character («?» is Shift+/).
                shift = false;
                symbol_name(c)
            }
        }
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => {
            shift = true;
            "tab".into()
        }
        KeyCode::Enter => "enter".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        _ => return None,
    };
    let mut parts = Vec::new();
    if with_prefix {
        parts.push("prefix".to_string());
    }
    for (active, modifier) in [(ctrl, "ctrl"), (alt, "alt"), (shift, "shift")] {
        if active {
            parts.push(modifier.to_string());
        }
    }
    parts.push(name);
    Some(parts.join("+"))
}

/// The symbols herdr cannot read written as they are inside a shortcut (`+` is the separator).
fn symbol_name(c: char) -> String {
    match c {
        '+' => "plus".into(),
        ' ' => "space".into(),
        '-' => "minus".into(),
        other => other.to_string(),
    }
}

/// What already uses `shortcut`: herdr action names (`settings`, `new_tab`…) and descriptions of
/// `[[keys.command]]` entries from other plugins. `config` is herdr's `config.toml`; `defaults`, the
/// output of `herdr --default-config`.
///
/// **Why the text of `--default-config` is read:** herdr exposes its list of shortcuts no other way.
/// That text is an example `config.toml` with every binding commented out (`# settings = "prefix+s"`)
/// in the `[keys]` section. If herdr ever offers the list through its API, switch to that call.
pub fn clashes(shortcut: &str, config: &str, defaults: &str, plugin: &str) -> Vec<String> {
    let wanted = normalize(shortcut);
    let mut actions = default_bindings(defaults);
    let doc: Option<DocumentMut> = config.parse().ok();
    let keys = doc.as_ref().and_then(|d| d.get("keys")).and_then(Item::as_table);
    for (name, value) in keys.into_iter().flat_map(|k| k.iter()) {
        // herdr accepts one binding (`"prefix+n"`) or several (`["prefix+n", "ctrl+alt+]"]`).
        let bindings: Vec<&str> = match (value.as_str(), value.as_array()) {
            (Some(binding), _) => vec![binding],
            (None, Some(list)) => list.iter().filter_map(|v| v.as_str()).collect(),
            _ => continue,
        };
        actions.retain(|(n, _)| n != name);
        actions.extend(bindings.into_iter().map(|b| (name.to_string(), b.to_string())));
    }
    let mut found: Vec<String> = actions
        .into_iter()
        .filter(|(_, binding)| expand(binding).iter().any(|b| normalize(b) == wanted))
        .map(|(name, _)| name)
        .collect();

    let commands = keys.and_then(|k| k.get("command")).and_then(Item::as_array_of_tables);
    for t in commands.into_iter().flatten() {
        let command = t.get("command").and_then(Item::as_str).unwrap_or("");
        if command.starts_with(&format!("{plugin}.")) {
            continue;
        }
        if t.get("key").and_then(Item::as_str).is_some_and(|k| normalize(k) == wanted) {
            let description = t.get("description").and_then(Item::as_str).unwrap_or(command);
            found.push(description.to_string());
        }
    }
    found.dedup();
    found
}

/// `(action, binding)` from the `# name = "binding"` lines of the `[keys]` section. It skips the
/// commented example blocks (`# [[keys.command]]`, `# [keys.indexed]`), whose keys are not actions,
/// and empty bindings.
fn default_bindings(text: &str) -> Vec<(String, String)> {
    let mut in_keys = false;
    let mut in_example = false;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_keys = line == "[keys]";
            in_example = false;
            continue;
        }
        let Some(commented) = line.strip_prefix('#').map(str::trim) else { continue };
        if commented.starts_with('[') {
            in_example = true;
            continue;
        }
        if !in_keys || in_example {
            continue;
        }
        let Some((name, rest)) = commented.split_once('=') else { continue };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            continue;
        }
        let value = rest.trim().strip_prefix('"').and_then(|r| r.split_once('"')).map(|(v, _)| v);
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            out.push((name.to_string(), value.to_string()));
        }
    }
    out
}

/// `prefix+1..9` is nine shortcuts.
fn expand(binding: &str) -> Vec<String> {
    match binding.strip_suffix("1..9") {
        Some(base) => (1..=9).map(|n| format!("{base}{n}")).collect(),
        None => vec![binding.to_string()],
    }
}

/// A single form of a shortcut for comparing, with the same equivalences as herdr: upper case =
/// `shift+` and lower case, symbols by name (`comma` = `,`), modifiers in any order.
fn normalize(shortcut: &str) -> String {
    let (mut prefix, mut ctrl, mut alt, mut shift) = (false, false, false, false);
    let mut key = String::new();
    for part in shortcut.split('+').map(str::trim) {
        match part.to_lowercase().as_str() {
            "prefix" => prefix = true,
            "ctrl" | "control" => ctrl = true,
            "alt" | "meta" | "option" => alt = true,
            "shift" => shift = true,
            _ => {
                shift |= part.len() == 1 && part.chars().all(|c| c.is_ascii_uppercase());
                key = part.to_lowercase();
            }
        }
    }
    let key = match key.as_str() {
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "backslash" => "\\",
        "quote" => "'",
        "double_quote" | "double-quote" => "\"",
        "semicolon" => ";",
        "colon" => ":",
        "percent" => "%",
        "ampersand" => "&",
        "backtick" => "`",
        "return" => "enter",
        "escape" => "esc",
        "bs" => "backspace",
        other => other,
    };
    format!("{prefix}|{ctrl}|{alt}|{shift}|{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULTS: &str = include_str!("testdata/herdr-default-config.toml");
    const PLUGIN: &str = "jeg-herdr-toolkit";

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn herdr_format() {
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(key_to_herdr(key(KeyCode::Char('f'), ctrl), true).as_deref(), Some("prefix+ctrl+f"));
        assert_eq!(key_to_herdr(key(KeyCode::Char('.'), KeyModifiers::NONE), true).as_deref(), Some("prefix+."));
        assert_eq!(key_to_herdr(key(KeyCode::Char('G'), KeyModifiers::SHIFT), true).as_deref(), Some("prefix+shift+g"));
        assert_eq!(key_to_herdr(key(KeyCode::Char('+'), KeyModifiers::NONE), true).as_deref(), Some("prefix+plus"));
        assert_eq!(key_to_herdr(key(KeyCode::Char('-'), KeyModifiers::NONE), true).as_deref(), Some("prefix+minus"));
        assert_eq!(key_to_herdr(key(KeyCode::F(5), KeyModifiers::NONE), false).as_deref(), Some("f5"));
        assert_eq!(
            key_to_herdr(key(KeyCode::Char('g'), ctrl | KeyModifiers::ALT), false).as_deref(),
            Some("ctrl+alt+g")
        );
    }

    #[test]
    fn keys_that_are_not_valid() {
        assert_eq!(key_to_herdr(key(KeyCode::Esc, KeyModifiers::NONE), true), None);
        // Without the prefix, a bare letter could no longer be typed.
        assert_eq!(key_to_herdr(key(KeyCode::Char('g'), KeyModifiers::NONE), false), None);
        assert_eq!(key_to_herdr(key(KeyCode::Char('G'), KeyModifiers::SHIFT), false), None);
    }

    #[test]
    fn clashes_with_a_herdr_action() {
        assert_eq!(clashes("prefix+s", "", DEFAULTS, PLUGIN), ["settings"]);
        assert_eq!(clashes("prefix+minus", "", DEFAULTS, PLUGIN), ["split_horizontal"]);
        // Upper case and shift+ are the same key for herdr.
        assert_eq!(clashes("prefix+N", "", DEFAULTS, PLUGIN), ["new_workspace"]);
        assert_eq!(clashes("prefix+3", "", DEFAULTS, PLUGIN), ["switch_tab"]);
    }

    #[test]
    fn free_does_not_clash() {
        assert!(clashes("prefix+.", "", DEFAULTS, PLUGIN).is_empty());
        assert!(clashes("prefix+ctrl+f", "", DEFAULTS, PLUGIN).is_empty());
        assert!(clashes("prefix+m", "", DEFAULTS, PLUGIN).is_empty());
    }

    #[test]
    fn user_overrides_win() {
        let config = "[keys]\nsettings = \"prefix+y\"\n";
        assert!(clashes("prefix+s", config, DEFAULTS, PLUGIN).is_empty());
        assert_eq!(clashes("prefix+y", config, DEFAULTS, PLUGIN), ["settings"]);
    }

    #[test]
    fn bindings_written_as_a_list() {
        let config = "[keys]\nnext_tab = [\"prefix+y\", \"ctrl+alt+]\"]\nprefix = [\"ctrl+space\", \"ctrl+s\"]\n";
        assert_eq!(clashes("prefix+y", config, DEFAULTS, PLUGIN), ["next_tab"]);
        assert_eq!(clashes("ctrl+alt+]", config, DEFAULTS, PLUGIN), ["next_tab"]);
        // The default no longer applies: the user changed it.
        assert!(clashes("prefix+n", config, DEFAULTS, PLUGIN).is_empty());
        assert_eq!(clashes("ctrl+s", config, DEFAULTS, PLUGIN), ["prefix"]);
    }

    #[test]
    fn clashes_with_a_foreign_command_but_not_with_ours() {
        let config = format!(
            "[[keys.command]]\nkey = \"prefix+t\"\ntype = \"popup\"\ncommand = \"lazygit\"\ndescription = \"git\"\n\n\
             [[keys.command]]\nkey = \"prefix+f\"\ntype = \"plugin_action\"\ncommand = \"{PLUGIN}.viewer-popup\"\n"
        );
        assert_eq!(clashes("prefix+t", &config, DEFAULTS, PLUGIN), ["git"]);
        assert!(clashes("prefix+f", &config, DEFAULTS, PLUGIN).is_empty());
    }
}
