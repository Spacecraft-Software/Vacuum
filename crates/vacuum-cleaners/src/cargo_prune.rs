// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Prune dead build units from Cargo `target/` directories.
//!
//! This complements the whole-directory build-artifacts cleaner. That one
//! removes a `target/` outright, costing a full rebuild; this one removes only
//! the units that are already dead, so an active project keeps its warm
//! incremental cache.
//!
//! # Why this is native
//!
//! The obvious implementation is to shell out to `cargo-sweep`. Vacuum does not,
//! for three reasons. `cargo-sweep` is unmaintained. It requires `cargo
//! metadata`, `rustup toolchain list`, and `rustc -vV` to run — none of which
//! work on a system whose toolchain is managed by Nix or Guix rather than
//! rustup, which is precisely where this tool is used. And its age filter reads
//! *atime*, which under the near-universal `relatime` mount option is not
//! updated when Cargo reuses an artifact, so it deletes warm caches.
//!
//! Vacuum executes nothing. Everything below is read off the filesystem.
//!
//! # How a dead unit is recognised
//!
//! A build unit is a directory `target/<profile>/.fingerprint/<pkg>-<hash>/`.
//! Its sibling outputs in `deps/`, `build/`, and the profile root share the same
//! 16-hex `<hash>` suffix, which is the only reliable way to correlate them —
//! `.fingerprint` spells the package name with hyphens while `deps` spells the
//! crate name with underscores.
//!
//! Each unit's `*.json` records `"rustc": <u64>`, Cargo's hash of the compiler's
//! `rustc -vV` output. Units are grouped by that value; the group with the most
//! recent activity is the toolchain in use, and every other group is left over
//! from a compiler that is no longer building this project.
//!
//! Freshness comes from the mtime of `.fingerprint/<unit>/invoked.timestamp`,
//! the file Cargo writes for exactly this purpose, falling back to the unit
//! directory's own mtime for the build-script units that have none.
//!
//! Rust guideline compliant 2026-05-18

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use vacuum_core::{
    Candidate, Category, Cleaner, Result, Risk, ScanContext, Target, sum_unique_files,
};

use crate::rustc_hash::{hash_verbose_version, toolchain_release};

/// The magic first line of a `CACHEDIR.TAG`, per the cache-directory-tagging
/// specification (<https://bford.info/cachedir/>).
///
/// Cargo writes this file at the root of every `target/` directory. Matching on
/// it identifies a target directory wherever it lives, which a search for the
/// literal name `target` would not: `CARGO_TARGET_DIR` and `build.target-dir`
/// both rename it freely.
const CACHEDIR_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

/// Directories inside a profile whose entries are named `<pkg>-<hash>` and are
/// therefore correlatable to a fingerprint unit.
///
/// `examples/` and the profile root's unhashed final artifacts are excluded on
/// purpose: they carry no hash, and the profile root's binaries are hardlinks of
/// files in `deps/` that Cargo re-creates on the next build.
const UNIT_DIRS: &[&str] = &["deps", "build", "native", ".fingerprint"];

/// How many hex characters a Cargo unit hash has.
const HASH_LEN: usize = 16;

/// The `"rustc"` value Cargo writes for fingerprints that track a build script's
/// output; it does not correspond to any real compiler.
///
/// Treated as always-live so these units are never swept as "old toolchain".
const BUILD_SCRIPT_RUSTC: u64 = 0;

/// Prunes dead build units out of Cargo target directories.
#[derive(Debug, Clone, Copy)]
pub struct CargoPrune;

impl Cleaner for CargoPrune {
    fn id(&self) -> &'static str {
        "cargo-prune"
    }

    fn name(&self) -> &'static str {
        "Stale Cargo build units"
    }

    fn category(&self) -> Category {
        Category::BuildArtifacts
    }

    fn scan(&self, ctx: &ScanContext) -> Result<Vec<Candidate>> {
        let max_age = Duration::from_secs(ctx.stale_days * 24 * 60 * 60);
        let now = SystemTime::now();

        let targets: Vec<PathBuf> = ctx
            .roots
            .iter()
            .flat_map(|root| find_target_dirs(root))
            .collect();

        // Names are pooled across every target directory found. A directory
        // records only the compiler that built it *last*, so the toolchain a
        // stale unit came from is usually nameable only because some other
        // project on this machine still builds with it.
        let mut names = ToolchainNames::default();
        for target in &targets {
            names.absorb(target);
        }

        let mut candidates = Vec::new();
        for target in &targets {
            candidates.extend(prune_plan(target, &names, max_age, now));
        }

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.bytes));
        Ok(candidates)
    }
}

// --- discovery ------------------------------------------------------------

/// Find every Cargo target directory beneath `root`.
///
/// A directory qualifies when it holds a `CACHEDIR.TAG` carrying the Cargo
/// signature. Matched directories are not descended into: a target directory
/// never contains another.
fn find_target_dirs(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if is_target_dir(&dir) {
            found.push(dir);
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // Never follow a symlink out of the tree being scanned.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(entry.path());
            }
        }
    }

    found.sort();
    found
}

/// Whether `dir` is a Cargo target directory.
fn is_target_dir(dir: &Path) -> bool {
    let Ok(tag) = std::fs::read_to_string(dir.join("CACHEDIR.TAG")) else {
        return false;
    };
    tag.lines().next() == Some(CACHEDIR_SIGNATURE)
}

/// Whether a Cargo build currently holds the lock on `profile`.
///
/// Cargo takes an exclusive `flock` on `<profile>/.cargo-lock` for the duration
/// of a build. Pruning underneath a running build would delete artifacts it is
/// about to link, so a locked profile is skipped entirely. The probe takes the
/// lock non-blockingly and drops it immediately; failing to acquire it is the
/// signal, not an error.
fn build_in_progress(profile: &Path) -> bool {
    use rustix::fs::{FlockOperation, flock};

    let lock = profile.join(".cargo-lock");
    let Ok(file) = std::fs::File::open(&lock) else {
        // No lock file means Cargo has never built here, so nothing is running.
        return false;
    };
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {
            let _ = flock(&file, FlockOperation::Unlock);
            false
        }
        Err(_) => true,
    }
}

/// The profile directories inside `target` that hold build units.
///
/// Handles both the common `target/<profile>/` layout and the
/// `target/<triple>/<profile>/` layout produced by `--target`.
fn profile_dirs(target: &Path) -> Vec<PathBuf> {
    let mut profiles = Vec::new();
    let Ok(entries) = std::fs::read_dir(target) else {
        return profiles;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join(".fingerprint").is_dir() {
            profiles.push(path);
            continue;
        }
        // A target-triple directory: the profiles are one level further down.
        let Ok(nested) = std::fs::read_dir(&path) else {
            continue;
        };
        for inner in nested.flatten() {
            let inner = inner.path();
            if inner.join(".fingerprint").is_dir() {
                profiles.push(inner);
            }
        }
    }

    profiles.sort();
    profiles
}

// --- fingerprints ---------------------------------------------------------

/// The only field of a fingerprint JSON this cleaner needs.
#[derive(Debug, Deserialize)]
struct Fingerprint {
    rustc: u64,
}

/// One build unit: its hash suffix, the compiler that produced it, and when it
/// was last built.
#[derive(Debug, Clone)]
struct Unit {
    hash: String,
    rustc: u64,
    last_used: SystemTime,
}

/// Read every build unit recorded under `profile`.
///
/// A unit whose fingerprint JSON is missing or unreadable is reported with
/// [`BUILD_SCRIPT_RUSTC`], which keeps it: an artifact whose provenance cannot
/// be established is never swept.
fn read_units(profile: &Path) -> Vec<Unit> {
    let Ok(entries) = std::fs::read_dir(profile.join(".fingerprint")) else {
        return Vec::new();
    };

    let mut units = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(hash) = dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(hash_suffix)
        else {
            continue;
        };

        units.push(Unit {
            hash: hash.to_owned(),
            rustc: read_rustc(&dir).unwrap_or(BUILD_SCRIPT_RUSTC),
            last_used: last_used(&dir),
        });
    }

    units
}

/// The `"rustc"` value from the first parseable `*.json` in a unit directory.
fn read_rustc(unit: &Path) -> Option<u64> {
    let entries = std::fs::read_dir(unit).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(fingerprint) = serde_json::from_str::<Fingerprint>(&text) {
            return Some(fingerprint.rustc);
        }
    }
    None
}

/// When a build unit was last produced.
///
/// `invoked.timestamp` is written by Cargo at the start of every build of the
/// unit and is the intended signal. The build-script units that lack it fall
/// back to the directory's own mtime. Access time is deliberately never
/// consulted — under `relatime` it is not updated when Cargo reuses an artifact,
/// which is the bug that makes `cargo-sweep --time` delete live caches.
fn last_used(unit: &Path) -> SystemTime {
    let stamp = mtime(&unit.join("invoked.timestamp"));
    let dir = mtime(unit);
    match (stamp, dir) {
        (Some(a), Some(b)) => a.max(b),
        (Some(only), None) | (None, Some(only)) => only,
        // Unreadable: treat as brand new so it is kept, never swept.
        (None, None) => SystemTime::now(),
    }
}

/// The modification time of `path`, if it can be read.
fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Extract the trailing 16-hex Cargo unit hash from a file or directory name.
///
/// Accepts `({prefix}-)?{name}-{16 hex}(.{ext})?`, which is how Cargo names
/// every correlatable artifact. Returns `None` for the unhashed final artifacts
/// at a profile root (`vacuum`, `libvacuum_core.rlib`), which must never be
/// swept — they are hardlinks Cargo recreates, and removing them buys nothing.
fn hash_suffix(name: &str) -> Option<&str> {
    let stem = name.split('.').next()?;
    let hash = stem.rsplit('-').next()?;
    if hash.len() != HASH_LEN || hash.len() == stem.len() {
        return None;
    }
    hash.chars()
        .all(|character| character.is_ascii_hexdigit())
        .then_some(hash)
}

// --- planning -------------------------------------------------------------

/// Why a set of units is being offered for removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    /// Built by a compiler this project no longer uses.
    OldToolchain,
    /// Built by the current compiler, but not in a long time.
    Cold,
}

/// Build the removal plan for one target directory.
fn prune_plan(
    target: &Path,
    label_of: &ToolchainNames,
    max_age: Duration,
    now: SystemTime,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    for profile in profile_dirs(target) {
        if build_in_progress(&profile) {
            continue;
        }

        // Note this runs even when there are no fingerprints left to classify:
        // a profile whose units have already been pruned can still be sitting on
        // a large incremental cache.
        candidates.extend(incremental_candidate(target, &profile));

        let units = read_units(&profile);
        if units.is_empty() {
            continue;
        }

        let live = live_rustc(&units);
        let mut old_toolchain: Vec<&Unit> = Vec::new();
        let mut cold: Vec<&Unit> = Vec::new();
        for unit in &units {
            // Build-script bookkeeping units have no real compiler; never sweep.
            if unit.rustc == BUILD_SCRIPT_RUSTC {
                continue;
            }
            if unit.rustc != live {
                old_toolchain.push(unit);
            } else if age(unit.last_used, now) >= max_age {
                cold.push(unit);
            }
        }

        // The two sets are disjoint by construction, so their bytes never
        // double-count. Reported in a fixed order, not a map's iteration order.
        let groups = [(Reason::OldToolchain, old_toolchain), (Reason::Cold, cold)];
        for (reason, stale) in groups {
            if stale.is_empty() {
                continue;
            }
            let hashes: HashSet<&str> = stale.iter().map(|unit| unit.hash.as_str()).collect();
            let paths = paths_for_hashes(&profile, &hashes);
            if paths.is_empty() {
                continue;
            }
            let bytes = sum_unique_files(&paths);
            if bytes == 0 {
                continue;
            }

            let (summary, detail) = match reason {
                Reason::OldToolchain => {
                    let from = stale
                        .first()
                        .and_then(|unit| label_of.name(unit.rustc))
                        .unwrap_or_else(|| "an older toolchain".to_owned());
                    let summary = format!("{} units from {from}", stale.len());
                    let detail = match label_of.name(live) {
                        Some(to) => format!("{summary}; this target now builds with {to}"),
                        None => summary.clone(),
                    };
                    (summary, detail)
                }
                Reason::Cold => {
                    let days = max_age.as_secs() / (24 * 60 * 60);
                    let summary = format!("{} units cold for {days}+ days", stale.len());
                    let detail = format!(
                        "{summary}; built by the toolchain still in use, but not rebuilt since"
                    );
                    (summary, detail)
                }
            };

            candidates.push(batch(target, &profile, paths, bytes, &summary, &detail));
        }
    }

    candidates
}

/// The incremental-cache candidate for one profile, if it holds anything.
///
/// The incremental cache is untracked by fingerprints and is pure rebuild-time
/// state. `cargo-sweep` never reclaims it, which is the main reason its reported
/// savings fall so far short of a `cargo clean`.
fn incremental_candidate(target: &Path, profile: &Path) -> Option<Candidate> {
    let incremental = profile.join("incremental");
    if !incremental.is_dir() {
        return None;
    }
    let paths = vec![incremental];
    let bytes = sum_unique_files(&paths);
    (bytes > 0).then(|| {
        batch(
            target,
            profile,
            paths,
            bytes,
            "incremental cache",
            "incremental compilation cache; costs one non-incremental rebuild",
        )
    })
}

/// The `rustc` value of the group that was built most recently.
///
/// Grouping is exact — it is Cargo's own per-compiler value. Only the choice of
/// *which* group is current is inferred, and the most recently built one is it.
fn live_rustc(units: &[Unit]) -> u64 {
    units
        .iter()
        .filter(|unit| unit.rustc != BUILD_SCRIPT_RUSTC)
        .max_by_key(|unit| unit.last_used)
        .map_or(BUILD_SCRIPT_RUSTC, |unit| unit.rustc)
}

/// How long ago `when` was, saturating at zero for future timestamps.
fn age(when: SystemTime, now: SystemTime) -> Duration {
    now.duration_since(when).unwrap_or(Duration::ZERO)
}

/// Every path under `profile` belonging to one of `hashes`.
fn paths_for_hashes(profile: &Path, hashes: &HashSet<&str>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in UNIT_DIRS {
        let Ok(entries) = std::fs::read_dir(profile.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let matches = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(hash_suffix)
                .is_some_and(|hash| hashes.contains(hash));
            if matches {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

/// Assemble a batch candidate for one profile.
///
/// `summary` goes in the label, because the human `list` output prints only the
/// size and the label: without it, a target directory's two or three rows would
/// be indistinguishable. `detail` carries the full explanation for JSON
/// consumers and the TUI.
fn batch(
    target: &Path,
    profile: &Path,
    paths: Vec<PathBuf>,
    bytes: u64,
    summary: &str,
    detail: &str,
) -> Candidate {
    let profile_name = profile
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?");
    Candidate {
        cleaner_id: "cargo-prune".to_owned(),
        category: Category::BuildArtifacts,
        label: format!("{} [{profile_name}] {summary}", target.display()),
        detail: Some(format!(
            "{detail}. Purged rather than trashed: these are many small \
             regenerable files, and trashing them would reclaim nothing until \
             the trash is emptied. This overlaps the whole-directory candidate \
             for the same target; take one or the other, not both."
        )),
        bytes,
        regenerable: true,
        trash_ok: false,
        risk: Risk::Safe,
        target: Target::Batch {
            root: target.to_path_buf(),
            paths,
        },
    }
}

// --- toolchain naming -----------------------------------------------------

/// Maps a fingerprint's `"rustc"` value to a human-readable release.
///
/// Built from `target/.rustc_info.json`, where Cargo caches the verbatim
/// `rustc -vV` output of the compiler it last used. Hashing that text reproduces
/// the fingerprint value exactly, so a toolchain can be named without running
/// anything. Purely cosmetic: when the mapping cannot be established the units
/// are still grouped and swept correctly, they are just described generically.
#[derive(Debug, Default)]
struct ToolchainNames {
    by_hash: HashMap<u64, String>,
}

impl ToolchainNames {
    /// Add the toolchains recorded in `target/.rustc_info.json` to the table.
    ///
    /// Called for every target directory in a scan, because a directory names
    /// only the compiler that built it last. The toolchain behind a *stale* unit
    /// is therefore usually nameable only thanks to another project that still
    /// builds with it.
    fn absorb(&mut self, target: &Path) {
        let Ok(text) = std::fs::read_to_string(target.join(".rustc_info.json")) else {
            return;
        };
        let Ok(info) = serde_json::from_str::<RustcInfo>(&text) else {
            return;
        };

        for output in info.outputs.into_values() {
            // A failed probe records an empty stdout; there is nothing to name,
            // and hashing it would invent a bogus mapping.
            if !output.success || !output.stdout.starts_with("rustc ") {
                continue;
            }
            if let Some(release) = toolchain_release(&output.stdout) {
                for hash in hash_verbose_version(&output.stdout) {
                    self.by_hash.insert(hash, release.clone());
                }
            }
        }
    }

    /// Read the toolchains recorded in a single target directory.
    #[cfg(test)]
    fn read(target: &Path) -> Self {
        let mut names = Self::default();
        names.absorb(target);
        names
    }

    /// The release name for a fingerprint's `"rustc"` value, if known.
    fn name(&self, rustc: u64) -> Option<String> {
        self.by_hash.get(&rustc).cloned()
    }
}

/// The parts of `.rustc_info.json` needed to name a toolchain.
#[derive(Debug, Deserialize)]
struct RustcInfo {
    outputs: HashMap<String, RustcOutput>,
}

/// One cached compiler probe.
#[derive(Debug, Deserialize)]
struct RustcOutput {
    success: bool,
    stdout: String,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    use vacuum_core::Target;

    use super::{
        CACHEDIR_SIGNATURE, Reason, ToolchainNames, build_in_progress, find_target_dirs,
        hash_suffix, is_target_dir, live_rustc, prune_plan, read_units,
    };

    /// The `rustc -vV` text Cargo recorded for rustc 1.95.0, captured verbatim
    /// from a real `target/.rustc_info.json` on the maintainer's machine.
    const VV_1_95: &str = "rustc 1.95.0 (59807616e 2026-04-14) (built from a source tarball)\nbinary: rustc\ncommit-hash: 59807616e1fa2540724bfbac14d7976d7e4a3860\ncommit-date: 2026-04-14\nhost: x86_64-unknown-linux-gnu\nrelease: 1.95.0\nLLVM version: 21.1.8\n";

    /// The `"rustc"` value every fingerprint in that target directory carries.
    const HASH_1_95: u64 = 9_571_511_559_510_505_644;

    /// Build a minimal but realistic target directory.
    fn target_dir(root: &Path) {
        fs::create_dir_all(root).unwrap();
        fs::write(
            root.join("CACHEDIR.TAG"),
            format!("{CACHEDIR_SIGNATURE}\n# created by cargo\n"),
        )
        .unwrap();
    }

    /// Add one build unit to `profile`, with its fingerprint and its outputs.
    fn unit(profile: &Path, name: &str, hash: &str, rustc: u64, bytes: usize) {
        let fingerprint = profile.join(".fingerprint").join(format!("{name}-{hash}"));
        fs::create_dir_all(&fingerprint).unwrap();
        fs::write(
            fingerprint.join(format!("lib-{name}.json")),
            format!("{{\"rustc\":{rustc},\"features\":\"\"}}"),
        )
        .unwrap();
        fs::write(fingerprint.join("invoked.timestamp"), b"x").unwrap();

        let deps = profile.join("deps");
        fs::create_dir_all(&deps).unwrap();
        fs::write(
            deps.join(format!("lib{name}-{hash}.rlib")),
            vec![0_u8; bytes],
        )
        .unwrap();
    }

    #[test]
    fn hash_extraction_accepts_cargo_artifacts_and_rejects_final_binaries() {
        assert_eq!(
            hash_suffix("libaho_corasick-9919f4a5cce3e50c.rlib"),
            Some("9919f4a5cce3e50c")
        );
        assert_eq!(
            hash_suffix("aho-corasick-7b8ead2bb4eea6e3"),
            Some("7b8ead2bb4eea6e3")
        );
        // The unhashed final artifacts must never be swept.
        assert_eq!(hash_suffix("vacuum"), None);
        assert_eq!(hash_suffix("libvacuum_core.rlib"), None);
        assert_eq!(hash_suffix("incremental"), None);
        // Wrong length and non-hex are both rejected.
        assert_eq!(hash_suffix("thing-abc"), None);
        assert_eq!(hash_suffix("thing-zzzzzzzzzzzzzzzz"), None);
    }

    #[test]
    fn identifies_a_target_dir_by_its_cachedir_tag() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("proj/target");
        target_dir(&target);
        assert!(is_target_dir(&target));
        assert!(!is_target_dir(tmp.path()));

        // Discovery finds it wherever it sits, under any name.
        let renamed = tmp.path().join("proj/build-output");
        target_dir(&renamed);
        let found = find_target_dirs(tmp.path());
        assert_eq!(found.len(), 2, "found {found:?}");
    }

    #[test]
    fn sweeps_the_older_toolchain_group_and_keeps_the_newer() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        target_dir(&target);
        let profile = target.join("debug");

        // Two compilers represented. The one written last is the live one.
        unit(&profile, "old_crate", "1111111111111111", 111, 4096);
        std::thread::sleep(Duration::from_millis(20));
        unit(&profile, "new_crate", "2222222222222222", 222, 4096);

        let units = read_units(&profile);
        assert_eq!(units.len(), 2);
        assert_eq!(live_rustc(&units), 222, "newest activity defines live");

        // A long max_age so nothing qualifies as merely cold.
        let plan = prune_plan(
            &target,
            &ToolchainNames::default(),
            Duration::from_secs(86_400 * 365),
            SystemTime::now(),
        );
        assert_eq!(plan.len(), 1, "only the old-toolchain group: {plan:?}");
        assert!(!plan[0].trash_ok);
        assert!(plan[0].regenerable);

        // The batch covers the whole unit — its fingerprint directory as well as
        // its output in deps/ — so it weighs slightly more than the rlib alone.
        assert!(
            plan[0].bytes >= 4096,
            "expected at least the rlib, got {}",
            plan[0].bytes
        );
        let Target::Batch { paths, .. } = &plan[0].target else {
            panic!("expected a batch target, got {:?}", plan[0].target);
        };
        let names: Vec<String> = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        assert!(
            names.iter().any(|name| name.contains(".fingerprint")),
            "the unit's fingerprint must be swept with it: {names:?}"
        );
        assert!(
            names
                .iter()
                .any(|name| name.ends_with("libold_crate-1111111111111111.rlib")),
            "the unit's output must be swept: {names:?}"
        );
        assert!(
            names.iter().all(|name| !name.contains("new_crate")),
            "the live toolchain's units must be untouched: {names:?}"
        );
    }

    #[test]
    fn cold_units_are_offered_separately_from_stale_toolchains() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        target_dir(&target);
        let profile = target.join("debug");
        unit(&profile, "only_crate", "3333333333333333", 333, 8192);

        // A future `now` makes the single (live) unit read as cold.
        let far_future = SystemTime::now() + Duration::from_secs(86_400 * 365);
        let plan = prune_plan(
            &target,
            &ToolchainNames::default(),
            Duration::from_secs(86_400 * 30),
            far_future,
        );

        assert_eq!(plan.len(), 1);
        assert!(
            plan[0]
                .detail
                .as_ref()
                .is_some_and(|d| d.contains("not rebuilt")),
            "expected a cold-unit detail, got {:?}",
            plan[0].detail
        );
    }

    #[test]
    fn a_unit_with_unreadable_fingerprint_json_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        target_dir(&target);
        let profile = target.join("debug");

        unit(&profile, "good", "4444444444444444", 444, 4096);
        // A unit whose JSON does not parse: provenance unknown, so keep it.
        let broken = profile.join(".fingerprint").join("broken-5555555555555555");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("lib-broken.json"), b"{not json").unwrap();
        fs::write(
            profile.join("deps").join("libbroken-5555555555555555.rlib"),
            vec![0_u8; 999_999],
        )
        .unwrap();

        let plan = prune_plan(
            &target,
            &ToolchainNames::default(),
            Duration::from_secs(86_400 * 365),
            SystemTime::now(),
        );
        let swept: u64 = plan.iter().map(|candidate| candidate.bytes).sum();
        assert_eq!(swept, 0, "nothing should be swept: {plan:?}");
    }

    #[test]
    fn incremental_cache_is_offered_on_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        target_dir(&target);
        let incremental = target.join("debug/incremental/thing-abc");
        fs::create_dir_all(&incremental).unwrap();
        fs::write(incremental.join("blob"), vec![0_u8; 16_384]).unwrap();
        // A `.fingerprint` dir is what marks this as a profile.
        fs::create_dir_all(target.join("debug/.fingerprint")).unwrap();

        let plan = prune_plan(
            &target,
            &ToolchainNames::default(),
            Duration::from_secs(86_400 * 30),
            SystemTime::now(),
        );
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].bytes, 16_384);
        assert!(
            plan[0]
                .detail
                .as_ref()
                .is_some_and(|d| d.contains("incremental")),
            "got {:?}",
            plan[0].detail
        );
    }

    #[test]
    fn a_locked_profile_is_skipped_entirely() {
        use rustix::fs::{FlockOperation, flock};

        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        target_dir(&target);
        let profile = target.join("debug");
        unit(&profile, "crate_a", "6666666666666666", 666, 4096);
        std::thread::sleep(Duration::from_millis(20));
        unit(&profile, "crate_b", "7777777777777777", 777, 4096);

        let lock_path = profile.join(".cargo-lock");
        fs::write(&lock_path, b"").unwrap();
        assert!(!build_in_progress(&profile), "an idle lock is not a build");

        // Hold the lock as a running `cargo build` would.
        let held = fs::File::open(&lock_path).unwrap();
        flock(&held, FlockOperation::NonBlockingLockExclusive).unwrap();

        assert!(build_in_progress(&profile));
        let plan = prune_plan(
            &target,
            &ToolchainNames::default(),
            Duration::from_secs(86_400 * 365),
            SystemTime::now(),
        );
        assert!(
            plan.is_empty(),
            "must not prune under a live build: {plan:?}"
        );

        flock(&held, FlockOperation::Unlock).unwrap();
    }

    #[test]
    fn toolchain_names_come_from_rustc_info_json() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        target_dir(&target);
        let info = serde_json::json!({
            "rustc_fingerprint": 1_u64,
            "outputs": {
                "17747080675513052775": {
                    "success": true, "status": "", "code": 0,
                    "stdout": VV_1_95, "stderr": ""
                }
            },
            "successes": {}
        });
        fs::write(
            target.join(".rustc_info.json"),
            serde_json::to_string(&info).unwrap(),
        )
        .unwrap();

        let names = ToolchainNames::read(&target);
        assert_eq!(
            names.name(HASH_1_95).as_deref(),
            Some("rustc 1.95.0"),
            "the recorded -vV text must hash to the fingerprint value"
        );
        assert_eq!(names.name(12_019_306_335_353_385_202), None);
    }

    #[test]
    fn a_failed_rustc_probe_names_nothing_and_does_not_panic() {
        // Real case: exif-remover/target on the maintainer's machine.
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        target_dir(&target);
        fs::write(
            target.join(".rustc_info.json"),
            r#"{"rustc_fingerprint":11255423990684031070,"outputs":{"17747080675513052775":{"success":false,"status":"exit status: 1","code":1,"stdout":"","stderr":"error"}},"successes":{}}"#,
        )
        .unwrap();

        let names = ToolchainNames::read(&target);
        assert_eq!(names.name(12_019_306_335_353_385_202), None);
    }

    #[test]
    fn reasons_are_distinct() {
        assert_ne!(Reason::OldToolchain, Reason::Cold);
    }
}
