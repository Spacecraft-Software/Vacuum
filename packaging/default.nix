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
# The flake passes `srcOverride = self`, skipping the fetchFromGitHub path.
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
  #
  # Deliberately not named `src`. callPackage fills any unbound argument from
  # nixpkgs, and nixpkgs *has* a `src` attribute (a package since renamed to
  # simple-revision-control), so a `src ? null` default is silently overridden
  # and `callPackage ./packaging/default.nix { }` fails with a baffling error
  # about that unrelated package. `version` has no such clash.
, srcOverride ? null
, version ? "0.1.0"
}:

rustPlatform.buildRustPackage {
  pname = "vacuum";
  inherit version;

  src =
    if srcOverride != null then
      srcOverride
    else
      fetchFromGitHub {
        owner = "Spacecraft-Software";
        repo = "Vacuum";
        rev = "v${version}";
        # Hash of the unpacked tree at tag v0.1.0. Unlike the tarball hashes
        # the other packagers need, this is independent of GitHub's archive
        # compression, so it stays valid even if that changes.
        hash = "sha256-ZS/oaqLIzBSTtJ/s5YnI3sHCqztNzpCwZ/Cyi/PLA40=";
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
