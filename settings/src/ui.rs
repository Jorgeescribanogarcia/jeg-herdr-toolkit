//! The panel's interface. Each change is validated at once and applied to herdr half a second after
//! the last one (so holding → does not reload herdr twenty times). `u` goes back to what there was
//! when the panel was opened.

use std::cell::Cell;
use std::fs;
use std::io;
use std::time::{Duration, Instant};

use common::publish::label;
use common::settings::Color as SettingsColor;
use common::{ICON_SETS, Settings, State};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::apply::{Env, save_and_reload};
use crate::herdr_config;
use crate::shortcuts::{clashes, key_to_herdr};

const DELAY: Duration = Duration::from_millis(500);

const TEXT: Color = Color::Rgb(0xab, 0xb2, 0xbf);
const DIM: Color = Color::Rgb(0x5c, 0x63, 0x70);
const ACCENT: Color = Color::Rgb(0x61, 0xaf, 0xef);
const ERROR: Color = Color::Rgb(0xe0, 0x6c, 0x75);
const OK: Color = Color::Rgb(0x98, 0xc3, 0x79);
const SELECTION_BG: Color = Color::Rgb(0x2c, 0x31, 0x3c);

/// One Dark.
const PALETTE: &[&str] =
    &["#e5c07b", "#e06c75", "#98c379", "#61afef", "#c678dd", "#56b6c2", "#abb2bf", "#5c6370", "#4b5263"];

/// Glyphs offered when changing a single icon: those of every set and a few more.
fn suggestions() -> Vec<char> {
    let extra = ['\u{f04b2}', '\u{f140b}', '\u{f06a9}', '\u{f0772}', '\u{f051f}', '\u{f03e6}', '\u{f0028}'];
    let mut all: Vec<char> = ICON_SETS.iter().flat_map(|s| s.glyphs).chain(extra).collect();
    let mut seen = std::collections::HashSet::new();
    all.retain(|g| seen.insert(*g));
    all
}

const SECTIONS: [&str; 4] = ["Icons", "Colours", "Viewer", "Shortcuts"];
const SHORTCUTS: [&str; 4] = ["Viewer on the right", "Viewer popup", "This panel", "Paste files / image"];

enum Mode {
    Normal,
    /// Typing a glyph code (`f05e0`) or a colour (`#rrggbb`).
    Typing {
        field: Field,
        text: String,
    },
    /// Waiting for the key combination of a shortcut.
    Capturing {
        which: usize,
        with_prefix: bool,
        candidate: Option<String>,
    },
}

#[derive(Clone, Copy)]
enum Field {
    Glyph(State),
    Color(State),
}

struct Panel<'a> {
    env: &'a Env<'a>,
    defaults: &'a str,
    settings: Settings,
    at_open: Settings,
    /// The last thing herdr accepted: if a change fails, it goes back here.
    applied: Settings,
    section: usize,
    row: usize,
    mode: Mode,
    message: Option<(String, bool)>,
    pending: Option<Instant>,
    quit: bool,
    // Where each part was drawn in the last frame, to know what the mouse hits.
    menu_area: Cell<Rect>,
    options_area: Cell<Rect>,
}

pub fn run(env: &Env, defaults: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (settings, warning) = Settings::load(&env.settings_dir);
    // If `config.toml` does not already hold what these settings generate (new install, or edited by
    // hand), it is applied without waiting for a change: opening the panel is enough to set things up.
    let config = fs::read_to_string(&env.config).unwrap_or_default();
    let out_of_date = herdr_config::apply(&config, &settings, &env.plugin).is_ok_and(|new| new != config);
    let mut panel = Panel {
        env,
        defaults,
        at_open: settings.clone(),
        applied: settings.clone(),
        settings,
        section: 0,
        row: 0,
        mode: Mode::Normal,
        message: warning.map(|w| (format!("{w}. Showing the defaults."), true)),
        pending: out_of_date.then(Instant::now),
        quit: false,
        menu_area: Cell::default(),
        options_area: Cell::default(),
    };
    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;
    let result = panel.run(&mut terminal);
    // On exit the popup closes; if this fails there is no terminal to give the mouse back to.
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

impl Panel<'_> {
    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<(), Box<dyn std::error::Error>> {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => self.key(k),
                    Event::Mouse(m) => self.mouse(m),
                    _ => {}
                }
            }
            if self.pending.is_some_and(|t| t.elapsed() >= DELAY) {
                self.apply();
            }
        }
        // Closing with a change not yet applied applies it: closing is not undoing.
        if self.pending.is_some() {
            self.apply();
        }
        Ok(())
    }

    fn apply(&mut self) {
        self.pending = None;
        match save_and_reload(self.env, &self.settings) {
            Ok(()) => {
                self.applied = self.settings.clone();
                self.message = Some(("Applied.".into(), false));
            }
            Err(e) => {
                self.settings = self.applied.clone();
                self.message = Some((e, true));
            }
        }
    }

    /// Tries a change: if it leaves the settings in an invalid state, it is not made and the reason is shown.
    fn change(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut new = self.settings.clone();
        change(&mut new);
        // Picking the set's own icon again means «follow the set», not a custom icon that looks the same.
        new.normalize();
        match new.validate() {
            Ok(()) => {
                self.settings = new;
                self.pending = Some(Instant::now());
                self.message = None;
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn rows(&self) -> usize {
        match self.section {
            0 => 1 + State::ALL.len(),
            1 => State::ALL.len(),
            2 => 5,
            _ => SHORTCUTS.len(),
        }
    }

    // --- Keyboard -------------------------------------------------------------------------------

    fn key(&mut self, k: KeyEvent) {
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.normal_key(k),
            Mode::Typing { field, text } => self.typing_key(k, field, text),
            Mode::Capturing { which, with_prefix, candidate } => self.capturing_key(k, which, with_prefix, candidate),
        }
    }

    fn normal_key(&mut self, k: KeyEvent) {
        let last = self.rows() - 1;
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.row = self.row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.row = (self.row + 1).min(last),
            KeyCode::Tab => self.go_to_section((self.section + 1) % SECTIONS.len()),
            KeyCode::BackTab => self.go_to_section((self.section + SECTIONS.len() - 1) % SECTIONS.len()),
            KeyCode::Char(c @ '1'..='4') => self.go_to_section(c as usize - '1' as usize),
            KeyCode::Left | KeyCode::Char('h') => self.adjust(-1),
            KeyCode::Right | KeyCode::Char('l') => self.adjust(1),
            KeyCode::Enter => self.edit(),
            KeyCode::Backspace | KeyCode::Delete => self.reset(),
            KeyCode::Char('b') if self.section == 1 => {
                let state = State::ALL[self.row];
                self.change(|s| {
                    let mut color = s.color(state);
                    color.bold = !color.bold;
                    s.colors.insert(state, color);
                });
            }
            KeyCode::Char('u') => {
                self.settings = self.at_open.clone();
                self.pending = Some(Instant::now());
                self.message = Some(("Undone: back to what there was when the panel opened.".into(), false));
            }
            _ => {}
        }
    }

    /// The selected row is a status icon with a custom glyph (not the set's).
    fn on_custom_icon(&self) -> bool {
        self.section == 0 && self.row > 0 && self.settings.icons.overrides.contains_key(&State::ALL[self.row - 1])
    }

    fn go_to_section(&mut self, section: usize) {
        self.section = section;
        self.row = 0;
        self.message = None;
    }

    /// Click on the menu: that section. Click on an option: selects it; if it was already selected,
    /// changes it like Enter (on the set's `◀`, backwards). Right click: changes backwards. Wheel: moves.
    fn mouse(&mut self, m: MouseEvent) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let point = Position::new(m.column, m.row);
        let (menu, options) = (self.menu_area.get(), self.options_area.get());
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if menu.contains(point) => {
                let i = (m.row - menu.y) as usize;
                if i < SECTIONS.len() {
                    self.go_to_section(i);
                }
            }
            MouseEventKind::Down(button @ (MouseButton::Left | MouseButton::Right)) if options.contains(point) => {
                let row = (m.row - options.y) as usize;
                if row >= self.rows() {
                    return;
                }
                let already_selected = row == self.row;
                self.row = row;
                // The value starts after the indent and the name (`  {name:<20}`); the set's `◀` is there.
                let on_back_arrow = m.column == options.x + 22;
                match button {
                    MouseButton::Right => self.adjust(-1),
                    _ if !already_selected => {}
                    _ if (self.section, row) == (0, 0) && on_back_arrow => self.adjust(-1),
                    _ => self.edit(),
                }
            }
            MouseEventKind::ScrollDown => self.row = (self.row + 1).min(self.rows() - 1),
            MouseEventKind::ScrollUp => self.row = self.row.saturating_sub(1),
            _ => {}
        }
    }

    /// ←→: the set, the single icon, the colour or the row's number, backwards or forwards.
    fn adjust(&mut self, step: i32) {
        let wrap = |i: usize, n: usize| ((i as i32 + step).rem_euclid(n as i32)) as usize;
        match (self.section, self.row) {
            (0, 0) => {
                let i = ICON_SETS.iter().position(|s| s.id == self.settings.icons.set).unwrap_or(0);
                let next = ICON_SETS[wrap(i, ICON_SETS.len())].id;
                self.change(|s| s.icons.set = next.into());
            }
            (0, row) => {
                let state = State::ALL[row - 1];
                let options = suggestions();
                let i = options.iter().position(|&g| g == self.settings.glyph(state)).unwrap_or(0);
                // Skips the ones another state already uses, to avoid hitting «same icon» at every step.
                for turn in 1..=options.len() as i32 {
                    let glyph = options[(i as i32 + step * turn).rem_euclid(options.len() as i32) as usize];
                    if State::ALL.iter().all(|&s| s == state || self.settings.glyph(s) != glyph) {
                        self.change(|s| {
                            s.icons.overrides.insert(state, glyph);
                        });
                        return;
                    }
                }
            }
            (1, row) => {
                let state = State::ALL[row];
                let current = self.settings.color(state);
                let i = PALETTE.iter().position(|&c| c == current.hex).unwrap_or(PALETTE.len() - 1);
                let hex = PALETTE[wrap(i, PALETTE.len())];
                self.change(|s| {
                    s.colors.insert(state, SettingsColor { hex: hex.into(), bold: current.bold });
                });
            }
            (2, 0) => self.change(|s| s.viewer.popup_width = add(s.viewer.popup_width, step * 5, 30, 100)),
            (2, 1) => self.change(|s| s.viewer.popup_height = add(s.viewer.popup_height, step * 5, 30, 100)),
            (2, 2) => self.change(|s| s.viewer.tree_width = add(s.viewer.tree_width, step * 5, 20, 60)),
            (2, 3) => self.change(|s| s.viewer.show_ignored = !s.viewer.show_ignored),
            (2, 4) => self.change(|s| s.viewer.refresh_secs = add(s.viewer.refresh_secs as u8, step, 1, 60).into()),
            _ => {}
        }
    }

    fn edit(&mut self) {
        self.message = None;
        self.mode = match (self.section, self.row) {
            (0, 0) => return self.adjust(1),
            (0, row) => Mode::Typing { field: Field::Glyph(State::ALL[row - 1]), text: String::new() },
            (1, row) => Mode::Typing { field: Field::Color(State::ALL[row]), text: String::new() },
            (2, _) => return self.adjust(1),
            (_, which) => Mode::Capturing { which, with_prefix: true, candidate: None },
        };
    }

    /// ⌫ returns a single icon to its set's, or a colour to the default.
    fn reset(&mut self) {
        match (self.section, self.row) {
            (0, row) if row > 0 => {
                let state = State::ALL[row - 1];
                self.change(|s| {
                    s.icons.overrides.remove(&state);
                });
            }
            (1, row) => {
                let state = State::ALL[row];
                self.change(|s| {
                    s.colors.remove(&state);
                });
            }
            _ => {}
        }
    }

    fn typing_key(&mut self, k: KeyEvent, field: Field, mut text: String) {
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Backspace => drop(text.pop()),
            KeyCode::Char(c) => text.push(c),
            KeyCode::Enter => {
                match field {
                    Field::Glyph(state) => {
                        let mut error = None;
                        self.change(|s| error = set_glyph_from_text(s, state, &text).err());
                        if let Some(e) = error {
                            self.message = Some((e, true));
                        }
                    }
                    Field::Color(state) => {
                        let hex = format!("#{}", text.trim_start_matches('#').to_lowercase());
                        let bold = self.settings.color(state).bold;
                        self.change(|s| {
                            s.colors.insert(state, SettingsColor { hex, bold });
                        });
                    }
                }
                return;
            }
            _ => {}
        }
        self.mode = Mode::Typing { field, text };
    }

    fn capturing_key(&mut self, k: KeyEvent, which: usize, mut with_prefix: bool, mut candidate: Option<String>) {
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Tab => {
                with_prefix = !with_prefix;
                candidate = None;
            }
            KeyCode::Enter if candidate.is_some() => {
                let shortcut = candidate.unwrap_or_default();
                let used_by = self.clashes(&shortcut);
                if !used_by.is_empty() {
                    self.message = Some((format!("{shortcut} is already used by: {}", used_by.join(", ")), true));
                    return;
                }
                self.change(|s| s.shortcuts.set(which, shortcut));
                return;
            }
            _ => candidate = key_to_herdr(k, with_prefix),
        }
        self.mode = Mode::Capturing { which, with_prefix, candidate };
    }

    fn clashes(&self, shortcut: &str) -> Vec<String> {
        let config = fs::read_to_string(&self.env.config).unwrap_or_default();
        clashes(shortcut, &config, self.defaults, &self.env.plugin)
    }

    // --- Drawing ------------------------------------------------------------------------------

    fn draw(&self, f: &mut Frame) {
        let [header, body, footer] =
            Layout::vertical([Constraint::Length(2), Constraint::Fill(1), Constraint::Length(2)]).areas(f.area());
        let title = Line::from(vec![
            Span::styled(" SETTINGS", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" · herdr toolkit", Style::new().fg(DIM)),
        ]);
        f.render_widget(Paragraph::new(title).block(Block::new().borders(Borders::BOTTOM).border_style(DIM)), header);

        let [menu, options] = Layout::horizontal([Constraint::Length(16), Constraint::Fill(1)]).areas(body);
        self.menu_area.set(menu);
        self.options_area.set(options.inner(ratatui::layout::Margin::new(1, 0)));
        let lines: Vec<Line> = SECTIONS
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let selected = i == self.section;
                let style =
                    if selected { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new().fg(TEXT) };
                Line::styled(format!(" {} {} {s}", if selected { '▸' } else { ' ' }, i + 1), style)
            })
            .collect();
        f.render_widget(Paragraph::new(lines).block(Block::new().borders(Borders::RIGHT).border_style(DIM)), menu);
        self.draw_options(f, options);

        let (text, color) = match (&self.mode, &self.message) {
            (Mode::Typing { field: Field::Glyph(_), text }, _) => {
                (format!(" Glyph code (e.g. f05e0), empty for the set's icon: {text}▏  ⏎ accept  Esc cancel"), ACCENT)
            }
            (Mode::Typing { field: Field::Color(_), text }, _) => {
                (format!(" Colour #rrggbb: {text}▏  ⏎ accept  Esc cancel"), ACCENT)
            }
            (Mode::Capturing { with_prefix, candidate, .. }, _) => {
                // Before any key is pressed it says whether the prefix will be used; after, the shortcut shows it.
                let before = match (candidate.is_some(), *with_prefix) {
                    (true, _) => "",
                    (false, true) => "with prefix: ",
                    (false, false) => "without prefix: ",
                };
                let shortcut = candidate.clone().unwrap_or_else(|| "…".into());
                let used_by = candidate.as_deref().map(|c| self.clashes(c)).unwrap_or_default();
                let state = if candidate.is_none() {
                    String::new()
                } else if used_by.is_empty() {
                    "  free".into()
                } else {
                    format!("  already used by: {}", used_by.join(", "))
                };
                (
                    format!(" Press the key ({before}{shortcut}){state}   Tab with/without prefix  ⏎ accept  Esc cancel"),
                    if used_by.is_empty() { ACCENT } else { ERROR },
                )
            }
            (_, Some((m, error))) => (format!(" {m}"), if *error { ERROR } else { OK }),
            _ if self.on_custom_icon() => (
                " Custom icon · ⌫ back to the set's icon · ←→ other icons · ⏎ type a code (empty: back to the set)"
                    .into(),
                ACCENT,
            ),
            _ => (
                " Click or 1-4: section · click: select and change (right: back) · ↑↓ ←→ ⏎ · ⌫ default · u undo · q close"
                    .into(),
                DIM,
            ),
        };
        f.render_widget(
            Paragraph::new(text)
                .style(Style::new().fg(color))
                .block(Block::new().borders(Borders::TOP).border_style(DIM)),
            footer,
        );
    }

    fn draw_options(&self, f: &mut Frame, area: Rect) {
        let s = &self.settings;
        let mut values: Vec<Vec<Span>> = Vec::new();
        match self.section {
            0 => {
                let set = ICON_SETS.iter().find(|i| i.id == s.icons.set).map_or("?", |i| i.name);
                values.push(vec![Span::raw(format!("◀ {set} ▶"))]);
                for state in State::ALL {
                    let origin = if s.icons.overrides.contains_key(&state) { "custom" } else { "from the set" };
                    values.push(vec![
                        Span::styled(s.glyph(state).to_string(), style_of(&s.color(state))),
                        Span::styled(format!("   ({origin})"), Style::new().fg(DIM)),
                    ]);
                }
            }
            1 => {
                for state in State::ALL {
                    let c = s.color(state);
                    let bold = if c.bold { "  bold" } else { "" };
                    values.push(vec![Span::styled("██ ", style_of(&c)), Span::raw(format!("{}{bold}", c.hex))]);
                }
            }
            2 => {
                let v = &s.viewer;
                for text in [
                    format!("{} %", v.popup_width),
                    format!("{} %", v.popup_height),
                    format!("{} %", v.tree_width),
                    if v.show_ignored { "shown, greyed out".into() } else { "hidden".into() },
                    format!("{} s", v.refresh_secs),
                ] {
                    values.push(vec![Span::raw(text)]);
                }
            }
            _ => {
                for shortcut in s.shortcuts.all() {
                    values.push(vec![Span::raw(shortcut.to_string())]);
                }
            }
        }
        let names: Vec<String> = match self.section {
            0 => std::iter::once("Style".to_string()).chain(State::ALL.iter().map(|s| s.label().to_string())).collect(),
            1 => State::ALL.iter().map(|s| s.label().to_string()).collect(),
            2 => ["Popup width", "Popup height", "Tree width", "Ignored by git", "Re-read every"]
                .map(String::from)
                .to_vec(),
            _ => SHORTCUTS.map(String::from).to_vec(),
        };
        let mut lines: Vec<Line> = names
            .into_iter()
            .zip(values)
            .enumerate()
            .map(|(i, (name, value))| {
                let mut parts = vec![Span::styled(format!("  {name:<20}"), Style::new().fg(TEXT))];
                parts.extend(value);
                let line = Line::from(parts);
                if i == self.row { line.style(Style::new().bg(SELECTION_BG)) } else { line }
            })
            .collect();
        if self.section <= 1 {
            // Preview: how the sidebar will look.
            lines.push(Line::raw(""));
            let mut sample = vec![Span::styled("  Preview:  ", Style::new().fg(DIM))];
            for (state, name) in
                [(State::Working, "Dev"), (State::Blocked, "Api"), (State::Done, "Docs"), (State::Idle, "Web")]
            {
                sample.push(Span::styled(label(s, state, Some(name)), style_of(&s.color(state))));
                sample.push(Span::raw("   "));
            }
            lines.push(Line::from(sample));
        }
        f.render_widget(Paragraph::new(lines), area.inner(ratatui::layout::Margin::new(1, 0)));
    }
}

fn style_of(color: &SettingsColor) -> Style {
    let rgb = u32::from_str_radix(color.hex.trim_start_matches('#'), 16).unwrap_or(0xabb2bf);
    let style = Style::new().fg(Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8));
    if color.bold { style.add_modifier(Modifier::BOLD) } else { style }
}

fn add(value: u8, step: i32, min: u8, max: u8) -> u8 {
    (value as i32 + step).clamp(min.into(), max.into()) as u8
}

/// What Enter does after typing a glyph: empty text goes back to the set's icon; otherwise the code or
/// character becomes the state's icon.
fn set_glyph_from_text(settings: &mut Settings, state: State, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        settings.icons.overrides.remove(&state);
        return Ok(());
    }
    let glyph = parse_glyph(text).ok_or_else(|| format!("“{text}” is not a glyph code (e.g. f05e0)"))?;
    settings.icons.overrides.insert(state, glyph);
    Ok(())
}

/// A hex code (`f05e0`, `U+F05E0`) or a character as it is.
fn parse_glyph(text: &str) -> Option<char> {
    let text = text.trim();
    let mut chars = text.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(c);
    }
    let code = text.trim_start_matches("U+").trim_start_matches("u+").trim_start_matches("\\u");
    u32::from_str_radix(code, 16).ok().and_then(char::from_u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_by_code_or_as_is() {
        assert_eq!(parse_glyph("f05e0"), Some('\u{f05e0}'));
        assert_eq!(parse_glyph("U+F05E0"), Some('\u{f05e0}'));
        assert_eq!(parse_glyph("✓"), Some('✓'));
        assert_eq!(parse_glyph("zz"), None);
    }

    #[test]
    fn typing_a_glyph() {
        let mut s = Settings::default();
        set_glyph_from_text(&mut s, State::Working, "✦").unwrap();
        assert_eq!(s.glyph(State::Working), '✦');
        // Empty text goes back to the set's icon instead of being an error.
        set_glyph_from_text(&mut s, State::Working, "  ").unwrap();
        assert!(s.icons.overrides.is_empty());
        assert!(set_glyph_from_text(&mut s, State::Working, "zz").is_err());
    }

    #[test]
    fn suggestions_have_no_repeats() {
        let s = suggestions();
        let unique: std::collections::HashSet<_> = s.iter().collect();
        assert_eq!(unique.len(), s.len());
    }
}
