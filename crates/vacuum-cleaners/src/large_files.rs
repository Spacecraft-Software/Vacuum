// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Large-file cleaner: the biggest individual files, for manual review.

use vacuum_core::{Candidate, Category, Cleaner, Result, Risk, ScanContext, Target, largest_files};

/// Minimum size for a file to be surfaced (200 MiB). Below this, individual
/// files rarely matter for a recovery pass and would only add noise.
const MIN_LARGE_FILE: u64 = 200 * 1024 * 1024;

/// Maximum number of large files to report.
const LIMIT: usize = 50;

/// Surfaces the largest individual files for manual selection.
#[derive(Debug, Clone, Copy)]
pub struct LargeFiles;

impl Cleaner for LargeFiles {
    fn id(&self) -> &'static str {
        "large-files"
    }

    fn name(&self) -> &'static str {
        "Large files"
    }

    fn category(&self) -> Category {
        Category::LargeFiles
    }

    fn scan(&self, ctx: &ScanContext) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        for root in &ctx.roots {
            for usage in largest_files(root, MIN_LARGE_FILE, LIMIT) {
                candidates.push(Candidate {
                    cleaner_id: self.id().to_owned(),
                    category: Category::LargeFiles,
                    label: usage.path.display().to_string(),
                    detail: None,
                    bytes: usage.bytes,
                    regenerable: false,
                    risk: Risk::Caution,
                    target: Target::Path { path: usage.path },
                });
            }
        }

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.bytes));
        candidates.truncate(LIMIT);
        Ok(candidates)
    }
}
