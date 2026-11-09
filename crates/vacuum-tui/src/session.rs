// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! One TUI session's terminal protocols: resolve them once, use them, and
//! restore the terminal on every exit path (`spacecraft-tui-protocols` §2.3).
//!
//! Restoration is shared by three paths through one [`RestoreState`]:
//!
//! - **normal exit and error returns** — [`TermSession::restore`], and `Drop`;
//! - **panic** — a hook installed after `ratatui::init`, so it runs *before*
//!   ratatui's own hook leaves the alternate screen (the kitty keyboard stack
//!   is per screen, so the pop must land on the alternate one);
//! - **SIGTERM / SIGHUP / SIGINT** — a flag the event loop polls every tick,
//!   which then exits through the normal path. In raw mode `Ctrl+C` arrives
//!   as a key, so these come only from outside (`kill`, a closed terminal).
//!
//! Each mode is recorded in an atomic and taken with `swap(false)`, so a
//! second restore is a no-op. Order is the reverse of setup: pop keyboard
//! flags, then clear progress; ratatui then leaves the alternate screen and
//! raw mode.

use std::io::{IsTerminal as _, Write as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;

use crate::TuiOptions;
use crate::probe::{self, QUERY_DA1, QUERY_KEYBOARD, QUERY_XTVERSION};
use crate::protocols::{
    self, Decision, ProgressState, Protocol, ProtocolReport, osc_progress, osc52,
};

/// Minimum spacing of OSC 9;4 percent updates: about five a second, as the
/// protocol guidance asks. State changes (start, finish) are never delayed.
const PROGRESS_MIN_INTERVAL: Duration = Duration::from_millis(200);

/// Which terminal modes are currently changed, shared with the panic hook.
#[derive(Debug, Default)]
struct RestoreState {
    keyboard_pushed: AtomicBool,
    progress_active: AtomicBool,
}

impl RestoreState {
    /// Undo every changed mode, newest first. Safe to call more than once and
    /// from a panic hook: write errors are ignored, since there is nowhere
    /// left to report them.
    fn restore(&self) {
        if self.keyboard_pushed.swap(false, Ordering::SeqCst) {
            let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        }
        if self.progress_active.swap(false, Ordering::SeqCst) {
            let mut err = std::io::stderr();
            let _ = err.write_all(osc_progress(None).as_bytes());
            let _ = err.flush();
        }
    }
}

/// Rate-limited OSC 9;4 writer.
#[derive(Debug, Default)]
struct ProgressReporter {
    last: Option<ProgressState>,
    last_at: Option<Instant>,
}

/// The protocols in force for one TUI session.
#[derive(Debug)]
pub(crate) struct TermSession {
    report: ProtocolReport,
    restore: Arc<RestoreState>,
    /// `Some` when OSC 9;4 is on.
    progress: Option<ProgressReporter>,
    /// Set by SIGTERM / SIGHUP / SIGINT.
    signalled: Arc<AtomicBool>,
    #[cfg(unix)]
    signal_ids: Vec<signal_hook::SigId>,
}

impl TermSession {
    /// Probe, decide, and switch on the protocols for this session.
    ///
    /// Call after `ratatui::init` (raw mode and the alternate screen are on)
    /// and before the event loop first reads input.
    pub(crate) fn start(options: &TuiOptions) -> Self {
        let env = |var: &str| std::env::var(var).ok();
        let gate = protocols::gate(std::io::stdout().is_terminal(), options.agent, &env);
        let overrides = options.protocols;

        // Probe only for what detection still has to decide.
        let mut queries = Vec::new();
        if gate.is_none() {
            if overrides.get(Protocol::Keyboard).is_none() {
                queries.extend_from_slice(QUERY_KEYBOARD);
            }
            if overrides.get(Protocol::Progress).is_none() {
                queries.extend_from_slice(QUERY_XTVERSION);
            }
        }
        let replies = if queries.is_empty() {
            probe::Replies::default()
        } else {
            queries.extend_from_slice(QUERY_DA1);
            probe::probe(&queries, options.probe_timeout)
        };

        let mux = protocols::multiplexer(&env);
        let decisions: Vec<Decision> = Protocol::ALL
            .into_iter()
            .map(|protocol| {
                let (on, reason) = match gate {
                    Some(reason) if protocol.used() => (false, reason.to_owned()),
                    _ => decide(protocol, options, &replies, mux, &env),
                };
                Decision::new(protocol, on, reason)
            })
            .collect();
        let report = ProtocolReport::new(decisions);

        let restore = Arc::new(RestoreState::default());
        if report.is_on(Protocol::Keyboard)
            && execute!(
                std::io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )
            .is_ok()
        {
            restore.keyboard_pushed.store(true, Ordering::SeqCst);
        }

        // Runs before ratatui's hook (installed earlier), which leaves the
        // alternate screen and then chains to the default message printer.
        let hook_state = Arc::clone(&restore);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            hook_state.restore();
            previous(info);
        }));

        let signalled = Arc::new(AtomicBool::new(false));
        #[cfg(unix)]
        let signal_ids = [
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
            signal_hook::consts::SIGINT,
        ]
        .into_iter()
        .filter_map(|signal| signal_hook::flag::register(signal, Arc::clone(&signalled)).ok())
        .collect();

        Self {
            progress: report
                .is_on(Protocol::Progress)
                .then(ProgressReporter::default),
            report,
            restore,
            signalled,
            #[cfg(unix)]
            signal_ids,
        }
    }

    /// The resolved protocol state, for `--verbose`.
    pub(crate) fn report(&self) -> &ProtocolReport {
        &self.report
    }

    /// Whether a termination signal asked the TUI to exit.
    pub(crate) fn should_exit(&self) -> bool {
        self.signalled.load(Ordering::SeqCst)
    }

    /// Copy `text` to the clipboard with OSC 52.
    ///
    /// # Errors
    ///
    /// Returns why the copy was not sent: clipboard off for this session, or a
    /// payload over the OSC 52 size cap. A sent copy can still be ignored by a
    /// terminal without OSC 52 (there is no reply), so the caller always shows
    /// the text as well.
    pub(crate) fn copy(&self, text: &str) -> Result<(), String> {
        if !self.report.is_on(Protocol::Clipboard) {
            let reason = self
                .report
                .decisions()
                .iter()
                .find(|decision| decision.protocol == Protocol::Clipboard)
                .map_or("off", |decision| decision.reason.as_str());
            return Err(format!("clipboard off: {reason}"));
        }
        let sequence = osc52(text).ok_or_else(|| "too large for OSC 52".to_owned())?;
        let mut out = std::io::stdout();
        out.write_all(sequence.as_bytes())
            .and_then(|()| out.flush())
            .map_err(|err| format!("clipboard write failed: {err}"))
    }

    /// Show `state` in the terminal's tab or taskbar; `None` clears it.
    pub(crate) fn progress(&mut self, state: Option<ProgressState>) {
        let Some(reporter) = self.progress.as_mut() else {
            return;
        };
        if state == reporter.last {
            return;
        }
        let both_percent = matches!(
            (reporter.last, state),
            (
                Some(ProgressState::Normal(_)),
                Some(ProgressState::Normal(_))
            )
        );
        if both_percent
            && reporter
                .last_at
                .is_some_and(|at| at.elapsed() < PROGRESS_MIN_INTERVAL)
        {
            return;
        }
        // Mark active before writing so a panic mid-write still clears it.
        if state.is_some() {
            self.restore.progress_active.store(true, Ordering::SeqCst);
        }
        let mut err = std::io::stderr();
        let _ = err.write_all(osc_progress(state).as_bytes());
        let _ = err.flush();
        if state.is_none() {
            self.restore.progress_active.store(false, Ordering::SeqCst);
        }
        reporter.last = state;
        reporter.last_at = Some(Instant::now());
    }

    /// Undo every protocol mode. Call before `ratatui::restore`.
    pub(crate) fn restore(&self) {
        self.restore.restore();
    }
}

impl Drop for TermSession {
    fn drop(&mut self) {
        self.restore.restore();
        #[cfg(unix)]
        for id in self.signal_ids.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

/// Decide one protocol once the §2.1 gates have passed.
fn decide(
    protocol: Protocol,
    options: &TuiOptions,
    replies: &probe::Replies,
    mux: Option<&'static str>,
    env: &impl Fn(&str) -> Option<String>,
) -> (bool, String) {
    let forced = options.protocols.get(protocol);
    if !protocol.used() {
        let note = if forced == Some(true) {
            "not used by Vacuum; override ignored"
        } else {
            "not used by Vacuum"
        };
        return (false, note.to_owned());
    }
    if protocol == Protocol::Progress && !std::io::stderr().is_terminal() {
        return (false, "standard error is not a terminal".to_owned());
    }
    if let Some(on) = forced {
        return (on, "forced by override".to_owned());
    }
    match protocol {
        Protocol::Keyboard => {
            if replies.kitty_keyboard {
                (
                    true,
                    "terminal answered the kitty keyboard query".to_owned(),
                )
            } else if let Some(mux) = mux {
                (false, format!("no reply inside {mux}; legacy keys"))
            } else if env("TERM_PROGRAM").as_deref() == Some("WezTerm") {
                (
                    false,
                    "no reply; set enable_kitty_keyboard = true in wezterm.lua".to_owned(),
                )
            } else {
                (
                    false,
                    "no reply to the kitty keyboard query; legacy keys".to_owned(),
                )
            }
        }
        Protocol::Clipboard => (true, "OSC 52 write; no query exists".to_owned()),
        Protocol::Progress => {
            if let Some(mux) = mux {
                return (false, format!("inside {mux}; skipped"));
            }
            match protocols::progress_allowed(env, replies.xtversion.as_deref()) {
                Ok(terminal) => (true, format!("{terminal} is on the allow-list")),
                Err(reason) => (false, reason.to_owned()),
            }
        }
        // `used()` covers exactly the three arms above.
        _ => (false, "not used by Vacuum".to_owned()),
    }
}
