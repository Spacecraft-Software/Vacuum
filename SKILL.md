---
name: vacuum
description: >-
  Fast, safe disk-space recovery for the terminal. Scans disk usage in
  parallel, groups reclaimable space into categories (build artifacts,
  package-manager garbage, app caches, large files), and removes selected
  items safely — dry-run by default, XDG trash unless --purge.
license: GPL-3.0-or-later
homepage: https://Vacuum.SpacecraftSoftware.org/
maintainer: Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
---

<!-- SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Vacuum — capability surface

`vacuum` is a dual-mode (human + agent) CLI plus an interactive TUI. It conforms
to the Spacecraft Software Dual-Mode Self-Documenting CLI Standard (SFRS v1.0.0).

## Discover the surface programmatically

```sh
vacuum schema     # JSON Schema (Draft 2020-12) of every command — drops into
                  # Anthropic input_schema / MCP inputSchema unchanged
vacuum describe --json   # compact capability manifest (commands, categories, safety)
```

Prefer `vacuum schema` over parsing `--help`: it is the authoritative, stable
contract.

## Commands

| Command | Purpose | Mutates? |
|---------|---------|----------|
| `vacuum scan [PATHS…] [--root P] [--limit N]` | Largest consumers per path | read-only |
| `vacuum list [--category C] [--root P]` | Reclaimable candidates by category | read-only |
| `vacuum clean [--category C] [--root P] [--apply] [--purge] [--yes]` | Remove space | **write** (dry-run unless `--apply`) |
| `vacuum empty-trash [--apply] [--yes]` | Permanently empty the OS trash | **write** (dry-run unless `--apply`; Linux/Windows) |
| `vacuum schema` | Emit the command-surface JSON Schema | read-only |
| `vacuum describe` | Emit the capability manifest | read-only |
| `vacuum tui [PATHS…] [--root P]` | Launch the interactive TUI over the given roots | interactive |

Categories: `build-artifacts`, `package-gc`, `caches`, `large-files`.

The TUI accepts roots like the scanning commands and adds an in-app directory
picker: press `r` to browse the filesystem and add/remove scan roots at runtime
(space toggles a root, `esc` re-scans). Deletions stay bounded by the same roots
and safety guards as `clean`.

## Configuration (scanning commands)

Default scan roots persist in a TOML file (`roots = ["~/projects", …]`) at
`--config`/`VACUUM_CONFIG`, else `$XDG_CONFIG_HOME/vacuum/config.toml`. Default-root
precedence: CLI paths/`--root` > `VACUUM_ROOTS` env > config `roots` > `$HOME`.
These apply to `scan`, `list`, `clean`, and `tui`; `empty-trash` ignores them (it
acts on the OS trash, not a scan root).

## Output contract

- `--json` / `--format json` — single `{ metadata, data }` envelope; UTC `Z`
  timestamps; compact when piped, pretty on a TTY; absent optional fields omitted.
- `--format jsonl` — one record per line, unenveloped, for streaming.
- `--fields a,b,c` — project records to named keys (token economy).
- Errors in machine mode: `{ "error": { code, exit_code, message, hint, … } }`
  on stderr, where `hint` is a runnable recovery command. stdout stays clean.

## Exit codes

`0` ok · `1` general failure · `2` usage error (fix args, don't retry) ·
`3` not found · `4` permission/safety refusal · `5` conflict ·
`127` required external tool missing.

## Agent environment

`AI_AGENT`, `AGENT`, or `CI` (non-empty) ⇒ JSON, no color, no TUI,
non-interactive. `CLAUDECODE` / `CURSOR_AGENT` / `GEMINI_CLI` are informational
only and do not change the format.

## Safety (do not assume otherwise)

Dry-run is the default; `--apply` is required to act and, non-interactively,
also `--yes`/`--force`. Deletions go to the XDG trash unless `--purge`. Vacuum
stays within `$HOME` plus explicit `--root`s, refuses protected system paths,
never follows symlinks on delete, and never escalates privileges (it prints the
`sudo` command for privileged GC instead).
