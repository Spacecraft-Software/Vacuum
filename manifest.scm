;;; SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
;;; SPDX-License-Identifier: GPL-3.0-or-later
;;;
;;; Vacuum --- GNU Guix development environment.
;;;
;;; The Guix counterpart of the flake's devShell: everything needed to build,
;;; test, and lint the workspace, without installing anything on the host.
;;;
;;;     guix shell -m manifest.scm                 # enter the environment
;;;     guix shell -m manifest.scm -- cargo test --workspace
;;;     guix shell --pure -m manifest.scm          # ignore the ambient environment
;;;
;;; Packages are named by specification rather than imported from their defining
;;; modules, so this file does not have to track which `gnu/packages/*.scm' each
;;; one currently lives in.  `rust:cargo' selects the "cargo" output of the rust
;;; package, which is where Guix puts the cargo binary.
;;;
;;; The installable package definition lives in packaging/guix.scm
;;; (Standard section 5.5); this file is only about having a toolchain to hand.

(use-modules (gnu packages))

(specifications->manifest
 (list "rust"                ; rustc
       "rust:cargo"          ; cargo lives in the rust package's "cargo" output
       "gcc-toolchain"       ; mimalloc's -sys crate compiles C
       "mold"                ; Standard section 3.2.1: the linker LTO wants
       "texinfo"             ; section 8 --- make info, make html
       "reuse"               ; section 4.3 --- `reuse lint` must pass
       "make"
       "git"))
