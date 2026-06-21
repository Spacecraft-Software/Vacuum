// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Deletion safety guards.
//!
//! Every path-based deletion passes through [`check_deletable`], which enforces
//! the project's hard invariants: never delete through a symlink, never touch a
//! protected system location, and never act outside the allowed roots.

use std::path::{Path, PathBuf};

use crate::error::{Result, VacuumError};

/// Path prefixes Vacuum will never delete under, regardless of the root list.
///
/// These are reclaimed (if at all) only through their owning tools — e.g. the
/// Nix store via `nix-collect-garbage`, never via a raw `rm`.
pub const DENY_PREFIXES: &[&str] = &[
    "/boot", "/etc", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/nix", "/proc", "/sys", "/dev",
    "/run", "/var/lib", "/opt", "/srv", "/root",
];

/// The default scan/delete root: the user's home directory.
///
/// Returns an empty list if `$HOME` is unset, which makes every path-based
/// deletion fail closed (outside roots) rather than open.
pub fn default_roots() -> Vec<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .into_iter()
        .collect()
}

/// Verify that `path` may be deleted given the allowed `roots`.
///
/// # Errors
///
/// Returns [`VacuumError::Symlink`] if the entry itself is a symlink,
/// [`VacuumError::ProtectedPath`] if it resolves under a [`DENY_PREFIXES`]
/// entry or is the filesystem root, and [`VacuumError::OutsideRoots`] if it
/// resolves outside every allowed root.
pub fn check_deletable(path: &Path, roots: &[PathBuf]) -> Result<()> {
    // 1. Never delete through a symlink — inspect the link itself, not its target.
    let meta = std::fs::symlink_metadata(path).map_err(|source| VacuumError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if meta.file_type().is_symlink() {
        return Err(VacuumError::Symlink(path.to_path_buf()));
    }

    // 2. Resolve symlinks in the ancestry so a linked parent cannot smuggle the
    //    target outside the allowed roots or into a protected location.
    let real = std::fs::canonicalize(path).map_err(|source| VacuumError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    // 3. Never the filesystem root.
    if real.parent().is_none() {
        return Err(VacuumError::ProtectedPath(real));
    }

    // 4. Never under a protected system prefix.
    if DENY_PREFIXES.iter().any(|deny| real.starts_with(deny)) {
        return Err(VacuumError::ProtectedPath(real));
    }

    // 5. Must resolve within at least one allowed root.
    let allowed = roots
        .iter()
        .any(|root| std::fs::canonicalize(root).is_ok_and(|canonical| real.starts_with(canonical)));
    if !allowed {
        return Err(VacuumError::OutsideRoots(real));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{DENY_PREFIXES, check_deletable};

    #[test]
    fn allows_path_within_root() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("build");
        fs::create_dir(&target).unwrap();
        let roots = vec![tmp.path().to_path_buf()];
        assert!(check_deletable(&target, &roots).is_ok());
    }

    #[test]
    fn rejects_outside_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let target = other.path().join("x");
        fs::create_dir(&target).unwrap();
        let roots = vec![tmp.path().to_path_buf()];
        let err = check_deletable(&target, &roots).unwrap_err();
        assert_eq!(err.code(), "OUTSIDE_ROOTS");
    }

    #[test]
    fn rejects_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let roots = vec![tmp.path().to_path_buf()];
        let err = check_deletable(&link, &roots).unwrap_err();
        assert_eq!(err.code(), "SYMLINK_REFUSED");
    }

    #[test]
    fn deny_prefixes_cover_nix_store() {
        assert!(DENY_PREFIXES.contains(&"/nix"));
    }
}
