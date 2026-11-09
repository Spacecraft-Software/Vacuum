// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The startup capability probe (`spacecraft-tui-protocols` §2.2).
//!
//! Vacuum asks the terminal rather than guessing: it writes the kitty keyboard
//! query (`CSI ? u`) and XTVERSION (`CSI > 0 q`), then DA1 (`CSI c`) last.
//! Every terminal answers DA1 and terminals answer in order, so a feature that
//! has not replied by the time the DA1 reply arrives is unsupported. The whole
//! exchange is capped by a deadline, and late bytes are drained so no reply
//! leaks into the shell as typed text.
//!
//! The probe runs once, after raw mode is on and before the event loop first
//! reads input, so crossterm's reader never sees the replies. Keys typed in
//! that ~200 ms window are discarded with them.
//!
//! It reads `/dev/tty` with `poll(2)` and never blocks past the deadline. On
//! macOS `poll` does not work on terminal devices, and Windows has no
//! `/dev/tty`, so there the probe reports nothing and every probed feature
//! stays off unless the user forces it.

use std::time::Duration;

/// The kitty keyboard flags query.
pub(crate) const QUERY_KEYBOARD: &[u8] = b"\x1b[?u";
/// XTVERSION: the terminal's own name and version, which survives SSH.
pub(crate) const QUERY_XTVERSION: &[u8] = b"\x1b[>0q";
/// DA1, the fence. Always sent last.
pub(crate) const QUERY_DA1: &[u8] = b"\x1b[c";

/// How long to keep swallowing late replies after the fence or the deadline.
#[cfg(all(unix, not(target_os = "macos")))]
const DRAIN_WAIT: Duration = Duration::from_millis(20);

/// Upper bound on bytes buffered during the probe. Replies are tens of bytes;
/// this only guards against a flood of held-down keys.
#[cfg(all(unix, not(target_os = "macos")))]
const MAX_PROBE_BYTES: usize = 16 * 1024;

/// What the terminal answered. Everything defaults to "no reply".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Replies {
    /// The terminal answered `CSI ? u`: it speaks the kitty keyboard protocol.
    pub kitty_keyboard: bool,
    /// The XTVERSION reply text, e.g. `kitty(0.47.0)` or `WezTerm 20250210-…`.
    pub xtversion: Option<String>,
    /// The DA1 fence reply arrived.
    pub da1: bool,
}

/// Send `queries` (which must end with [`QUERY_DA1`]) and collect the replies
/// within `deadline`. Never fails: any I/O error means "nothing supported".
pub(crate) fn probe(queries: &[u8], deadline: Duration) -> Replies {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        tty::probe(queries, deadline).unwrap_or_default()
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = (queries, deadline);
        Replies::default()
    }
}

/// Parse every complete reply in `buf`. Incomplete trailing sequences are left
/// for the next read; anything that is not a reply (a typed key) is skipped.
pub(crate) fn parse_replies(buf: &[u8]) -> Replies {
    const ESC: u8 = 0x1b;
    let mut replies = Replies::default();
    let mut i = 0;
    while i < buf.len() {
        if buf[i] != ESC {
            i += 1;
            continue;
        }
        match buf.get(i + 1) {
            // CSI: parameters, then one final byte in 0x40..=0x7E.
            Some(b'[') => {
                let params_start = i + 2;
                let Some(offset) = buf[params_start..]
                    .iter()
                    .position(|byte| (0x40..=0x7e).contains(byte))
                else {
                    break; // incomplete
                };
                let end = params_start + offset;
                if buf.get(params_start) == Some(&b'?') {
                    match buf[end] {
                        b'u' => replies.kitty_keyboard = true,
                        b'c' => replies.da1 = true,
                        _ => {}
                    }
                }
                i = end + 1;
            }
            // DCS: body up to ST (`ESC \`) or BEL.
            Some(b'P') => {
                let body_start = i + 2;
                let Some((body_end, next)) = find_string_end(buf, body_start) else {
                    break; // incomplete
                };
                if let Some(text) = buf[body_start..body_end].strip_prefix(b">|") {
                    let text = String::from_utf8_lossy(text).trim().to_owned();
                    if !text.is_empty() {
                        replies.xtversion = Some(text);
                    }
                }
                i = next;
            }
            None => break,
            Some(_) => i += 1,
        }
    }
    replies
}

/// Find the end of a string-type sequence starting at `from`: the index where
/// its terminator begins and the index just past it.
fn find_string_end(buf: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut j = from;
    while j < buf.len() {
        match buf[j] {
            0x07 => return Some((j, j + 1)),
            0x1b if buf.get(j + 1) == Some(&b'\\') => return Some((j, j + 2)),
            _ => j += 1,
        }
    }
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
mod tty {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read as _, Write as _};
    use std::time::{Duration, Instant};

    use rustix::event::{PollFd, PollFlags, Timespec, poll};

    use super::{DRAIN_WAIT, MAX_PROBE_BYTES, Replies, parse_replies};

    pub(super) fn probe(queries: &[u8], deadline: Duration) -> io::Result<Replies> {
        let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        tty.write_all(queries)?;
        tty.flush()?;

        let end = Instant::now() + deadline;
        let mut buf = Vec::new();
        let mut chunk = [0_u8; 1024];
        let mut replies = Replies::default();
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if !wait_readable(&tty, left)? {
                break; // deadline: unanswered features stay off
            }
            let n = tty.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            replies = parse_replies(&buf);
            if replies.da1 || buf.len() >= MAX_PROBE_BYTES {
                break;
            }
        }
        drain(&mut tty);
        Ok(replies)
    }

    /// Wait up to `timeout` for `tty` to become readable.
    fn wait_readable(tty: &File, timeout: Duration) -> io::Result<bool> {
        let timeout = Timespec::try_from(timeout).map_err(io::Error::other)?;
        let mut fds = [PollFd::new(tty, PollFlags::IN)];
        let ready = poll(&mut fds, Some(&timeout))?;
        Ok(ready > 0 && fds[0].revents().contains(PollFlags::IN))
    }

    /// Swallow replies that arrive after the fence or the deadline, so none
    /// reaches the event loop (or the shell) as typed text. Bounded so a
    /// held-down key cannot keep it spinning.
    fn drain(tty: &mut File) {
        let mut chunk = [0_u8; 1024];
        for _ in 0..64 {
            match wait_readable(tty, DRAIN_WAIT) {
                Ok(true) => {
                    if !matches!(tty.read(&mut chunk), Ok(n) if n > 0) {
                        return;
                    }
                }
                _ => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Replies, parse_replies};

    #[test]
    fn full_reply_set() {
        let replies = parse_replies(b"\x1b[?1u\x1bP>|kitty(0.47.0)\x1b\\\x1b[?62;4;22c");
        assert_eq!(
            replies,
            Replies {
                kitty_keyboard: true,
                xtversion: Some("kitty(0.47.0)".to_owned()),
                da1: true,
            }
        );
    }

    #[test]
    fn da1_only_means_unsupported() {
        let replies = parse_replies(b"\x1b[?65;1;9c");
        assert!(replies.da1);
        assert!(!replies.kitty_keyboard);
        assert_eq!(replies.xtversion, None);
    }

    #[test]
    fn wezterm_xtversion_with_bel_and_typed_keys() {
        let replies = parse_replies(b"q\x1bP>|WezTerm 20250210-000000-fb02395a\x07j\x1b[?62c");
        assert_eq!(
            replies.xtversion.as_deref(),
            Some("WezTerm 20250210-000000-fb02395a")
        );
        assert!(replies.da1);
    }

    #[test]
    fn split_reply_is_incomplete_until_finished() {
        assert!(!parse_replies(b"\x1b[?0").kitty_keyboard);
        assert!(!parse_replies(b"\x1bP>|foot(1.2").da1);
        assert!(parse_replies(b"\x1b[?0u").kitty_keyboard);
    }

    #[test]
    fn non_private_csi_is_not_a_reply() {
        // A cursor report (`CSI 12;4 R`) or an arrow key is not DA1.
        let replies = parse_replies(b"\x1b[12;4R\x1b[A");
        assert_eq!(replies, Replies::default());
    }
}
