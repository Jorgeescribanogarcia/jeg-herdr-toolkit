//! `settings.toml`: everything chosen in the settings panel. It is the single source of truth; what
//! herdr has to read from its own `config.toml` is generated from it.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::icon_sets::{self, ICON_SETS};
use crate::state::State;

pub const FILE: &str = "settings.toml";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub icons: Icons,
    pub colors: BTreeMap<State, Color>,
    pub viewer: Viewer,
    pub shortcuts: Shortcuts,
    pub marketplace: Marketplace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Icons {
    /// `id` of one of `ICON_SETS`.
    pub set: String,
    /// Single icons that replace the set's ones.
    pub overrides: BTreeMap<State, char>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    /// `#rrggbb`
    pub hex: String,
    #[serde(default)]
    pub bold: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Viewer {
    /// Size of the popup viewer, in % of the terminal.
    pub popup_width: u8,
    pub popup_height: u8,
    /// Tree width when it fits next to the preview, in % of the pane.
    pub tree_width: u8,
    pub show_ignored: bool,
    /// How often the disk and git are re-read, in seconds.
    pub refresh_secs: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub viewer_side: String,
    pub viewer_popup: String,
    pub settings: String,
    pub paste: String,
    pub marketplace: String,
}

/// How often the update check may run again while herdr stays open, in hours; 0 turns it off.
pub const CHECK_INTERVALS: [u8; 6] = [0, 1, 3, 6, 12, 24];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Marketplace {
    pub check_at_startup: bool,
    /// One of `CHECK_INTERVALS`.
    pub check_every_hours: u8,
    /// Whether the updates window starts with every update marked.
    pub mark_all: bool,
    /// The explorer's order when it opens.
    pub sort: Sort,
    /// Plugins whose `platforms` leave Windows out.
    pub hide_other_platforms: bool,
    pub hide_installed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// When the plugin entered herdr's index.
    Newest,
    /// Stars.
    Popular,
    /// Last push.
    Active,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            icons: Icons::default(),
            colors: State::ALL.iter().map(|&s| (s, default_color(s))).collect(),
            viewer: Viewer::default(),
            shortcuts: Shortcuts::default(),
            marketplace: Marketplace::default(),
        }
    }
}

impl Default for Icons {
    fn default() -> Icons {
        Icons { set: "material".into(), overrides: BTreeMap::new() }
    }
}

impl Default for Viewer {
    fn default() -> Viewer {
        Viewer { popup_width: 85, popup_height: 85, tree_width: 35, show_ignored: true, refresh_secs: 2 }
    }
}

impl Default for Shortcuts {
    fn default() -> Shortcuts {
        Shortcuts {
            viewer_side: "prefix+ctrl+f".into(),
            viewer_popup: "prefix+f".into(),
            settings: "prefix+.".into(),
            paste: "prefix+i".into(),
            marketplace: "prefix+m".into(),
        }
    }
}

impl Default for Marketplace {
    fn default() -> Marketplace {
        Marketplace {
            check_at_startup: true,
            check_every_hours: 6,
            mark_all: true,
            sort: Sort::Newest,
            hide_other_platforms: true,
            hide_installed: false,
        }
    }
}

impl Shortcuts {
    /// In the order of the plugin's actions: viewer on the right, viewer popup, settings, paste,
    /// marketplace.
    pub fn all(&self) -> [&str; 5] {
        [&self.viewer_side, &self.viewer_popup, &self.settings, &self.paste, &self.marketplace]
    }

    /// `index` in the order of `all`.
    pub fn set(&mut self, index: usize, shortcut: String) {
        let slot = match index {
            0 => &mut self.viewer_side,
            1 => &mut self.viewer_popup,
            2 => &mut self.settings,
            3 => &mut self.paste,
            _ => &mut self.marketplace,
        };
        *slot = shortcut;
    }
}

impl Settings {
    /// If the file does not exist: defaults, no warning. If it is wrong: defaults plus a warning about
    /// what failed, so whoever reads it (viewer, status icons) keeps working and says so.
    pub fn load(dir: &Path) -> (Settings, Option<String>) {
        let path = dir.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (Settings::default(), None),
            Err(e) => return (Settings::default(), Some(format!("could not read {}: {e}", path.display()))),
        };
        let settings = match toml::from_str::<Settings>(&text) {
            Ok(s) => s,
            Err(e) => return (Settings::default(), Some(format!("{} is malformed: {e}", path.display()))),
        };
        match settings.validate() {
            Ok(()) => (settings, None),
            Err(e) => (Settings::default(), Some(format!("{} is not valid: {e}", path.display()))),
        }
    }

    /// Only `[marketplace]`, for the update-check hooks. With `load`, one bad shortcut elsewhere would
    /// bring back the defaults and switch on checks the user had turned off; here only this table counts.
    /// A missing file or table gives the defaults; an invalid table is an error, so the hooks do nothing.
    pub fn load_marketplace(dir: &Path) -> Result<Marketplace, String> {
        let path = dir.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Marketplace::default()),
            Err(e) => return Err(format!("could not read {}: {e}", path.display())),
        };
        #[derive(Deserialize)]
        struct OnlyMarketplace {
            #[serde(default)]
            marketplace: Marketplace,
        }
        let m = toml::from_str::<OnlyMarketplace>(&text)
            .map_err(|e| format!("{} is malformed: {e}", path.display()))?
            .marketplace;
        if !CHECK_INTERVALS.contains(&m.check_every_hours) {
            return Err(format!(
                "the update check interval ({} h) must be one of {CHECK_INTERVALS:?}",
                m.check_every_hours
            ));
        }
        Ok(m)
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        crate::fs_util::write_atomic(&dir.join(FILE), text.as_bytes())
    }

    pub fn validate(&self) -> Result<(), String> {
        for (i, &a) in State::ALL.iter().enumerate() {
            for &b in &State::ALL[i + 1..] {
                if self.glyph(a) == self.glyph(b) {
                    return Err(format!("“{}” and “{}” have the same icon", a.label(), b.label()));
                }
            }
        }
        for state in State::ALL {
            let hex = &self.color(state).hex;
            let valid = hex.len() == 7 && hex.starts_with('#') && hex[1..].chars().all(|c| c.is_ascii_hexdigit());
            if !valid {
                return Err(format!("the colour of “{}” ({hex}) is not #rrggbb", state.label()));
            }
        }
        let v = &self.viewer;
        if !(30..=100).contains(&v.popup_width) || !(30..=100).contains(&v.popup_height) {
            return Err("the popup viewer must be between 30 and 100 %".into());
        }
        if !(20..=60).contains(&v.tree_width) {
            return Err("the tree must take between 20 and 60 %".into());
        }
        if !(1..=60).contains(&v.refresh_secs) {
            return Err("the refresh interval must be between 1 and 60 s".into());
        }
        if !CHECK_INTERVALS.contains(&self.marketplace.check_every_hours) {
            return Err(format!(
                "the update check interval ({} h) must be one of {CHECK_INTERVALS:?}",
                self.marketplace.check_every_hours
            ));
        }
        let shortcuts = self.shortcuts.all();
        if shortcuts.iter().any(|s| s.is_empty()) {
            return Err("a shortcut is empty".into());
        }
        for (i, a) in shortcuts.iter().enumerate() {
            if shortcuts[i + 1..].contains(a) {
                return Err("two plugin actions have the same shortcut".into());
            }
        }
        Ok(())
    }

    /// The state's override icon if any; otherwise its set's; if the set does not exist, Material's.
    pub fn glyph(&self, state: State) -> char {
        if let Some(&g) = self.icons.overrides.get(&state) {
            return g;
        }
        icon_sets::icon_set(&self.icons.set).unwrap_or(&ICON_SETS[0]).glyphs[state.index()]
    }

    /// Drops overrides equal to the set's own glyph: picking the set's icon again means «use the set»,
    /// so it follows the set when the set changes.
    pub fn normalize(&mut self) {
        let set = icon_sets::icon_set(&self.icons.set).unwrap_or(&ICON_SETS[0]);
        self.icons.overrides.retain(|state, glyph| set.glyphs[state.index()] != *glyph);
    }

    pub fn color(&self, state: State) -> Color {
        self.colors.get(&state).cloned().unwrap_or_else(|| default_color(state))
    }
}

/// One Dark palette.
fn default_color(state: State) -> Color {
    let (hex, bold) = match state {
        State::Working => ("#e5c07b", false),
        State::Blocked => ("#e06c75", true),
        State::Done => ("#98c379", false),
        State::Idle => ("#5c6370", false),
        State::Unknown => ("#4b5263", false),
    };
    Color { hex: hex.into(), bold }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-toolkit-tests-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert_eq!(s.glyph(State::Working), '\u{f006a}');
        assert_eq!(s.glyph(State::Idle), '\u{f05e1}');
        assert_eq!(s.color(State::Blocked), Color { hex: "#e06c75".into(), bold: true });
        assert_eq!(s.viewer.refresh_secs, 2);
        assert_eq!((s.viewer.popup_width, s.viewer.popup_height, s.viewer.tree_width), (85, 85, 35));
        assert!(s.viewer.show_ignored);
        assert_eq!(s.shortcuts.viewer_side, "prefix+ctrl+f");
        assert_eq!(s.shortcuts.viewer_popup, "prefix+f");
        assert_eq!(s.shortcuts.settings, "prefix+.");
        assert_eq!(s.shortcuts.paste, "prefix+i");
        assert_eq!(s.shortcuts.marketplace, "prefix+m");
        assert_eq!(
            s.marketplace,
            Marketplace {
                check_at_startup: true,
                check_every_hours: 6,
                mark_all: true,
                sort: Sort::Newest,
                hide_other_platforms: true,
                hide_installed: false,
            }
        );
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn marketplace_survives_an_invalid_shortcut() {
        let dir = folder("marketplace-alone");
        // Two actions with the same shortcut make the whole file invalid...
        fs::write(
            dir.join(FILE),
            "[shortcuts]\nsettings = \"prefix+f\"\n[marketplace]\ncheck_at_startup = false\ncheck_every_hours = 0\n",
        )
        .unwrap();
        assert!(Settings::load(&dir).1.is_some());
        // ...but the update check still honours the user's choice to turn it off.
        let m = Settings::load_marketplace(&dir).unwrap();
        assert!(!m.check_at_startup);
        assert_eq!(m.check_every_hours, 0);
        // An invalid [marketplace] itself is an error, not the defaults.
        fs::write(dir.join(FILE), "[marketplace]\ncheck_every_hours = 300\n").unwrap();
        assert!(Settings::load_marketplace(&dir).is_err());
        // No file: the defaults.
        assert_eq!(Settings::load_marketplace(&folder("marketplace-none")).unwrap(), Marketplace::default());
    }

    #[test]
    fn invalid_interval() {
        let mut s = Settings::default();
        s.marketplace.check_every_hours = 5;
        assert!(s.validate().is_err());
    }

    #[test]
    fn old_file_without_marketplace() {
        let dir = folder("no-marketplace");
        fs::write(
            dir.join(FILE),
            "[viewer]
refresh_secs = 3
[shortcuts]
settings = \"prefix+.\"
",
        )
        .unwrap();
        let (s, warning) = Settings::load(&dir);
        assert_eq!(warning, None);
        assert_eq!(s.marketplace, Marketplace::default());
        assert_eq!(s.shortcuts.marketplace, "prefix+m");
    }

    #[test]
    fn round_trip() {
        let dir = folder("round-trip");
        let mut s = Settings::default();
        s.icons.set = "text".into();
        s.icons.overrides.insert(State::Idle, '·');
        s.colors.insert(State::Working, Color { hex: "#61afef".into(), bold: false });
        s.viewer.refresh_secs = 5;
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir), (s, None));
    }

    #[test]
    fn broken_file_gives_defaults_and_warning() {
        let dir = folder("broken");
        fs::write(dir.join(FILE), "icons = 3\n").unwrap();
        let (s, warning) = Settings::load(&dir);
        assert_eq!(s, Settings::default());
        assert!(warning.is_some());
    }

    #[test]
    fn missing_file_no_warning() {
        let dir = folder("missing");
        assert_eq!(Settings::load(&dir), (Settings::default(), None));
    }

    #[test]
    fn override_equal_to_the_set_is_no_longer_custom() {
        let mut s = Settings::default();
        s.icons.overrides.insert(State::Working, '\u{f006a}'); // Material's own Working glyph
        s.icons.overrides.insert(State::Idle, '✓');
        s.normalize();
        assert_eq!(s.icons.overrides.len(), 1);
        assert_eq!(s.glyph(State::Working), '\u{f006a}');
        assert_eq!(s.glyph(State::Idle), '✓');
    }

    #[test]
    fn missing_field_takes_its_default() {
        let dir = folder("partial");
        fs::write(dir.join(FILE), "[viewer]\nrefresh_secs = 7\n").unwrap();
        let (s, warning) = Settings::load(&dir);
        assert_eq!(warning, None);
        assert_eq!(s.viewer.refresh_secs, 7);
        assert_eq!(s.viewer.tree_width, 35);
        assert_eq!(s.shortcuts, Shortcuts::default());
    }

    #[test]
    fn override_wins() {
        let mut s = Settings::default();
        s.icons.overrides.insert(State::Idle, '✓');
        assert_eq!(s.glyph(State::Idle), '✓');
        assert_eq!(s.glyph(State::Done), '\u{f05e0}');
    }

    #[test]
    fn unknown_set_uses_material() {
        let mut s = Settings::default();
        s.icons.set = "does-not-exist".into();
        assert_eq!(s.glyph(State::Working), '\u{f006a}');
    }

    #[test]
    fn repeated_glyphs_are_rejected() {
        let mut s = Settings::default();
        s.icons.overrides.insert(State::Working, '\u{f009e}');
        let error = s.validate().unwrap_err();
        assert!(error.contains("Waiting for you"), "{error}");
        assert!(error.contains("Working"), "{error}");
    }

    #[test]
    fn malformed_color_is_rejected() {
        let mut s = Settings::default();
        s.colors.insert(State::Done, Color { hex: "#12345".into(), bold: false });
        assert!(s.validate().is_err());
    }

    #[test]
    fn repeated_shortcuts_are_rejected() {
        let mut s = Settings::default();
        s.shortcuts.settings = s.shortcuts.viewer_popup.clone();
        assert!(s.validate().is_err());
    }

    #[test]
    fn sets_have_no_repeated_glyphs() {
        for set in ICON_SETS {
            for (i, g) in set.glyphs.iter().enumerate() {
                assert!(!set.glyphs[i + 1..].contains(g), "{} repeats {g}", set.id);
            }
        }
    }
}
