// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Structured CLI errors: canonical exit codes (the CLI Standard §4) and tips-thinking
//! hints (agentic-cli §3).
//!
//! Every failure carries a stable `code`, a canonical `exit_code`, a one-line
//! `message`, and a **runnable** `hint` — the exact next command an agent
//! should try. In machine mode the error is emitted to stderr as a single JSON
//! object `{ "error": { … } }`; in human mode it is a short `vacuum: …` line
//! plus a `hint:` line. stdout is never touched on the error path.

use std::path::Path;
use std::process::ExitCode;

use is_terminal::IsTerminal as _;
use jiff::Timestamp;
use serde::Serialize;
use vacuum_core::VacuumError;

use crate::agent::OutputMode;

/// Documentation anchor surfaced in every structured error.
const DOCS_URL: &str = "https://Vacuum.SpacecraftSoftware.org/";

/// A structured, agent-friendly CLI error.
///
/// Construct via the typed helpers (or `From<VacuumError>`) so the `code`,
/// `exit_code`, and `hint` stay consistent with the canonical map.
#[derive(Debug)]
pub struct CliError {
    /// Stable `SCREAMING_SNAKE_CASE` error code.
    code: &'static str,
    /// Canonical POSIX-friendly exit code (the CLI Standard §4).
    exit_code: u8,
    /// One-line, human-readable description of what went wrong.
    message: String,
    /// A runnable command the caller should try next (never prose).
    hint: String,
}

/// The on-the-wire shape of the JSON error object (the CLI Standard §1.8).
#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    exit_code: u8,
    message: &'a str,
    hint: &'a str,
    timestamp: String,
    command: &'a str,
    docs_url: &'a str,
}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

impl CliError {
    /// A bad invocation the agent should fix rather than retry (exit 2).
    pub fn usage(code: &'static str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code,
            exit_code: 2,
            message: message.into(),
            hint: hint.into(),
        }
    }

    /// An unknown `--category` slug. Points at the introspection command.
    pub fn unknown_category(message: impl Into<String>) -> Self {
        Self::usage(
            "UNKNOWN_CATEGORY",
            message,
            "vacuum schema --json | jaq '.commands[] | select(.name==\"list\") | .categories'",
        )
    }

    /// A destructive run refused for want of confirmation (exit 1).
    ///
    /// `retry_hint` is the exact command to re-run non-interactively — it differs
    /// per subcommand (`clean` vs `empty-trash`), so the caller supplies it.
    pub fn confirmation_required(verb: &str, retry_hint: impl Into<String>) -> Self {
        Self {
            code: "CONFIRMATION_REQUIRED",
            exit_code: 1,
            message: format!("refusing to {verb} without confirmation in non-interactive mode"),
            hint: retry_hint.into(),
        }
    }

    /// A config file that exists but cannot be parsed as TOML (exit 2 — the
    /// user must fix the file; retrying verbatim will not help).
    pub fn config_invalid(path: &Path, detail: &str) -> Self {
        Self::usage(
            "CONFIG_INVALID",
            format!("invalid config at {}: {detail}", path.display()),
            format!(
                "$EDITOR {}   # expected TOML, e.g. roots = [\"~/projects\"]",
                path.display()
            ),
        )
    }

    /// An explicitly named config file (`--config`/`VACUUM_CONFIG`) that does
    /// not exist (exit 2). A *default*-location miss is not an error.
    pub fn config_not_found(path: &Path) -> Self {
        Self::usage(
            "CONFIG_NOT_FOUND",
            format!("config file not found: {}", path.display()),
            format!(
                "printf 'roots = [\"~/projects\"]\\n' > {}   # create it, or drop --config",
                path.display()
            ),
        )
    }

    /// A config file that exists but cannot be read (permissions, I/O) (exit 1).
    pub fn config_unreadable(path: &Path, detail: &str) -> Self {
        Self {
            code: "CONFIG_UNREADABLE",
            exit_code: 1,
            message: format!("cannot read config at {}: {detail}", path.display()),
            hint: format!("eza -l {}   # check the file's permissions", path.display()),
        }
    }

    /// A failure talking to the OS trash backend, e.g. listing or purging (exit 1).
    pub fn trash(detail: &str) -> Self {
        Self {
            code: "TRASH_ERROR",
            exit_code: 1,
            message: format!("trash operation failed: {detail}"),
            hint: "vacuum empty-trash   # retry; if it persists, empty the trash from your file manager"
                .to_owned(),
        }
    }

    /// An operation the current platform's trash backend cannot perform (exit 1).
    ///
    /// The `trash` crate exposes listing/emptying only on Linux/Freedesktop and
    /// Windows; macOS and others fall here. Compiled only on those platforms, so
    /// it is not dead code on the supported (Linux/Windows) build.
    #[cfg(not(any(
        windows,
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    )))]
    pub fn unsupported(operation: &str) -> Self {
        Self {
            code: "UNSUPPORTED_PLATFORM",
            exit_code: 1,
            message: format!("{operation} is not supported on this platform"),
            hint: "vacuum list   # trash listing/emptying needs Linux or Windows".to_owned(),
        }
    }

    /// No subcommand was given outside a terminal (exit 2).
    pub fn no_command() -> Self {
        Self::usage(
            "NO_COMMAND",
            "no subcommand given",
            "vacuum scan   # or: vacuum list --json",
        )
    }

    /// Wrap an unexpected internal failure, e.g. output serialization (exit 1).
    pub fn internal(err: &anyhow::Error) -> Self {
        Self {
            code: "INTERNAL_ERROR",
            exit_code: 1,
            message: format!("internal error: {err:#}"),
            hint: "vacuum describe   # report this at https://Vacuum.SpacecraftSoftware.org/"
                .to_owned(),
        }
    }

    /// Wrap a failure from the interactive TUI frontend (exit 1).
    pub fn tui(err: &anyhow::Error) -> Self {
        Self {
            code: "TUI_ERROR",
            exit_code: 1,
            message: format!("interactive interface failed: {err:#}"),
            hint: "vacuum list   # use the non-interactive CLI instead".to_owned(),
        }
    }

    /// The one-line, human-readable description of what went wrong.
    ///
    /// Used to fold a sub-operation failure into a larger run's per-item warning
    /// without surfacing the full structured error.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Render the error to stderr in the active mode and return its exit code.
    pub fn emit(&self, mode: OutputMode, command: &str) -> ExitCode {
        match mode {
            OutputMode::Human => {
                eprintln!("vacuum: {}", self.message);
                eprintln!("hint: {}", self.hint);
            }
            // JSON and JSONL both want a single structured error object.
            OutputMode::Json | OutputMode::Jsonl => self.emit_json(command),
        }
        ExitCode::from(self.exit_code)
    }

    fn emit_json(&self, command: &str) {
        let envelope = ErrorEnvelope {
            error: ErrorBody {
                code: self.code,
                exit_code: self.exit_code,
                message: &self.message,
                hint: &self.hint,
                timestamp: Timestamp::now().to_string(),
                command,
                docs_url: DOCS_URL,
            },
        };
        // Pretty only when a human is watching a terminal; compact otherwise so
        // agents paying for tokens are not billed for whitespace (the CLI Standard §8).
        let rendered = if std::io::stderr().is_terminal() {
            serde_json::to_string_pretty(&envelope)
        } else {
            serde_json::to_string(&envelope)
        };
        match rendered {
            Ok(text) => eprintln!("{text}"),
            Err(err) => eprintln!(
                "vacuum: {} (and failed to serialize error: {err})",
                self.message
            ),
        }
    }
}

impl From<VacuumError> for CliError {
    /// Map a core engine error onto the canonical exit-code table with a
    /// runnable hint for each variant.
    fn from(err: VacuumError) -> Self {
        let code = err.code();
        let message = err.to_string();
        let (exit_code, hint): (u8, String) = match &err {
            VacuumError::Io { path, .. } => (
                1,
                format!(
                    "vacuum scan {}   # confirm the path is readable",
                    path.display()
                ),
            ),
            VacuumError::OutsideRoots(path) => (
                4,
                format!(
                    "vacuum clean --root {}   # name the path as an explicit root",
                    path.display()
                ),
            ),
            VacuumError::ProtectedPath(_) => (
                4,
                "vacuum scan ~   # operate under $HOME; system paths are refused by design"
                    .to_owned(),
            ),
            VacuumError::Symlink(_) => (
                4,
                "vacuum clean --root <real-target>   # Vacuum never deletes through symlinks"
                    .to_owned(),
            ),
            VacuumError::ToolNotFound(tool) => (
                127,
                format!("nix shell nixpkgs#{tool}   # then re-run, or skip with --category"),
            ),
            VacuumError::CommandFailed { command, .. } => (
                1,
                format!("{command}   # run manually to see the underlying failure"),
            ),
            VacuumError::Trash(_) => (
                1,
                "vacuum clean --apply --purge   # trash unavailable; purge deletes permanently"
                    .to_owned(),
            ),
            // `VacuumError` is #[non_exhaustive]: future variants degrade to a
            // safe general failure rather than failing to compile downstream.
            _ => (
                1,
                "vacuum describe   # see https://Vacuum.SpacecraftSoftware.org/".to_owned(),
            ),
        };
        Self {
            code,
            exit_code,
            message,
            hint,
        }
    }
}

/// Convenience alias for the CLI's fallible operations.
pub type CliResult<T> = std::result::Result<T, CliError>;
