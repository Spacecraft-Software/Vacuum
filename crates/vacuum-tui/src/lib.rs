// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! # vacuum-tui
//!
//! The interactive [ratatui](https://ratatui.rs/) frontend for Vacuum: a
//! checkbox list of reclaimable candidates grouped by category, with a running
//! selection total, dry-run-by-intent confirmation, and the Steelbore theme.
//!
//! Keys: `↑/↓` or `j/k` move · `space` toggle · `a` toggle all · `p` purge mode ·
//! `enter` apply (with confirm) · `q`/`Esc` quit.
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize as _};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use vacuum_cleaners::all_cleaners;
use vacuum_core::{
    Action, Candidate, DeleteMode, Deleter, ScanContext, human_bytes, safety, total_bytes,
};
use vacuum_theme::Steelbore;

/// How long to wait for input before redrawing.
const TICK: Duration = Duration::from_millis(250);

/// Launch the interactive TUI, restoring the terminal on exit.
///
/// # Errors
///
/// Returns an error if scanning or terminal I/O fails.
pub fn run() -> anyhow::Result<()> {
    let mut app = App::new()?;
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

/// TUI application state.
struct App {
    candidates: Vec<Candidate>,
    selected: Vec<bool>,
    rows: Vec<Row>,
    cursor: usize,
    purge: bool,
    confirming: bool,
    status: String,
}

impl App {
    fn new() -> anyhow::Result<Self> {
        let (candidates, rows) = scan_all()?;
        let selected = vec![false; candidates.len()];
        let cursor = rows
            .iter()
            .position(|row| matches!(row, Row::Item { .. }))
            .unwrap_or(0);
        Ok(Self {
            candidates,
            selected,
            rows,
            cursor,
            purge: false,
            confirming: false,
            status: "Select items to reclaim, then press enter.".to_owned(),
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
        let deleter = Deleter::new(mode, false, safety::default_roots());

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
        let (candidates, rows) = scan_all()?;
        self.selected = vec![false; candidates.len()];
        self.candidates = candidates;
        self.rows = rows;
        self.cursor = self
            .rows
            .iter()
            .position(|row| matches!(row, Row::Item { .. }))
            .unwrap_or(0);
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
        let keys = Line::from("↑/↓ move · space toggle · a all · p purge · enter apply · q quit")
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
}

/// Scan every cleaner over the default roots and build the row layout.
fn scan_all() -> anyhow::Result<(Vec<Candidate>, Vec<Row>)> {
    let ctx = ScanContext {
        roots: safety::default_roots(),
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
