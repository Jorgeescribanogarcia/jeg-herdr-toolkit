//! What herdr has to read from its own `config.toml` (the sidebar rows with their colours and the
//! plugin's shortcuts), generated from `settings.toml`. It is edited with `toml_edit`, which keeps the
//! rest of the file as it is, comments included: only `ui.sidebar.agents.rows`,
//! `ui.sidebar.spaces.rows` and the plugin's `[[keys.command]]` entries are touched.

use common::{Settings, State};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value, value};

/// Plugin actions with a shortcut: (action id, description herdr will show).
/// Same order as `Shortcuts::all`.
const ACTIONS: [(&str, &str); 4] = [
    ("viewer", "file viewer on the right"),
    ("viewer-popup", "file viewer popup"),
    ("settings", "herdr toolkit settings"),
    ("paste", "paste clipboard files or image as paths"),
];

/// Returns the new `config.toml`. `plugin` is the plugin id (`jeg-herdr-toolkit`).
pub fn apply(config: &str, settings: &Settings, plugin: &str) -> Result<String, String> {
    let mut doc: DocumentMut = config.parse().map_err(|e| format!("herdr's config.toml cannot be parsed: {e}"))?;

    // Missing tables are created as regular tables: if `doc["ui"][…] = …` were left to create them,
    // they would come out inline (`ui = { sidebar = { … } }`) and adding `[ui.sidebar.agents]` by hand
    // later would break the file. `ui` and `ui.sidebar` are implicit: only `[ui.sidebar.agents]` shows.
    prepare_table(doc.as_table_mut(), "ui", true)?;
    prepare_table(doc["ui"].as_table_like_mut().ok_or("[ui] is not a table")?, "sidebar", true)?;
    for panel in ["agents", "spaces"] {
        let sidebar = doc["ui"]["sidebar"].as_table_like_mut().ok_or("[ui.sidebar] is not a table")?;
        prepare_table(sidebar, panel, false)?;
    }

    let first = |rest: &str| format!("[\n  [{}{rest}],\n", status_token(settings));
    doc["ui"]["sidebar"]["agents"]["rows"] =
        Item::Value(parse_value(&format!("{}  [\"agent\"],\n]", first(", \"machine\", \"tab\"")))?);
    doc["ui"]["sidebar"]["spaces"]["rows"] =
        Item::Value(parse_value(&format!("{}  [\"branch\", \"git_status\"],\n]", first("")))?);

    if doc.get("keys").is_none() {
        doc["keys"] = Item::Table(Table::new());
    }
    if doc["keys"].get("command").is_none() {
        doc["keys"]["command"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    let bindings = doc["keys"]["command"].as_array_of_tables_mut().ok_or("keys.command is not an array of tables")?;
    let ours =
        |t: &Table| t.get("command").and_then(Item::as_str).is_some_and(|c| c.starts_with(&format!("{plugin}.")));
    let ids: Vec<String> = ACTIONS.iter().map(|(id, _)| format!("{plugin}.{id}")).collect();
    // The plugin's entries that no longer exist go; the remaining ones are updated in place, not moved.
    bindings.retain(|t| !ours(t) || ids.iter().any(|id| t.get("command").and_then(Item::as_str) == Some(id)));
    let keys = settings.shortcuts.all();
    for ((id, (_, description)), key) in ids.iter().zip(ACTIONS).zip(keys) {
        let existing = bindings.iter_mut().find(|t| t.get("command").and_then(Item::as_str) == Some(id));
        match existing {
            Some(t) => t["key"] = value(key),
            None => {
                let mut t = Table::new();
                t["key"] = value(key);
                t["type"] = value("plugin_action");
                t["command"] = value(id.as_str());
                t["description"] = value(description);
                bindings.push(t);
            }
        }
    }
    Ok(doc.to_string())
}

/// Makes sure `parent.key` is a table (regular or inline, whatever the user has) and, if missing,
/// creates a regular one. If it exists and is something else (`ui = "x"`), it is an error and nothing
/// is touched.
fn prepare_table(parent: &mut dyn toml_edit::TableLike, key: &str, implicit: bool) -> Result<(), String> {
    match parent.get(key) {
        None => {
            let mut table = Table::new();
            table.set_implicit(implicit);
            parent.insert(key, Item::Table(table));
            Ok(())
        }
        Some(item) if item.is_table_like() => Ok(()),
        Some(_) => Err(format!("“{key}” in config.toml is not a table; fix it by hand")),
    }
}

/// `{ token = "$status", rules = [...] }`: one rule per state, colouring the token by the glyph it
/// starts with (the only way to colour a plugin's value in the sidebar).
fn status_token(settings: &Settings) -> String {
    let rules: String = State::ALL
        .iter()
        .map(|&s| {
            let color = settings.color(s);
            let bold = if color.bold { ", bold = true" } else { "" };
            format!(
                "    {{ starts_with = {}, fg = \"{}\"{bold} }}, # {}\n",
                toml_string(settings.glyph(s)),
                color.hex,
                s.label()
            )
        })
        .collect();
    format!("{{ token = \"$status\", rules = [\n{rules}  ] }}")
}

/// Nerd Font glyphs are written as `\UXXXXXXXX`: in the file they read as a code rather than as an
/// empty box in an editor without the font. Every other character is written as it is.
fn toml_string(glyph: char) -> String {
    // Unicode private use areas, which is where Nerd Fonts puts its icons.
    let private = matches!(glyph as u32, 0xE000..=0xF8FF | 0xF0000..=0x10FFFF);
    if private || glyph == '"' || glyph == '\\' {
        format!("\"\\U{:08X}\"", glyph as u32)
    } else {
        format!("\"{glyph}\"")
    }
}

fn parse_value(text: &str) -> Result<Value, String> {
    text.parse().map_err(|e| format!("internal error generating the configuration: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLUGIN: &str = "jeg-herdr-toolkit";

    const FOREIGN: &str = r#"onboarding = false
# mine: do not touch
[ui.sound]
enabled = true

[[keys.command]]
key = "prefix+t"
type = "plugin_action"
command = "other.plugin.thing"
description = "from another plugin"
"#;

    #[test]
    fn keeps_what_is_not_ours() {
        let out = apply(FOREIGN, &Settings::default(), PLUGIN).unwrap();
        assert!(out.contains("# mine: do not touch"), "{out}");
        assert!(out.contains("[ui.sound]\nenabled = true"), "{out}");
        assert!(out.contains("command = \"other.plugin.thing\""), "{out}");
        assert!(out.starts_with("onboarding = false\n"), "{out}");
    }

    #[test]
    fn idempotent() {
        let once = apply(FOREIGN, &Settings::default(), PLUGIN).unwrap();
        let twice = apply(&once, &Settings::default(), PLUGIN).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn rules_use_the_set_glyph() {
        let mut s = Settings::default();
        s.icons.set = "text".into();
        let out = apply("", &s, PLUGIN).unwrap();
        let doc: DocumentMut = out.parse().unwrap();
        let rules = doc["ui"]["sidebar"]["spaces"]["rows"][0][0]["rules"].as_array().unwrap();
        let done = rules.get(State::Done.index()).unwrap().as_inline_table().unwrap();
        assert_eq!(done.get("starts_with").unwrap().as_str(), Some("✔"));
        assert_eq!(done.get("fg").unwrap().as_str(), Some("#98c379"));
        let blocked = rules.get(State::Blocked.index()).unwrap().as_inline_table().unwrap();
        assert_eq!(blocked.get("bold").unwrap().as_bool(), Some(true));
        assert_eq!(doc["ui"]["sidebar"]["agents"]["rows"][0][0]["token"].as_str(), Some("$status"));
    }

    #[test]
    fn plugin_shortcuts() {
        let mut s = Settings::default();
        s.shortcuts.viewer_popup = "prefix+g".into();
        let old = format!(
            "{FOREIGN}\n[[keys.command]]\nkey = \"prefix+f\"\ntype = \"plugin_action\"\ncommand = \"{PLUGIN}.viewer-popup\"\n"
        );
        let doc: DocumentMut = apply(&old, &s, PLUGIN).unwrap().parse().unwrap();
        let bindings = doc["keys"]["command"].as_array_of_tables().unwrap();
        let popups: Vec<_> =
            bindings.iter().filter(|t| t["command"].as_str() == Some("jeg-herdr-toolkit.viewer-popup")).collect();
        assert_eq!(popups.len(), 1);
        assert_eq!(popups[0]["key"].as_str(), Some("prefix+g"));
        assert_eq!(bindings.len(), 5); // the foreign one and the plugin's four
    }

    #[test]
    fn stale_plugin_shortcut_is_removed() {
        let old =
            format!("[[keys.command]]\nkey = \"prefix+z\"\ntype = \"plugin_action\"\ncommand = \"{PLUGIN}.gone\"\n");
        let out = apply(&old, &Settings::default(), PLUGIN).unwrap();
        assert!(!out.contains(".gone"), "{out}");
    }

    #[test]
    fn empty_config() {
        let doc: DocumentMut = apply("", &Settings::default(), PLUGIN).unwrap().parse().unwrap();
        let bindings = doc["keys"]["command"].as_array_of_tables().unwrap();
        let keys: Vec<_> = bindings.iter().map(|t| t["key"].as_str().unwrap().to_string()).collect();
        assert_eq!(keys, ["prefix+ctrl+f", "prefix+f", "prefix+.", "prefix+i"]);
    }

    #[test]
    fn creates_regular_tables_that_can_be_extended_by_hand() {
        for config in ["", "[ui]\nstatus_indicators = \"symbols\"\n"] {
            let out = apply(config, &Settings::default(), PLUGIN).unwrap();
            assert!(out.contains("[ui.sidebar.agents]"), "{out}");
            assert!(out.contains("[ui.sidebar.spaces]"), "{out}");
            assert!(!out.contains("sidebar = {") && !out.contains("ui = {"), "{out}");
            // What someone would do by hand later: add a key to the sidebar table.
            let by_hand = out.replace("[ui.sidebar.agents]\n", "[ui.sidebar.agents]\nrow_gap = 1\n");
            assert!(by_hand.parse::<DocumentMut>().is_ok(), "{by_hand}");
        }
    }

    #[test]
    fn broken_config_is_an_error() {
        assert!(apply("this = = no", &Settings::default(), PLUGIN).is_err());
    }
}
