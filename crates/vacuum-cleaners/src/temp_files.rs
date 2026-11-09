// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Stale temporary-file cleaner: `/tmp`, `/var/tmp`, and `$TMPDIR`.
//!
//! This is the safe replacement for the `sudo rm -r /tmp/*` reflex. That command
//! destroys other users' files and the live state of running processes — X11 and
//! Wayland sockets, systemd private directories, in-flight builds. Instead, an
//! entry is offered only when *all* of the following hold:
//!
//! - it is not a symlink;
//! - it is owned by the invoking user (never another user's data, never root's);
//! - it has not been modified for [`MIN_AGE_DAYS`] days;
//! - its name is not on the [`LIVE_STATE`] denylist of session-state entries.
//!
//! Root-owned leftovers are deliberately out of scope here; they are reclaimed
//! through `sudo systemd-tmpfiles --clean`, which the package-GC cleaner offers
//! as a printed command (Vacuum never escalates on its own).
//!
//! Candidates are marked `trash_ok: false`: the freedesktop trash for a path
//! under `/tmp` is `/tmp/.Trash-$uid`, on the very same filesystem, so trashing
//! would reclaim nothing at exactly the moment the space is needed.
//!
//! Rust guideline compliant 2026-05-18

use std::collections::BTreeSet;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use vacuum_core::{Candidate, Category, Cleaner, Result, Risk, ScanContext, Target, dir_size};

/// How long an entry must sit untouched before it is offered for removal.
///
/// Seven days comfortably outlives an interactive session, a long build, and a
/// weekend away from the machine, so anything older is very unlikely to still be
/// in use. Lowering this risks offering the scratch space of a running job;
/// `systemd-tmpfiles` uses a comparable default for `/tmp`.
const MIN_AGE_DAYS: u64 = 7;

/// [`MIN_AGE_DAYS`] as a [`Duration`], for comparison against entry mtimes.
const MIN_AGE: Duration = Duration::from_secs(MIN_AGE_DAYS * 24 * 60 * 60);

/// The temp directories inspected, in report order.
///
/// `$TMPDIR` is added on top of these when it is set and points somewhere else.
const TEMP_DIRS: &[&str] = &["/tmp", "/var/tmp"];

/// Exact entry names that hold live session state and must never be offered.
///
/// These are sockets and directories the desktop session and the display server
/// keep open for their whole lifetime; their mtime is the session start, so an
/// age filter alone would happily nominate them after a week of uptime.
const LIVE_STATE: &[&str] = &[
    ".X11-unix",
    ".ICE-unix",
    ".font-unix",
    ".XIM-unix",
    ".Test-unix",
    ".XDG-unix",
];

/// Name prefixes that mark live or privileged state, matched against entry names.
///
/// `systemd-private-*` and `snap-private-*` are per-service bind-mount roots for
/// running units; `.Trash-` is the trash directory itself (deleting it would
/// destroy the user's own recovery copies); `.mount_` is a live FUSE mount.
const LIVE_STATE_PREFIXES: &[&str] = &[
    "systemd-private-",
    "snap-private-",
    ".Trash-",
    ".mount_",
    ".nfs",
];

/// Offers stale, user-owned entries under the system temporary directories.
#[derive(Debug, Clone, Copy)]
pub struct TempFiles;

impl Cleaner for TempFiles {
    fn id(&self) -> &'static str {
        "temp-files"
    }

    fn name(&self) -> &'static str {
        "Stale temp files"
    }

    fn category(&self) -> Category {
        Category::TempFiles
    }

    fn scan(&self, _ctx: &ScanContext) -> Result<Vec<Candidate>> {
        let uid = rustix::process::geteuid().as_raw();
        let now = SystemTime::now();

        let mut candidates = Vec::new();
        for dir in temp_dirs() {
            candidates.extend(collect(&dir, uid, MIN_AGE, now));
        }

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.bytes));
        Ok(candidates)
    }

    fn extra_roots(&self) -> Vec<PathBuf> {
        temp_dirs()
    }
}

/// The temp directories to inspect: [`TEMP_DIRS`] plus `$TMPDIR`, deduplicated.
///
/// Only existing directories are returned, so the delete allowlist never grows
/// to cover a path that is not actually there.
fn temp_dirs() -> Vec<PathBuf> {
    let from_env = std::env::var_os("TMPDIR").map(PathBuf::from);
    let mut seen = BTreeSet::new();

    TEMP_DIRS
        .iter()
        .map(PathBuf::from)
        .chain(from_env)
        .filter(|dir| dir.is_dir())
        // Canonicalize so `$TMPDIR=/tmp` and a `/tmp` symlink both collapse onto
        // the entry already present rather than widening the roots twice.
        .filter_map(|dir| std::fs::canonicalize(&dir).ok())
        .filter(|dir| seen.insert(dir.clone()))
        .collect()
}

/// Collect stale, `uid`-owned entries directly inside `dir`.
///
/// `now` and `min_age` are parameters rather than constants so tests can age
/// entries without backdating files on disk. Unreadable entries are skipped
/// rather than reported as errors: a temp directory is shared, racy, and full of
/// things this user cannot stat, and one `EACCES` must not abort the whole scan.
fn collect(dir: &Path, uid: u32, min_age: Duration, now: SystemTime) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if is_live_state(name) {
            continue;
        }

        // `symlink_metadata` inspects the link itself; a symlink is never
        // followed, and never removed (the deleter refuses them outright).
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        if meta.file_type().is_symlink() || meta.uid() != uid {
            continue;
        }

        let Some(age) = age_of(&meta, now) else {
            continue;
        };
        if age < min_age {
            continue;
        }

        let bytes = if meta.is_dir() {
            dir_size(&path)
        } else {
            meta.len()
        };
        if bytes == 0 {
            continue;
        }

        candidates.push(Candidate {
            cleaner_id: "temp-files".to_owned(),
            category: Category::TempFiles,
            label: path.display().to_string(),
            detail: Some(format!(
                "yours, untouched for {} days — purged, not trashed",
                age.as_secs() / (24 * 60 * 60)
            )),
            bytes,
            // Temp files are scratch data, not regenerable build output: whatever
            // wrote them will not put them back.
            regenerable: false,
            // Trashing a `/tmp` entry lands it in `/tmp/.Trash-$uid`, on the same
            // filesystem — zero bytes reclaimed. Purge outright instead.
            trash_ok: false,
            risk: Risk::Caution,
            target: Target::Path { path },
        });
    }

    candidates
}

/// Whether `name` denotes live session state that must never be offered.
fn is_live_state(name: &str) -> bool {
    LIVE_STATE.contains(&name)
        || LIVE_STATE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// How long ago `meta` was last modified, or `None` if that cannot be determined.
///
/// A future mtime yields `Some(ZERO)`, which fails the age test — clock skew
/// must not make an entry look ancient.
fn age_of(meta: &Metadata, now: SystemTime) -> Option<Duration> {
    let modified = meta.modified().ok()?;
    Some(now.duration_since(modified).unwrap_or(Duration::ZERO))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use super::{MIN_AGE, collect, is_live_state};

    /// Candidate labels, so an unexpected offer shows up in the failure message.
    fn labels(found: &[vacuum_core::Candidate]) -> Vec<&str> {
        found
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect()
    }

    /// A `now` far enough ahead that every file written by a test reads as stale.
    fn far_future() -> SystemTime {
        SystemTime::now() + Duration::from_secs(365 * 24 * 60 * 60)
    }

    fn uid() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    #[test]
    fn finds_aged_user_owned_entry() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("scratch.bin"), vec![0_u8; 4096]).unwrap();

        let found = collect(tmp.path(), uid(), MIN_AGE, far_future());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bytes, 4096);
        assert!(!found[0].trash_ok, "temp entries must not be trashed");
    }

    #[test]
    fn skips_entry_that_is_not_old_enough() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("scratch.bin"), vec![0_u8; 4096]).unwrap();

        // Real `now`: the file was written moments ago.
        let found = collect(tmp.path(), uid(), MIN_AGE, SystemTime::now());

        assert_eq!(
            labels(&found),
            Vec::<&str>::new(),
            "a fresh entry must never be offered"
        );
    }

    #[test]
    fn skips_entry_owned_by_another_user() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("scratch.bin"), vec![0_u8; 4096]).unwrap();

        // Ask for a uid that cannot match the one the file was created with.
        let found = collect(tmp.path(), uid().wrapping_add(1), MIN_AGE, far_future());

        assert_eq!(
            labels(&found),
            Vec::<&str>::new(),
            "another user's data is never offered"
        );
    }

    #[test]
    fn skips_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.bin");
        fs::write(&real, vec![0_u8; 4096]).unwrap();
        std::os::unix::fs::symlink(&real, tmp.path().join("link.bin")).unwrap();

        let found = collect(tmp.path(), uid(), MIN_AGE, far_future());

        assert_eq!(found.len(), 1, "only the real file, never the symlink");
        assert!(found[0].label.ends_with("real.bin"));
    }

    #[test]
    fn skips_live_session_state() {
        let tmp = tempfile::tempdir().unwrap();
        for name in [".X11-unix", "systemd-private-abc", ".Trash-1000"] {
            let dir = tmp.path().join(name);
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join("blob"), vec![0_u8; 4096]).unwrap();
        }

        let found = collect(tmp.path(), uid(), MIN_AGE, far_future());

        assert_eq!(
            labels(&found),
            Vec::<&str>::new(),
            "live session state is never offered"
        );
    }

    #[test]
    fn live_state_matching_covers_names_and_prefixes() {
        assert!(is_live_state(".X11-unix"));
        assert!(is_live_state("systemd-private-9f2c"));
        assert!(is_live_state(".Trash-1000"));
        assert!(!is_live_state("cargo-installXYZ"));
        assert!(!is_live_state("nix-build-foo.drv-0"));
    }
}
