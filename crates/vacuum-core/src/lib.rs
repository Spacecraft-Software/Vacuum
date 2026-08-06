// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! # vacuum-core
//!
//! The engine behind [Vacuum](https://Vacuum.SpacecraftSoftware.org/): a parallel
//! filesystem scanner, the [`Cleaner`] abstraction for reclaimable-space sources,
//! a safety-checked deletion engine, and human-readable byte formatting.
//!
//! This crate is frontend-agnostic — both the CLI and the TUI build on it.
//!
//! ## Safety model
//!
//! Path deletions always pass through [`safety::check_deletable`], which refuses
//! symlinks, protected system paths, and anything outside the configured roots.
//! The [`delete::Deleter`] is dry-run by default and routes to the trash unless
//! purging is explicitly requested.
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

pub mod bytes;
pub mod cleaner;
pub mod delete;
pub mod error;
pub mod safety;
pub mod scan;

pub use bytes::human_bytes;
pub use cleaner::{
    Candidate, Category, Cleaner, CommandSpec, Risk, ScanContext, Target, total_bytes,
};
pub use delete::{Action, DeleteMode, Deleter, Outcome};
pub use error::{Result, VacuumError};
pub use scan::{Usage, dir_size, find_pruned_dirs, largest_files, sum_unique_files, top_consumers};
