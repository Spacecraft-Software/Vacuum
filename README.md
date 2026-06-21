<!-- SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Vacuum

**Vacuum** is a fast, safe disk-space recovery tool for the terminal — a CLI with an
interactive TUI. It scans your disk usage in parallel, groups the reclaimable space into
clear categories, and lets you delete what you choose. Like jettisoning ballast, it frees
capacity without throwing away anything you need.

Part of the [Spacecraft Software](https://SpacecraftSoftware.org/) ecosystem, governed by
The Steelbore Standard.

## What it finds

| Category | Examples | Risk |
|----------|----------|------|
| **Dev build artifacts** | Rust `target/`, `node_modules`, `.next`, `dist`, `build`, `__pycache__` | Safe — fully regenerable |
| **Package-manager garbage** | `nix-collect-garbage -d`, unused Flatpak runtimes, cargo registry cache, journald logs | Low — reclaimed by each tool |
| **App / user caches** | `~/.cache`, browser caches, regenerable model blobs | Low |
| **Large files** | The biggest files and directories, browsed interactively | Your call |

## Safety first

- **Dry-run by default.** Nothing is deleted until you pass `--apply`.
- **Recoverable deletes.** Removals go to the XDG trash unless you pass `--purge`.
- **Bounded scope.** Vacuum only operates inside your home directory (plus paths you name)
  and refuses to touch system locations. It never follows symlinks when deleting and never
  deletes source code.
- **No silent privilege escalation.** Operations that need root (e.g. a system Nix GC) print
  the exact `sudo` command for you to run — Vacuum never escalates on its own.

## Usage

```sh
vacuum                       # interactive TUI (in a terminal)
vacuum scan ~                # show what's using space (parallel du/df)
vacuum list                  # list reclaimable candidates by category
vacuum clean --category build-artifacts          # dry-run: show what would be freed
vacuum clean --category build-artifacts --apply  # move to trash
vacuum clean --category build-artifacts --apply --purge   # permanent delete
vacuum list --json           # machine-readable output for scripts/agents
```

## Install

Requires Rust (edition 2024, MSRV 1.85+). From a clone:

```sh
cargo install --path crates/vacuum-cli
```

## Project Posture

Vacuum is a **Personal / Hobby** project (Steelbore Standard §5). Provided **AS IS**, with no
warranty and no service-level commitments — see [`NOTICE.md`](NOTICE.md). Contributions are
welcome but accepted at the maintainer's discretion — see [`CONTRIBUTING.md`](CONTRIBUTING.md).
Forking is encouraged.

## Maintainer

Maintained by Mohamed Hammad — `Mohamed.Hammad [at] SpacecraftSoftware.org`
Copyright (C) 2026 Mohamed Hammad & Spacecraft Software | License: `GPL-3.0-or-later`
<https://Vacuum.SpacecraftSoftware.org/>
