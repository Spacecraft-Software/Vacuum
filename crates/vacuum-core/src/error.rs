// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Error type for the Vacuum core engine.
//!
//! Errors carry a stable, machine-readable [`VacuumError::code`] in
//! `SCREAMING_SNAKE_CASE` so the CLI can surface them in JSON output.

use std::path::PathBuf;

use thiserror::Error;

/// A fallible result within the Vacuum core engine.
pub type Result<T> = std::result::Result<T, VacuumError>;

/// Everything that can go wrong while scanning or deleting.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VacuumError {
    /// An I/O operation failed for a specific path.
    #[error("i/o error at {path}: {source}")]
    Io {
        /// The path the operation was attempted on.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A delete target resolved outside every allowed root.
    #[error("refusing to act outside the allowed roots: {0}")]
    OutsideRoots(PathBuf),

    /// A delete target resolved onto a protected system path.
    #[error("refusing to act on a protected system path: {0}")]
    ProtectedPath(PathBuf),

    /// A delete target is a symlink; Vacuum never deletes through symlinks.
    #[error("refusing to delete a symlink: {0}")]
    Symlink(PathBuf),

    /// A required external tool (e.g. `nix-collect-garbage`) is not installed.
    #[error("required tool not found on PATH: {0}")]
    ToolNotFound(String),

    /// An external command ran but exited unsuccessfully.
    #[error("command failed: `{command}` (exit code {code})")]
    CommandFailed {
        /// The command line that was executed.
        command: String,
        /// The process exit code (`-1` if terminated by signal).
        code: i32,
    },

    /// Moving an item to the trash failed.
    #[error("trash operation failed: {0}")]
    Trash(String),
}

impl VacuumError {
    /// A stable error code for machine-readable output.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "IO_ERROR",
            Self::OutsideRoots(_) => "OUTSIDE_ROOTS",
            Self::ProtectedPath(_) => "PROTECTED_PATH",
            Self::Symlink(_) => "SYMLINK_REFUSED",
            Self::ToolNotFound(_) => "TOOL_NOT_FOUND",
            Self::CommandFailed { .. } => "COMMAND_FAILED",
            Self::Trash(_) => "TRASH_FAILED",
        }
    }
}
