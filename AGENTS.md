<!-- SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# AGENTS.md — Vacuum

Authoritative agent context (Standard §5.7). `CLAUDE.md` imports this file and
adds only Claude-Code-only notes.

## Project identity
**Vacuum** is a Rust TUI/CLI disk-space recovery tool under the Spacecraft Software
umbrella. It scans disk usage in parallel, categorizes reclaimable space
(regenerable build artifacts, package-manager garbage, app caches, large files),
and deletes selected items safely — XDG trash by default, `--purge` for permanent
removal.

- **Org:** Spacecraft Software · **Repo:** `github.com/Spacecraft-Software/Vacuum`
- **License:** software is `GPL-3.0-or-later` (local tool, not network-facing);
  the Texinfo manual (`doc/*.texi`) is `CC-BY-SA-4.0` — see Coding conventions.
- **Standard:** The Steelbore Standard
- **Maintainer:** Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
- **URL:** https://Vacuum.SpacecraftSoftware.org/

## Layout
- `crates/vacuum-core` — scan engine, `Cleaner` trait, `Deleter`, safety guards (`#![forbid(unsafe_code)]`).
- `crates/vacuum-cleaners` — the four cleaner catalogs.
- `crates/vacuum-theme` — `steelbore` theme tokens (§11.1).
- `crates/vacuum-cli` — binary `vacuum`: scan/list/clean + JSON envelope.
- `crates/vacuum-tui` — ratatui frontend.

## Coding conventions
- Edition 2024, MSRV 1.85; `#![forbid(unsafe_code)]` everywhere.
- REUSE two-tag SPDX header (§4.3) on every `.rs` file and manifest:
  ```
  // SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
  // SPDX-License-Identifier: GPL-3.0-or-later
  ```
- **Manual licensing (decided 2026-06-21, keep as-is):** the Texinfo manual is
  document-class → `CC-BY-SA-4.0` per Standard §4.1.1, *not* GPL. `doc/*.texi`
  carries the CC-BY-SA-4.0 SPDX tag and a matching `@copying` grant; `REUSE.toml`
  has a scoped `doc/*.texi` override so the repo-wide `**` GPL rule does not
  reclaim it, while software-class build assets under `doc/` (e.g.
  `spacecraft.css`) stay `GPL-3.0-or-later`. Do not "unify" the manual back to GPL.
- `thiserror` in libs, `anyhow` in the binary; no `unwrap`/`expect` on fallible input.
- `jiff`, UTC `Z` only, for all timestamps (§14).
- Concurrency designed-in: parallel scan via `jwalk` (§3.2).
- Colors only via the `steelbore` theme; Void Navy `#000027` background
  (§11/§11.1). No bare hex.
- Keybindings: CUA + vim `hjkl` (§10). PFA: zero telemetry (§9).

## CLI conformance (the CLI Standard)
- Commands: `scan`, `list`, `clean`, `empty-trash`, `schema`, `describe`, `tui`. Run
  `vacuum schema` for the authoritative, function-calling-ready contract; `vacuum describe`
  for a manifest. `empty-trash` permanently empties the OS trash (dry-run unless `--apply`;
  Linux/Windows only). `scan` and `tui` accept `--root` (merged with positional paths) like
  `list`/`clean`; the TUI also has a runtime directory picker (`r`) to add/remove scan roots.
  TUI scanning/deleting run on a background thread with a live progress overlay (spinner + bar);
  `esc` cancels a deletion (after the current item) or quits a scan.
- Terminal protocols (`spacecraft-tui-protocols`, `crates/vacuum-tui/src/{protocols,probe,session}.rs`):
  kitty keyboard (DA1-fenced `CSI ? u` probe), OSC 52 copy (`y`), OSC 9;4 progress
  (identity allow-list only — never on an unknown terminal, WezTerm stable, or inside a
  multiplexer). Graphics/animation/OSC 22/OSC 5522 are unused. Gate: stdout TTY, not
  agent/CI, `TERM` not dumb/unset. Probe runs after `ratatui::init` and before the first
  `event::poll`. Restore (keyboard pop, progress clear) runs on exit, panic (hook chained
  ahead of ratatui's), and SIGTERM/SIGHUP/SIGINT (flag polled each tick). Override:
  `--term-protocols` beats `SPACECRAFT_TERM_PROTOCOLS` name by name.
- Accessible mode (§18.1): `--accessible`/`--no-accessible` > `SPACECRAFT_A11Y` >
  config `[accessibility] enabled` > hints `NO_COLOR`/`TERM=dumb`; off by default. Today it
  only swaps the spinner for static percent text — the §18.2.3 linear TUI mode and the
  high-contrast theme are not implemented yet.
- Global flags: `--json` / `--format {human,json,jsonl,explore}`, `--fields`, `--apply`,
  `--purge`, `--yes`/`--force`, `--verbose`/`-v`, `--quiet`/`-q`, `--color`, `--no-color`,
  `--config <path>`, `--accessible`/`--no-accessible`, `--term-protocols <list>`.
- Config (scanning commands `scan`/`list`/`clean`/`tui`; `empty-trash` exempt): default roots
  come from `--config`/`VACUUM_CONFIG` (TOML, `roots = [...]`), else
  `$XDG_CONFIG_HOME/vacuum/config.toml`. Precedence for default roots:
  CLI paths/`--root` > `VACUUM_ROOTS` env > config `roots` > `$HOME`.
- JSON envelope: `{ metadata, data }` with maintainer + website; UTC `Z` timestamps;
  compact when piped, pretty on a TTY; optional fields omitted when absent.
- Errors (machine mode): `{ "error": { code, exit_code, message, hint, timestamp,
  command, docs_url } }` on stderr; `hint` is a runnable recovery command. stdout stays data-only.
- Canonical exit codes: `0` ok · `1` failure · `2` usage · `3` not-found · `4` refused/permission ·
  `5` conflict · `127` missing tool.
- Agent env detection (agentic-cli §4): **forcing** vars `AI_AGENT`/`AGENT`/`CI` suppress the
  TUI and force machine output; **informational** vars `CLAUDECODE`/`CURSOR_AGENT`/`GEMINI_CLI`
  do not change the mode — they only surface as `metadata.invoking_agent`.

## Safety invariants (do not weaken)
- Dry-run default; `--apply` required to act; trash unless `--purge`.
- Root allowlist (`$HOME` + explicit args); system-path denylist; no symlink-follow on delete;
  never delete source files; privileged GC prints `sudo`, never escalates.

## Local install (daily-drive testing)
Vacuum is the maintainer's daily driver, installed **declaratively from the Bravais flake**
(`/spacecraft-software/bravais`: the `vacuum` flake input, consumed in `users/mj/home.nix`),
not with `cargo install`. A change reaches the daily driver only after it is merged and
pushed to `main` here: Bravais's `rebuild` (preflight) then bumps every input, the
`vacuum` one included, and switches. That rebuild escalates, so the maintainer runs it;
an agent never does.

Verify with `vacuum --version`: the flake build reports `0.1.0-git`, and `command -v vacuum`
resolves under `/etc/profiles/per-user/mj/bin`. Do not `cargo install` this repo: the copy
would land in `~/.cargo/bin` and silently go stale (Bravais appends that directory to PATH,
so the flake build still wins). An unrelated crates.io crate is also named `vacuum` (a
spaceflight CLI), so never `cargo install vacuum` either. For a quick look at an unmerged
change, run `nix run .` or `./target/debug/vacuum` from the checkout.

## Toolchain
The dev shell's Rust comes from the `nixpkgs` flake input (`nix develop`), currently 1.99.
CI's fmt/clippy/test job installs the newest stable with rustup, so a newer clippy there
can flag code that passes locally. When that happens, `nix flake update nixpkgs` brings
the shell up to match, and the new lints get fixed in the same PR.

## Pre-commit gate
```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
reuse lint
```

All commits signed and "Verified" (§6.3).

## Don't
- Don't `git init` or create the GitHub repo without explicit maintainer authorization.
- Don't add network access or telemetry (§9 PFA).
- Don't use `apt`/`dnf`/`pacman` — Guix/Nix/Cargo/Flatpak only.
- Don't bypass commit signing (§6.3) or the pre-commit gate.
