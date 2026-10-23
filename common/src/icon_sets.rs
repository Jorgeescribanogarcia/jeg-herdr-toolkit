//! Status icon sets. Within a set the five glyphs must differ: herdr colours the sidebar value by
//! the glyph it starts with, and there is no other way to colour it.

pub struct IconSet {
    pub id: &'static str,
    pub name: &'static str,
    /// In the order of `State::ALL`.
    pub glyphs: [char; 5],
}

pub const ICON_SETS: &[IconSet] = &[
    IconSet {
        id: "material",
        name: "Material",
        // md-autorenew, md-bell_ring, md-check_circle, md-check_circle_outline, md-help_circle_outline
        glyphs: ['\u{f006a}', '\u{f009e}', '\u{f05e0}', '\u{f05e1}', '\u{f0625}'],
    },
    IconSet {
        id: "codicons",
        name: "Codicons (VS Code)",
        // cod-sync, cod-bell_dot, cod-pass_filled, cod-pass, cod-question
        glyphs: ['\u{ea77}', '\u{eb9a}', '\u{ebb3}', '\u{eba4}', '\u{eb32}'],
    },
    IconSet {
        id: "circles",
        name: "Circles",
        // md-circle_slice_5, md-alert_circle, md-circle, md-circle_outline, md-help_circle_outline
        glyphs: ['\u{f0aa2}', '\u{f0028}', '\u{f0765}', '\u{f0766}', '\u{f0625}'],
    },
    IconSet { id: "text", name: "Text (no Nerd Font)", glyphs: ['↻', '!', '✔', '✓', '?'] },
];

pub fn icon_set(id: &str) -> Option<&'static IconSet> {
    ICON_SETS.iter().find(|s| s.id == id)
}
