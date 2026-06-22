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

/// Variables that **force** machine output and suppress the TUI: a generic
/// agent, or CI (agentic-cli §4). These are the only behavioral switches.
const AGENT_FORCE_VARS: &[&str] = &["AI_AGENT", "AGENT", "CI"];

/// **Informational** variables that merely name the calling agent. Per
/// agentic-cli §4 they MUST NOT change the output format on their own — they
/// only surface in `metadata.invoking_agent`. TTY / `AGENT_FORCE_VARS` decide
/// the mode.
const AGENT_INFO_VARS: &[&str] = &["CLAUDECODE", "CURSOR_AGENT", "GEMINI_CLI"];

/// Return the first variable in `vars` that is set to a non-empty value,
/// looked up via `get`. Pure over `get` so it can be unit-tested without
/// mutating the process environment.
fn first_set<'a>(vars: &[&'a str], get: impl Fn(&str) -> Option<String>) -> Option<&'a str> {
    vars.iter()
        .copied()
        .find(|var| get(var).is_some_and(|value| !value.is_empty()))
}

/// Whether an AI agent or CI environment is forcing non-interactive output.
pub fn is_agent() -> bool {
    first_set(AGENT_FORCE_VARS, |var| std::env::var(var).ok()).is_some()
}

/// The name of the specific invoking agent, if one identified itself, for
/// telemetry in the JSON envelope (`metadata.invoking_agent`). Never a
/// behavioral switch.
pub fn invoking_agent() -> Option<&'static str> {
    first_set(AGENT_INFO_VARS, |var| std::env::var(var).ok())
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

#[cfg(test)]
mod tests {
    use super::{AGENT_FORCE_VARS, AGENT_INFO_VARS, first_set};

    /// Build a fake env lookup from a fixed (name, value) table.
    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |var| {
            pairs
                .iter()
                .find(|(name, _)| *name == var)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn force_vars_are_detected() {
        let env = env_of(&[("AI_AGENT", "1")]);
        assert_eq!(first_set(AGENT_FORCE_VARS, &env), Some("AI_AGENT"));
    }

    #[test]
    fn informational_vars_do_not_force_mode() {
        // CLAUDECODE et al. must NOT count as a forcing agent (agentic-cli §4).
        let env = env_of(&[("CLAUDECODE", "1")]);
        assert_eq!(first_set(AGENT_FORCE_VARS, &env), None);
        assert_eq!(first_set(AGENT_INFO_VARS, &env), Some("CLAUDECODE"));
    }

    #[test]
    fn empty_value_is_not_set() {
        let env = env_of(&[("CI", "")]);
        assert_eq!(first_set(AGENT_FORCE_VARS, &env), None);
    }
}
