// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Platform-gated access to the OS trash bin for `vacuum empty-trash`.
//!
//! The `trash` crate can enumerate and purge trashed entries only on Linux/
//! Freedesktop and Windows (its `os_limited` module). On every other target the
//! two entry points return a structured `UNSUPPORTED_PLATFORM` error rather than
//! failing to compile. Vacuum is Linux-first, so the supported path is the
//! common case.

#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
use crate::error::CliError;
use crate::error::CliResult;

/// One trashed entry: display metadata plus an opaque purge handle.
#[derive(Debug)]
pub struct Entry {
    /// File or directory name as shown in the trash.
    pub name: String,
    /// The absolute path the entry was trashed from.
    pub original_path: String,
    /// Size in bytes, when known. Directories report an entry count rather than
    /// a byte size, so this stays `None` for them (a `trash`-crate limitation);
    /// the item is still counted, only its bytes are unknown.
    pub bytes: Option<u64>,
    /// Deletion time as ISO 8601 UTC, when representable.
    pub deleted_at: Option<String>,

    /// Backend handle used to purge this exact entry. Present only on platforms
    /// whose trash backend supports purging.
    #[cfg(any(
        windows,
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    ))]
    handle: trash::TrashItem,
}

/// List every entry currently in the OS trash.
///
/// # Errors
///
/// Returns `TRASH_ERROR` if the backend cannot be queried, or
/// `UNSUPPORTED_PLATFORM` on targets without a supported trash backend.
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
pub fn list() -> CliResult<Vec<Entry>> {
    use jiff::Timestamp;

    let items =
        trash::os_limited::list().map_err(|err| crate::error::CliError::trash(&err.to_string()))?;
    let entries = items
        .into_iter()
        .map(|item| {
            // Size is queried per item; directories yield an entry count (None here).
            let bytes = trash::os_limited::metadata(&item)
                .ok()
                .and_then(|meta| meta.size.size());
            let deleted_at = Timestamp::from_second(item.time_deleted)
                .ok()
                .map(|stamp| stamp.to_string());
            Entry {
                name: item.name.to_string_lossy().into_owned(),
                original_path: item.original_path().display().to_string(),
                bytes,
                deleted_at,
                handle: item,
            }
        })
        .collect();
    Ok(entries)
}

/// Permanently remove a single trashed entry.
///
/// Purging one entry at a time (rather than the whole set at once) lets the
/// caller drive a live spinner and isolate a single failure without aborting the
/// rest of the sweep.
///
/// # Errors
///
/// Returns `TRASH_ERROR` if the backend refuses the purge.
#[cfg(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
pub fn purge(entry: &Entry) -> CliResult<()> {
    trash::os_limited::purge_all(std::iter::once(&entry.handle))
        .map_err(|err| crate::error::CliError::trash(&format!("purging {}: {err}", entry.name)))
}

/// Fallback for platforms without a supported trash backend (e.g. macOS).
#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
pub fn list() -> CliResult<Vec<Entry>> {
    Err(CliError::unsupported("listing the trash"))
}

/// Fallback for platforms without a supported trash backend (e.g. macOS).
#[cfg(not(any(
    windows,
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
pub fn purge(_entry: &Entry) -> CliResult<()> {
    Err(CliError::unsupported("emptying the trash"))
}
