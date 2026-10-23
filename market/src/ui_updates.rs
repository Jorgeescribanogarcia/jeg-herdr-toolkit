//! The updates window: what the check found, all marked by default; Update runs `herdr plugin update`
//! for the marked ones, one after another; Cancel (or Esc) closes. What is left unmarked or cancelled
//! is not offered again until the plugin publishes something newer or herdr restarts.

use std::cell::Cell;
use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::install;
use crate::state;
use crate::text::clean;
use crate::theme::{ACCENT, BACKGROUND, DIM, ERROR, OK, SELECTION_BG, TEXT, WARNING};
use crate::updates::Update;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Waiting,
    Running,
    Updated,
    UpToDate,
    Failed(String),
}

pub struct Row {
    pub update: Update,
    pub marked: bool,
    pub status: Status,
}

#[derive(PartialEq)]
enum Phase {
    Choosing,
    Running,
    Done,
}

pub struct UpdatesView {
    pub rows: Vec<Row>,
    pub cursor: usize,
    phase: Phase,
    quit: bool,
    // Where things were drawn in the last frame, for the mouse.
    list_area: Cell<Rect>,
    update_button: Cell<Rect>,
    cancel_button: Cell<Rect>,
}

impl UpdatesView {
    /// `installed`: ids herdr has now. An update for a plugin removed since the check is dropped.
    pub fn new(updates: Vec<Update>, installed: &[String], mark_all: bool) -> UpdatesView {
        let rows = updates
            .into_iter()
            .filter(|u| installed.contains(&u.id))
            .map(|update| Row { update, marked: mark_all, status: Status::Waiting })
            .collect();
        UpdatesView {
            rows,
            cursor: 0,
            phase: Phase::Choosing,
            quit: false,
            list_area: Cell::default(),
            update_button: Cell::default(),
            cancel_button: Cell::default(),
        }
    }

    pub fn toggle(&mut self) {
        if let Some(row) = self.rows.get_mut(self.cursor) {
            row.marked = !row.marked;
        }
    }

    /// All marked if any was not; otherwise none.
    pub fn toggle_all(&mut self) {
        let mark = self.rows.iter().any(|r| !r.marked);
        self.rows.iter_mut().for_each(|r| r.marked = mark);
    }

    pub fn selected(&self) -> Vec<&Update> {
        self.rows.iter().filter(|r| r.marked).map(|r| &r.update).collect()
    }

    pub fn dismissed_on_cancel(&self) -> Vec<(String, String)> {
        self.rows.iter().map(|r| (r.update.id.clone(), r.update.new_commit.clone())).collect()
    }

    pub fn dismissed_on_update(&self) -> Vec<(String, String)> {
        self.rows.iter().filter(|r| !r.marked).map(|r| (r.update.id.clone(), r.update.new_commit.clone())).collect()
    }
}

/// How one `herdr plugin update` went, from its exit status and output. herdr prints the result on
/// stdout and its preview and errors on stderr, so on failure the last stderr line is the reason.
pub fn outcome(success: bool, stdout: &str, stderr: &str) -> Status {
    let last_line = |s: &str| s.lines().map(str::trim).rfind(|l| !l.is_empty()).map(clean);
    match success {
        true if stdout.contains("already up to date") => Status::UpToDate,
        true => Status::Updated,
        false => Status::Failed(
            last_line(stderr).or_else(|| last_line(stdout)).unwrap_or_else(|| "herdr gave no reason".into()),
        ),
    }
}

/// Opens the window on the updates the check left in `pending`, and removes the file when it closes.
pub fn run(herdr: &str, state_dir: &Path, pending: &Path, mark_all: bool) -> crate::AppResult {
    let updates: Vec<Update> = serde_json::from_str(&std::fs::read_to_string(pending)?)?;
    let installed = crate::installed::installed_ids(&crate::installed::list(herdr)?);
    let mut view = UpdatesView::new(updates, &installed, mark_all);
    if view.rows.is_empty() {
        std::fs::remove_file(pending)?;
        return Ok(());
    }
    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;
    let result = view.event_loop(&mut terminal, herdr, state_dir);
    // On exit the popup closes; if this fails there is no terminal to give the mouse back to.
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    std::fs::remove_file(pending)?;
    result
}

impl UpdatesView {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal, herdr: &str, state_dir: &Path) -> crate::AppResult {
        let mut progress: Option<Receiver<(usize, Status)>> = None;
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if let Some(rx) = &progress {
                while let Ok((i, status)) = rx.try_recv() {
                    self.rows[i].status = status;
                }
                if self.rows.iter().filter(|r| r.marked).all(|r| !matches!(r.status, Status::Waiting | Status::Running))
                {
                    self.phase = Phase::Done;
                    progress = None;
                }
            }
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let action = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => self.key(k.code),
                Event::Mouse(m) => self.mouse(m.kind, Position::new(m.column, m.row)),
                _ => None,
            };
            match action {
                Some(Action::Update) => {
                    state::dismiss(state_dir, &self.dismissed_on_update())?;
                    if self.selected().is_empty() {
                        self.quit = true;
                    } else {
                        self.phase = Phase::Running;
                        progress = Some(self.start(herdr));
                    }
                }
                Some(Action::Cancel) => {
                    state::dismiss(state_dir, &self.dismissed_on_cancel())?;
                    self.quit = true;
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Updates the marked plugins one after another in a thread, reporting each row's status.
    fn start(&self, herdr: &str) -> Receiver<(usize, Status)> {
        let (tx, rx) = mpsc::channel();
        let jobs: Vec<(usize, String)> =
            self.rows.iter().enumerate().filter(|(_, r)| r.marked).map(|(i, r)| (i, r.update.id.clone())).collect();
        let herdr = herdr.to_string();
        thread::spawn(move || {
            for (i, id) in jobs {
                // If the window was closed the receiver is gone; the updates still finish.
                let _ = tx.send((i, Status::Running));
                let status = install::run(&herdr, &install::update_args(&id));
                let _ = tx.send((i, status));
            }
        });
        rx
    }

    fn key(&mut self, code: KeyCode) -> Option<Action> {
        match self.phase {
            Phase::Running => None,
            Phase::Done => {
                self.quit = true;
                None
            }
            Phase::Choosing => {
                match code {
                    KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(self.rows.len() - 1),
                    KeyCode::Char(' ') => self.toggle(),
                    KeyCode::Char('a') => self.toggle_all(),
                    KeyCode::Enter => return Some(Action::Update),
                    KeyCode::Esc | KeyCode::Char('q') => return Some(Action::Cancel),
                    _ => {}
                }
                None
            }
        }
    }

    fn mouse(&mut self, kind: MouseEventKind, at: Position) -> Option<Action> {
        if self.phase == Phase::Done && matches!(kind, MouseEventKind::Down(_)) {
            self.quit = true;
            return None;
        }
        if self.phase != Phase::Choosing {
            return None;
        }
        match kind {
            MouseEventKind::Down(MouseButton::Left) if self.update_button.get().contains(at) => Some(Action::Update),
            MouseEventKind::Down(MouseButton::Left) if self.cancel_button.get().contains(at) => Some(Action::Cancel),
            MouseEventKind::Down(MouseButton::Left) if self.list_area.get().contains(at) => {
                let row = (at.y - self.list_area.get().y) as usize;
                if row < self.rows.len() {
                    self.cursor = row;
                    self.toggle();
                }
                None
            }
            MouseEventKind::ScrollDown => {
                self.cursor = (self.cursor + 1).min(self.rows.len() - 1);
                None
            }
            MouseEventKind::ScrollUp => {
                self.cursor = self.cursor.saturating_sub(1);
                None
            }
            _ => None,
        }
    }

    fn draw(&self, f: &mut Frame) {
        let [header, body, footer] =
            Layout::vertical([Constraint::Length(2), Constraint::Fill(1), Constraint::Length(2)]).areas(f.area());
        let title = Line::from(vec![
            Span::styled(" UPDATES", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" · {} available", self.rows.len()), Style::new().fg(DIM)),
        ]);
        f.render_widget(Paragraph::new(title).block(Block::new().borders(Borders::BOTTOM).border_style(DIM)), header);

        let body = body.inner(ratatui::layout::Margin::new(1, 1));
        let [list, details, buttons] =
            Layout::vertical([Constraint::Length(self.rows.len() as u16), Constraint::Fill(1), Constraint::Length(1)])
                .areas(body);
        self.list_area.set(list);

        let name_width = self.rows.iter().map(|r| r.update.name.chars().count()).max().unwrap_or(0).min(30);
        let lines: Vec<Line> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let u = &r.update;
                let mark = if r.marked { "[x]" } else { "[ ]" };
                let mut spans = vec![
                    Span::styled(format!("{mark} "), Style::new().fg(if r.marked { ACCENT } else { DIM })),
                    Span::styled(format!("{:<name_width$}  ", clean(&u.name)), Style::new().fg(TEXT)),
                    Span::styled(format!("{} → ", clean(&u.from)), Style::new().fg(DIM)),
                    Span::styled(format!("{:<10}", clean(&u.to)), Style::new().fg(OK)),
                ];
                if u.runs_changed {
                    spans.push(Span::styled(" ⚠", Style::new().fg(WARNING)));
                }
                // Once updating, the result takes the repo's place: in a narrow window it must still show.
                let (text, color) = match &r.status {
                    Status::Waiting => (format!("  {}", clean(&u.repo)), DIM),
                    Status::Running => ("  updating…".into(), ACCENT),
                    Status::Updated => ("  ✓ updated".into(), OK),
                    Status::UpToDate => ("  ✓ already up to date".into(), OK),
                    Status::Failed(e) => (format!("  ✗ {e}"), ERROR),
                };
                spans.push(Span::styled(text, Style::new().fg(color)));
                let line = Line::from(spans);
                if i == self.cursor && self.phase == Phase::Choosing {
                    line.style(Style::new().bg(SELECTION_BG))
                } else {
                    line
                }
            })
            .collect();
        f.render_widget(Paragraph::new(lines), list);

        // What the selected update will run, if it differs from what is installed.
        if let Some(r) = self.rows.get(self.cursor).filter(|r| r.update.runs_changed) {
            let mut lines =
                vec![Line::raw(""), Line::styled("⚠ This version runs something different:", Style::new().fg(WARNING))];
            for (label, commands) in [("on update", &r.update.new_build), ("at startup", &r.update.new_startup)] {
                for c in commands {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {label}: "), Style::new().fg(DIM)),
                        Span::styled(clean(&c.join(" ")), Style::new().fg(TEXT)),
                    ]));
                }
            }
            f.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }), details);
        }

        if self.phase == Phase::Choosing {
            let update = format!(" Update ({}) ", self.selected().len());
            let cancel = " Cancel ";
            let width = (update.chars().count() + 3 + cancel.len()) as u16;
            let x = buttons.x + buttons.width.saturating_sub(width) / 2;
            let update_area = Rect::new(x, buttons.y, update.chars().count() as u16, 1);
            let cancel_area = Rect::new(x + update_area.width + 3, buttons.y, cancel.len() as u16, 1);
            self.update_button.set(update_area);
            self.cancel_button.set(cancel_area);
            f.render_widget(
                Paragraph::new(update).style(Style::new().fg(BACKGROUND).bg(ACCENT).add_modifier(Modifier::BOLD)),
                update_area,
            );
            f.render_widget(Paragraph::new(cancel).style(Style::new().fg(TEXT).bg(SELECTION_BG)), cancel_area);
        }

        let text = match self.phase {
            Phase::Choosing => " ↑↓ move · Space mark · a all/none · ⏎ update · Esc cancel",
            Phase::Running => " Updating… herdr builds each plugin; this can take a while.",
            Phase::Done => " Done. Press any key to close.",
        };
        f.render_widget(
            Paragraph::new(text)
                .style(Style::new().fg(DIM))
                .block(Block::new().borders(Borders::TOP).border_style(DIM)),
            footer,
        );
    }
}

enum Action {
    Update,
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(id: &str) -> Update {
        Update {
            id: id.into(),
            name: id.into(),
            repo: format!("someone/{id}"),
            from: "1.0.0".into(),
            to: "1.1.0".into(),
            new_commit: format!("{id}-commit"),
            runs_changed: false,
            new_build: vec![],
            new_startup: vec![],
        }
    }

    fn view(mark_all: bool) -> UpdatesView {
        let installed = ["a", "b", "c"].map(String::from);
        UpdatesView::new(vec![update("a"), update("b"), update("c")], &installed, mark_all)
    }

    fn ids(list: &[(String, String)]) -> Vec<&str> {
        list.iter().map(|(id, _)| id.as_str()).collect()
    }

    #[test]
    fn all_marked_by_default() {
        let v = view(true);
        assert_eq!(v.selected().len(), 3);
    }

    #[test]
    fn mark_all_false_starts_empty() {
        let mut v = view(false);
        assert!(v.selected().is_empty());
        v.toggle_all();
        assert_eq!(v.selected().len(), 3);
        v.toggle_all();
        assert!(v.selected().is_empty());
    }

    #[test]
    fn cancel_dismisses_everything() {
        let v = view(true);
        assert_eq!(ids(&v.dismissed_on_cancel()), ["a", "b", "c"]);
        assert_eq!(v.dismissed_on_cancel()[0].1, "a-commit");
    }

    #[test]
    fn update_dismisses_only_unmarked() {
        let mut v = view(true);
        v.cursor = 1;
        v.toggle();
        assert_eq!(v.selected().iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), ["a", "c"]);
        assert_eq!(ids(&v.dismissed_on_update()), ["b"]);
    }

    #[test]
    fn drops_rows_no_longer_installed() {
        let installed = ["a".to_string()];
        let v = UpdatesView::new(vec![update("a"), update("gone")], &installed, true);
        assert_eq!(v.rows.len(), 1);
        assert_eq!(v.rows[0].update.id, "a");
    }

    #[test]
    fn result_of_an_update() {
        assert_eq!(outcome(true, "Updated x from a/b.\n", ""), Status::Updated);
        assert_eq!(outcome(true, "x is already up to date.\n", ""), Status::UpToDate);
        assert_eq!(
            outcome(false, "", "Plugin update preview:\n  id: x\nbuild failed: cargo exited with 101\n"),
            Status::Failed("build failed: cargo exited with 101".into())
        );
        assert_eq!(outcome(false, "", ""), Status::Failed("herdr gave no reason".into()));
    }
}
