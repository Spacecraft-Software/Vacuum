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

## CLI conformance (SFRS)
- Global flags: `--json` / `--format`, `--dry-run` / `--apply`, `--yes`, `--color`, `--no-color`.
- JSON envelope: `{ metadata, data }` with maintainer + website; UTC `Z` timestamps.
- Agent env detection (`AI_AGENT`, `AGENT`, `CI`, `CLAUDECODE`, `CURSOR_AGENT`, `GEMINI_CLI`)
  suppresses the TUI and prefers machine output.

## Safety invariants (do not weaken)
- Dry-run default; `--apply` required to act; trash unless `--purge`.
- Root allowlist (`$HOME` + explicit args); system-path denylist; no symlink-follow on delete;
  never delete source files; privileged GC prints `sudo`, never escalates.

## Pre-commit gate
`cargo fmt --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test
--workspace` · `reuse lint`. All commits signed and "Verified" (§6.3).
