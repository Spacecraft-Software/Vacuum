// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parallel filesystem scanning.
//!
//! Directory sizes are summed in parallel with [`jwalk`]. Symlinks are never
//! followed, so a link can never inflate a size or escape the scanned tree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use jwalk::WalkDir;
use serde::Serialize;

use crate::error::{Result, VacuumError};

/// A single disk-usage entry: a file or a directory and its aggregate size.
#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    /// Absolute or root-relative path of the entry.
    pub path: PathBuf,
    /// Aggregate size in bytes (recursive for directories).
    pub bytes: u64,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// Sum the apparent size of every regular file beneath `path`, in parallel.
///
/// Symlinks are not followed. Unreadable entries are skipped rather than
/// aborting the whole walk (a cleanup tool must tolerate permission gaps).
pub fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .skip_hidden(false)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

/// Sum the size of every regular file under `paths`, counting each inode once.
///
/// Cargo hardlinks its final artifacts into place: `target/debug/vacuum` and
/// `target/debug/deps/vacuum-7fa5fb…` are one 63 MiB inode with two names.
/// Summing [`std::fs::Metadata::len`] naively would report that space twice —
/// on this workspace, a 12% over-estimate — and then claim to have freed it
/// twice. Removing one name frees nothing until the last one goes, so only
/// distinct `(device, inode)` pairs are counted, exactly as `du` does.
///
/// Symlinks are not followed. Unreadable entries are skipped.
pub fn sum_unique_files(paths: &[PathBuf]) -> u64 {
    use std::os::unix::fs::MetadataExt as _;

    let mut seen = HashSet::new();
    let mut total = 0_u64;

    for path in paths {
        for entry in WalkDir::new(path)
            .skip_hidden(false)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            // Count a multiply-linked inode only the first time it is met.
            if meta.nlink() > 1 && !seen.insert((meta.dev(), meta.ino())) {
                continue;
            }
            total += meta.len();
        }
    }

    total
}

/// List the immediate children of `root`, sized and sorted largest-first.
///
/// This is the equivalent of `du --max-depth=1 | sort -rh`.
pub fn top_consumers(root: &Path, limit: usize) -> Result<Vec<Usage>> {
    let read = std::fs::read_dir(root).map_err(|source| VacuumError::Io {
        path: root.to_path_buf(),
        source,
    })?;

    let mut entries = Vec::new();
    for entry in read.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let bytes = if file_type.is_dir() {
            dir_size(&path)
        } else {
            file_len(&path)
        };
        entries.push(Usage {
            path,
            bytes,
            is_dir: file_type.is_dir(),
        });
    }

    entries.sort_by_key(|entry| std::cmp::Reverse(entry.bytes));
    entries.truncate(limit);
    Ok(entries)
}

/// Find directories whose base name satisfies `matches`, pruning matched
/// subtrees so a match's interior is never descended into.
///
/// Pruning means a matched `target/` is recorded but not walked, so a
/// `node_modules/` nested inside it is never double-counted. Symlinked
/// directories are skipped entirely.
pub fn find_pruned_dirs(root: &Path, matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let is_match = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(&matches);
            if is_match {
                // Record the match and prune: do not descend into it.
                found.push(path);
            } else {
                stack.push(path);
            }
        }
    }

    found
}

/// Collect the largest regular files beneath `root` at or above `min_bytes`,
/// sorted largest-first and truncated to `limit`.
pub fn largest_files(root: &Path, min_bytes: u64, limit: usize) -> Vec<Usage> {
    let mut files: Vec<Usage> = WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let len = entry.metadata().ok()?.len();
            (len >= min_bytes).then(|| Usage {
                path: entry.path(),
                bytes: len,
                is_dir: false,
            })
        })
        .collect();

    files.sort_by_key(|file| std::cmp::Reverse(file.bytes));
    files.truncate(limit);
    files
}

/// Size of a single path without following symlinks; `0` if unreadable.
fn file_len(path: &Path) -> u64 {
    std::fs::symlink_metadata(path).map_or(0, |meta| meta.len())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{dir_size, find_pruned_dirs, largest_files, sum_unique_files, top_consumers};

    fn write(path: &std::path::Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0_u8; bytes]).unwrap();
    }

    #[test]
    fn sum_unique_files_counts_a_hardlinked_inode_once() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("deps/lib-abcdef.rlib");
        write(&original, 4096);
        // Cargo "uplifts" its artifacts by hardlinking, not copying.
        let uplifted = tmp.path().join("libthing.rlib");
        fs::hard_link(&original, &uplifted).unwrap();

        let paths = vec![tmp.path().to_path_buf()];
        assert_eq!(
            sum_unique_files(&paths),
            4096,
            "one inode with two names is 4096 bytes of disk, not 8192"
        );
        // dir_size is deliberately left naive; this documents the difference.
        assert_eq!(dir_size(tmp.path()), 8192);
    }

    #[test]
    fn sum_unique_files_spans_multiple_roots_without_double_counting() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a/one"), 1000);
        write(&tmp.path().join("b/two"), 2000);
        let linked = tmp.path().join("b/one-link");
        fs::hard_link(tmp.path().join("a/one"), &linked).unwrap();

        let paths = vec![tmp.path().join("a"), tmp.path().join("b")];
        assert_eq!(sum_unique_files(&paths), 3000);
    }

    #[test]
    fn dir_size_sums_files() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a.bin"), 100);
        write(&tmp.path().join("sub/b.bin"), 200);
        assert_eq!(dir_size(tmp.path()), 300);
    }

    #[test]
    fn top_consumers_sorted_desc() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("small/x"), 10);
        write(&tmp.path().join("big/y"), 1000);
        let top = top_consumers(tmp.path(), 10).unwrap();
        assert_eq!(top[0].path.file_name().unwrap(), "big");
        assert!(top[0].bytes > top[1].bytes);
    }

    #[test]
    fn find_pruned_does_not_descend_into_matches() {
        let tmp = tempfile::tempdir().unwrap();
        // A `target` dir containing a nested `node_modules` — only `target`
        // should be reported, because matches are pruned.
        write(&tmp.path().join("proj/target/node_modules/dep/x"), 1);
        write(&tmp.path().join("proj/Cargo.toml"), 1);
        let hits = find_pruned_dirs(tmp.path(), |name| {
            name == "target" || name == "node_modules"
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_name().unwrap(), "target");
    }

    #[test]
    fn largest_files_respects_threshold() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("tiny"), 10);
        write(&tmp.path().join("huge"), 5000);
        let big = largest_files(tmp.path(), 1000, 10);
        assert_eq!(big.len(), 1);
        assert_eq!(big[0].path.file_name().unwrap(), "huge");
    }
}
