// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! App / user cache cleaner: regenerable cache directories.

use std::path::PathBuf;

use vacuum_core::{Candidate, Category, Cleaner, Result, Risk, ScanContext, Target, dir_size};

/// Cache subdirectory names found under `~/.config/<app>/`.
const CONFIG_CACHE_NAMES: &[&str] = &["Cache", "CachedData", "Code Cache", "GPUCache"];

/// Finds regenerable application and user caches.
#[derive(Debug, Clone, Copy)]
pub struct Caches;

impl Cleaner for Caches {
    fn id(&self) -> &'static str {
        "caches"
    }

    fn name(&self) -> &'static str {
        "App / user caches"
    }

    fn category(&self) -> Category {
        Category::Caches
    }

    fn scan(&self, ctx: &ScanContext) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        for root in &ctx.roots {
            // ~/.cache/* — each immediate entry is an independent candidate.
            if let Ok(entries) = std::fs::read_dir(root.join(".cache")) {
                for entry in entries.flatten() {
                    Self::push(&mut candidates, entry.path());
                }
            }

            // ~/.var/app/<app>/cache — Flatpak per-app caches.
            if let Ok(apps) = std::fs::read_dir(root.join(".var/app")) {
                for app in apps.flatten() {
                    Self::push(&mut candidates, app.path().join("cache"));
                }
            }

            // ~/.config/<app>/{Cache,CachedData,...} — Electron/Chromium caches.
            if let Ok(configs) = std::fs::read_dir(root.join(".config")) {
                for config in configs.flatten() {
                    for name in CONFIG_CACHE_NAMES {
                        Self::push(&mut candidates, config.path().join(name));
                    }
                }
            }
        }

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.bytes));
        Ok(candidates)
    }
}

impl Caches {
    /// Add a cache candidate for `path` if it exists and is non-empty.
    fn push(candidates: &mut Vec<Candidate>, path: PathBuf) {
        if !path.exists() {
            return;
        }
        let bytes = dir_size(&path);
        if bytes == 0 {
            return;
        }
        candidates.push(Candidate {
            cleaner_id: "caches".to_owned(),
            category: Category::Caches,
            label: path.display().to_string(),
            detail: Some("cache (regenerated on demand)".to_owned()),
            bytes,
            regenerable: true,
            trash_ok: true,
            risk: Risk::Safe,
            target: Target::Path { path },
        });
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use vacuum_core::{Cleaner, ScanContext};

    use super::Caches;

    #[test]
    fn finds_cache_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join(".cache/some-app");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("blob"), vec![0_u8; 2048]).unwrap();

        let ctx = ScanContext::new(vec![tmp.path().to_path_buf()]);
        let found = Caches.scan(&ctx).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].bytes, 2048);
    }
}
