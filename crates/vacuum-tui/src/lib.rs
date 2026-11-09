// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! # vacuum-tui
//!
//! The interactive [ratatui](https://ratatui.rs/) frontend for Vacuum: a
//! checkbox list of reclaimable candidates grouped by category, with a running
//! selection total, dry-run-by-intent confirmation, and the Steelbore theme.
//!
//! Keys: `↑/↓` or `j/k` move · `space` toggle · `a` toggle all · `y` copy path
//! · `r` choose roots · `p` purge mode · `enter` apply (with confirm) ·
//! `q`/`Esc` quit.
//!
//! Scanning and deleting run on a background thread and show a live progress
//! overlay (an animated spinner plus, for deletion, a determinate bar). While a
//! job runs, `esc` cancels a deletion (after the current item) or quits during a
//! scan (the scan is read-only, so its thread is safely abandoned).
//!
//! The set of scanned roots is supplied by the caller (the `vacuum tui` command
//! resolves it from the command line, `VACUUM_ROOTS`, the config file, or
//! `$HOME`) and can be changed at runtime through the `r` directory picker. The
//! same roots bound every deletion via [`vacuum_core::safety`], so the picker
//! cannot widen the deletion boundary past what the CLI would allow.
//!
//! Modern terminal protocols (kitty keyboard, OSC 52 copy, OSC 9;4 progress)
//! light up where the terminal supports them and stay silent elsewhere; see
//! [`protocols`]. In accessible mode (Standard §18) the spinner is replaced by
//! a static line with a text percentage.
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

mod probe;
pub mod protocols;
mod session;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use vacuum_cleaners::all_cleaners;
use vacuum_core::{
    Action, Candidate, Category, DeleteMode, Deleter, ScanContext, Target, human_bytes, total_bytes,
};
use vacuum_theme::Steelbore;

use crate::protocols::ProgressState;
pub use crate::protocols::{Protocol, ProtocolOverrides, ProtocolReport};
use crate::session::TermSession;

/// How long to wait for input before redrawing while idle.
const TICK: Duration = Duration::from_millis(250);

/// Redraw cadence while a background job runs: ~12 fps so the spinner animates
/// smoothly. Used only during a job; idle redraws use [`TICK`] to stay cheap.
const ACTIVE_TICK: Duration = Duration::from_millis(80);

/// Braille spinner frames, advanced once per [`ACTIVE_TICK`] while a job runs.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The idle status line shown once a scan completes.
const READY: &str = "Select items to reclaim, then press enter. Press r to choose roots.";

/// How long the startup probe waits for the terminal's DA1 fence before
/// treating every unanswered feature as unsupported. Local terminals answer
/// in a few milliseconds; 200 ms leaves room for SSH.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// How the TUI should present itself; resolved once by the caller.
#[derive(Debug, Clone)]
pub struct TuiOptions {
    /// Standard §18 accessible mode: no animation, progress also as text.
    pub accessible: bool,
    /// An AI agent or CI is driving the process: emit no protocol sequences.
    pub agent: bool,
    /// The user's `--term-protocols` / `SPACECRAFT_TERM_PROTOCOLS` overrides.
    pub protocols: ProtocolOverrides,
    /// Deadline for the startup capability probe.
    pub probe_timeout: Duration,
}

impl Default for TuiOptions {
    fn default() -> Self {
        Self {
            accessible: false,
            agent: false,
            protocols: ProtocolOverrides::default(),
            probe_timeout: DEFAULT_PROBE_TIMEOUT,
        }
    }
}

/// Launch the interactive TUI over `roots`, restoring the terminal on exit.
///
/// `roots` bounds both the scan and every deletion performed from the frontend.
/// Returns the terminal protocols that were in force, for `--verbose`.
///
/// # Errors
///
/// Returns an error if terminal I/O fails. Scan failures are non-fatal and are
/// surfaced in the UI rather than aborting.
pub fn run(roots: Vec<PathBuf>, options: &TuiOptions) -> anyhow::Result<ProtocolReport> {
    let mut terminal = ratatui::init();
    // After `init` (raw mode on, so the probe's replies are not echoed) and
    // before the first `event::poll`, so crossterm never sees the replies.
    let term = TermSession::start(options);
    let report = term.report().clone();
    let mut app = App::new(roots, term, options.accessible);
    let result = app.event_loop(&mut terminal);
    // Protocol modes first (the keyboard stack lives on the alternate screen),
    // then ratatui leaves the alternate screen and raw mode.
    app.term.restore();
    drop(app);
    ratatui::restore();
    result.map(|()| report)
}

/// A rendered row: a category header or a selectable candidate.
enum Row {
    Header { title: String, total: u64 },
    Item { index: usize },
}

/// One row in the directory picker overlay.
enum Entry {
    /// Ascend to the parent of the browsed directory.
    Parent(PathBuf),
    /// A sub-directory that can be toggled as a scan root or descended into.
    Dir(PathBuf),
}

/// Filesystem directory picker for choosing scan roots at runtime.
struct Picker {
    /// The directory currently being browsed.
    cwd: PathBuf,
    /// A parent (`..`) row when one exists, then the sorted sub-directories.
    entries: Vec<Entry>,
    /// The highlighted entry.
    cursor: usize,
}

impl Picker {
    /// Open a picker browsing `start`.
    fn open(start: PathBuf) -> Self {
        let entries = list_entries(&start);
        Self {
            cwd: start,
            entries,
            cursor: 0,
        }
    }

    /// Re-read `cwd` into `entries`, keeping the cursor in range.
    fn refresh(&mut self) {
        self.entries = list_entries(&self.cwd);
        if self.cursor >= self.entries.len() {
            self.cursor = self.entries.len().saturating_sub(1);
        }
    }

    fn down(&mut self) {
        if !self.entries.is_empty() {
            self.cursor = (self.cursor + 1).min(self.entries.len() - 1);
        }
    }

    fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// The highlighted entry, if any.
    fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }
}

/// Which background operation a [`Job`] is running, for labeling the overlay.
#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Scan,
    Delete,
}

/// A message streamed from a background worker to the UI thread.
enum Progress {
    /// Scanning moved on to a category: cleaner `index` of `total`.
    Scanning {
        label: String,
        index: usize,
        total: usize,
    },
    /// Scan finished; install these results.
    Scanned {
        candidates: Vec<Candidate>,
        rows: Vec<Row>,
        /// The scan roots widened by the cleaners' `extra_roots`, bounding every
        /// deletion made from this scan.
        delete_roots: Vec<PathBuf>,
    },
    /// About to act on a delete item: `done` already removed of `total`.
    Deleting {
        label: String,
        done: usize,
        total: usize,
        reclaimed: u64,
    },
    /// Deletion finished (or was cancelled).
    Deleted {
        count: usize,
        reclaimed: u64,
        cancelled: bool,
    },
    /// A non-fatal note (a cleaner failed, an item was skipped, sudo needed).
    Note(String),
}

/// An in-flight background operation that feeds the progress overlay.
struct Job {
    kind: JobKind,
    rx: Receiver<Progress>,
    /// Cooperative cancel flag the worker checks between units of work.
    cancel: Arc<AtomicBool>,
    /// Spinner animation frame.
    frame: usize,
    /// Current phase label (category being scanned, or item being deleted).
    phase: String,
    /// Items finished of the total: cleaners for a scan, selected items for a
    /// delete.
    done: usize,
    total: usize,
    /// Delete progress: bytes reclaimed so far, and the selected total.
    reclaimed: u64,
    total_bytes: u64,
    /// The most recent non-fatal note, shown in the overlay.
    note: Option<String>,
    /// Status line to set when this scan completes (`None` keeps the current
    /// one, e.g. so a delete summary survives its automatic rescan).
    post_status: Option<String>,
}

/// TUI application state.
struct App {
    roots: Vec<PathBuf>,
    /// The scan roots widened by the cleaners' `extra_roots`, as reported by the
    /// most recent scan. This — not [`App::roots`] — bounds deletions, so a
    /// temp-file candidate under `/tmp` can be acted on while every other guard
    /// stays in force.
    delete_roots: Vec<PathBuf>,
    candidates: Vec<Candidate>,
    selected: Vec<bool>,
    rows: Vec<Row>,
    cursor: usize,
    purge: bool,
    confirming: bool,
    picker: Option<Picker>,
    job: Option<Job>,
    status: String,
    /// Terminal protocols in force (clipboard, progress, restore).
    term: TermSession,
    /// Standard §18 accessible mode: static progress text, no spinner.
    accessible: bool,
}

impl App {
    fn new(roots: Vec<PathBuf>, term: TermSession, accessible: bool) -> Self {
        let mut app = Self {
            delete_roots: roots.clone(),
            roots,
            candidates: Vec::new(),
            selected: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            purge: false,
            confirming: false,
            picker: None,
            job: None,
            status: "Scanning…".to_owned(),
            term,
            accessible,
        };
        app.start_scan(Some(READY.to_owned()));
        app
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        loop {
            if self.term.should_exit() {
                return Ok(());
            }
            self.pump_job();
            let progress = self.job.as_ref().map(Job::progress_state);
            self.term.progress(progress);
            terminal.draw(|frame| self.draw(frame))?;
            // The fast tick only animates the spinner, which accessible mode
            // does not draw.
            let timeout = if self.job.is_some() && !self.accessible {
                ACTIVE_TICK
            } else {
                TICK
            };
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press && self.handle_key(key.code) {
                        return Ok(());
                    }
                }
            }
            if let Some(job) = self.job.as_mut() {
                job.frame = job.frame.wrapping_add(1);
            }
        }
    }

    /// Drain progress from the active job, updating state and, on completion,
    /// installing results. A finished delete chains an automatic rescan.
    fn pump_job(&mut self) {
        /// What the drain loop decided once the worker signalled an end.
        enum End {
            Running,
            /// Scan results, the delete roots they need, and the status to set
            /// afterward.
            Scanned(Vec<Candidate>, Vec<Row>, Vec<PathBuf>, Option<String>),
            /// A delete summary to show; triggers a rescan.
            Deleted(String),
            /// The worker vanished without a final message; just stop.
            Aborted,
        }

        let mut outcome = End::Running;
        if let Some(job) = self.job.as_mut() {
            loop {
                match job.rx.try_recv() {
                    Ok(Progress::Scanning {
                        label,
                        index,
                        total,
                    }) => {
                        job.phase = label;
                        job.done = index;
                        job.total = total;
                    }
                    Ok(Progress::Deleting {
                        label,
                        done,
                        total,
                        reclaimed,
                    }) => {
                        job.phase = label;
                        job.done = done;
                        job.total = total;
                        job.reclaimed = reclaimed;
                    }
                    Ok(Progress::Note(note)) => job.note = Some(note),
                    Ok(Progress::Scanned {
                        candidates,
                        rows,
                        delete_roots,
                    }) => {
                        outcome =
                            End::Scanned(candidates, rows, delete_roots, job.post_status.take());
                        break;
                    }
                    Ok(Progress::Deleted {
                        count,
                        reclaimed,
                        cancelled,
                    }) => {
                        let verb = if cancelled {
                            "Cancelled — removed"
                        } else {
                            "Reclaimed"
                        };
                        let mut summary =
                            format!("{verb} {count} item(s), freed ~{}.", human_bytes(reclaimed));
                        if let Some(note) = &job.note {
                            summary = format!("{summary} ({note})");
                        }
                        outcome = End::Deleted(summary);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        outcome = End::Aborted;
                        break;
                    }
                }
            }
        }

        match outcome {
            End::Running => {}
            End::Scanned(candidates, rows, delete_roots, post_status) => {
                self.job = None;
                self.install_scan(candidates, rows, delete_roots);
                if let Some(status) = post_status {
                    self.status = status;
                }
            }
            End::Deleted(summary) => {
                self.job = None;
                self.status = summary;
                // Reflect what is now gone; keep the summary through the rescan.
                self.start_scan(None);
            }
            End::Aborted => {
                self.job = None;
                self.status = "Operation ended.".to_owned();
            }
        }
    }

    /// Install scan results, clearing the previous selection.
    fn install_scan(
        &mut self,
        candidates: Vec<Candidate>,
        rows: Vec<Row>,
        delete_roots: Vec<PathBuf>,
    ) {
        self.selected = vec![false; candidates.len()];
        self.candidates = candidates;
        self.rows = rows;
        self.delete_roots = delete_roots;
        self.cursor = first_item_row(&self.rows);
    }

    /// Start a background scan of the active roots. `ready_status` is shown when
    /// it completes (`None` leaves the current status untouched).
    fn start_scan(&mut self, ready_status: Option<String>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let rx = spawn_scan(self.roots.clone(), Arc::clone(&cancel));
        self.job = Some(Job {
            kind: JobKind::Scan,
            rx,
            cancel,
            frame: 0,
            phase: "Scanning…".to_owned(),
            done: 0,
            total: 0,
            reclaimed: 0,
            total_bytes: 0,
            note: None,
            post_status: ready_status,
        });
    }

    /// Start a background deletion of the currently selected candidates.
    fn start_delete(&mut self) {
        let items: Vec<Candidate> = self
            .candidates
            .iter()
            .zip(&self.selected)
            .filter(|&(_, &selected)| selected)
            .map(|(candidate, _)| candidate.clone())
            .collect();
        let total = items.len();
        let total_bytes = total_bytes(&items);
        let mode = if self.purge {
            DeleteMode::Purge
        } else {
            DeleteMode::Trash
        };
        let verb = if self.purge { "Removing" } else { "Trashing" };
        let deleter = Deleter::new(mode, false, self.delete_roots.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let rx = spawn_delete(items, deleter, verb, Arc::clone(&cancel));
        self.job = Some(Job {
            kind: JobKind::Delete,
            rx,
            cancel,
            frame: 0,
            phase: "Starting…".to_owned(),
            done: 0,
            total,
            reclaimed: 0,
            total_bytes,
            note: None,
            post_status: None,
        });
    }

    /// Handle a key press; returns `true` to quit.
    fn handle_key(&mut self, code: KeyCode) -> bool {
        // While a background job runs it is modal: only cancel/quit is accepted.
        if let Some(kind) = self.job.as_ref().map(|job| job.kind) {
            if matches!(code, KeyCode::Esc | KeyCode::Char('q')) {
                match kind {
                    JobKind::Delete => {
                        if let Some(job) = self.job.as_ref() {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                        self.status = "Cancelling after the current item…".to_owned();
                    }
                    // A scan is read-only; quitting safely abandons its thread.
                    JobKind::Scan => return true,
                }
            }
            return false;
        }

        // The picker and the confirm prompt are modal too, mirroring each other.
        if self.picker.is_some() {
            self.handle_picker_key(code);
            return false;
        }

        if self.confirming {
            if code == KeyCode::Char('y') {
                self.start_delete();
            } else {
                self.status = "Cancelled.".to_owned();
            }
            self.confirming = false;
            return false;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Down | KeyCode::Char('j') => self.cursor_down(),
            KeyCode::Up | KeyCode::Char('k') => self.cursor_up(),
            KeyCode::Char(' ') => self.toggle_current(),
            KeyCode::Char('a') => self.toggle_all(),
            KeyCode::Char('y') => {
                if let Some(Row::Item { index }) = self.rows.get(self.cursor) {
                    let text = copy_text(&self.candidates[*index]);
                    self.copy(&text);
                }
            }
            KeyCode::Char('r') => self.open_picker(),
            KeyCode::Char('p') => {
                self.purge = !self.purge;
                self.status = if self.purge {
                    "Purge mode: deletions are PERMANENT.".to_owned()
                } else {
                    "Trash mode: deletions are recoverable.".to_owned()
                };
            }
            KeyCode::Enter => {
                if self.selected_count() == 0 {
                    self.status = "Nothing selected.".to_owned();
                } else {
                    self.confirming = true;
                }
            }
            _ => {}
        }
        false
    }

    /// Copy `text` with OSC 52 and say so. The text is always shown too: a
    /// terminal without OSC 52 ignores the sequence silently, so the user can
    /// still copy it by hand.
    fn copy(&mut self, text: &str) {
        self.status = match self.term.copy(text) {
            Ok(()) => format!("[OK] Copied ({} chars): {text}", text.chars().count()),
            Err(reason) => format!("[INFO] Not copied ({reason}): {text}"),
        };
    }

    /// Open the directory picker, starting at the current working directory.
    fn open_picker(&mut self) {
        let start = std::env::current_dir()
            .or_else(|_| std::env::var("HOME").map(PathBuf::from))
            .unwrap_or_else(|_| PathBuf::from("/"));
        self.picker = Some(Picker::open(start));
        self.status = "Choose roots: space adds/removes, esc applies.".to_owned();
    }

    /// Route a key press while the directory picker is open.
    fn handle_picker_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.down();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.up();
                }
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.picker_descend(),
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => self.picker_ascend(),
            KeyCode::Char(' ') => self.picker_toggle_root(),
            KeyCode::Char('d' | 'x') => self.picker_remove_root(),
            KeyCode::Char('y') => {
                if let Some(Entry::Parent(path) | Entry::Dir(path)) =
                    self.picker.as_ref().and_then(Picker::current)
                {
                    match path.to_str() {
                        Some(text) => {
                            let text = text.to_owned();
                            self.copy(&text);
                        }
                        None => {
                            self.status = "[WARN] Path is not valid UTF-8; not copied.".to_owned();
                        }
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.picker = None;
                self.start_scan(Some(READY.to_owned()));
            }
            _ => {}
        }
    }

    /// Descend into the highlighted directory (or ascend through `..`).
    fn picker_descend(&mut self) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        let target = match picker.current() {
            Some(Entry::Parent(path) | Entry::Dir(path)) => path.clone(),
            None => return,
        };
        picker.cwd = target;
        picker.cursor = 0;
        picker.refresh();
    }

    /// Browse the parent of the current directory.
    fn picker_ascend(&mut self) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        if let Some(parent) = picker.cwd.parent() {
            picker.cwd = parent.to_path_buf();
            picker.cursor = 0;
            picker.refresh();
        }
    }

    /// The highlighted directory, ignoring the `..` parent row.
    fn picker_current_dir(&self) -> Option<PathBuf> {
        match self.picker.as_ref()?.current()? {
            Entry::Dir(path) => Some(path.clone()),
            Entry::Parent(_) => None,
        }
    }

    /// Toggle the highlighted directory in the active root set.
    fn picker_toggle_root(&mut self) {
        let Some(path) = self.picker_current_dir() else {
            return;
        };
        toggle_root(&mut self.roots, &path);
    }

    /// Remove the highlighted directory from the active root set, if present.
    fn picker_remove_root(&mut self) {
        let Some(path) = self.picker_current_dir() else {
            return;
        };
        let key = canonical_key(&path);
        self.roots.retain(|root| canonical_key(root) != key);
    }

    /// Whether `path` is one of the active scan roots.
    fn is_root(&self, path: &Path) -> bool {
        let key = canonical_key(path);
        self.roots.iter().any(|root| canonical_key(root) == key)
    }

    fn cursor_down(&mut self) {
        if !self.rows.is_empty() {
            self.cursor = (self.cursor + 1).min(self.rows.len() - 1);
        }
    }

    fn cursor_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn toggle_current(&mut self) {
        if let Some(Row::Item { index }) = self.rows.get(self.cursor) {
            let index = *index;
            self.selected[index] = !self.selected[index];
        }
    }

    fn toggle_all(&mut self) {
        let select = !self.selected.iter().all(|&s| s);
        self.selected.fill(select);
    }

    fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    fn selected_bytes(&self) -> u64 {
        self.candidates
            .iter()
            .zip(&self.selected)
            .filter(|&(_, &s)| s)
            .map(|(c, _)| c.bytes)
            .sum()
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let base = Style::new()
            .bg(Steelbore::BACKGROUND)
            .fg(Steelbore::FOREGROUND);
        frame.render_widget(Block::new().style(base), area);

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(5),
        ])
        .split(area);

        let title =
            Paragraph::new("Vacuum — disk-space recovery").style(base.fg(Steelbore::ACCENT).bold());
        frame.render_widget(title, chunks[0]);

        let items: Vec<ListItem> = self
            .rows
            .iter()
            .map(|row| self.row_item(row, base))
            .collect();
        let list = List::new(items).style(base).highlight_style(
            Style::new()
                .bg(Steelbore::FOREGROUND)
                .fg(Steelbore::BACKGROUND),
        );
        let mut state = ListState::default();
        state.select(Some(self.cursor));
        frame.render_stateful_widget(list, chunks[1], &mut state);

        frame.render_widget(self.footer(base), chunks[2]);

        // A running job takes the screen; the picker cannot be open during one.
        if let Some(job) = self.job.as_ref() {
            Self::draw_progress(frame, job, base, self.accessible);
        } else if let Some(picker) = self.picker.as_ref() {
            self.draw_picker(frame, picker, base);
        }
    }

    fn row_item(&self, row: &Row, base: Style) -> ListItem<'static> {
        match row {
            Row::Header { title, total } => {
                ListItem::new(Line::from(format!("▸ {title}  ({})", human_bytes(*total))))
                    .style(base.fg(Steelbore::ACCENT).bold())
            }
            Row::Item { index } => {
                let candidate = &self.candidates[*index];
                let mark = if self.selected[*index] { "[x]" } else { "[ ]" };
                let size = if candidate.bytes == 0 {
                    "unknown".to_owned()
                } else {
                    human_bytes(candidate.bytes)
                };
                let style = if self.selected[*index] {
                    base.fg(Steelbore::SUCCESS)
                } else {
                    base
                };
                ListItem::new(Line::from(format!(
                    "  {mark} {size:>10}  {}",
                    candidate.label
                )))
                .style(style)
            }
        }
    }

    fn footer(&self, base: Style) -> Paragraph<'static> {
        let mode = if self.purge {
            Line::from("Mode: PURGE (permanent)").style(base.fg(Steelbore::ERROR).bold())
        } else {
            Line::from("Mode: TRASH (recoverable)").style(base.fg(Steelbore::INFO))
        };
        let selection = Line::from(format!(
            "Selected: {} item(s), ~{}",
            self.selected_count(),
            human_bytes(self.selected_bytes())
        ));
        let keys = Line::from(
            "↑/↓ move · space toggle · a all · y copy · r roots · p purge · enter apply · q quit",
        )
        .style(base.fg(Steelbore::ACCENT));
        let status = if self.confirming {
            let verb = if self.purge {
                "permanently delete"
            } else {
                "trash"
            };
            Line::from(format!(
                "Confirm: press y to {verb} {} selected item(s), any other key to cancel",
                self.selected_count()
            ))
            .style(base.fg(Steelbore::ERROR).bold())
        } else {
            Line::from(self.status.clone()).style(base.fg(Steelbore::FOREGROUND))
        };

        Paragraph::new(vec![mode, selection, keys, status]).style(base)
    }

    /// Render the progress overlay for the active background job.
    fn draw_progress(frame: &mut Frame, job: &Job, base: Style, accessible: bool) {
        // 60% wide, 30% tall: a compact band centered over the list.
        let area = centered_rect(60, 30, frame.area());
        frame.render_widget(Clear, area);

        let title = match job.kind {
            JobKind::Scan => " Scanning ",
            JobKind::Delete => " Working ",
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .title(title)
            .style(base.fg(Steelbore::ACCENT));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

        // Accessible mode: no spinner (screen readers pronounce the glyphs);
        // a static line with a monotonic percentage instead.
        let phase = if accessible {
            match job.progress_state() {
                ProgressState::Normal(percent) => format!("Working… {percent}%  {}", job.phase),
                ProgressState::Indeterminate => format!("Working…  {}", job.phase),
            }
        } else {
            format!("{}  {}", SPINNER[job.frame % SPINNER.len()], job.phase)
        };
        frame.render_widget(
            Paragraph::new(phase).style(base.fg(Steelbore::INFO)),
            chunks[0],
        );

        match job.kind {
            JobKind::Delete => {
                let stats = format!(
                    "[{}/{}]  freed {} of {}",
                    job.done,
                    job.total,
                    human_bytes(job.reclaimed),
                    human_bytes(job.total_bytes)
                );
                frame.render_widget(Paragraph::new(stats).style(base), chunks[1]);

                let ratio = progress_ratio(job.done, job.total, job.reclaimed, job.total_bytes);
                let gauge = Gauge::default()
                    .gauge_style(
                        Style::new()
                            .fg(Steelbore::SUCCESS)
                            .bg(Steelbore::BACKGROUND),
                    )
                    .ratio(ratio);
                frame.render_widget(gauge, chunks[2]);

                frame.render_widget(
                    Paragraph::new("esc to cancel").style(base.fg(Steelbore::ACCENT)),
                    chunks[3],
                );
            }
            JobKind::Scan => {
                if let Some(note) = job.note.as_ref() {
                    frame.render_widget(
                        Paragraph::new(note.clone()).style(base.fg(Steelbore::ERROR)),
                        chunks[1],
                    );
                }
                frame.render_widget(
                    Paragraph::new("esc to quit").style(base.fg(Steelbore::ACCENT)),
                    chunks[3],
                );
            }
        }
    }

    /// Render the directory-picker overlay centered over the main screen.
    fn draw_picker(&self, frame: &mut Frame, picker: &Picker, base: Style) {
        // 70% of each axis: large enough to browse, small enough to keep the
        // category list visible behind it for context.
        let area = centered_rect(70, 70, frame.area());
        frame.render_widget(Clear, area);

        let active = if self.roots.is_empty() {
            "no roots".to_owned()
        } else {
            format!("{} root(s)", self.roots.len())
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .title(format!(
                " Pick roots — {active} — {} ",
                picker.cwd.display()
            ))
            .style(base.fg(Steelbore::ACCENT));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

        let items: Vec<ListItem> = picker
            .entries
            .iter()
            .map(|entry| self.picker_item(entry, base))
            .collect();
        let list = List::new(items).style(base).highlight_style(
            Style::new()
                .bg(Steelbore::FOREGROUND)
                .fg(Steelbore::BACKGROUND),
        );
        let mut state = ListState::default();
        state.select(Some(picker.cursor));
        frame.render_stateful_widget(list, chunks[0], &mut state);

        let hint = Paragraph::new(
            "↑/↓ move · enter/l open · h/⌫ up · space add/remove · d remove · esc apply",
        )
        .style(base.fg(Steelbore::ACCENT));
        frame.render_widget(hint, chunks[1]);
    }

    fn picker_item(&self, entry: &Entry, base: Style) -> ListItem<'static> {
        match entry {
            Entry::Parent(_) => ListItem::new(Line::from("  ..")).style(base.fg(Steelbore::ACCENT)),
            Entry::Dir(path) => {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("?");
                let selected = self.is_root(path);
                let mark = if selected { "[✓]" } else { "[ ]" };
                let style = if selected {
                    base.fg(Steelbore::SUCCESS)
                } else {
                    base
                };
                ListItem::new(Line::from(format!("  {mark} {name}/"))).style(style)
            }
        }
    }
}

impl Job {
    /// The job's progress as a whole percentage, or indeterminate before the
    /// worker has reported a total.
    fn progress_state(&self) -> ProgressState {
        match self.kind {
            JobKind::Delete if self.total > 0 || self.total_bytes > 0 => {
                ProgressState::Normal(percent(progress_ratio(
                    self.done,
                    self.total,
                    self.reclaimed,
                    self.total_bytes,
                )))
            }
            JobKind::Scan if self.total > 0 => {
                ProgressState::Normal(percent(progress_ratio(self.done, self.total, 0, 0)))
            }
            _ => ProgressState::Indeterminate,
        }
    }
}

/// A 0.0–1.0 ratio as a whole percentage, rounded down so it never claims
/// 100% before the work is done.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the ratio is clamped to 0.0–1.0, so the product fits in 0–100"
)]
fn percent(ratio: f64) -> u8 {
    (ratio.clamp(0.0, 1.0) * 100.0).floor() as u8
}

/// The text `y` copies for a candidate: the path it would remove, the root of
/// a batch, or its label for a command.
fn copy_text(candidate: &Candidate) -> String {
    let path = match &candidate.target {
        Target::Path { path } => Some(path),
        Target::Batch { root, .. } => Some(root),
        Target::Command { .. } => None,
    };
    path.and_then(|path| path.to_str())
        .map_or_else(|| candidate.label.clone(), str::to_owned)
}

/// The index of the first selectable item row, or 0 when there is none.
fn first_item_row(rows: &[Row]) -> usize {
    rows.iter()
        .position(|row| matches!(row, Row::Item { .. }))
        .unwrap_or(0)
}

/// The fill ratio (0.0–1.0) for the delete gauge: by bytes when the selection
/// has a measured size, else by item count, else empty.
#[expect(
    clippy::cast_precision_loss,
    reason = "gauge ratio tolerates f64 rounding of large byte/item counts"
)]
fn progress_ratio(done: usize, total: usize, reclaimed: u64, total_bytes: u64) -> f64 {
    let ratio = if total_bytes > 0 {
        reclaimed as f64 / total_bytes as f64
    } else if total > 0 {
        done as f64 / total as f64
    } else {
        0.0
    };
    ratio.clamp(0.0, 1.0)
}

/// A stable comparison key for a root path: its canonical form when resolvable,
/// otherwise the path as given (so unresolvable paths still compare by value).
fn canonical_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Add `path` to `roots` if absent, or remove it if already present.
///
/// Paths are compared by [`canonical_key`] so that `.`/`..` segments and
/// symlinks do not create duplicate roots.
fn toggle_root(roots: &mut Vec<PathBuf>, path: &Path) {
    let key = canonical_key(path);
    if let Some(pos) = roots.iter().position(|root| canonical_key(root) == key) {
        roots.remove(pos);
    } else {
        roots.push(key);
    }
}

/// Build the picker rows for `dir`: a parent (`..`) row when one exists, then
/// the readable sub-directories.
fn list_entries(dir: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    if let Some(parent) = dir.parent() {
        entries.push(Entry::Parent(parent.to_path_buf()));
    }
    for path in read_subdirs(dir) {
        entries.push(Entry::Dir(path));
    }
    entries
}

/// The readable sub-directories of `dir`, sorted by path.
///
/// Symbolic links are not followed — `DirEntry::file_type` reports the link
/// itself, so a symlink-to-directory is excluded, mirroring the deletion safety
/// guard. Unreadable directories yield an empty list rather than an error.
fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(reader) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = reader
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_dir()))
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs
}

/// A rectangle centered within `area`, sized to `percent_x` × `percent_y`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

/// Spawn a worker that scans `roots`, streaming per-category progress and the
/// final candidate list. The thread is abandoned on quit; send failures (a
/// dropped receiver) are ignored. `cancel` stops it before the next cleaner.
fn spawn_scan(roots: Vec<PathBuf>, cancel: Arc<AtomicBool>) -> Receiver<Progress> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let ctx = ScanContext::new(roots.clone());
        let mut delete_roots = roots;
        // Several cleaners can share a category, so results are collected first
        // and only then turned into rows — one header per category, not one per
        // cleaner.
        let mut found: Vec<(Category, Vec<Candidate>)> = Vec::new();

        let cleaners = all_cleaners();
        let total = cleaners.len();
        for (index, cleaner) in cleaners.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let _ = tx.send(Progress::Scanning {
                label: cleaner.category().title().to_owned(),
                index,
                total,
            });
            match cleaner.scan(&ctx) {
                Ok(items) => {
                    for root in cleaner.extra_roots() {
                        if !delete_roots.contains(&root) {
                            delete_roots.push(root);
                        }
                    }
                    if items.is_empty() {
                        continue;
                    }
                    let category = cleaner.category();
                    if let Some((_, existing)) =
                        found.iter_mut().find(|(seen, _)| *seen == category)
                    {
                        existing.extend(items);
                    } else {
                        found.push((category, items));
                    }
                }
                Err(err) => {
                    let _ = tx.send(Progress::Note(format!("scan: {err}")));
                }
            }
        }

        // Display in canonical category order, not the order cleaners ran in.
        found.sort_by_key(|(category, _)| {
            Category::ALL
                .iter()
                .position(|known| known == category)
                .unwrap_or(usize::MAX)
        });

        let mut candidates = Vec::new();
        let mut rows = Vec::new();
        for (category, items) in found {
            rows.push(Row::Header {
                title: category.title().to_owned(),
                total: total_bytes(&items),
            });
            for candidate in items {
                rows.push(Row::Item {
                    index: candidates.len(),
                });
                candidates.push(candidate);
            }
        }

        let _ = tx.send(Progress::Scanned {
            candidates,
            rows,
            delete_roots,
        });
    });
    rx
}

/// Spawn a worker that deletes `items` in order, streaming per-item progress.
/// `cancel` stops it after the current item; send failures are ignored.
fn spawn_delete(
    items: Vec<Candidate>,
    deleter: Deleter,
    verb: &'static str,
    cancel: Arc<AtomicBool>,
) -> Receiver<Progress> {
    let (tx, rx) = mpsc::channel();
    let total = items.len();
    std::thread::spawn(move || {
        let mut reclaimed = 0_u64;
        let mut count = 0_usize;
        let mut cancelled = false;
        for (done, candidate) in items.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let _ = tx.send(Progress::Deleting {
                label: format!("{verb} {}", candidate.label),
                done,
                total,
                reclaimed,
            });
            match deleter.execute(candidate) {
                Ok(outcome) => {
                    if matches!(outcome.action, Action::Trashed | Action::Purged) {
                        reclaimed += outcome.bytes;
                        count += 1;
                    } else if outcome.action == Action::NeedsSudo {
                        let _ = tx.send(Progress::Note(
                            outcome.note.unwrap_or_else(|| "needs root".to_owned()),
                        ));
                    }
                }
                Err(err) => {
                    let _ = tx.send(Progress::Note(format!("skipped: {err}")));
                }
            }
        }
        let _ = tx.send(Progress::Deleted {
            count,
            reclaimed,
            cancelled,
        });
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::{percent, progress_ratio, read_subdirs, toggle_root};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn read_subdirs_lists_only_directories_sorted() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("beta")).expect("mkdir beta");
        fs::create_dir(dir.path().join("alpha")).expect("mkdir alpha");
        fs::write(dir.path().join("file.txt"), b"x").expect("write file");

        let found = read_subdirs(dir.path());
        let names: Vec<_> = found
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect();
        // Sorted, directories only — the plain file is excluded.
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn read_subdirs_on_unreadable_path_is_empty() {
        let missing = PathBuf::from("/vacuum-tui/definitely/not/here");
        assert_eq!(read_subdirs(&missing), Vec::<PathBuf>::new());
    }

    #[test]
    fn toggle_root_adds_then_removes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("project");
        fs::create_dir(&target).expect("mkdir project");

        let mut roots: Vec<PathBuf> = Vec::new();
        toggle_root(&mut roots, &target);
        assert_eq!(roots.len(), 1, "first toggle adds the root");
        toggle_root(&mut roots, &target);
        assert_eq!(
            roots,
            Vec::<PathBuf>::new(),
            "second toggle removes the root"
        );
    }

    #[test]
    fn progress_ratio_prefers_bytes_then_items() {
        // Bytes-based when a measured total exists.
        assert!((progress_ratio(1, 4, 50, 100) - 0.5).abs() < 1e-9);
        // Falls back to item count when no bytes are measured.
        assert!((progress_ratio(2, 4, 0, 0) - 0.5).abs() < 1e-9);
        // Nothing to do yet.
        assert!(progress_ratio(0, 0, 0, 0).abs() < 1e-9);
        // Over-100% is clamped.
        assert!((progress_ratio(5, 4, 150, 100) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn percent_rounds_down_and_clamps() {
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.999), 99);
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(7.0), 100);
        assert_eq!(percent(-1.0), 0);
    }
}
