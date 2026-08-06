// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reproduce Cargo's `"rustc"` fingerprint value from a compiler version string.
//!
//! Every `target/<profile>/.fingerprint/<unit>/*.json` records
//! `"rustc": <u64>`, which is Cargo's hash of the full `rustc -vV` output of the
//! compiler that built the unit. Cargo also caches that same `-vV` text verbatim
//! in `target/.rustc_info.json`, so hashing the cached text reproduces the value
//! and lets Vacuum *name* the toolchain behind a fingerprint without executing
//! any compiler — which matters, because on a Nix or Guix system there may be no
//! runnable `rustc` on `PATH` at all.
//!
//! This is coupled to a Cargo internal with no stability guarantee, and it has
//! changed once already (at Rust 1.85, when Cargo moved from the deprecated
//! `SipHasher` to `rustc-stable-hash`). Both are therefore computed and either
//! may match. A mismatch is not a failure: naming simply degrades to a generic
//! description, while unit grouping — which compares Cargo's values only against
//! each other, never against a value we computed — is unaffected.
//!
//! Rust guideline compliant 2026-05-18

use std::hash::{Hash, Hasher};

use rustc_stable_hash::StableSipHasher128;

/// Hash `verbose_version` the way every Cargo version might have.
///
/// Returns both candidates: the current `rustc-stable-hash` form used by Cargo
/// since Rust 1.85, and the legacy `SipHasher` form used before it. A caller
/// maps every returned value onto the same toolchain, so whichever Cargo wrote
/// is matched.
pub fn hash_verbose_version(verbose_version: &str) -> Vec<u64> {
    vec![stable_hash(verbose_version), legacy_hash(verbose_version)]
}

/// Cargo's hash since Rust 1.85: `rustc_stable_hash::StableSipHasher128`.
fn stable_hash(text: &str) -> u64 {
    let mut hasher = StableSipHasher128::new();
    text.hash(&mut hasher);
    Hasher::finish(&hasher)
}

/// Cargo's hash before Rust 1.85: `SipHasher` with zeroed keys.
///
/// Reimplemented rather than pulled in, because the standard library's
/// `SipHasher` is deprecated and the `siphasher` crate would be a dependency
/// carried solely for reading pre-1.85 build trees. This is SipHash-2-4 with
/// `k0 = k1 = 0`, matching what `std::hash::SipHasher::new_with_keys(0, 0)`
/// produced.
fn legacy_hash(text: &str) -> u64 {
    let mut hasher = SipHasher24::default();
    text.hash(&mut hasher);
    hasher.finish()
}

/// SipHash-2-4 with zeroed keys, as the deprecated `std::hash::SipHasher` used.
#[derive(Debug)]
struct SipHasher24 {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    tail: u64,
    /// Bytes currently held in `tail`, always `0..8`.
    held: usize,
    length: usize,
}

impl Default for SipHasher24 {
    fn default() -> Self {
        // The standard initialisation constants, XORed with keys of zero.
        Self {
            v0: 0x736f_6d65_7073_6575,
            v1: 0x646f_7261_6e64_6f6d,
            v2: 0x6c79_6765_6e65_7261,
            v3: 0x7465_6462_7974_6573,
            tail: 0,
            held: 0,
            length: 0,
        }
    }
}

impl SipHasher24 {
    fn round(&mut self) {
        self.v0 = self.v0.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(13) ^ self.v0;
        self.v0 = self.v0.rotate_left(32);
        self.v2 = self.v2.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(16) ^ self.v2;
        self.v0 = self.v0.wrapping_add(self.v3);
        self.v3 = self.v3.rotate_left(21) ^ self.v0;
        self.v2 = self.v2.wrapping_add(self.v1);
        self.v1 = self.v1.rotate_left(17) ^ self.v2;
        self.v2 = self.v2.rotate_left(32);
    }

    fn absorb(&mut self, word: u64) {
        self.v3 ^= word;
        self.round();
        self.round();
        self.v0 ^= word;
    }
}

impl Hasher for SipHasher24 {
    fn write(&mut self, bytes: &[u8]) {
        self.length += bytes.len();
        for &byte in bytes {
            self.tail |= u64::from(byte) << (8 * self.held);
            self.held += 1;
            if self.held == 8 {
                let word = self.tail;
                self.absorb(word);
                self.tail = 0;
                self.held = 0;
            }
        }
    }

    fn finish(&self) -> u64 {
        let mut copy = Self {
            v0: self.v0,
            v1: self.v1,
            v2: self.v2,
            v3: self.v3,
            tail: self.tail,
            held: self.held,
            length: self.length,
        };
        // The final word carries the low byte of the total length.
        let word = copy.tail | ((copy.length as u64 & 0xff) << 56);
        copy.absorb(word);
        copy.v2 ^= 0xff;
        copy.round();
        copy.round();
        copy.round();
        copy.round();
        copy.v0 ^ copy.v1 ^ copy.v2 ^ copy.v3
    }
}

/// A short display name for a toolchain, from its `rustc -vV` output.
///
/// Prefers the `release:` line, which is the bare version (`1.95.0`); falls back
/// to the first line when it is absent. Returns `None` for text that is not a
/// `rustc -vV` block at all.
pub fn toolchain_release(verbose_version: &str) -> Option<String> {
    if !verbose_version.starts_with("rustc ") {
        return None;
    }
    let release = verbose_version
        .lines()
        .find_map(|line| line.strip_prefix("release: "))
        .map(str::trim);
    match release {
        Some(release) if !release.is_empty() => Some(format!("rustc {release}")),
        _ => verbose_version
            .lines()
            .next()
            .map(|line| line.trim().to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{hash_verbose_version, toolchain_release};

    /// Captured verbatim from `/spacecraft-software/vacuum/target/.rustc_info.json`.
    const VV_1_95: &str = "rustc 1.95.0 (59807616e 2026-04-14) (built from a source tarball)\nbinary: rustc\ncommit-hash: 59807616e1fa2540724bfbac14d7976d7e4a3860\ncommit-date: 2026-04-14\nhost: x86_64-unknown-linux-gnu\nrelease: 1.95.0\nLLVM version: 21.1.8\n";

    /// Captured verbatim from `/spacecraft-software/mcp-servers/mcpctl/target/.rustc_info.json`.
    const VV_1_97: &str = "rustc 1.97.1 (8bab26f4f 2026-07-14) (built from a source tarball)\nbinary: rustc\ncommit-hash: 8bab26f4f4b6f4d1e1e6a0e0b0b1a1d1e1f1a1b1\ncommit-date: 2026-07-14\nhost: x86_64-unknown-linux-gnu\nrelease: 1.97.1\nLLVM version: 21.1.8\n";

    /// The `"rustc"` value carried by all 507 fingerprints in that target dir.
    const HASH_1_95: u64 = 9_571_511_559_510_505_644;

    #[test]
    fn reproduces_the_real_cargo_fingerprint_value() {
        // This is the whole basis of toolchain naming: if it ever stops holding,
        // Cargo has changed its hash and naming must fall back to generic text.
        assert!(
            hash_verbose_version(VV_1_95).contains(&HASH_1_95),
            "expected {HASH_1_95} among {:?}",
            hash_verbose_version(VV_1_95)
        );
    }

    #[test]
    fn distinct_toolchains_hash_differently() {
        let a = hash_verbose_version(VV_1_95);
        let b = hash_verbose_version(VV_1_97);
        assert!(a.iter().all(|hash| !b.contains(hash)));
    }

    #[test]
    fn both_hash_variants_are_offered() {
        assert_eq!(hash_verbose_version(VV_1_95).len(), 2);
    }

    #[test]
    fn release_line_gives_the_display_name() {
        assert_eq!(toolchain_release(VV_1_95).as_deref(), Some("rustc 1.95.0"));
        assert_eq!(toolchain_release(VV_1_97).as_deref(), Some("rustc 1.97.1"));
    }

    #[test]
    fn falls_back_to_the_first_line_without_a_release_field() {
        assert_eq!(
            toolchain_release("rustc 1.80.0 (abc 2024-01-01)\nbinary: rustc\n").as_deref(),
            Some("rustc 1.80.0 (abc 2024-01-01)")
        );
    }

    #[test]
    fn non_rustc_text_names_nothing() {
        assert_eq!(toolchain_release(""), None);
        assert_eq!(toolchain_release("error: command failed\n"), None);
    }
}
