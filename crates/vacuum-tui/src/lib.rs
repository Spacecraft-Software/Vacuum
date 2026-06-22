// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! # vacuum-tui
//!
//! The interactive [ratatui](https://ratatui.rs/) frontend for Vacuum: a
//! checkbox list of reclaimable candidates grouped by category, with a running
//! selection total, dry-run-by-intent confirmation, and the Steelbore theme.
//!
//! Keys: `↑/↓` or `j/k` move · `space` toggle · `a` toggle all · `r` choose
//! roots · `p` purge mode · `enter` apply (with confirm) · `q`/`Esc` quit.
//!
//! The set of scanned roots is supplied by the caller (the `vacuum tui` command
//! resolves it from the command line, `VACUUM_ROOTS`, the config file, or
//! `$HOME`) and can be changed at runtime through the `r` directory picker. The
//! same roots bound every deletion via [`vacuum_core::safety`], so the picker
//! cannot widen the deletion boundary past what the CLI would allow.
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use vacuum_cleaners::all_cleaners;
use vacuum_core::{Action, Candidate, DeleteMode, Deleter, ScanContext, human_bytes, total_bytes};
use vacuum_theme::Steelbore;

/// How long to wait for input before redrawing.
const TICK: Duration = Duration::from_millis(250);

/// Launch the interactive TUI over `roots`, restoring the terminal on exit.
///
/// `roots` bounds both the scan and every deletion performed from the frontend.
///
/// # Errors
///
/// Returns an error if scanning or terminal I/O fails.
pub fn run(roots: Vec<PathBuf>) -> anyhow::Result<()> {
    let mut app = App::new(roots)?;
    let mut terminal = ratatui::init();
    let result = app.event_loop(&mut terminal);
    ratatui::restore();
    result
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

/// TUI application state.
struct App {
    roots: Vec<PathBuf>,
    candidates: Vec<Candidate>,
    selected: Vec<bool>,
    rows: Vec<Row>,
    cursor: usize,
    purge: bool,
    confirming: bool,
    picker: Option<Picker>,
    status: String,
}

impl App {
    fn new(roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let (candidates, rows) = scan_all(&roots)?;
        let selected = vec![false; candidates.len()];
        let cursor = first_item_row(&rows);
        Ok(Self {
            roots,
            candidates,
            selected,
            rows,
            cursor,
            purge: false,
            confirming: false,
            picker: None,
            status: "Select items to reclaim, then press enter. Press r to choose roots."
                .to_owned(),
        })
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if !event::poll(TICK)? {
                continue;
            }
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if self.handle_key(key.code)? {
                    return Ok(());
                }
            }
        }
    }

    /// Handle a key press; returns `true` to quit.
    fn handle_key(&mut self, code: KeyCode) -> anyhow::Result<bool> {
        // The picker and the confirm prompt are modal: while either is open it
        // intercepts every key, mirroring the existing confirm flow.
        if self.picker.is_some() {
            self.handle_picker_key(code)?;
            return Ok(false);
        }

        if self.confirming {
            if code == KeyCode::Char('y') {
                self.apply()?;
            } else {
                self.status = "Cancelled.".to_owned();
            }
            self.confirming = false;
            return Ok(false);
        }

        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Down | KeyCode::Char('j') => self.cursor_down(),
            KeyCode::Up | KeyCode::Char('k') => self.cursor_up(),
            KeyCode::Char(' ') => self.toggle_current(),
            KeyCode::Char('a') => self.toggle_all(),
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
        Ok(false)
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
    fn handle_picker_key(&mut self, code: KeyCode) -> anyhow::Result<()> {
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
            KeyCode::Esc | KeyCode::Char('q') => {
                self.picker = None;
                self.rescan()?;
                self.status = format!("Scanning {} root(s). Press r to change.", self.roots.len());
            }
            _ => {}
        }
        Ok(())
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
        for flag in &mut self.selected {
            *flag = select;
        }
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

    fn apply(&mut self) -> anyhow::Result<()> {
        let mode = if self.purge {
            DeleteMode::Purge
        } else {
            DeleteMode::Trash
        };
        let deleter = Deleter::new(mode, false, self.roots.clone());

        let mut reclaimed = 0_u64;
        let mut count = 0_usize;
        for (candidate, selected) in self.candidates.iter().zip(&self.selected) {
            if !*selected {
                continue;
            }
            match deleter.execute(candidate) {
                Ok(outcome) => {
                    if matches!(outcome.action, Action::Trashed | Action::Purged) {
                        reclaimed += outcome.bytes;
                        count += 1;
                    } else if outcome.action == Action::NeedsSudo {
                        // Surface the sudo command rather than silently skipping.
                        self.status = outcome.note.unwrap_or_else(|| "needs root".to_owned());
                    }
                }
                Err(err) => self.status = format!("skipped: {err}"),
            }
        }

        let verb = if self.purge { "Deleted" } else { "Trashed" };
        self.status = format!("{verb} {count} item(s), freed ~{}.", human_bytes(reclaimed));

        // Rescan so the list reflects what is now gone.
        self.rescan()
    }

    /// Re-scan the active roots and rebuild the row layout, clearing selection.
    fn rescan(&mut self) -> anyhow::Result<()> {
        let (candidates, rows) = scan_all(&self.roots)?;
        self.selected = vec![false; candidates.len()];
        self.candidates = candidates;
        self.rows = rows;
        self.cursor = first_item_row(&self.rows);
        Ok(())
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

        if let Some(picker) = self.picker.as_ref() {
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
            "↑/↓ move · space toggle · a all · r roots · p purge · enter apply · q quit",
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

/// The index of the first selectable item row, or 0 when there is none.
fn first_item_row(rows: &[Row]) -> usize {
    rows.iter()
        .position(|row| matches!(row, Row::Item { .. }))
        .unwrap_or(0)
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

/// Scan every cleaner over `roots` and build the row layout.
fn scan_all(roots: &[PathBuf]) -> anyhow::Result<(Vec<Candidate>, Vec<Row>)> {
    let ctx = ScanContext {
        roots: roots.to_vec(),
    };
    let mut candidates = Vec::new();
    let mut rows = Vec::new();

    for cleaner in all_cleaners() {
        let found = cleaner.scan(&ctx)?;
        if found.is_empty() {
            continue;
        }
        rows.push(Row::Header {
            title: cleaner.category().title().to_owned(),
            total: total_bytes(&found),
        });
        for candidate in found {
            rows.push(Row::Item {
                index: candidates.len(),
            });
            candidates.push(candidate);
        }
    }

    Ok((candidates, rows))
}

#[cfg(test)]
mod tests {
    use super::{read_subdirs, toggle_root};
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
        assert!(read_subdirs(&missing).is_empty());
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
        assert!(roots.is_empty(), "second toggle removes the root");
    }
}
