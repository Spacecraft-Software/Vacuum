<!-- SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# AGENTS.md — Vacuum

Tool-agnostic conventions for AI agents working in this repository. (Claude Code: see also
`CLAUDE.md`, which is gitignored.)

## Project identity
- **Name:** Vacuum — disk-space recovery TUI/CLI
- **Org:** Spacecraft Software · **License:** `GPL-3.0-or-later`
- **Standard:** The Steelbore Standard v1.25
- **Maintainer:** Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
- **URL:** https://Vacuum.SpacecraftSoftware.org/

## Layout
- `crates/vacuum-core` — scan engine, `Cleaner` trait, `Deleter`, safety guards.
- `crates/vacuum-cleaners` — the four cleaner catalogs.
- `crates/vacuum-theme` — `steelbore` theme tokens.
- `crates/vacuum-cli` — binary `vacuum`.
- `crates/vacuum-tui` — ratatui frontend.

## Coding conventions
- Edition 2024, MSRV 1.85; `#![forbid(unsafe_code)]` everywhere.
- REUSE two-tag SPDX header on every `.rs` file and manifest.
- `thiserror` in libs, `anyhow` in the binary; no `unwrap`/`expect` on fallible input.
- `jiff` UTC `Z` for all timestamps.
- Concurrency designed-in (parallel `jwalk` scan).

## CLI conformance (the CLI Standard)
- Commands: `scan`, `list`, `clean`, `empty-trash`, `schema`, `describe`, `tui`. Run
  `vacuum schema` for the authoritative, function-calling-ready contract; `vacuum describe`
  for a manifest. `empty-trash` permanently empties the OS trash (dry-run unless `--apply`;
  Linux/Windows only). `scan` and `tui` accept `--root` (merged with positional paths) like
  `list`/`clean`; the TUI also has a runtime directory picker (`r`) to add/remove scan roots.
  TUI scanning/deleting run on a background thread with a live progress overlay (spinner + bar);
  `esc` cancels a deletion (after the current item) or quits a scan.
- Global flags: `--json` / `--format {human,json,jsonl,explore}`, `--fields`, `--apply`,
  `--purge`, `--yes`/`--force`, `--verbose`/`-v`, `--quiet`/`-q`, `--color`, `--no-color`,
  `--config <path>`.
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
Vacuum is the maintainer's daily driver. After landing changes, **build and install the
latest build to `~/.cargo/bin` so `vacuum` runs from any directory** (not just the repo):

```
cargo install --path crates/vacuum-cli --force
```

`--force` is required because the binary is named `vacuum` (it replaces any prior install).
Verify with `vacuum --version` (current `0.1.0` + maintainer line) and `command -v vacuum`
→ `~/.cargo/bin/vacuum`. Note: an unrelated crates.io crate is also named `vacuum` (a
spaceflight CLI); re-installing from this repo keeps `vacuum` pointed at the disk cleaner.
Prefer this over running `./target/release/vacuum` so daily use matches a real install.

## Pre-commit gate
`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test
--workspace` · `reuse lint`. All commits signed and "Verified" (§6.3).
