//! File viewer for herdr: a tree with Material icons and git status, and a Markdown and code preview,
//! in a pane next to the terminal.
//!
//!   herdr-viewer               the pane (launched by herdr; the folder arrives in VIEWER_DIR)
//!   herdr-viewer open side     shortcut: pane to the right of the active one, or closes it if present
//!   herdr-viewer open popup    shortcut: popup above everything (Esc or q close it)

mod git;
mod icons;
mod open;
mod preview;
mod tree;

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use common::Settings;
use common::settings;
use git::{GitStatus, Status};
use tree::Tree;

pub type AppResult = Result<(), Box<dyn std::error::Error>>;

/// Below this width the tree and the preview do not fit side by side, and take turns.
const DOUBLE_WIDTH: u16 = 80;
const WHEEL: u16 = 3;

const TEXT: Color = Color::Rgb(0xcc, 0xcc, 0xcc);
const DIM: Color = Color::Rgb(0x6e, 0x76, 0x81);
const SELECTION_BG: Color = Color::Rgb(0x04, 0x39, 0x5e); // VS Code Dark+ focused selection
const SELECTION_BG_UNFOCUSED: Color = Color::Rgb(0x37, 0x37, 0x3d);
const CHEVRON_CLOSED: char = '\u{eab6}'; // cod-chevron_right
const CHEVRON_OPEN: char = '\u{eab4}'; // cod-chevron_down
const BRANCH: char = '\u{e0a0}'; // pl-branch

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("open") => open::run(args.get(1).map(String::as_str)),
        _ => viewer(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("herdr-viewer: {e}");
            ExitCode::FAILURE
        }
    }
}

fn viewer() -> AppResult {
    let folder = match env::var_os("VIEWER_DIR") {
        Some(d) => PathBuf::from(d),
        None => env::current_dir()?,
    };
    // A broken-settings warning cannot go to stderr: it would be drawn over the pane. It goes to the footer.
    let dir = env::var_os("HERDR_PLUGIN_CONFIG_DIR").map(PathBuf::from).unwrap_or_default();
    let (settings, warning) = Settings::load(&dir);
    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;
    let mut app = App::new(folder, settings.viewer);
    app.warning = warning;
    app.popup = env::var_os("VIEWER_POPUP").is_some();
    let result = app.run(&mut terminal);
    // On exit the pane closes; if this fails there is no terminal to give the mouse back to.
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Tree,
    Preview,
}

struct PreviewState {
    path: PathBuf,
    width: u16,
    modified: Option<SystemTime>,
    text: Text<'static>,
    scroll: u16,
}

struct App {
    tree: Tree,
    git: Option<Rc<GitStatus>>,
    settings: settings::Viewer,
    /// If `settings.toml` is wrong: defaults are used and the footer says so.
    warning: Option<String>,
    selected: usize,
    offset: usize,
    focus: Focus,
    preview: Option<PreviewState>,
    /// Number of the last requested preview: replies to older requests are discarded.
    request: u64,
    sender: Sender<(u64, Text<'static>)>,
    receiver: Receiver<(u64, Text<'static>)>,
    last_refresh: Instant,
    quit: bool,
    /// Opened as a popup: Esc closes it, like any floating window.
    popup: bool,
    // Where each part was drawn in the last frame, to know what the mouse hits.
    tree_area: Rect,
    preview_area: Rect,
}

impl App {
    fn new(root: PathBuf, settings: settings::Viewer) -> App {
        let (sender, receiver) = mpsc::channel();
        let mut app = App {
            git: None,
            tree: Tree::new(root),
            settings,
            warning: None,
            selected: 0,
            offset: 0,
            focus: Focus::Tree,
            preview: None,
            request: 0,
            sender,
            receiver,
            last_refresh: Instant::now(),
            quit: false,
            popup: false,
            tree_area: Rect::default(),
            preview_area: Rect::default(),
        };
        app.read_git();
        app.tree.reload();
        app
    }

    /// Reads git and, if ignored files are not shown, tells the tree for its next reload.
    fn read_git(&mut self) {
        self.git = GitStatus::read(&self.tree.root).map(Rc::new);
        self.tree.filter = match (&self.git, self.settings.show_ignored) {
            (Some(git), false) => {
                let (git, root) = (Rc::clone(git), self.tree.root.clone());
                Some(Box::new(move |path: &Path, is_folder| {
                    git.of(&relative(&root, path), is_folder) == Some(Status::Ignored)
                }))
            }
            _ => None,
        };
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> AppResult {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            self.request_preview();
            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => self.key(k),
                    Event::Mouse(m) => self.mouse(m),
                    _ => {}
                }
            }
            while let Ok((n, text)) = self.receiver.try_recv() {
                if let (true, Some(p)) = (n == self.request, self.preview.as_mut()) {
                    p.text = text;
                }
            }
            if self.last_refresh.elapsed() >= Duration::from_secs(self.settings.refresh_secs.into()) {
                self.refresh();
            }
        }
        Ok(())
    }

    fn refresh(&mut self) {
        let selected = self.tree.rows.get(self.selected).map(|r| r.path.clone());
        self.read_git();
        self.tree.reload();
        if let Some(i) = selected.and_then(|p| self.tree.position(&p)) {
            self.selected = i;
        }
        self.selected = self.selected.min(self.tree.rows.len().saturating_sub(1));
        self.last_refresh = Instant::now();
    }

    /// Requests the preview of the selected file unless there already is one for that file, that width
    /// and that modification time. It is rendered on another thread: glow is slow with a long Markdown.
    fn request_preview(&mut self) {
        let Some(row) = self.tree.rows.get(self.selected) else { return };
        if row.is_folder || self.preview_area.width == 0 {
            return;
        }
        let width = self.preview_area.width;
        let modified = std::fs::metadata(&row.path).and_then(|m| m.modified()).ok();
        let scroll = match &self.preview {
            Some(p) if p.path == row.path && p.width == width && p.modified == modified => return,
            // The same file, changed or resized, keeps where you were.
            Some(p) if p.path == row.path => p.scroll,
            _ => 0,
        };
        let text = match self.preview.take() {
            Some(p) if p.path == row.path => p.text,
            _ => Text::styled("Loading…", Style::new().fg(DIM)),
        };
        self.request += 1;
        let (n, path, sender) = (self.request, row.path.clone(), self.sender.clone());
        thread::spawn(move || {
            // If the viewer has already closed, nobody is waiting for the reply.
            let _ = sender.send((n, preview::render(&path, width)));
        });
        self.preview = Some(PreviewState { path: row.path.clone(), width, modified, text, scroll });
    }

    // --- Keyboard and mouse ---------------------------------------------------------------------

    fn key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('q') => return self.quit = true,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return self.quit = true,
            KeyCode::Char('r') => return self.refresh(),
            KeyCode::Esc if self.popup && self.focus == Focus::Tree => return self.quit = true,
            KeyCode::Tab => {
                return self.focus = match self.focus {
                    Focus::Tree if self.preview.is_some() => Focus::Preview,
                    _ => Focus::Tree,
                };
            }
            _ => {}
        }
        match self.focus {
            Focus::Tree => self.tree_key(k.code),
            Focus::Preview => self.preview_key(k.code),
        }
    }

    fn tree_key(&mut self, code: KeyCode) {
        let height = self.tree_area.height.max(1) as usize;
        let last = self.tree.rows.len().saturating_sub(1);
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(height),
            KeyCode::PageDown => self.selected = (self.selected + height).min(last),
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => self.selected = last,
            KeyCode::Enter | KeyCode::Char(' ') => self.activate(self.selected),
            KeyCode::Right | KeyCode::Char('l') => {
                let Some(row) = self.tree.rows.get(self.selected) else { return };
                match (row.is_folder, self.tree.is_open(&row.path)) {
                    (true, true) => self.selected = (self.selected + 1).min(last),
                    _ => self.activate(self.selected),
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let Some(row) = self.tree.rows.get(self.selected) else { return };
                if row.is_folder && self.tree.is_open(&row.path) {
                    let path = row.path.clone();
                    self.tree.close(&path);
                } else if let Some(parent) = self.tree.parent(self.selected) {
                    self.selected = parent;
                }
            }
            _ => return,
        }
        self.keep_visible();
    }

    fn preview_key(&mut self, code: KeyCode) {
        let height = self.preview_area.height.max(1);
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_preview(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_preview(1),
            KeyCode::PageUp => self.scroll_preview(-(height as i32)),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_preview(height as i32),
            KeyCode::Home | KeyCode::Char('g') => self.scroll_preview(i32::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.scroll_preview(i32::MAX / 2),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc | KeyCode::Backspace => self.focus = Focus::Tree,
            _ => {}
        }
    }

    /// What Enter or a click does: a folder opens or closes; a file goes to the preview.
    fn activate(&mut self, i: usize) {
        let Some(row) = self.tree.rows.get(i) else { return };
        self.selected = i;
        if row.is_folder {
            let path = row.path.clone();
            self.tree.toggle(&path);
        } else if self.tree_area.width + self.preview_area.width < DOUBLE_WIDTH {
            // When narrow the preview replaces the tree; when wide it is already visible and focus stays.
            self.focus = Focus::Preview;
        }
    }

    fn mouse(&mut self, m: MouseEvent) {
        let point = Position::new(m.column, m.row);
        let in_tree = self.tree_area.contains(point);
        let in_preview = self.preview_area.contains(point);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if in_tree => {
                self.focus = Focus::Tree;
                self.activate(self.offset + (m.row - self.tree_area.y) as usize);
            }
            MouseEventKind::Down(MouseButton::Left) if in_preview => self.focus = Focus::Preview,
            MouseEventKind::ScrollDown if in_preview => self.scroll_preview(WHEEL as i32),
            MouseEventKind::ScrollUp if in_preview => self.scroll_preview(-(WHEEL as i32)),
            // As in VS Code, the wheel scrolls the list without moving the selection.
            MouseEventKind::ScrollDown if in_tree => self.offset += WHEEL as usize,
            MouseEventKind::ScrollUp if in_tree => self.offset = self.offset.saturating_sub(WHEEL as usize),
            _ => {}
        }
    }

    fn scroll_preview(&mut self, lines: i32) {
        let height = self.preview_area.height as i32;
        if let Some(p) = self.preview.as_mut() {
            let max = (p.text.lines.len() as i32 - height).max(0);
            p.scroll = (p.scroll as i32 + lines).clamp(0, max) as u16;
        }
    }

    fn keep_visible(&mut self) {
        let height = self.tree_area.height.max(1) as usize;
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
    }

    // --- Drawing ----------------------------------------------------------------------------------

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let [header, body, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        (self.tree_area, self.preview_area) = if area.width >= DOUBLE_WIDTH {
            let [tree, preview] = Layout::horizontal([
                Constraint::Length(tree_width(area.width, self.settings.tree_width)),
                Constraint::Fill(1),
            ])
            .areas(body);
            let border = Block::new().borders(Borders::LEFT).border_style(Style::new().fg(DIM));
            let inner = border.inner(preview);
            f.render_widget(border, preview);
            (tree, inner)
        } else if self.focus == Focus::Preview {
            (Rect::default(), body)
        } else {
            (body, Rect::default())
        };

        self.draw_header(f, header);
        self.draw_tree(f);
        if let Some(p) = &self.preview {
            f.render_widget(Paragraph::new(p.text.clone()).scroll((p.scroll, 0)), self.preview_area);
        } else if self.preview_area.width > 0 {
            let hint = Paragraph::new(" Pick a file to preview it here.").style(Style::new().fg(DIM));
            f.render_widget(hint, self.preview_area);
        }
        let help = match (self.focus, area.width >= 64) {
            (Focus::Tree, true) => " ↑↓ move  ⏎ open  ←→ fold  ⇥ preview  r reload  q close",
            (Focus::Tree, false) => " ↑↓ ⏎ ←→  ⇥ preview  q close",
            (Focus::Preview, true) => " ↑↓ ⇞⇟ scroll  ← back  ⇥ tree  q close",
            (Focus::Preview, false) => " ↑↓ ⇞⇟  ← back  q close",
        };
        match &self.warning {
            Some(warning) => {
                let text = format!(" settings.toml: {warning}");
                f.render_widget(Paragraph::new(text).style(Style::new().fg(Color::Rgb(0xe0, 0x6c, 0x75))), footer);
            }
            None => f.render_widget(Paragraph::new(help).style(Style::new().fg(DIM)), footer),
        }
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let name = self.tree.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let icon = icons::for_folder(&name, true, true);
        let mut parts = vec![
            Span::styled(format!(" {} ", icon.glyph), Style::new().fg(icon.color)),
            Span::styled(name.to_uppercase(), Style::new().fg(TEXT).add_modifier(Modifier::BOLD)),
        ];
        if let Some(branch) = self.git.as_ref().and_then(|g| g.branch.as_deref()) {
            parts.push(Span::styled(format!("  {BRANCH} {branch}"), Style::new().fg(DIM)));
        }
        f.render_widget(Paragraph::new(Line::from(parts)), area);
    }

    fn draw_tree(&mut self, f: &mut Frame) {
        let area = self.tree_area;
        if area.width == 0 {
            return;
        }
        let height = area.height as usize;
        self.offset = self.offset.min(self.tree.rows.len().saturating_sub(height));
        let width = area.width as usize;
        let lines: Vec<Line> = self
            .tree
            .rows
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(height)
            .map(|(i, row)| {
                let status = self.git.as_ref().and_then(|g| g.of(&relative(&self.tree.root, &row.path), row.is_folder));
                let mut line = self.row_line(row, status, width);
                if i == self.selected {
                    let bg = if self.focus == Focus::Tree { SELECTION_BG } else { SELECTION_BG_UNFOCUSED };
                    line = line.style(Style::new().bg(bg));
                }
                line
            })
            .collect();
        f.render_widget(Paragraph::new(lines), area);
    }

    fn row_line(&self, row: &tree::Row, status: Option<Status>, width: usize) -> Line<'static> {
        let open = row.is_folder && self.tree.is_open(&row.path);
        let chevron = match (row.is_folder, open) {
            (false, _) => ' ',
            (true, false) => CHEVRON_CLOSED,
            (true, true) => CHEVRON_OPEN,
        };
        let icon = if row.is_folder { icons::for_folder(&row.name, open, false) } else { icons::for_file(&row.name) };
        // Folders with changes get a dot, files their letter, as in VS Code.
        let mark = match (status, row.is_folder) {
            (Some(s), true) if s != Status::Ignored => "●".to_string(),
            (Some(s), false) => s.letter().map(String::from).unwrap_or_default(),
            _ => String::new(),
        };
        let name_color = status.map_or(TEXT, Status::color);

        let indent = format!(" {}{chevron} ", "  ".repeat(row.depth));
        let fixed = indent.chars().count() + 2 + mark.chars().count() + 1;
        let name = truncate(&row.name, width.saturating_sub(fixed));
        let gap = width.saturating_sub(fixed + name.chars().count());
        Line::from(vec![
            Span::styled(indent, Style::new().fg(DIM)),
            Span::styled(format!("{} ", icon.glyph), Style::new().fg(icon.color)),
            Span::styled(name, Style::new().fg(name_color)),
            Span::raw(" ".repeat(gap)),
            Span::styled(format!("{mark} "), Style::new().fg(name_color)),
        ])
    }
}

/// Tree width when it fits next to the preview: the settings' percentage, but never so narrow that
/// names cannot be read nor so wide that it eats the preview.
fn tree_width(total: u16, percent: u8) -> u16 {
    (total * u16::from(percent) / 100).clamp(28, 48)
}

/// Path from the viewer's folder with «/», which is how `GitStatus::of` is asked.
fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut short: String = text.chars().take(max.saturating_sub(1)).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_width_is_bounded() {
        assert_eq!(tree_width(200, 35), 48);
        assert_eq!(tree_width(100, 35), 35);
        assert_eq!(tree_width(100, 10), 28);
    }
}
