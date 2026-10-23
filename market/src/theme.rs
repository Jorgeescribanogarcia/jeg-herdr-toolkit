//! `common::theme` as the UI library's colours.

use common::theme;
use ratatui::style::Color;

const fn rgb(c: [u8; 3]) -> Color {
    Color::Rgb(c[0], c[1], c[2])
}

pub const TEXT: Color = rgb(theme::TEXT);
pub const DIM: Color = rgb(theme::DIM);
pub const ACCENT: Color = rgb(theme::ACCENT);
pub const ERROR: Color = rgb(theme::ERROR);
pub const OK: Color = rgb(theme::OK);
pub const WARNING: Color = rgb(theme::WARNING);
pub const SELECTION_BG: Color = rgb(theme::SELECTION_BG);
pub const BACKGROUND: Color = rgb(theme::BACKGROUND);
