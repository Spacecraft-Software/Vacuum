// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Modern terminal protocols (the `spacecraft-tui-protocols` contract): which ones
//! Vacuum speaks, the user override, the emission gates, and the escape-sequence
//! encoders.
//!
//! Vacuum reads keys, runs long background jobs and has paths worth copying, so
//! it speaks three protocols:
//!
//! | Protocol | Tier | Detection |
//! |----------|------|-----------|
//! | kitty keyboard (`CSI > 1 u`) | 1 | `CSI ? u` probe, fenced by DA1 |
//! | OSC 52 clipboard write | 1 | none exists: sent once the gates pass |
//! | OSC 9;4 progress | 2 | terminal allow-list (OSC 9 can mean "notify") |
//!
//! It shows no images and has no mouse-interactive regions, so kitty graphics,
//! animation and OSC 22 pointer shapes are not used; OSC 5522 is optional and
//! Vacuum copies plain text only. Those names are still accepted by the
//! override so one `SPACECRAFT_TERM_PROTOCOLS` value works across every
//! Spacecraft Software program.
//!
//! Everything here is pure: the environment is passed in as a lookup function,
//! so the decisions are unit-tested without touching the process environment.

use std::fmt;

/// The environment variable carrying the user override; `--term-protocols`
/// beats it name by name.
pub const OVERRIDE_ENV: &str = "SPACECRAFT_TERM_PROTOCOLS";

/// A terminal protocol named by the Spacecraft Software override syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// The kitty keyboard protocol.
    Keyboard,
    /// The kitty graphics protocol (and its image fallback ladder).
    Graphics,
    /// Kitty graphics animation.
    Animation,
    /// OSC 52 clipboard write.
    Clipboard,
    /// OSC 5522 rich (MIME) clipboard.
    ClipboardRich,
    /// OSC 9;4 progress.
    Progress,
    /// OSC 22 mouse pointer shapes.
    Pointer,
}

impl Protocol {
    /// Every protocol, in the order the override syntax documents them.
    pub const ALL: [Self; 7] = [
        Self::Keyboard,
        Self::Graphics,
        Self::Animation,
        Self::Clipboard,
        Self::ClipboardRich,
        Self::Progress,
        Self::Pointer,
    ];

    /// The name used by `--term-protocols` and `SPACECRAFT_TERM_PROTOCOLS`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Graphics => "graphics",
            Self::Animation => "animation",
            Self::Clipboard => "clipboard",
            Self::ClipboardRich => "clipboard-rich",
            Self::Progress => "progress",
            Self::Pointer => "pointer",
        }
    }

    /// Whether Vacuum has a use for this protocol at all.
    pub fn used(self) -> bool {
        matches!(self, Self::Keyboard | Self::Clipboard | Self::Progress)
    }

    fn index(self) -> usize {
        // `ALL` lists every variant exactly once, so this always finds it.
        Self::ALL
            .iter()
            .position(|known| *known == self)
            .unwrap_or_default()
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.name() == name)
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A `--term-protocols` / `SPACECRAFT_TERM_PROTOCOLS` value that names an
/// unknown protocol.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "unknown terminal protocol `{name}` (expected keyboard, graphics, animation, \
     clipboard, clipboard-rich, progress, pointer or all, each prefixed + or -)"
)]
pub struct OverrideError {
    name: String,
}

/// User overrides that beat detection: `Some(true)` forces a protocol on,
/// `Some(false)` forces it off, `None` leaves it to detection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProtocolOverrides {
    forced: [Option<bool>; 7],
}

impl ProtocolOverrides {
    /// Parse a comma-separated list such as `-graphics,+progress`.
    ///
    /// `+name` forces on, `-name` forces off, and a bare name means `+name`.
    /// `all` addresses every protocol. Items apply left to right, so
    /// `-all,+keyboard` leaves only the keyboard protocol forced on. Empty
    /// items (and an empty list) are ignored.
    ///
    /// # Errors
    ///
    /// Returns [`OverrideError`] naming the first unknown protocol.
    pub fn parse(list: &str) -> Result<Self, OverrideError> {
        let mut overrides = Self::default();
        for item in list
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            let (on, name) = if let Some(name) = item.strip_prefix('+') {
                (true, name)
            } else if let Some(name) = item.strip_prefix('-') {
                (false, name)
            } else {
                (true, item)
            };
            if name == "all" {
                overrides.forced = [Some(on); 7];
            } else {
                let protocol = Protocol::from_name(name).ok_or_else(|| OverrideError {
                    name: name.to_owned(),
                })?;
                overrides.forced[protocol.index()] = Some(on);
            }
        }
        Ok(overrides)
    }

    /// Combine with a lower-precedence set: wherever `self` says nothing,
    /// `lower` decides. Used to let the flag beat the environment variable.
    #[must_use]
    pub fn over(self, lower: Self) -> Self {
        let mut forced = self.forced;
        for (slot, fallback) in forced.iter_mut().zip(lower.forced) {
            if slot.is_none() {
                *slot = fallback;
            }
        }
        Self { forced }
    }

    /// The override for `protocol`, if any.
    pub fn get(self, protocol: Protocol) -> Option<bool> {
        self.forced[protocol.index()]
    }
}

/// Whether one protocol ended up on, and why — reported under `--verbose`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The protocol decided.
    pub protocol: Protocol,
    /// Whether Vacuum will emit it.
    pub on: bool,
    /// Why, in a few words.
    pub reason: String,
}

impl Decision {
    pub(crate) fn new(protocol: Protocol, on: bool, reason: impl Into<String>) -> Self {
        Self {
            protocol,
            on,
            reason: reason.into(),
        }
    }
}

/// The resolved state of every protocol for one TUI session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolReport {
    decisions: Vec<Decision>,
}

impl ProtocolReport {
    pub(crate) fn new(decisions: Vec<Decision>) -> Self {
        Self { decisions }
    }

    /// The per-protocol decisions, in [`Protocol::ALL`] order.
    pub fn decisions(&self) -> &[Decision] {
        &self.decisions
    }

    /// Whether `protocol` is on for this session.
    pub fn is_on(&self, protocol: Protocol) -> bool {
        self.decisions
            .iter()
            .any(|decision| decision.protocol == protocol && decision.on)
    }
}

impl fmt::Display for ProtocolReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, decision) in self.decisions.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            let state = if decision.on { "on" } else { "off" };
            write!(f, "{}={state} ({})", decision.protocol, decision.reason)?;
        }
        Ok(())
    }
}

/// Why no protocol may be emitted at all (SKILL.md §2.1), or `None` when the
/// gates pass.
///
/// `TERM` unset blocks emission on Unix only: Windows terminals do not set it,
/// so there it carries no information.
pub(crate) fn gate(
    stdout_tty: bool,
    agent: bool,
    env: &impl Fn(&str) -> Option<String>,
) -> Option<&'static str> {
    if !stdout_tty {
        return Some("output is not a terminal");
    }
    if agent {
        return Some("agent or CI mode");
    }
    match env("TERM").as_deref() {
        Some("dumb") => Some("TERM=dumb"),
        None | Some("") if cfg!(unix) => Some("TERM is unset"),
        _ => None,
    }
}

/// The multiplexer Vacuum is running inside, if any (SKILL.md §3).
pub(crate) fn multiplexer(env: &impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    let set = |var: &str| env(var).is_some_and(|value| !value.is_empty());
    if set("TMUX") {
        Some("tmux")
    } else if set("ZELLIJ") {
        Some("zellij")
    } else if set("STY") {
        Some("GNU screen")
    } else {
        None
    }
}

/// The first `WezTerm` build whose OSC 9;4 is progress rather than a desktop
/// notification (`references/wezterm.md`): nightly `fb02395`, 2025-02-10.
const WEZTERM_PROGRESS_SINCE: u32 = 20_250_210;

/// The first kitty release with OSC 9;4 progress: 0.47.0, which added the
/// `progress_bar` option. Conservative — earlier releases are skipped rather
/// than risk a notification per update.
const KITTY_PROGRESS_SINCE: (u32, u32, u32) = (0, 47, 0);

/// The first iTerm2 release with OSC 9;4 progress (3.6.0). Older releases
/// show OSC 9 as a notification.
const ITERM_PROGRESS_SINCE: (u32, u32, u32) = (3, 6, 0);

/// Decide OSC 9;4 by terminal identity. There is no query that tells progress
/// from "notify", so this is an allow-list: anything not recognized gets
/// nothing (`references/progress-pointer.md`).
///
/// `xtversion` is the XTVERSION reply, which survives SSH where the
/// environment does not.
pub(crate) fn progress_allowed(
    env: &impl Fn(&str) -> Option<String>,
    xtversion: Option<&str>,
) -> Result<&'static str, &'static str> {
    let var = |name: &str| env(name).filter(|value| !value.is_empty());
    let program = var("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();
    let program_version = var("TERM_PROGRAM_VERSION");
    let term = var("TERM").unwrap_or_default();
    let xt = xtversion.unwrap_or_default();

    if var("WT_SESSION").is_some() {
        return Ok("Windows Terminal");
    }
    if var("ConEmuPID").is_some() || var("ConEmuANSI").as_deref() == Some("ON") {
        return Ok("ConEmu");
    }
    for (needle, name) in [
        ("ghostty", "Ghostty"),
        ("vscode", "VS Code"),
        ("rio", "Rio"),
        ("contour", "contour"),
    ] {
        if program.contains(needle) {
            return Ok(name);
        }
    }
    if program.contains("wezterm") || xt.starts_with("WezTerm") {
        let date = program_version
            .as_deref()
            .and_then(leading_date)
            .or_else(|| xt.split_whitespace().nth(1).and_then(leading_date));
        return match date {
            Some(date) if date >= WEZTERM_PROGRESS_SINCE => Ok("WezTerm nightly"),
            Some(_) => Err("WezTerm stable shows OSC 9 as a notification"),
            None => Err("WezTerm version unknown"),
        };
    }
    if program.contains("iterm") || xt.starts_with("iTerm2") {
        let version = program_version
            .as_deref()
            .and_then(semver)
            .or_else(|| xt.split_whitespace().nth(1).and_then(semver));
        return match version {
            Some(version) if version >= ITERM_PROGRESS_SINCE => Ok("iTerm2"),
            Some(_) => Err("iTerm2 before 3.6 shows OSC 9 as a notification"),
            None => Err("iTerm2 version unknown"),
        };
    }
    if term == "xterm-kitty" || var("KITTY_WINDOW_ID").is_some() || xt.starts_with("kitty") {
        let version = xt
            .strip_prefix("kitty(")
            .and_then(|rest| rest.strip_suffix(')'))
            .and_then(semver);
        return match version {
            Some(version) if version >= KITTY_PROGRESS_SINCE => Ok("kitty"),
            Some(_) => Err("kitty before 0.47 may show OSC 9 as a notification"),
            None => Err("kitty version unknown"),
        };
    }
    if term.starts_with("foot") {
        return Ok("foot");
    }
    Err("terminal is not on the OSC 9;4 allow-list")
}

/// The leading `YYYYMMDD` of a `WezTerm` version such as `20240203-110809-5046fc22`.
fn leading_date(version: &str) -> Option<u32> {
    let digits = version.get(..8)?;
    if digits.bytes().all(|byte| byte.is_ascii_digit()) {
        digits.parse().ok()
    } else {
        None
    }
}

/// `major.minor[.patch]` with any pre-release suffix on the last part ignored.
fn semver(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().split('.').map(|part| {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u32>().ok()
    });
    let major = parts.next()??;
    let minor = parts.next()??;
    let patch = parts.next().flatten().unwrap_or(0);
    Some((major, minor, patch))
}

/// The OSC 9;4 progress states Vacuum uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressState {
    /// Determinate progress, 0–100.
    Normal(u8),
    /// Busy with no measurable fraction.
    Indeterminate,
}

/// Encode an OSC 9;4 update; `None` removes the indicator.
pub(crate) fn osc_progress(state: Option<ProgressState>) -> String {
    match state {
        None => "\x1b]9;4;0;0\x1b\\".to_owned(),
        Some(ProgressState::Normal(percent)) => format!("\x1b]9;4;1;{}\x1b\\", percent.min(100)),
        Some(ProgressState::Indeterminate) => "\x1b]9;4;3;0\x1b\\".to_owned(),
    }
}

/// The largest base64 payload Vacuum puts in one OSC 52 sequence. Several
/// terminals and tmux cap the sequence near 100 KB; staying just under keeps
/// the copy from being silently truncated or dropped.
pub(crate) const OSC52_MAX_BASE64: usize = 99_000;

/// Encode an OSC 52 clipboard write of `text`, or `None` when the payload
/// would exceed [`OSC52_MAX_BASE64`].
pub(crate) fn osc52(text: &str) -> Option<String> {
    let payload = base64(text.as_bytes());
    (payload.len() <= OSC52_MAX_BASE64).then(|| format!("\x1b]52;c;{payload}\x1b\\"))
}

/// Standard (RFC 4648 §4) padded base64. Ten lines here beat a dependency.
fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let byte = |i: usize| usize::from(chunk.get(i).copied().unwrap_or(0));
        let n = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        out.push(char::from(TABLE[(n >> 18) & 63]));
        out.push(char::from(TABLE[(n >> 12) & 63]));
        out.push(if chunk.len() > 1 {
            char::from(TABLE[(n >> 6) & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(TABLE[n & 63])
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |var| {
            pairs
                .iter()
                .find(|(name, _)| *name == var)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn parse_applies_items_left_to_right() {
        let overrides = ProtocolOverrides::parse("-all,+keyboard").expect("valid list");
        assert_eq!(overrides.get(Protocol::Keyboard), Some(true));
        assert_eq!(overrides.get(Protocol::Progress), Some(false));
        assert_eq!(overrides.get(Protocol::Pointer), Some(false));
    }

    #[test]
    fn parse_accepts_bare_names_spaces_and_empty_items() {
        let overrides = ProtocolOverrides::parse(" progress , ,-clipboard ").expect("valid list");
        assert_eq!(overrides.get(Protocol::Progress), Some(true));
        assert_eq!(overrides.get(Protocol::Clipboard), Some(false));
        assert_eq!(overrides.get(Protocol::Keyboard), None);
        assert_eq!(
            ProtocolOverrides::parse("").expect("empty"),
            ProtocolOverrides::default()
        );
    }

    #[test]
    fn parse_rejects_unknown_names() {
        let err = ProtocolOverrides::parse("+keyboard,-sixel").expect_err("sixel is not a name");
        assert!(err.to_string().contains("`sixel`"));
        assert!(ProtocolOverrides::parse("+").is_err());
    }

    #[test]
    fn flag_beats_env_name_by_name() {
        let env = ProtocolOverrides::parse("-progress,-keyboard").expect("env");
        let flag = ProtocolOverrides::parse("+progress").expect("flag");
        let merged = flag.over(env);
        assert_eq!(merged.get(Protocol::Progress), Some(true));
        assert_eq!(merged.get(Protocol::Keyboard), Some(false));
    }

    #[test]
    fn gate_blocks_pipes_agents_and_dumb_terminals() {
        let xterm = env_of(&[("TERM", "xterm-256color")]);
        assert_eq!(gate(true, false, &xterm), None);
        assert!(gate(false, false, &xterm).is_some());
        assert!(gate(true, true, &xterm).is_some());
        assert!(gate(true, false, &env_of(&[("TERM", "dumb")])).is_some());
        if cfg!(unix) {
            assert!(gate(true, false, &env_of(&[])).is_some());
        }
    }

    #[test]
    fn progress_allow_list_by_identity() {
        let ok =
            |pairs: &[(&str, &str)], xt: Option<&str>| progress_allowed(&env_of(pairs), xt).is_ok();
        assert!(ok(&[("WT_SESSION", "abc")], None));
        assert!(ok(&[("TERM_PROGRAM", "ghostty")], None));
        assert!(ok(&[("TERM", "foot-extra")], None));
        assert!(!ok(&[("TERM", "xterm-256color")], None));
        assert!(!ok(&[("TERM_PROGRAM", "Apple_Terminal")], None));
    }

    #[test]
    fn wezterm_needs_a_nightly_build() {
        let wez = |version: &str| {
            progress_allowed(
                &env_of(&[
                    ("TERM_PROGRAM", "WezTerm"),
                    ("TERM_PROGRAM_VERSION", version),
                ]),
                None,
            )
            .is_ok()
        };
        // The 2024 stable pops a notification per update: never.
        assert!(!wez("20240203-110809-5046fc22"));
        assert!(wez("20250210-000000-fb02395a"));
        assert!(wez("20261026-120000-e68a8ec0"));
        // Over SSH the environment is gone but XTVERSION survives.
        assert!(progress_allowed(&env_of(&[]), Some("WezTerm 20261026-120000-e68a8ec0")).is_ok());
        assert!(progress_allowed(&env_of(&[("TERM_PROGRAM", "WezTerm")]), None).is_err());
    }

    #[test]
    fn kitty_and_iterm_need_a_known_version() {
        let kitty = env_of(&[("TERM", "xterm-kitty")]);
        assert!(progress_allowed(&kitty, None).is_err());
        assert!(progress_allowed(&kitty, Some("kitty(0.46.2)")).is_err());
        assert!(progress_allowed(&kitty, Some("kitty(0.47.0)")).is_ok());

        let iterm = |version: &str| {
            progress_allowed(
                &env_of(&[
                    ("TERM_PROGRAM", "iTerm.app"),
                    ("TERM_PROGRAM_VERSION", version),
                ]),
                None,
            )
            .is_ok()
        };
        assert!(!iterm("3.5.11"));
        assert!(iterm("3.6.0beta2"));
    }

    #[test]
    fn multiplexers_are_detected() {
        assert_eq!(
            multiplexer(&env_of(&[("TMUX", "/tmp/tmux-1000/default,1,0")])),
            Some("tmux")
        );
        assert_eq!(multiplexer(&env_of(&[("ZELLIJ", "0")])), Some("zellij"));
        assert_eq!(multiplexer(&env_of(&[("TMUX", "")])), None);
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
    }

    #[test]
    fn osc52_encodes_and_caps_the_payload() {
        assert_eq!(osc52("/tmp").as_deref(), Some("\x1b]52;c;L3RtcA==\x1b\\"));
        assert!(osc52(&"x".repeat(OSC52_MAX_BASE64)).is_none());
    }

    #[test]
    fn osc_progress_sequences() {
        assert_eq!(osc_progress(None), "\x1b]9;4;0;0\x1b\\");
        assert_eq!(
            osc_progress(Some(ProgressState::Normal(140))),
            "\x1b]9;4;1;100\x1b\\"
        );
        assert_eq!(
            osc_progress(Some(ProgressState::Indeterminate)),
            "\x1b]9;4;3;0\x1b\\"
        );
    }
}
