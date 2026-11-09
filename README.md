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
| **Dev build artifacts** | Rust `target/`, `node_modules`, `.next`, `dist`, `build`, `__pycache__` — plus a native prune of dead units *inside* a `target/` you want to keep | Safe — fully regenerable |
| **Package-manager garbage** | `nix-collect-garbage -d` (user and system-wide), unused Flatpak runtimes, `journalctl --vacuum-time`, `systemd-tmpfiles --clean`, podman/docker prune | Low — reclaimed by each tool |
| **App / user caches** | `~/.cache`, browser caches, regenerable model blobs | Low |
| **Stale temp files** | Entries under `/tmp`, `/var/tmp`, `$TMPDIR` that are yours and untouched for 7+ days | Low — never touches live session state |
| **Large files** | The biggest files and directories, browsed interactively | Your call |

### About the Cargo prune

Deleting a whole `target/` costs a full rebuild. The prune instead removes only the
units that are already dead, so a project you are still working on keeps its warm
cache. It runs entirely off the filesystem — Vacuum never executes `cargo`,
`rustup`, or `rustc`, which matters on a Nix or Guix system where there may be no
runnable toolchain on `PATH` at all.

Three things are offered per target directory:

- **units from an old toolchain.** Each build unit records the compiler that made
  it. Units are grouped by that value and the group with the most recent activity
  is the one in use; the rest are left over from a compiler you have since
  changed. Where the version can be named it is (`2 units from rustc 1.95.0`).
- **cold units**, not rebuilt in `--stale-days` days (default 30). Freshness comes
  from the `invoked.timestamp` file Cargo writes for the purpose — never access
  time, which is not updated on `relatime` mounts when Cargo reuses an artifact.
- **the incremental cache**, which is pure rebuild-time state and often the
  largest single item in a `target/`.

Guards: a profile whose `.cargo-lock` is held by a running build is skipped
entirely; the unhashed final binaries are never touched; a unit whose fingerprint
cannot be read is kept, never swept; and sizes count hardlinked inodes once, so
the reported figure is what you actually get back.

These candidates overlap the whole-directory one for the same target — take one or
the other. Use `--cleaner cargo-prune` to select only the prune:

```sh
vacuum list  --cleaner cargo-prune
vacuum clean --cleaner cargo-prune --apply
```

### About the temp-file cleaner

It is the safe replacement for `sudo rm -r /tmp/*`, which destroys other users'
files and the live state of running processes. An entry is offered only when it is
not a symlink, is owned by you, has gone untouched for seven days, and is not
session state (`.X11-unix`, `systemd-private-*`, `.Trash-*`, and friends). Root-owned
leftovers are left to `sudo systemd-tmpfiles --clean`, which Vacuum prints for you.

These candidates are **purged rather than trashed**, even without `--purge`: the trash
for a path under `/tmp` is `/tmp/.Trash-$uid`, on the same filesystem, so trashing
would reclaim nothing at exactly the moment you need the space. Vacuum says so in
its output rather than doing it quietly, and `--apply` is still required.

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

In the TUI, `y` copies the highlighted path (OSC 52), and supported terminals get the
kitty keyboard protocol and tab/taskbar progress (OSC 9;4). Nothing is sent to a terminal
that does not support it; `--term-protocols=-all` turns all of it off. `--accessible` (or
`SPACECRAFT_A11Y=1`) replaces the spinner with static progress text.

## Install

> Do **not** run `cargo install vacuum`. An unrelated crates.io crate holds that
> name (a spaceflight CLI). Install from this repository, as below.

### Nix and NixOS

The flake needs no Rust toolchain on the host — it supplies one.

```sh
nix run github:Spacecraft-Software/Vacuum          # run it once, install nothing
nix profile install github:Spacecraft-Software/Vacuum   # install for this user
nix build github:Spacecraft-Software/Vacuum        # build only; ./result/bin/vacuum
```

To install it system-wide, add the flake as an input and enable the module:

```nix
{
  inputs.vacuum.url = "github:Spacecraft-Software/Vacuum";

  # in your NixOS configuration:
  # imports = [ inputs.vacuum.nixosModules.default ];
  # programs.vacuum.enable = true;
}
```

Or drop the package straight into `environment.systemPackages`:

```nix
environment.systemPackages = [ inputs.vacuum.packages.${pkgs.system}.default ];
```

For hacking on Vacuum itself, `nix develop` gives you rustc, cargo, clippy,
rustfmt, rust-analyzer, mold, reuse, and texinfo. `nix flake check` builds the
package and runs the test suite.

### GNU Guix System

`manifest.scm` supplies the toolchain; nothing is installed on the host until you
ask for it:

```sh
git clone https://github.com/Spacecraft-Software/Vacuum
cd Vacuum
guix shell -m manifest.scm -- cargo install --path crates/vacuum-cli
```

That puts `vacuum` in `~/.cargo/bin`. To work on the source instead:

```sh
guix shell -m manifest.scm            # toolchain only, host untouched
guix shell --pure -m manifest.scm     # ignore the ambient environment
```

A package definition lives in `packaging/guix.scm`, but it is **not buildable
yet**: `cargo-build-system` resolves dependencies through `#:cargo-inputs` rather
than from `Cargo.lock`, and this workspace has 191 locked crates. That list is
generated with `guix import crate -r vacuum` at release time. Until then, use the
`guix shell` route above.

### Arch Linux

```sh
cd packaging && makepkg -si
```

### From source

Requires Rust (edition 2024, MSRV 1.85+):

```sh
cargo install --path crates/vacuum-cli --force
```

`--force` replaces any previous install of the `vacuum` binary.

## Project Posture

Vacuum is a **Personal / Hobby** project (Steelbore Standard §5). Provided **AS IS**, with no
warranty and no service-level commitments — see [`NOTICE.md`](NOTICE.md). Contributions are
welcome but accepted at the maintainer's discretion — see [`CONTRIBUTING.md`](CONTRIBUTING.md).
Forking is encouraged.

## Maintainer

Maintained by Mohamed Hammad — `Mohamed.Hammad [at] SpacecraftSoftware.org`
Copyright (C) 2026 Mohamed Hammad & Spacecraft Software | License: `GPL-3.0-or-later`
<https://Vacuum.SpacecraftSoftware.org/>
