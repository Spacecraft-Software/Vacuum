// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The cleaner abstraction.
//!
//! A [`Cleaner`] inspects the filesystem (and the set of installed tools) and
//! reports reclaimable [`Candidate`]s. A candidate's [`Target`] is either a
//! path to remove or an external command to run.

use std::path::PathBuf;

use serde::Serialize;

use crate::error::Result;

/// The reclaimable-space categories Vacuum knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Regenerable developer build artifacts (`target/`, `node_modules`, …).
    BuildArtifacts,
    /// Space reclaimed via a package manager's own garbage collection.
    PackageManagerGc,
    /// Application and user caches.
    Caches,
    /// Stale temporary files under `/tmp`, `/var/tmp`, and `$TMPDIR`.
    TempFiles,
    /// The largest individual files, for manual review.
    LargeFiles,
}

impl Category {
    /// All categories, in display order.
    pub const ALL: [Self; 5] = [
        Self::BuildArtifacts,
        Self::PackageManagerGc,
        Self::Caches,
        Self::TempFiles,
        Self::LargeFiles,
    ];

    /// The stable CLI slug for this category (e.g. `build-artifacts`).
    pub fn slug(self) -> &'static str {
        match self {
            Self::BuildArtifacts => "build-artifacts",
            Self::PackageManagerGc => "package-gc",
            Self::Caches => "caches",
            Self::TempFiles => "temp-files",
            Self::LargeFiles => "large-files",
        }
    }

    /// A human-readable title for headings and the TUI.
    pub fn title(self) -> &'static str {
        match self {
            Self::BuildArtifacts => "Dev build artifacts",
            Self::PackageManagerGc => "Package-manager garbage",
            Self::Caches => "App / user caches",
            Self::TempFiles => "Stale temp files",
            Self::LargeFiles => "Large files",
        }
    }

    /// Parse a category from its CLI slug.
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.slug() == slug)
    }
}

/// How likely a candidate is to need careful review before removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    /// Fully regenerable or transient; safe to remove.
    Safe,
    /// Could contain wanted data; review before removing.
    Caution,
}

/// An external command a command-based cleaner would run.
#[derive(Debug, Clone, Serialize)]
pub struct CommandSpec {
    /// The program name (resolved on `PATH`).
    pub program: String,
    /// Arguments passed to the program.
    pub args: Vec<String>,
    /// Whether the command requires root. Vacuum never escalates; it prints the
    /// `sudo` form for the user to run instead.
    pub needs_root: bool,
}

impl CommandSpec {
    /// Render the command as a copy-pasteable shell line.
    pub fn display(&self) -> String {
        let mut line = self.program.clone();
        for arg in &self.args {
            line.push(' ');
            line.push_str(arg);
        }
        if self.needs_root {
            line.insert_str(0, "sudo ");
        }
        line
    }
}

/// What a candidate would remove: a filesystem path, or a command to run.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Target {
    /// A path to delete (to trash, or permanently with `--purge`).
    Path {
        /// The path that would be removed.
        path: PathBuf,
    },
    /// An external command to invoke (e.g. `nix-collect-garbage -d`).
    Command {
        /// The command specification.
        spec: CommandSpec,
    },
    /// Many paths removed as one unit, all confined beneath `root`.
    ///
    /// Used where a single logical reclaim spans hundreds of scattered paths —
    /// pruning dead build units out of a Cargo `target/` directory, for
    /// instance. Presenting those individually would bury the frontend in rows;
    /// the batch keeps one row while still deleting at file granularity.
    ///
    /// Every path is safety-checked independently, exactly as a
    /// [`Target::Path`] would be.
    Batch {
        /// The directory the batch is confined to. Reported to the user so the
        /// affected project is identifiable from the row alone.
        root: PathBuf,
        /// The paths to remove. All lie beneath `root`.
        paths: Vec<PathBuf>,
    },
}

/// A single unit of reclaimable space surfaced by a cleaner.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    /// Identifier of the cleaner that produced this candidate.
    pub cleaner_id: String,
    /// The category this candidate belongs to.
    pub category: Category,
    /// A short human-readable label (typically the path or tool name).
    pub label: String,
    /// Optional extra detail (e.g. why it is safe to remove).
    ///
    /// Omitted from JSON output when absent (token-economy hygiene, the CLI Standard §8).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Estimated bytes reclaimed; `0` when the amount cannot be measured ahead
    /// of time (true for some command-based cleaners).
    pub bytes: u64,
    /// Whether the target is fully regenerable.
    pub regenerable: bool,
    /// Whether routing this candidate through the trash actually reclaims space.
    ///
    /// Normally `true`. Set to `false` for paths whose trash directory lives on
    /// the same filesystem as the path itself — the freedesktop trash spec puts
    /// them in `<mount>/.Trash-$uid`, so trashing frees nothing. [`Deleter`]
    /// purges such candidates outright rather than pretending to reclaim space.
    ///
    /// [`Deleter`]: crate::delete::Deleter
    pub trash_ok: bool,
    /// The review risk for this candidate.
    pub risk: Risk,
    /// What removal would actually do.
    pub target: Target,
}

/// How long a build artifact must go untouched before it is considered stale.
///
/// Thirty days spans a monthly work cycle: a project touched within the last
/// month keeps its warm incremental cache, anything older is rebuilt on next
/// use. Overridable per run via `--stale-days` or the `stale_days` config key.
pub const DEFAULT_STALE_DAYS: u64 = 30;

/// Context handed to each cleaner's [`Cleaner::scan`].
#[derive(Debug, Clone)]
pub struct ScanContext {
    /// The roots to scan within (defaults to the user's home directory).
    pub roots: Vec<PathBuf>,
    /// Days of disuse after which a build artifact counts as stale.
    pub stale_days: u64,
}

impl ScanContext {
    /// A context over `roots` with every other setting left at its default.
    ///
    /// Preferred over the struct literal so that adding a setting does not break
    /// callers that do not care about it.
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            stale_days: DEFAULT_STALE_DAYS,
        }
    }
}

/// A source of reclaimable-space candidates.
pub trait Cleaner {
    /// A stable identifier for this cleaner.
    fn id(&self) -> &'static str;
    /// A human-readable name.
    fn name(&self) -> &'static str;
    /// The category this cleaner contributes to.
    fn category(&self) -> Category;
    /// Inspect the system and report reclaimable candidates.
    fn scan(&self, ctx: &ScanContext) -> Result<Vec<Candidate>>;

    /// Delete roots this cleaner's candidates need beyond [`ScanContext::roots`].
    ///
    /// Path deletions are bounded by the caller's root list (see
    /// [`safety::check_deletable`]), which defaults to `$HOME`. A cleaner that
    /// legitimately reports paths elsewhere — the temp-file cleaner and `/tmp`,
    /// for instance — declares those locations here, and the frontend widens the
    /// delete allowlist only when that cleaner is actually selected. The
    /// protected-prefix and symlink guards still apply to every path.
    ///
    /// [`safety::check_deletable`]: crate::safety::check_deletable
    fn extra_roots(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// Sum the estimated reclaimable bytes across a set of candidates.
pub fn total_bytes(candidates: &[Candidate]) -> u64 {
    candidates.iter().map(|candidate| candidate.bytes).sum()
}
