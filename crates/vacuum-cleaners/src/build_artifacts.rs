// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Dev build-artifact cleaner: regenerable output directories.

use vacuum_core::{
    Candidate, Category, Cleaner, Result, Risk, ScanContext, Target, dir_size, find_pruned_dirs,
};

/// Directory base names treated as regenerable build output.
const BUILD_DIR_NAMES: &[&str] = &[
    "target",        // Rust / Maven (Rust requires a sibling Cargo.toml; see below)
    "node_modules",  // Node
    ".next",         // Next.js
    ".nuxt",         // Nuxt
    "dist",          // bundlers
    "build",         // many build systems
    "__pycache__",   // CPython
    ".gradle",       // Gradle
    ".dart_tool",    // Dart / Flutter
    ".parcel-cache", // Parcel
    ".turbo",        // Turborepo
    "_build",        // Elixir / dune
];

/// Finds regenerable developer build artifacts (`target/`, `node_modules`, …).
#[derive(Debug, Clone, Copy)]
pub struct BuildArtifacts;

impl Cleaner for BuildArtifacts {
    fn id(&self) -> &'static str {
        "build-artifacts"
    }

    fn name(&self) -> &'static str {
        "Dev build artifacts"
    }

    fn category(&self) -> Category {
        Category::BuildArtifacts
    }

    fn scan(&self, ctx: &ScanContext) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        for root in &ctx.roots {
            let hits = find_pruned_dirs(root, |name| BUILD_DIR_NAMES.contains(&name));
            for path in hits {
                // A directory literally named `target` is only treated as a Rust
                // build dir when it neighbors a `Cargo.toml`, to avoid matching
                // unrelated directories that happen to be called "target".
                let is_target = path.file_name().and_then(|n| n.to_str()) == Some("target");
                if is_target
                    && !path
                        .parent()
                        .is_some_and(|parent| parent.join("Cargo.toml").exists())
                {
                    continue;
                }

                let bytes = dir_size(&path);
                if bytes == 0 {
                    continue;
                }

                candidates.push(Candidate {
                    cleaner_id: self.id().to_owned(),
                    category: Category::BuildArtifacts,
                    label: path.display().to_string(),
                    detail: Some("regenerable build output".to_owned()),
                    bytes,
                    regenerable: true,
                    trash_ok: true,
                    risk: Risk::Safe,
                    target: Target::Path { path },
                });
            }
        }

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.bytes));
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use vacuum_core::{Cleaner, ScanContext, Target};

    use super::BuildArtifacts;

    #[test]
    fn detects_rust_target_with_cargo_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        fs::create_dir_all(proj.join("target")).unwrap();
        fs::write(proj.join("Cargo.toml"), b"[package]").unwrap();
        fs::write(proj.join("target/artifact"), vec![0_u8; 4096]).unwrap();

        let ctx = ScanContext::new(vec![tmp.path().to_path_buf()]);
        let found = BuildArtifacts.scan(&ctx).unwrap();
        assert_eq!(found.len(), 1);
        match &found[0].target {
            Target::Path { path } => assert_eq!(path.file_name().unwrap(), "target"),
            other => panic!("expected a path target, got {other:?}"),
        }
    }

    #[test]
    fn ignores_bare_target_without_cargo() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("misc/target")).unwrap();
        fs::write(tmp.path().join("misc/target/data"), vec![0_u8; 4096]).unwrap();

        let ctx = ScanContext::new(vec![tmp.path().to_path_buf()]);
        assert!(BuildArtifacts.scan(&ctx).unwrap().is_empty());
    }
}
