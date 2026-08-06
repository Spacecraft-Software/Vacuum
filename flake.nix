# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Nix flake for Vacuum --- fast, safe disk-space recovery for the terminal.
#
# Usage:
#   nix run .                          # run vacuum without installing it
#   nix build                          # build; result/bin/vacuum
#   nix profile install .              # install for the current user
#   nix develop                        # development shell
#   nix develop -c cargo test --workspace
#   nix flake check                    # build + test suite
#
# The package is defined once, in packaging/default.nix (Standard section 5.5);
# this flake passes `src = self` so it builds the checkout rather than a tagged
# tarball. That matters today: no release tag exists yet, so the fetchFromGitHub
# path in packaging/default.nix cannot resolve.
{
  description = "Vacuum --- fast, safe disk-space recovery for the terminal (CLI + TUI)";

  inputs = {
    nixpkgs.url     = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      # System-independent: the NixOS module resolves the package from the
      # host's own pkgs, so it works regardless of the flake's eval system.
      nixosModules.default = { config, lib, pkgs, ... }:
        let cfg = config.programs.vacuum;
        in {
          options.programs.vacuum = {
            enable = lib.mkEnableOption "Vacuum, a disk-space recovery CLI and TUI";
            package = lib.mkOption {
              type = lib.types.package;
              default = pkgs.callPackage ./packaging/default.nix {
                srcOverride = self;
                version = "0.1.0-git";
              };
              defaultText = lib.literalMD "the `vacuum` package from this flake";
              description = "The Vacuum package to install.";
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ];
          };
        };
    in
    {
      inherit nixosModules;
    }
    // flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        vacuum = pkgs.callPackage ./packaging/default.nix {
          srcOverride = self;
          # Not a release: mark it so `vacuum --version` never claims to be the
          # tagged 0.1.0 when it is really whatever is checked out.
          version = "0.1.0-git";
        };

        common = with pkgs; [
          # The Rust toolchain from nixpkgs rather than through rustup.
          #
          # rustup would honour rust-toolchain.toml, but it downloads and stores
          # a second complete toolchain (~1.5 GB) beside the one nixpkgs already
          # has. nixpkgs' rustc is well past this workspace's 1.85 minimum, so
          # the pin buys nothing here and costs real disk. rust-toolchain.toml
          # stays in the tree for contributors who build outside Nix; inside
          # this shell it is simply not consulted.
          rustc
          cargo
          clippy
          rustfmt
          rust-analyzer

          gcc             # mimalloc's -sys crate compiles C.
          pkg-config
          mold            # Standard section 3.2.1: the linker LTO wants on NixOS.

          reuse           # section 4.3 --- `reuse lint` must pass.
          texinfo         # section 8 --- make info, make html.

          git
          gnumake
        ];
      in
      {
        packages.default = vacuum;
        packages.vacuum = vacuum;

        apps.default = {
          type = "app";
          program = "${vacuum}/bin/vacuum";
        };

        # `nix flake check` builds the package, which runs `cargo test` as part
        # of buildRustPackage's check phase.
        checks.default = vacuum;

        devShells.default = pkgs.mkShell {
          name = "vacuum-dev";
          nativeBuildInputs = common;
          shellHook = ''
            echo "vacuum dev shell."
            echo "  cargo test --workspace                       tests"
            echo "  cargo clippy --workspace --all-targets -- -D warnings"
            echo "  cargo fmt --all -- --check                   formatting"
            echo "  reuse lint                                   section 4.3 gate"
            echo "  make info                                    Texinfo manual"
            echo "  nix develop .#docs                           + texi2pdf"
          '';
        };

        # `make pdf` only. Standard section 8 wants all three formats, but only
        # at release time --- TeX Live is hundreds of megabytes that someone
        # editing a cleaner never touches.
        devShells.docs = pkgs.mkShell {
          name = "vacuum-docs";
          nativeBuildInputs = common ++ [ pkgs.texliveSmall ];
        };
      }
    );
}
