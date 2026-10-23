//! The marketplace explorer: herdr's plugin index with search, order, «NEW» and «installed» marks,
//! the selected plugin's details (including what it runs), and install, update or uninstall with Enter.

use std::cell::Cell;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use common::settings::{Marketplace, Sort};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::cache::{self, Freshness};
use crate::index::{self, Listing};
use crate::install;
use crate::installed::{self, Installed, Manifest};
use crate::state;
use crate::text::{clean, valid_repo};
use crate::theme::{ACCENT, BACKGROUND, DIM, ERROR, OK, SELECTION_BG, TEXT, WARNING};
use crate::ui_updates::Status;

/// How long the cursor has to rest on a plugin before its manifest is fetched: scrolling through the
/// list must not fire a download per row.
const DWELL: Duration = Duration::from_millis(300);
/// A manifest that could not be read is tried again after this long, if still selected.
const RETRY_MANIFEST: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Job {
    Install,
    Update,
    Uninstall,
}

impl Job {
    fn label(self) -> &'static str {
        match self {
            Job::Install => "Install",
            Job::Update => "Update",
            Job::Uninstall => "Uninstall",
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Request {
    Run(Job, Box<Listing>),
    OpenRepo(String),
}

pub enum Mode {
    Loading,
    /// The index could not be loaded and there is no copy on disk.
    Failed(String),
    Browsing,
    Confirm {
        listing: Box<Listing>,
        choices: Vec<Job>,
        choice: usize,
    },
    Working {
        job: Job,
        name: String,
    },
    Done {
        job: Job,
        name: String,
        status: Status,
    },
}

pub struct BrowseView {
    all: Vec<Listing>,
    installed: Vec<Installed>,
    pub opts: Marketplace,
    pub query: String,
    searching: bool,
    pub cursor: usize,
    last_open: Option<String>,
    pub mode: Mode,
    pub quit: bool,
    stale: Option<SystemTime>,
    /// Manifests already fetched, by `repo/commit/path`; a failed one is tried again after a while.
    manifests: HashMap<String, Result<Manifest, String>>,
    failed_at: HashMap<String, Instant>,
    // Where things were drawn in the last frame, for the mouse and to keep the cursor in view.
    list_area: Cell<Rect>,
    scroll: Cell<usize>,
}

fn manifest_key(l: &Listing) -> String {
    format!("{}/{}/{}", l.repo, l.head_commit, l.path)
}

impl BrowseView {
    pub fn new(
        all: Vec<Listing>,
        installed: Vec<Installed>,
        opts: Marketplace,
        last_open: Option<String>,
    ) -> BrowseView {
        BrowseView {
            all,
            installed,
            opts,
            query: String::new(),
            searching: false,
            cursor: 0,
            last_open,
            mode: Mode::Browsing,
            quit: false,
            stale: None,
            manifests: HashMap::new(),
            failed_at: HashMap::new(),
            list_area: Cell::default(),
            scroll: Cell::new(0),
        }
    }

    pub fn visible(&self) -> Vec<&Listing> {
        let mut list = index::filter(&self.all, &self.query, &self.installed, &self.opts);
        index::sort(&mut list, self.opts.sort);
        list
    }

    pub fn is_new(&self, l: &Listing) -> bool {
        index::is_new(l, self.last_open.as_deref())
    }

    pub fn is_installed(&self, l: &Listing) -> bool {
        self.installed.iter().any(|p| installed::matches(p, l))
    }

    /// Whether to (re)fetch a manifest: never fetched, or failed long enough ago that a network blip
    /// should be over.
    pub fn needs_fetch(&self, key: &str, now: Instant) -> bool {
        match self.manifests.get(key) {
            None => true,
            Some(Ok(_)) => false,
            Some(Err(_)) => self.failed_at.get(key).is_none_or(|at| now.duration_since(*at) >= RETRY_MANIFEST),
        }
    }

    pub fn manifest_loaded(&mut self, key: String, manifest: Result<Manifest, String>, now: Instant) {
        if manifest.is_err() {
            self.failed_at.insert(key.clone(), now);
        } else {
            self.failed_at.remove(&key);
        }
        self.manifests.insert(key, manifest);
    }

    /// Installing runs the plugin's build: only once what it runs has been shown.
    fn commands_shown(&self, l: &Listing) -> bool {
        matches!(self.manifests.get(&manifest_key(l)), Some(Ok(_)))
    }

    fn selected(&self) -> Option<Listing> {
        self.visible().get(self.cursor).map(|l| (*l).clone())
    }

    fn move_cursor(&mut self, delta: i64) {
        let last = self.visible().len().saturating_sub(1) as i64;
        self.cursor = (self.cursor as i64 + delta).clamp(0, last) as usize;
    }

    pub fn key(&mut self, code: KeyCode) -> Option<Request> {
        match &mut self.mode {
            Mode::Loading | Mode::Failed(_) => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.quit = true;
                }
                None
            }
            Mode::Working { .. } => None,
            Mode::Done { .. } => {
                self.mode = Mode::Browsing;
                None
            }
            Mode::Confirm { listing, choices, choice } => match code {
                KeyCode::Left | KeyCode::Char('h') => {
                    *choice = choice.saturating_sub(1);
                    None
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                    *choice = (*choice + 1).min(choices.len() - 1);
                    None
                }
                KeyCode::Enter => {
                    let job = choices[*choice];
                    if job == Job::Install && !matches!(self.manifests.get(&manifest_key(listing)), Some(Ok(_))) {
                        return None; // the footer says why
                    }
                    let request = Request::Run(job, listing.clone());
                    self.mode = Mode::Browsing;
                    Some(request)
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.mode = Mode::Browsing;
                    None
                }
                KeyCode::Char('o') => Some(Request::OpenRepo(listing.repo.clone())),
                _ => None,
            },
            Mode::Browsing if self.searching => {
                match code {
                    KeyCode::Char(c) => {
                        self.query.push(c);
                        self.cursor = 0;
                    }
                    KeyCode::Backspace => {
                        self.query.pop();
                        self.cursor = 0;
                    }
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Down => self.searching = false,
                    _ => {}
                }
                None
            }
            Mode::Browsing => {
                match code {
                    KeyCode::Esc | KeyCode::Char('q') => self.quit = true,
                    KeyCode::Char('/') => self.searching = true,
                    KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
                    KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
                    KeyCode::PageUp => self.move_cursor(-10),
                    KeyCode::PageDown => self.move_cursor(10),
                    KeyCode::Home => self.cursor = 0,
                    KeyCode::End => self.move_cursor(i64::MAX / 2),
                    KeyCode::Char('s') => {
                        const SORTS: [Sort; 3] = [Sort::Newest, Sort::Popular, Sort::Active];
                        let i = SORTS.iter().position(|&s| s == self.opts.sort).unwrap_or(0);
                        self.opts.sort = SORTS[(i + 1) % SORTS.len()];
                        self.cursor = 0;
                    }
                    KeyCode::Char('o') => return self.selected().map(|l| Request::OpenRepo(l.repo)),
                    KeyCode::Enter => {
                        let l = self.selected()?;
                        if !l.runs_on_windows() {
                            return None;
                        }
                        let choices = match self.installed.iter().find(|p| installed::matches(p, &l)) {
                            None => vec![Job::Install],
                            // The index knows only the default branch's commit: if that is what is
                            // installed, there is nothing to update to.
                            Some(p) if p.git_ref.is_none() && p.commit.as_deref() == Some(l.head_commit.as_str()) => {
                                vec![Job::Uninstall]
                            }
                            Some(_) => vec![Job::Update, Job::Uninstall],
                        };
                        self.mode = Mode::Confirm { listing: Box::new(l), choices, choice: 0 };
                    }
                    _ => {}
                }
                None
            }
        }
    }
}

/// How long ago, in words, from two RFC 3339 dates (UTC); empty if either does not parse.
pub fn age(then: &str, now: &str) -> String {
    let (Some(a), Some(b)) = (day_number(then), day_number(now)) else {
        return String::new();
    };
    match b - a {
        i64::MIN..=0 => "today".into(),
        1 => "yesterday".into(),
        d @ 2..=13 => format!("{d} days ago"),
        d @ 14..=59 => format!("{} weeks ago", d / 7),
        d => format!("{} months ago", d / 30),
    }
}

/// Days since 1970-01-01 of the date part of an RFC 3339 timestamp (Howard Hinnant's algorithm).
fn day_number(rfc3339: &str) -> Option<i64> {
    let date = rfc3339.get(..10)?;
    let mut parts = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

enum Message {
    Index(Result<(Vec<Listing>, Freshness), String>),
    Manifest(String, Result<Manifest, String>),
    JobDone(Status, Option<Vec<Installed>>),
}

/// Opens the explorer. The index loads in the background so the window shows at once.
pub fn run(herdr: &str, state_dir: &Path, opts: Marketplace) -> crate::AppResult {
    let installed = installed::list(herdr).unwrap_or_else(|e| {
        eprintln!("herdr-market: {e}; nothing will show as installed");
        Vec::new()
    });
    let mut view = BrowseView::new(Vec::new(), installed, opts, state::last_open(state_dir));
    view.mode = Mode::Loading;
    let (tx, rx) = mpsc::channel();
    {
        let (tx, dir) = (tx.clone(), state_dir.to_path_buf());
        thread::spawn(move || {
            // The copy on disk first, whatever its age, so there is a list at once; then the fresh one.
            if let Some((list, at)) = cache::cached(&dir) {
                let _ = tx.send(Message::Index(Ok((list, Freshness::Stale(at)))));
            }
            let loaded = cache::load_index(&dir, SystemTime::now(), || crate::http::get(index::URL));
            let _ = tx.send(Message::Index(loaded));
        });
    }
    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;
    let result = view.event_loop(&mut terminal, herdr, &tx, &rx);
    // On exit the popup closes; if this fails there is no terminal to give the mouse back to.
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    // What was listed now is no longer new next time; if this cannot be saved, it is shown as new again.
    if let Err(e) = state::set_last_open(state_dir, SystemTime::now()) {
        eprintln!("herdr-market: {e}");
    }
    result
}

impl BrowseView {
    fn event_loop(
        &mut self,
        terminal: &mut DefaultTerminal,
        herdr: &str,
        tx: &mpsc::Sender<Message>,
        rx: &Receiver<Message>,
    ) -> crate::AppResult {
        let mut rested_since = (Instant::now(), None::<String>);
        let mut fetching: Option<String> = None;
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            while let Ok(message) = rx.try_recv() {
                match message {
                    Message::Index(Ok((list, freshness))) => {
                        self.all = list;
                        self.cursor = self.cursor.min(self.visible().len().saturating_sub(1));
                        self.stale = match freshness {
                            Freshness::Fresh => None,
                            Freshness::Stale(at) => Some(at),
                        };
                        if matches!(self.mode, Mode::Loading) {
                            self.mode = Mode::Browsing;
                        }
                    }
                    // With the old copy already on screen, a failed refresh only leaves its time showing.
                    Message::Index(Err(e)) if matches!(self.mode, Mode::Loading) => self.mode = Mode::Failed(e),
                    Message::Index(Err(_)) => {}
                    Message::Manifest(key, manifest) => {
                        self.manifest_loaded(key, manifest, Instant::now());
                        fetching = None;
                    }
                    Message::JobDone(status, installed) => {
                        if let Some(list) = installed {
                            self.installed = list;
                        }
                        if let Mode::Working { job, name } = &self.mode {
                            self.mode = Mode::Done { job: *job, name: name.clone(), status };
                        }
                    }
                }
            }

            // The selected plugin's manifest, once the cursor has rested on it; at once when confirming.
            let confirming = match &self.mode {
                Mode::Confirm { listing, .. } => Some(listing.as_ref().clone()),
                _ => None,
            };
            let selected = confirming.clone().or_else(|| self.selected()).map(|l| (manifest_key(&l), l));
            let key = selected.as_ref().map(|(k, _)| k.clone());
            if key != rested_since.1 {
                rested_since = (Instant::now(), key);
            }
            if let Some((key, l)) = selected
                && fetching.is_none()
                && self.needs_fetch(&key, Instant::now())
                && (confirming.is_some() || rested_since.0.elapsed() >= DWELL)
            {
                fetching = Some(key.clone());
                let tx = tx.clone();
                thread::spawn(move || {
                    let _ = tx.send(Message::Manifest(key, install::fetch_manifest(&l)));
                });
            }

            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let request = match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => self.key(k.code),
                Event::Mouse(m) => self.mouse(m.kind, Position::new(m.column, m.row)),
                _ => None,
            };
            match request {
                Some(Request::Run(job, l)) => self.start(herdr, job, &l, tx),
                Some(Request::OpenRepo(repo)) if valid_repo(&repo) => {
                    // `start` hands the URL to the default browser; the repo name was checked above.
                    let url = format!("https://github.com/{repo}");
                    if let Err(e) = Command::new("cmd").args(["/c", "start", "", &url]).spawn() {
                        eprintln!("herdr-market: could not open the browser: {e}");
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn start(&mut self, herdr: &str, job: Job, l: &Listing, tx: &mpsc::Sender<Message>) {
        let args = match job {
            Job::Install => match install::install_args(l) {
                Ok(args) => args,
                Err(e) => {
                    self.mode = Mode::Done { job, name: clean(&l.name), status: Status::Failed(e) };
                    return;
                }
            },
            Job::Update => install::update_args(&l.id),
            Job::Uninstall => install::uninstall_args(&l.id),
        };
        self.mode = Mode::Working { job, name: clean(&l.name) };
        let (tx, herdr) = (tx.clone(), herdr.to_string());
        thread::spawn(move || {
            let status = install::run(&herdr, &args);
            // If this fails, the marks stay as they were until the explorer is opened again.
            let installed = installed::list(&herdr).ok();
            let _ = tx.send(Message::JobDone(status, installed));
        });
    }

    fn mouse(&mut self, kind: MouseEventKind, at: Position) -> Option<Request> {
        if !matches!(self.mode, Mode::Browsing) {
            return if matches!(kind, MouseEventKind::Down(_)) && matches!(self.mode, Mode::Done { .. }) {
                self.key(KeyCode::Enter)
            } else {
                None
            };
        }
        let area = self.list_area.get();
        match kind {
            MouseEventKind::Down(MouseButton::Left) if area.contains(at) => {
                let row = self.scroll.get() + (at.y - area.y) as usize;
                if row >= self.visible().len() {
                    return None;
                }
                self.searching = false;
                // A click selects; a click on the selected one is Enter.
                if row == self.cursor {
                    return self.key(KeyCode::Enter);
                }
                self.cursor = row;
                None
            }
            MouseEventKind::ScrollDown => {
                self.move_cursor(3);
                None
            }
            MouseEventKind::ScrollUp => {
                self.move_cursor(-3);
                None
            }
            _ => None,
        }
    }

    fn draw(&self, f: &mut Frame) {
        let [header, search, list, details, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Fill(1),
            Constraint::Length(7),
            Constraint::Length(2),
        ])
        .areas(f.area());
        let visible = self.visible();

        let sort = match self.opts.sort {
            Sort::Newest => "newest",
            Sort::Popular => "most stars",
            Sort::Active => "recently active",
        };
        let mut title = vec![
            Span::styled(" MARKETPLACE", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" · {} plugins · sorted by: {sort}", visible.len()), Style::new().fg(DIM)),
        ];
        if let Some(at) = self.stale {
            let time = state::rfc3339(at);
            title.push(Span::styled(
                format!("  · list from {} UTC (could not refresh)", time.get(11..16).unwrap_or("?")),
                Style::new().fg(WARNING),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(title)).block(bottom_rule()), header);

        let search_line = if self.searching || !self.query.is_empty() {
            let cursor = if self.searching { "▏" } else { "" };
            Line::from(vec![
                Span::styled(" Search: ", Style::new().fg(DIM)),
                Span::styled(format!("{}{cursor}", self.query), Style::new().fg(WARNING)),
            ])
        } else {
            Line::styled(" / to search by name, description or author", Style::new().fg(DIM))
        };
        f.render_widget(Paragraph::new(search_line).block(bottom_rule()), search);

        match &self.mode {
            Mode::Loading => {
                f.render_widget(Paragraph::new(" Loading herdr's plugin index…").style(Style::new().fg(DIM)), list)
            }
            Mode::Failed(e) => f.render_widget(
                Paragraph::new(format!(" Could not load herdr's plugin index: {e}"))
                    .style(Style::new().fg(ERROR))
                    .wrap(Wrap { trim: false }),
                list,
            ),
            _ => self.draw_list(f, list, &visible),
        }
        self.draw_details(f, details, visible.get(self.cursor).copied());

        let (text, color) = match &self.mode {
            Mode::Confirm { listing, choices, choice }
                if choices[*choice] == Job::Install && !self.commands_shown(listing) =>
            {
                match self.manifests.get(&manifest_key(listing)) {
                    Some(Err(_)) => (
                        " Cannot show what it runs, so it cannot be installed from here · o open on GitHub · Esc"
                            .to_string(),
                        ERROR,
                    ),
                    _ => (" Reading what it runs… · Esc cancel".to_string(), DIM),
                }
            }
            Mode::Confirm { .. } => (" ←→ choose · ⏎ confirm · Esc cancel".to_string(), ACCENT),
            Mode::Working { job, name } => (
                format!(" {}ing {name}… herdr builds it; this can take a while.", job.label().trim_end_matches('e')),
                ACCENT,
            ),
            Mode::Done { job, name, status } => match status {
                Status::Failed(e) => (format!(" ✗ {} {name} failed: {e} · any key to continue", job.label()), ERROR),
                Status::UpToDate => (format!(" ✓ {name} is already up to date · any key to continue"), OK),
                _ => (format!(" ✓ {} {name}: done · any key to continue", job.label()), OK),
            },
            _ if self.searching => (" type to filter · ⏎ or Esc back to the list".into(), DIM),
            _ => (" / search · s sort · ⏎ install, update or uninstall · o open on GitHub · Esc close".into(), DIM),
        };
        f.render_widget(
            Paragraph::new(text)
                .style(Style::new().fg(color))
                .block(Block::new().borders(Borders::TOP).border_style(DIM)),
            footer,
        );
    }

    fn draw_list(&self, f: &mut Frame, area: Rect, visible: &[&Listing]) {
        self.list_area.set(area);
        let height = area.height as usize;
        // Only the rows in view are drawn: the index has close to two thousand plugins.
        let mut scroll = self.scroll.get();
        if self.cursor < scroll {
            scroll = self.cursor;
        } else if height > 0 && self.cursor >= scroll + height {
            scroll = self.cursor + 1 - height;
        }
        self.scroll.set(scroll);
        let now = state::rfc3339(SystemTime::now());
        let lines: Vec<Line> = visible
            .iter()
            .enumerate()
            .skip(scroll)
            .take(height)
            .map(|(i, l)| {
                let other_platform = !l.runs_on_windows();
                let base = if other_platform { DIM } else { TEXT };
                let new = if self.is_new(l) {
                    Span::styled(" NEW ", Style::new().fg(OK).add_modifier(Modifier::BOLD))
                } else {
                    Span::raw("     ")
                };
                let when = match self.opts.sort {
                    Sort::Newest => &l.first_seen,
                    _ => &l.pushed,
                };
                let tail = if self.is_installed(l) {
                    Span::styled("✓ installed", Style::new().fg(OK))
                } else if other_platform {
                    Span::styled("not for Windows", Style::new().fg(DIM))
                } else {
                    Span::styled(clean(&l.description), Style::new().fg(DIM))
                };
                let line = Line::from(vec![
                    new,
                    Span::styled(format!("{:<26.26} ", clean(&l.name)), Style::new().fg(base)),
                    Span::styled(format!("★{:<6} ", l.stars), Style::new().fg(WARNING)),
                    Span::styled(format!("{:<14} ", age(when, &now)), Style::new().fg(DIM)),
                    tail,
                ]);
                if i == self.cursor { line.style(Style::new().bg(SELECTION_BG)) } else { line }
            })
            .collect();
        f.render_widget(Paragraph::new(lines), area);
    }

    fn draw_details(&self, f: &mut Frame, area: Rect, selected: Option<&Listing>) {
        let block = Block::new().borders(Borders::TOP).border_style(DIM);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let mut lines = Vec::new();
        let (l, confirm) = match &self.mode {
            Mode::Confirm { listing, choices, choice } => (Some(listing.as_ref()), Some((choices, *choice))),
            _ => (selected, None),
        };
        let Some(l) = l else {
            f.render_widget(Paragraph::new(lines), inner);
            return;
        };
        let platforms =
            if l.platforms.is_empty() { "any platform".to_string() } else { clean(&l.platforms.join(", ")) };
        let herdr = l.min_herdr.as_deref().map(|v| format!(" · herdr ≥ {}", clean(v))).unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(format!(" {}", clean(&l.name)), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("  {} · v{}{herdr} · {platforms}", clean(&l.repo), clean(&l.version)),
                Style::new().fg(DIM),
            ),
        ]));
        lines.push(Line::styled(format!(" {}", clean(&l.description)), Style::new().fg(TEXT)));
        match self.manifests.get(&manifest_key(l)) {
            None => lines.push(Line::styled(" What it runs: loading…", Style::new().fg(DIM))),
            Some(Err(e)) => lines.push(Line::styled(format!(" What it runs: unknown ({e})"), Style::new().fg(ERROR))),
            Some(Ok(m)) => {
                let show = |c: &[Vec<String>]| {
                    if c.is_empty() {
                        "nothing".to_string()
                    } else {
                        c.iter().map(|c| clean(&c.join(" "))).collect::<Vec<_>>().join(" ; ")
                    }
                };
                lines.push(Line::from(vec![
                    Span::styled(" Runs on install: ", Style::new().fg(WARNING)),
                    Span::styled(show(&m.build), Style::new().fg(TEXT)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled(" At every herdr start: ", Style::new().fg(WARNING)),
                    Span::styled(show(&m.startup), Style::new().fg(TEXT)),
                ]));
            }
        }
        if let Some((choices, choice)) = confirm {
            let mut buttons = vec![Span::raw(" ")];
            for (i, job) in choices.iter().enumerate() {
                let style = if i == choice {
                    Style::new().fg(BACKGROUND).bg(ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(TEXT).bg(SELECTION_BG)
                };
                buttons.push(Span::styled(format!(" {} ", job.label()), style));
                buttons.push(Span::raw("   "));
            }
            buttons.push(Span::styled("Esc cancel", Style::new().fg(DIM)));
            // The buttons get the last line to themselves: long commands above must not push them out.
            let [text, button_line] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text);
            f.render_widget(Paragraph::new(Line::from(buttons)), button_line);
            return;
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

fn bottom_rule() -> Block<'static> {
    Block::new().borders(Borders::BOTTOM).border_style(DIM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::parse;
    use crate::installed::parse_list;
    use common::settings::Marketplace;

    fn view() -> BrowseView {
        let all = parse(include_str!("testdata/index.json")).unwrap();
        let mut installed = parse_list(include_str!("testdata/plugin-list.json")).unwrap();
        installed[0].id = "annotate".into(); // pretend Annotate is installed, from its own repo
        installed[0].repo = Some("plannotator/herdr-annotate".into());
        let opts = Marketplace { hide_other_platforms: false, ..Marketplace::default() };
        BrowseView::new(all, installed, opts, Some("2026-09-01T00:00:00Z".into()))
    }

    fn select(v: &mut BrowseView, id: &str) {
        v.cursor = v.visible().iter().position(|l| l.id == id).unwrap();
    }

    #[test]
    fn typing_filters() {
        let mut v = view();
        assert_eq!(v.visible().len(), 5);
        v.key(KeyCode::Char('/'));
        for c in "links".chars() {
            v.key(KeyCode::Char(c));
        }
        assert_eq!(v.visible().iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["annotate-links"]);
        v.key(KeyCode::Backspace);
        v.key(KeyCode::Esc); // leaves the search, keeps the filter
        assert_eq!(v.query, "link");
        assert!(!v.quit);
        v.key(KeyCode::Esc); // closes
        assert!(v.quit);
    }

    #[test]
    fn enter_on_other_platform_does_nothing() {
        let mut v = view();
        select(&mut v, "zenbu-labs.terminal-browser");
        assert_eq!(v.key(KeyCode::Enter), None);
        assert!(matches!(v.mode, Mode::Browsing));
    }

    #[test]
    fn enter_offers_install_and_installed_offers_update_or_uninstall() {
        let mut v = view();
        select(&mut v, "annotate-links");
        v.key(KeyCode::Enter);
        assert!(matches!(&v.mode, Mode::Confirm { choices, .. } if choices == &[Job::Install]));
        v.key(KeyCode::Esc);
        select(&mut v, "annotate");
        v.key(KeyCode::Enter);
        assert!(matches!(&v.mode, Mode::Confirm { choices, .. } if choices == &[Job::Update, Job::Uninstall]));
    }

    #[test]
    fn install_needs_the_manifest() {
        let mut v = view();
        select(&mut v, "annotate-links");
        v.key(KeyCode::Enter);
        // Not loaded yet: Enter does not install.
        assert_eq!(v.key(KeyCode::Enter), None);
        assert!(matches!(v.mode, Mode::Confirm { .. }));
        let key = manifest_key(&v.selected().unwrap());
        v.manifests.insert(key.clone(), Err("offline".into()));
        assert_eq!(v.key(KeyCode::Enter), None);
        v.manifests.insert(key, Ok(Manifest { version: "1".into(), build: vec![], startup: vec![] }));
        assert!(matches!(v.key(KeyCode::Enter), Some(Request::Run(Job::Install, _))));
    }

    #[test]
    fn failed_manifest_is_retried_later() {
        let mut v = view();
        let key = manifest_key(&v.selected().unwrap());
        let now = Instant::now();
        assert!(v.needs_fetch(&key, now));
        v.manifest_loaded(key.clone(), Err("offline".into()), now);
        assert!(!v.needs_fetch(&key, now + Duration::from_secs(5)));
        assert!(v.needs_fetch(&key, now + Duration::from_secs(31)));
        v.manifest_loaded(key.clone(), Ok(Manifest { version: "1".into(), build: vec![], startup: vec![] }), now);
        assert!(!v.needs_fetch(&key, now + Duration::from_secs(3600)));
    }

    #[test]
    fn update_only_when_the_index_has_a_newer_commit() {
        let mut v = view();
        select(&mut v, "annotate");
        let head = v.selected().unwrap().head_commit;
        let choices = |v: &mut BrowseView| {
            v.key(KeyCode::Enter);
            let found = match &v.mode {
                Mode::Confirm { choices, .. } => choices.clone(),
                _ => vec![],
            };
            v.key(KeyCode::Esc);
            found
        };
        v.installed[0].commit = Some(head.clone());
        assert_eq!(choices(&mut v), [Job::Uninstall]);
        v.installed[0].commit = Some("0000000".into());
        assert_eq!(choices(&mut v), [Job::Update, Job::Uninstall]);
        // Following another branch: the index only knows the default one, so it cannot tell.
        v.installed[0].commit = Some(head);
        v.installed[0].git_ref = Some("dev".into());
        assert_eq!(choices(&mut v), [Job::Update, Job::Uninstall]);
    }

    #[test]
    fn new_and_installed_marks() {
        let v = view();
        let new: Vec<_> = v.visible().iter().filter(|l| v.is_new(l)).map(|l| l.id.as_str()).collect();
        assert_eq!(new, ["herdr.harbor"]); // the only one first seen after 2026-09-01
        let all = parse(include_str!("testdata/index.json")).unwrap();
        assert!(v.is_installed(all.iter().find(|l| l.id == "annotate").unwrap()));
        assert!(!v.is_installed(all.iter().find(|l| l.id == "annotate-links").unwrap()));
    }

    #[test]
    fn s_cycles_the_order() {
        let mut v = view();
        assert_eq!(v.opts.sort, Sort::Newest);
        v.key(KeyCode::Char('s'));
        assert_eq!(v.opts.sort, Sort::Popular);
        assert_eq!(v.visible()[0].id, "zenbu-labs.terminal-browser");
    }

    #[test]
    fn age_in_words() {
        let now = "2026-10-23T12:00:00Z";
        assert_eq!(age("2026-10-23T08:00:00Z", now), "today");
        assert_eq!(age("2026-10-22T23:00:00Z", now), "yesterday");
        assert_eq!(age("2026-10-18T00:00:00Z", now), "5 days ago");
        assert_eq!(age("2026-09-01T00:00:00Z", now), "7 weeks ago");
        assert_eq!(age("2025-10-01T00:00:00Z", now), "12 months ago");
        assert_eq!(age("nonsense", now), "");
    }
}
