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
    IconSet {
        id: "shapes",
        name: "Shapes (no Nerd Font)",
        // half circle, diamond, filled circle, circle, hollow diamond
        glyphs: ['◐', '◆', '●', '○', '◇'],
    },
    IconSet {
        id: "font-awesome",
        name: "Font Awesome",
        // fa-spinner, fa-bell, fa-check_circle, fa-check, fa-question_circle
        glyphs: ['\u{f110}', '\u{f0f3}', '\u{f058}', '\u{f00c}', '\u{f059}'],
    },
    IconSet {
        id: "octicons",
        name: "Octicons (GitHub)",
        // oct-sync, oct-bell_fill, oct-check_circle_fill, oct-check_circle, oct-question
        glyphs: ['\u{f46a}', '\u{f476}', '\u{f4a4}', '\u{f49e}', '\u{f420}'],
    },
    IconSet {
        id: "progress",
        name: "Progress",
        // md-progress_clock, md-progress_alert, md-progress_check, md-circle_outline, md-progress_question
        glyphs: ['\u{f0996}', '\u{f0cbc}', '\u{f0995}', '\u{f0766}', '\u{f1522}'],
    },
    IconSet {
        id: "time",
        name: "Time",
        // md-timer_sand, md-alarm, md-timer_check, md-clock_outline, md-clock_alert_outline
        glyphs: ['\u{f051f}', '\u{f0020}', '\u{f1ad0}', '\u{f0150}', '\u{f05ce}'],
    },
    IconSet {
        id: "signals",
        name: "Signals",
        // md-play_circle, md-hand_back_right, md-flag_checkered, md-sleep, md-help_circle
        glyphs: ['\u{f040c}', '\u{f0e47}', '\u{f023c}', '\u{f04b2}', '\u{f02d7}'],
    },
    IconSet {
        id: "robots",
        name: "Robots",
        // md-robot_excited, md-robot_confused, md-robot_happy, md-robot_outline, md-robot_off
        glyphs: ['\u{f16a3}', '\u{f169f}', '\u{f1719}', '\u{f167a}', '\u{f16a7}'],
    },
    IconSet {
        id: "weather",
        name: "Weather",
        // md-weather_lightning, md-weather_pouring, md-weather_sunny, md-weather_night, md-weather_fog
        glyphs: ['\u{f0593}', '\u{f0596}', '\u{f0599}', '\u{f0594}', '\u{f0591}'],
    },
];

pub fn icon_set(id: &str) -> Option<&'static IconSet> {
    ICON_SETS.iter().find(|s| s.id == id)
}
