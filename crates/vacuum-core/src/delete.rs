// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The deletion engine.
//!
//! [`Deleter`] turns a [`Candidate`] into an [`Outcome`]. It is dry-run by
//! default; path deletions go to the XDG trash unless [`DeleteMode::Purge`] is
//! selected; and commands that need root are never escalated — the `sudo` form
//! is reported back for the user to run.

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use crate::cleaner::{Candidate, CommandSpec, Target};
use crate::error::{Result, VacuumError};
use crate::safety;

/// How a path-based deletion is performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeleteMode {
    /// Move to the XDG trash (recoverable).
    Trash,
    /// Permanently remove (not recoverable).
    Purge,
}

/// What the deleter did, or would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Dry run: would move this path to trash.
    WouldTrash,
    /// Moved this path to trash.
    Trashed,
    /// Dry run: would permanently remove this path.
    WouldPurge,
    /// Permanently removed this path.
    Purged,
    /// Dry run: would run this command.
    WouldRun,
    /// Ran this command.
    Ran,
    /// Skipped: requires root. The `sudo` command is in [`Outcome::note`].
    NeedsSudo,
}

/// The result of acting (or planning to act) on a single candidate.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    /// The candidate's label, echoed for reporting.
    pub label: String,
    /// Estimated bytes reclaimed by this action.
    pub bytes: u64,
    /// What was done, or would be done.
    pub action: Action,
    /// Optional human-readable detail (e.g. the `sudo` command to run).
    ///
    /// Omitted from JSON output when absent (token-economy hygiene, SFRS §8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Executes deletions according to a configured mode and dry-run setting.
#[derive(Debug, Clone)]
pub struct Deleter {
    mode: DeleteMode,
    dry_run: bool,
    roots: Vec<PathBuf>,
}

impl Deleter {
    /// Create a deleter.
    ///
    /// `roots` bounds every path deletion (see [`safety::check_deletable`]).
    pub fn new(mode: DeleteMode, dry_run: bool, roots: Vec<PathBuf>) -> Self {
        Self {
            mode,
            dry_run,
            roots,
        }
    }

    /// Whether this deleter is in dry-run mode.
    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// Act on a single candidate, returning what happened.
    ///
    /// # Errors
    ///
    /// Propagates safety-guard failures for path targets, and tool/command
    /// failures for command targets.
    pub fn execute(&self, candidate: &Candidate) -> Result<Outcome> {
        match &candidate.target {
            Target::Path { path } => self.act_on_path(candidate, path),
            Target::Command { spec } => self.act_on_command(candidate, spec),
        }
    }

    fn act_on_path(&self, candidate: &Candidate, path: &PathBuf) -> Result<Outcome> {
        // The safety gate runs even on a dry run, so a dry run reliably
        // predicts what an `--apply` run would refuse.
        safety::check_deletable(path, &self.roots)?;

        if self.dry_run {
            let action = match self.mode {
                DeleteMode::Trash => Action::WouldTrash,
                DeleteMode::Purge => Action::WouldPurge,
            };
            return Ok(Self::outcome(candidate, action, None));
        }

        match self.mode {
            DeleteMode::Trash => {
                trash::delete(path).map_err(|err| VacuumError::Trash(err.to_string()))?;
                Ok(Self::outcome(candidate, Action::Trashed, None))
            }
            DeleteMode::Purge => {
                remove_path(path)?;
                Ok(Self::outcome(candidate, Action::Purged, None))
            }
        }
    }

    fn act_on_command(&self, candidate: &Candidate, spec: &CommandSpec) -> Result<Outcome> {
        if spec.needs_root {
            // Never escalate. Hand the exact command back to the user.
            let note = format!("requires root — run: {}", spec.display());
            return Ok(Self::outcome(candidate, Action::NeedsSudo, Some(note)));
        }

        if self.dry_run {
            return Ok(Self::outcome(
                candidate,
                Action::WouldRun,
                Some(spec.display()),
            ));
        }

        which::which(&spec.program).map_err(|_| VacuumError::ToolNotFound(spec.program.clone()))?;

        let status = Command::new(&spec.program)
            .args(&spec.args)
            .status()
            .map_err(|source| VacuumError::Io {
                path: PathBuf::from(&spec.program),
                source,
            })?;

        if !status.success() {
            return Err(VacuumError::CommandFailed {
                command: spec.display(),
                code: status.code().unwrap_or(-1),
            });
        }

        Ok(Self::outcome(candidate, Action::Ran, Some(spec.display())))
    }

    fn outcome(candidate: &Candidate, action: Action, note: Option<String>) -> Outcome {
        Outcome {
            label: candidate.label.clone(),
            bytes: candidate.bytes,
            action,
            note,
        }
    }
}

/// Permanently remove a path, dispatching on file vs directory.
fn remove_path(path: &PathBuf) -> Result<()> {
    let meta = std::fs::metadata(path).map_err(|source| VacuumError::Io {
        path: path.clone(),
        source,
    })?;
    let result = if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|source| VacuumError::Io {
        path: path.clone(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Action, DeleteMode, Deleter};
    use crate::cleaner::{Candidate, Category, Risk, Target};

    fn path_candidate(path: std::path::PathBuf) -> Candidate {
        Candidate {
            cleaner_id: "test".to_owned(),
            category: Category::BuildArtifacts,
            label: path.display().to_string(),
            detail: None,
            bytes: 0,
            regenerable: true,
            risk: Risk::Safe,
            target: Target::Path { path },
        }
    }

    #[test]
    fn dry_run_does_not_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir(&target).unwrap();
        let deleter = Deleter::new(DeleteMode::Purge, true, vec![tmp.path().to_path_buf()]);
        let outcome = deleter.execute(&path_candidate(target.clone())).unwrap();
        assert_eq!(outcome.action, Action::WouldPurge);
        assert!(target.exists(), "dry run must not remove anything");
    }

    #[test]
    fn purge_removes_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("artifact"), b"x").unwrap();
        let deleter = Deleter::new(DeleteMode::Purge, false, vec![tmp.path().to_path_buf()]);
        let outcome = deleter.execute(&path_candidate(target.clone())).unwrap();
        assert_eq!(outcome.action, Action::Purged);
        assert!(!target.exists());
    }

    #[test]
    fn purge_refuses_outside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let target = other.path().join("target");
        fs::create_dir(&target).unwrap();
        // Root list does not include `other`.
        let deleter = Deleter::new(DeleteMode::Purge, false, vec![tmp.path().to_path_buf()]);
        let err = deleter
            .execute(&path_candidate(target.clone()))
            .unwrap_err();
        assert_eq!(err.code(), "OUTSIDE_ROOTS");
        assert!(target.exists());
    }
}
