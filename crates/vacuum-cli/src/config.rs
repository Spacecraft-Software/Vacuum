// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Optional configuration file: persistent default scan roots.
//!
//! Vacuum is zero-config by default ($HOME). A user who repeatedly scans the
//! same directories can record them once instead of typing them every run.
//!
//! The *fallback roots* (used only when the command line names no path/`--root`)
//! are resolved with this precedence:
//!
//! 1. `VACUUM_ROOTS` — an OS-path-separated list (e.g. `~/a:~/b` on Unix);
//! 2. the `roots` key of the TOML config file;
//! 3. `$HOME` (`vacuum_core::safety::default_roots`).
//!
//! The config-file path is `--config` / `VACUUM_CONFIG` when given (and then
//! must exist), otherwise `$XDG_CONFIG_HOME/vacuum/config.toml`, otherwise
//! `$HOME/.config/vacuum/config.toml`. A missing *default*-location file is not
//! an error — it simply yields no configured roots.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use vacuum_core::safety;

use crate::error::{CliError, CliResult};

/// The environment variable that overrides both the config file and `$HOME`.
const ROOTS_ENV: &str = "VACUUM_ROOTS";

/// Parsed contents of `config.toml`.
///
/// Unknown keys are deliberately *ignored* (not rejected) so a config written
/// for a newer Vacuum still loads on an older binary — forward compatibility
/// favours stability (Standard §3.1) over typo detection.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Default roots to scan when none are given on the command line.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
}

impl Config {
    /// Parse a TOML document, attributing any error to `path` for the message.
    fn parse(text: &str, path: &Path) -> CliResult<Self> {
        toml::from_str(text).map_err(|err| CliError::config_invalid(path, &err.to_string()))
    }

    /// Read and parse the config at `path`.
    ///
    /// When `required` is false, a missing file yields [`Config::default`]; when
    /// true (an explicitly named path), a missing file is a usage error.
    fn read(path: &Path, required: bool) -> CliResult<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if required {
                    Err(CliError::config_not_found(path))
                } else {
                    Ok(Self::default())
                }
            }
            Err(err) => Err(CliError::config_unreadable(path, &err.to_string())),
        }
    }
}

/// The default config path: `$XDG_CONFIG_HOME/vacuum/config.toml`, falling back
/// to `$HOME/.config/vacuum/config.toml`. `None` when neither var is set.
fn default_config_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(dir).join("vacuum").join("config.toml"));
    }
    let home = std::env::var_os("HOME").filter(|value| !value.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("vacuum")
            .join("config.toml"),
    )
}

/// Expand a leading `~` component to `$HOME`. `~user` forms and paths without a
/// leading `~` are returned unchanged.
fn expand_tilde(path: &Path) -> PathBuf {
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(rest),
        None => path.to_path_buf(),
    }
}

/// The roots named by `VACUUM_ROOTS`, if it is set to a non-empty value.
fn env_roots() -> Option<Vec<PathBuf>> {
    let raw = std::env::var_os(ROOTS_ENV).filter(|value| !value.is_empty())?;
    let roots: Vec<PathBuf> = std::env::split_paths(&raw)
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| expand_tilde(&path))
        .collect();
    (!roots.is_empty()).then_some(roots)
}

/// Resolve the fallback roots used when the command line names no path.
///
/// `explicit_config` is the value of `--config` / `VACUUM_CONFIG` (which must
/// exist when present); when `None`, the default location is consulted and a
/// missing file is harmless.
///
/// # Errors
///
/// Returns a [`CliError`] when a config file exists but cannot be read or parsed,
/// or when an explicitly named config file is absent.
pub fn fallback_roots(explicit_config: Option<&Path>) -> CliResult<Vec<PathBuf>> {
    if let Some(roots) = env_roots() {
        return Ok(roots);
    }
    let config = match explicit_config {
        Some(path) => Config::read(path, true)?,
        None => match default_config_path() {
            Some(path) => Config::read(&path, false)?,
            None => Config::default(),
        },
    };
    if config.roots.is_empty() {
        Ok(safety::default_roots())
    } else {
        Ok(config.roots.iter().map(|path| expand_tilde(path)).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Config, expand_tilde};

    #[test]
    fn parses_roots_list() {
        let config = Config::parse("roots = [\"/a\", \"~/b\"]", Path::new("config.toml")).unwrap();
        assert_eq!(
            config.roots,
            vec![PathBuf::from("/a"), PathBuf::from("~/b")]
        );
    }

    #[test]
    fn missing_roots_key_defaults_empty() {
        let config = Config::parse("", Path::new("config.toml")).unwrap();
        assert!(config.roots.is_empty());
    }

    #[test]
    fn unknown_keys_are_ignored_for_forward_compat() {
        // A future key must not break an older binary.
        let config = Config::parse("future_key = 1\nroots = [\"/x\"]", Path::new("c")).unwrap();
        assert_eq!(config.roots, vec![PathBuf::from("/x")]);
    }

    #[test]
    fn malformed_toml_is_an_error() {
        assert!(Config::parse("roots = [", Path::new("config.toml")).is_err());
    }

    #[test]
    fn tilde_is_expanded_against_home() {
        // SAFETY of the test: only reads $HOME, never mutates the environment.
        if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
            let expanded = expand_tilde(Path::new("~/projects"));
            assert_eq!(expanded, PathBuf::from(home).join("projects"));
        }
    }

    #[test]
    fn non_tilde_path_is_unchanged() {
        assert_eq!(expand_tilde(Path::new("/etc/x")), PathBuf::from("/etc/x"));
    }
}
