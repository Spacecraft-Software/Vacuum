// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Output-mode and TUI-launch resolution, including AI-agent detection (SFRS).

use is_terminal::IsTerminal as _;

use crate::cli::{Format, GlobalArgs};

/// How output should be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Human-readable text.
    Human,
    /// Machine-readable JSON envelope (single document).
    Json,
    /// Newline-delimited JSON: one record per line.
    Jsonl,
}

impl OutputMode {
    /// Whether this mode emits machine-readable JSON (enveloped or streamed).
    pub fn is_machine(self) -> bool {
        matches!(self, Self::Json | Self::Jsonl)
    }
}

/// Environment variables that signal an AI agent / CI is driving the CLI.
const AGENT_VARS: &[&str] = &[
    "AI_AGENT",
    "AGENT",
    "CI",
    "CLAUDECODE",
    "CURSOR_AGENT",
    "GEMINI_CLI",
];

/// Whether an AI agent or CI environment is detected.
pub fn is_agent() -> bool {
    AGENT_VARS
        .iter()
        .any(|var| std::env::var(var).is_ok_and(|value| !value.is_empty()))
}

/// Resolve the effective output mode from flags, agent detection, and TTY state.
///
/// Explicit flags win; otherwise agents and pipes default to JSON and an
/// interactive terminal defaults to human-readable text.
pub fn output_mode(global: &GlobalArgs) -> OutputMode {
    if global.json || global.format == Some(Format::Json) {
        return OutputMode::Json;
    }
    if global.format == Some(Format::Jsonl) {
        return OutputMode::Jsonl;
    }
    if global.format == Some(Format::Human) {
        return OutputMode::Human;
    }
    // `--format explore` requests the TUI; when that cannot run (agent/pipe) the
    // data still has to render, so it degrades to JSON like any other pipe.
    if is_agent() || !std::io::stdout().is_terminal() {
        return OutputMode::Json;
    }
    OutputMode::Human
}

/// Whether the interactive TUI should launch when no subcommand is given.
pub fn should_launch_tui(global: &GlobalArgs) -> bool {
    if global.json || global.format.is_some() {
        return false;
    }
    if is_agent() {
        return false;
    }
    std::io::stdout().is_terminal()
}
