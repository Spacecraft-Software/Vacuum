// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Human-readable byte formatting (IEC binary units, matching `du -h`).

/// One binary kibibyte.
const UNIT: f64 = 1024.0;
const SUFFIXES: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

/// Format a byte count using IEC binary units, e.g. `1536` becomes `1.5 KiB`.
///
/// # Examples
///
/// ```
/// use vacuum_core::human_bytes;
/// assert_eq!(human_bytes(0), "0 B");
/// assert_eq!(human_bytes(1024), "1.0 KiB");
/// assert_eq!(human_bytes(1_073_741_824), "1.0 GiB");
/// ```
pub fn human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "display formatting only; exact precision is unnecessary"
    )]
    let mut value = bytes as f64;
    let mut idx = 0;
    while value >= UNIT && idx < SUFFIXES.len() - 1 {
        value /= UNIT;
        idx += 1;
    }
    format!("{value:.1} {}", SUFFIXES[idx])
}

#[cfg(test)]
mod tests {
    use super::human_bytes;

    #[test]
    fn formats_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1_572_864), "1.5 MiB");
        assert_eq!(human_bytes(1_073_741_824), "1.0 GiB");
    }
}
