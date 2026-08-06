// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Package-manager garbage-collection cleaner: command-based reclaim.
//!
//! Each entry is only offered when its tool is present on `PATH`. The amount
//! reclaimed cannot be measured ahead of time, so candidates report `0` bytes
//! (the frontend renders this as "unknown"). Commands that need root are marked
//! accordingly and are never escalated by Vacuum.

use vacuum_core::{Candidate, Category, Cleaner, CommandSpec, Result, Risk, ScanContext, Target};

/// A package-manager GC recipe: `(program, args, needs_root, detail)`.
type Recipe = (&'static str, &'static [&'static str], bool, &'static str);

/// The GC recipes, tried in order; only those whose tool is installed are offered.
const RECIPES: &[Recipe] = &[
    (
        "nix-collect-garbage",
        &["-d"],
        false,
        "Delete dead Nix store paths and old generations (user profile).",
    ),
    (
        // The user-profile form above cannot reach the system profile or other
        // users' roots; on NixOS that is usually where the bulk of the store is.
        "nix-collect-garbage",
        &["-d", "--verbose"],
        true,
        "Delete dead Nix store paths and old generations system-wide (all profiles).",
    ),
    (
        "flatpak",
        &["uninstall", "--unused", "-y"],
        false,
        "Remove unused Flatpak runtimes and extensions.",
    ),
    (
        "paccache",
        &["-r"],
        true,
        "Remove old cached pacman packages, keeping the last three.",
    ),
    (
        "journalctl",
        &["--vacuum-time=7d"],
        true,
        "Drop systemd journal entries older than 7 days.",
    ),
    (
        "journalctl",
        &["--vacuum-size=200M"],
        true,
        "Trim the systemd journal to 200 MiB.",
    ),
    (
        // The counterpart to the temp-files cleaner: that one handles entries
        // owned by the invoking user, this reclaims the root-owned remainder
        // under the policy in `tmpfiles.d`.
        "systemd-tmpfiles",
        &["--clean"],
        true,
        "Remove expired temp files owned by root, per tmpfiles.d policy.",
    ),
    (
        "podman",
        &["system", "prune", "-f"],
        false,
        "Remove unused podman containers, images, networks, and volumes.",
    ),
    (
        "docker",
        &["system", "prune", "-f"],
        false,
        "Remove unused docker data.",
    ),
];

/// Offers package-manager garbage-collection commands for installed tools.
#[derive(Debug, Clone, Copy)]
pub struct PackageGc;

impl Cleaner for PackageGc {
    fn id(&self) -> &'static str {
        "package-gc"
    }

    fn name(&self) -> &'static str {
        "Package-manager garbage"
    }

    fn category(&self) -> Category {
        Category::PackageManagerGc
    }

    fn scan(&self, _ctx: &ScanContext) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();

        for &(program, args, needs_root, detail) in RECIPES {
            if which::which(program).is_err() {
                continue;
            }
            let spec = CommandSpec {
                program: program.to_owned(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                needs_root,
            };
            candidates.push(Candidate {
                cleaner_id: self.id().to_owned(),
                category: Category::PackageManagerGc,
                label: spec.display(),
                detail: Some(detail.to_owned()),
                bytes: 0, // Reclaim is unknown until the tool runs.
                regenerable: true,
                trash_ok: true,
                risk: Risk::Caution,
                target: Target::Command { spec },
            });
        }

        Ok(candidates)
    }
}
