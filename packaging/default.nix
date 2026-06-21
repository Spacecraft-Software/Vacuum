# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Nix derivation for Vacuum. Build with:
#   nix-build packaging/default.nix

{ lib
, rustPlatform
, fetchFromGitHub
, texinfo
, installShellFiles ? null
}:

rustPlatform.buildRustPackage rec {
  pname = "vacuum";
  version = "0.1.0";

  src = fetchFromGitHub {
    owner = "Spacecraft-Software";
    repo = "Vacuum";
    rev = "v${version}";
    # TODO(release): replace with the real hash before tagging v${version}.
    hash = lib.fakeHash;
  };

  # TODO(release): set to the committed Cargo.lock hash before tagging.
  cargoHash = lib.fakeHash;

  nativeBuildInputs = [ texinfo ];

  cargoBuildFlags = [ "-p" "vacuum-cli" ];

  postBuild = ''
    make info
  '';

  postInstall = ''
    install -Dm644 doc/vacuum.info "$out/share/info/vacuum.info"
  '';

  meta = with lib; {
    description = "Fast, safe disk-space recovery for the terminal (CLI + TUI)";
    homepage = "https://Vacuum.SpacecraftSoftware.org/";
    license = licenses.gpl3Plus;
    maintainers = [ "Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>" ];
    mainProgram = "vacuum";
  };
}
