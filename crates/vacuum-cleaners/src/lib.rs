// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! # vacuum-cleaners
//!
//! The catalog of [`Cleaner`](vacuum_core::Cleaner)s Vacuum ships:
//!
//! - [`BuildArtifacts`] — regenerable build output (`target/`, `node_modules`, …)
//! - [`PackageGc`] — package-manager garbage collection (Nix, Flatpak, …)
//! - [`Caches`] — application and user caches
//! - [`LargeFiles`] — the biggest individual files, for manual review
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

mod build_artifacts;
mod caches;
mod large_files;
mod package_gc;

pub use build_artifacts::BuildArtifacts;
pub use caches::Caches;
pub use large_files::LargeFiles;
pub use package_gc::PackageGc;

use vacuum_core::{Category, Cleaner};

/// Every cleaner Vacuum ships, in category order.
pub fn all_cleaners() -> Vec<Box<dyn Cleaner>> {
    vec![
        Box::new(BuildArtifacts),
        Box::new(PackageGc),
        Box::new(Caches),
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
