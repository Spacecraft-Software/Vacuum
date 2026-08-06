# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Nix derivation for Vacuum (Standard section 5.5).
#
# Two ways in:
#
#   nix build                          # via flake.nix, builds this checkout
#   nix-build packaging/default.nix    # release build from the tagged tarball
#
# The flake passes `src = self`, which skips the fetchFromGitHub path entirely.
# That is what makes a from-checkout build work before a release is tagged.
#
# Dependencies are taken from the committed Cargo.lock rather than a vendor
# hash, so there is no `cargoHash` to regenerate on every dependency bump.

{ lib
, rustPlatform
, fetchFromGitHub
, texinfo
  # Override to build a working tree (the flake does). When null, the tagged
  # release tarball is fetched instead.
, src ? null
, version ? "0.1.0"
}:

rustPlatform.buildRustPackage {
  pname = "vacuum";
  inherit version;

  src =
    if src != null then
      src
    else
      fetchFromGitHub {
        owner = "Spacecraft-Software";
        repo = "Vacuum";
        rev = "v${version}";
        # TODO(release): replace with the real hash before tagging v0.1.0.
        # Until that tag exists this path cannot build; use the flake
        # (`nix build`), which builds the checkout directly.
        hash = lib.fakeHash;
      };

  # The lockfile is committed, so Nix can vendor from it directly. `outputHashes`
  # stays empty because every dependency comes from crates.io — no git deps.
  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  nativeBuildInputs = [ texinfo ];

  # Only the CLI crate produces a binary; the rest are libraries it links.
  cargoBuildFlags = [ "-p" "vacuum-cli" ];

  # Standard section 8: the Texinfo manual ships with the package. It is
  # generated, not committed, so it is built here.
  postBuild = ''
    make info
  '';

  postInstall = ''
    install -Dm644 doc/vacuum.info "$out/share/info/vacuum.info"
    install -Dm644 README.md   "$out/share/doc/vacuum/README.md"
    install -Dm644 NOTICE.md   "$out/share/doc/vacuum/NOTICE.md"
  '';

  meta = {
    description = "Fast, safe disk-space recovery for the terminal (CLI + TUI)";
    homepage = "https://Vacuum.SpacecraftSoftware.org/";
    license = lib.licenses.gpl3Plus;
    mainProgram = "vacuum";
    platforms = lib.platforms.unix;
  };
}
