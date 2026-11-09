// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Standard §18.1 accessible-mode toggle, resolved once per run.
//!
//! Precedence, highest first: `--accessible` / `--no-accessible`, then
//! `SPACECRAFT_A11Y`, then the config file's `[accessibility] enabled`, then
//! unambiguous hints (`NO_COLOR`, `TERM=dumb`). An explicit "off" at any level
//! beats every lower one; silence at every level leaves the default
//! presentation unchanged.

use std::fmt;

use crate::cli::GlobalArgs;

/// The environment variable carrying the toggle.
pub const A11Y_ENV: &str = "SPACECRAFT_A11Y";

/// Where the accessible-mode decision came from, reported under `--verbose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `--accessible` or `--no-accessible`.
    Flag,
    /// `SPACECRAFT_A11Y`.
    Env,
    /// `[accessibility] enabled` in the config file.
    Config,
    /// An auto-detect hint, named.
    Hint(&'static str),
    /// Nothing said anything: off.
    Default,
}

/// The resolved toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accessibility {
    /// Whether accessible mode is on.
    pub enabled: bool,
    /// What decided it.
    pub source: Source,
}

impl fmt::Display for Accessibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = if self.enabled { "on" } else { "off" };
        match self.source {
            Source::Flag => write!(f, "{state} (command-line flag)"),
            Source::Env => write!(f, "{state} ({A11Y_ENV})"),
            Source::Config => write!(f, "{state} (config file)"),
            Source::Hint(hint) => write!(f, "{state} (hint: {hint})"),
            Source::Default => write!(f, "{state} (default)"),
        }
    }
}

/// Apply the §18.1 precedence to already-read inputs.
fn resolve(
    flag: Option<bool>,
    env: Option<bool>,
    config: Option<bool>,
    hint: Option<&'static str>,
) -> Accessibility {
    let (enabled, source) = match (flag, env, config, hint) {
        (Some(on), ..) => (on, Source::Flag),
        (None, Some(on), ..) => (on, Source::Env),
        (None, None, Some(on), _) => (on, Source::Config),
        (None, None, None, Some(hint)) => (true, Source::Hint(hint)),
        (None, None, None, None) => (false, Source::Default),
    };
    Accessibility { enabled, source }
}

/// Read a boolean-ish environment value. Anything unrecognized (including an
/// empty value) says nothing, so a typo never silently flips the mode.
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// The unambiguous hints only; never `$LANG`, terminal size, or a missing TTY.
fn hint(get: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    if get("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        Some("NO_COLOR")
    } else if get("TERM").as_deref() == Some("dumb") {
        Some("TERM=dumb")
    } else {
        None
    }
}

/// Resolve accessible mode from the flags, the environment, and the config
/// file's value.
pub fn resolve_for(global: &GlobalArgs, config: Option<bool>) -> Accessibility {
    let flag = if global.accessible {
        Some(true)
    } else if global.no_accessible {
        Some(false)
    } else {
        None
    };
    let get = |var: &str| std::env::var(var).ok();
    let env = get(A11Y_ENV).as_deref().and_then(parse_bool);
    resolve(flag, env, config, hint(get))
}

#[cfg(test)]
mod tests {
    use super::{Source, hint, parse_bool, resolve};

    #[test]
    fn silence_means_off() {
        let resolved = resolve(None, None, None, None);
        assert!(!resolved.enabled);
        assert_eq!(resolved.source, Source::Default);
    }

    #[test]
    fn explicit_off_beats_every_lower_level() {
        let resolved = resolve(Some(false), Some(true), Some(true), Some("NO_COLOR"));
        assert!(!resolved.enabled);
        assert_eq!(resolved.source, Source::Flag);

        let resolved = resolve(None, Some(false), Some(true), Some("NO_COLOR"));
        assert!(!resolved.enabled);
        assert_eq!(resolved.source, Source::Env);

        let resolved = resolve(None, None, Some(false), Some("TERM=dumb"));
        assert!(!resolved.enabled);
        assert_eq!(resolved.source, Source::Config);
    }

    #[test]
    fn hint_turns_it_on_only_when_nothing_else_spoke() {
        let resolved = resolve(None, None, None, Some("NO_COLOR"));
        assert!(resolved.enabled);
        assert_eq!(resolved.to_string(), "on (hint: NO_COLOR)");
    }

    #[test]
    fn env_values() {
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool(" On "), Some(true));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool(""), None);
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn hints_are_unambiguous_only() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |var: &str| {
                pairs
                    .iter()
                    .find(|(name, _)| *name == var)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert_eq!(hint(env(&[("NO_COLOR", "1")])), Some("NO_COLOR"));
        assert_eq!(hint(env(&[("NO_COLOR", "")])), None);
        assert_eq!(hint(env(&[("TERM", "dumb")])), Some("TERM=dumb"));
        assert_eq!(
            hint(env(&[("TERM", "xterm-256color"), ("LANG", "C")])),
            None
        );
    }
}
