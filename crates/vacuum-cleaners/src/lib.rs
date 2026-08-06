// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! # vacuum-cleaners
//!
//! The catalog of [`Cleaner`](vacuum_core::Cleaner)s Vacuum ships:
//!
//! - [`BuildArtifacts`] — regenerable build output (`target/`, `node_modules`, …)
//! - [`CargoPrune`] — dead build units *inside* a Cargo `target/` directory
//! - [`PackageGc`] — package-manager garbage collection (Nix, Flatpak, …)
//! - [`Caches`] — application and user caches
//! - [`TempFiles`] — stale, user-owned entries under `/tmp` and friends
//! - [`LargeFiles`] — the biggest individual files, for manual review
//!
//! Several cleaners may share a [`Category`]; frontends group by category, not
//! by cleaner.
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

mod build_artifacts;
mod caches;
mod cargo_prune;
mod large_files;
mod package_gc;
mod rustc_hash;
mod temp_files;

pub use build_artifacts::BuildArtifacts;
pub use caches::Caches;
pub use cargo_prune::CargoPrune;
pub use large_files::LargeFiles;
pub use package_gc::PackageGc;
pub use temp_files::TempFiles;

use vacuum_core::{Category, Cleaner};

/// Every cleaner Vacuum ships, in category order.
pub fn all_cleaners() -> Vec<Box<dyn Cleaner>> {
    vec![
        Box::new(BuildArtifacts),
        Box::new(CargoPrune),
        Box::new(PackageGc),
        Box::new(Caches),
        Box::new(TempFiles),
        Box::new(LargeFiles),
    ]
}

/// The cleaners whose category is in `categories`, preserving catalog order.
pub fn cleaners_for(categories: &[Category]) -> Vec<Box<dyn Cleaner>> {
    all_cleaners()
        .into_iter()
        .filter(|cleaner| categories.contains(&cleaner.category()))
        .collect()
}
