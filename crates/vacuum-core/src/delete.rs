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
    /// Omitted from JSON output when absent (token-economy hygiene, the CLI Standard §8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Notes that a candidate was purged despite the trash mode being selected.
///
/// Surfaced on [`Outcome::note`] so the override is never silent — the user
/// asked for a recoverable delete and is getting a permanent one instead. The
/// *reason* is candidate-specific and lives in [`Candidate::detail`], since it
/// differs per cleaner (a temp file's trash sits on the same filesystem; a
/// build-artifact batch would otherwise scatter hundreds of trash entries).
const PURGED_NOT_TRASHED: &str = "purged, not trashed — see the candidate's detail for why";

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
            Target::Batch { paths, .. } => self.act_on_batch(candidate, paths),
        }
    }

    /// Remove every path in a batch, reporting one aggregate [`Outcome`].
    ///
    /// The safety gate runs over *all* paths before anything is removed, so a
    /// batch containing even one refused path is rejected whole rather than
    /// half-applied — a partially pruned build tree is worse than an untouched
    /// one. Failures during removal itself are tolerated (a concurrent build may
    /// legitimately have taken a file away) and summarised in the note.
    fn act_on_batch(&self, candidate: &Candidate, paths: &[PathBuf]) -> Result<Outcome> {
        // A path that has already gone is not a failure — another candidate in
        // the same run may legitimately have removed the tree containing it
        // (the whole-directory and per-unit candidates for one Cargo target
        // deliberately overlap). Drop those before the safety gate, which would
        // otherwise report them as unreadable.
        let paths: Vec<&PathBuf> = paths
            .iter()
            .filter(|path| path.symlink_metadata().is_ok())
            .collect();

        for path in &paths {
            safety::check_deletable(path, &self.roots)?;
        }

        // A batch is many small regenerable files; the trash is the wrong home
        // for them, so cleaners mark these `trash_ok: false` and they purge.
        let effective_mode = if candidate.trash_ok {
            self.mode
        } else {
            DeleteMode::Purge
        };

        if self.dry_run {
            let action = match effective_mode {
                DeleteMode::Trash => Action::WouldTrash,
                DeleteMode::Purge => Action::WouldPurge,
            };
            return Ok(Self::outcome(
                candidate,
                action,
                Some(format!("{} path(s)", paths.len())),
            ));
        }

        let mut failed = 0_usize;
        for path in &paths {
            let removed = match effective_mode {
                DeleteMode::Trash => {
                    trash::delete(path).map_err(|err| VacuumError::Trash(err.to_string()))
                }
                DeleteMode::Purge => remove_path(path),
            };
            if removed.is_err() {
                failed += 1;
            }
        }

        let action = match effective_mode {
            DeleteMode::Trash => Action::Trashed,
            DeleteMode::Purge => Action::Purged,
        };
        let note = if failed == 0 {
            format!("{} path(s)", paths.len())
        } else {
            format!("{} path(s), {failed} could not be removed", paths.len())
        };
        Ok(Self::outcome(candidate, action, Some(note)))
    }

    fn act_on_path(&self, candidate: &Candidate, path: &PathBuf) -> Result<Outcome> {
        // The safety gate runs even on a dry run, so a dry run reliably
        // predicts what an `--apply` run would refuse.
        safety::check_deletable(path, &self.roots)?;

        // A candidate whose trash directory shares its filesystem reclaims
        // nothing when trashed, so purge it instead of reporting a phantom
        // saving. The note makes the override visible in every output mode.
        let effective_mode = if candidate.trash_ok {
            self.mode
        } else {
            DeleteMode::Purge
        };
        let note = (!candidate.trash_ok && self.mode == DeleteMode::Trash)
            .then(|| PURGED_NOT_TRASHED.to_owned());

        if self.dry_run {
            let action = match effective_mode {
                DeleteMode::Trash => Action::WouldTrash,
                DeleteMode::Purge => Action::WouldPurge,
            };
            return Ok(Self::outcome(candidate, action, note));
        }

        match effective_mode {
            DeleteMode::Trash => {
                trash::delete(path).map_err(|err| VacuumError::Trash(err.to_string()))?;
                Ok(Self::outcome(candidate, Action::Trashed, note))
            }
            DeleteMode::Purge => {
                remove_path(path)?;
                Ok(Self::outcome(candidate, Action::Purged, note))
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
            trash_ok: true,
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
    fn dry_run_reports_purge_when_trash_would_not_reclaim() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("stale");
        fs::create_dir(&target).unwrap();
        let mut candidate = path_candidate(target);
        candidate.trash_ok = false;

        // Trash mode requested, but the candidate opts out of it.
        let deleter = Deleter::new(DeleteMode::Trash, true, vec![tmp.path().to_path_buf()]);
        let outcome = deleter.execute(&candidate).unwrap();

        assert_eq!(outcome.action, Action::WouldPurge);
        assert!(
            outcome
                .note
                .is_some_and(|note| note.contains("purged, not trashed")),
            "the trash override must be explained to the user"
        );
    }

    #[test]
    fn trash_mode_purges_when_trash_would_not_reclaim() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("stale");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("blob"), b"x").unwrap();
        let mut candidate = path_candidate(target.clone());
        candidate.trash_ok = false;

        let deleter = Deleter::new(DeleteMode::Trash, false, vec![tmp.path().to_path_buf()]);
        let outcome = deleter.execute(&candidate).unwrap();

        assert_eq!(outcome.action, Action::Purged);
        assert!(!target.exists());
    }

    #[test]
    fn trash_ok_candidates_are_unaffected_by_the_override() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        fs::create_dir(&target).unwrap();

        let deleter = Deleter::new(DeleteMode::Trash, true, vec![tmp.path().to_path_buf()]);
        let outcome = deleter.execute(&path_candidate(target)).unwrap();

        assert_eq!(outcome.action, Action::WouldTrash);
        assert!(outcome.note.is_none());
    }

    fn batch_candidate(root: std::path::PathBuf, paths: Vec<std::path::PathBuf>) -> Candidate {
        Candidate {
            cleaner_id: "test".to_owned(),
            category: Category::BuildArtifacts,
            label: root.display().to_string(),
            detail: None,
            bytes: 0,
            regenerable: true,
            trash_ok: false,
            risk: Risk::Safe,
            target: Target::Batch { root, paths },
        }
    }

    #[test]
    fn batch_dry_run_deletes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let one = tmp.path().join("one");
        let two = tmp.path().join("two");
        fs::write(&one, b"x").unwrap();
        fs::write(&two, b"y").unwrap();

        let deleter = Deleter::new(DeleteMode::Trash, true, vec![tmp.path().to_path_buf()]);
        let candidate = batch_candidate(tmp.path().to_path_buf(), vec![one.clone(), two.clone()]);
        let outcome = deleter.execute(&candidate).unwrap();

        assert_eq!(outcome.action, Action::WouldPurge);
        assert!(one.exists() && two.exists());
    }

    #[test]
    fn batch_apply_removes_every_path() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        let dir = tmp.path().join("dir");
        fs::write(&file, b"x").unwrap();
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("nested"), b"y").unwrap();

        let deleter = Deleter::new(DeleteMode::Purge, false, vec![tmp.path().to_path_buf()]);
        let candidate = batch_candidate(tmp.path().to_path_buf(), vec![file.clone(), dir.clone()]);
        let outcome = deleter.execute(&candidate).unwrap();

        assert_eq!(outcome.action, Action::Purged);
        assert!(!file.exists() && !dir.exists());
    }

    #[test]
    fn batch_is_refused_whole_when_one_path_is_outside_the_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let inside = tmp.path().join("inside");
        let outside = other.path().join("outside");
        fs::write(&inside, b"x").unwrap();
        fs::write(&outside, b"y").unwrap();

        let deleter = Deleter::new(DeleteMode::Purge, false, vec![tmp.path().to_path_buf()]);
        let candidate = batch_candidate(
            tmp.path().to_path_buf(),
            vec![inside.clone(), outside.clone()],
        );
        let err = deleter.execute(&candidate).unwrap_err();

        assert_eq!(err.code(), "OUTSIDE_ROOTS");
        assert!(inside.exists(), "a rejected batch must not be half-applied");
        assert!(outside.exists());
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
