;;; SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
;;; SPDX-License-Identifier: GPL-3.0-or-later
;;;
;;; Vacuum --- GNU Guix package definition (Standard section 5.5).
;;;
;;; Build with:  guix build -f packaging/guix.scm
;;;
;;; RELEASE NOTE: before pushing a release tag, bump `version', replace the
;;; base32 placeholder in the `origin' with the real tarball hash, and populate
;;; #:cargo-inputs from the committed Cargo.lock.  A placeholder hash must never
;;; reach a tagged release.
;;;
;;; STATUS: cargo-build-system resolves every dependency through #:cargo-inputs
;;; rather than from Cargo.lock, and this workspace has 191 locked crates.  That
;;; list is generated, not hand-written:
;;;
;;;     guix import crate -r vacuum
;;;
;;; Until it is generated and pasted in below, this definition will not build
;;; offline.  For a working install from a checkout today, see the "GNU Guix
;;; System" section of README.md, which uses `guix shell' plus `cargo install'.

(define-module (spacecraft vacuum)
  #:use-module (guix packages)
  #:use-module (guix download)
  #:use-module (guix build-system cargo)
  #:use-module (guix utils)
  #:use-module ((guix licenses) #:prefix license:)
  #:use-module (gnu packages crates-io)
  #:use-module (gnu packages texinfo))

(define-public vacuum
  (package
    (name "vacuum")
    (version "0.1.0")
    (source
     (origin
       (method url-fetch)
       (uri (string-append
             "https://github.com/Spacecraft-Software/Vacuum/archive/refs/tags/v"
             version ".tar.gz"))
       (file-name (string-append name "-" version ".tar.gz"))
       ;; TODO(release): replace with the real base32 hash before tagging.
       (sha256
        (base32 "0000000000000000000000000000000000000000000000000000"))))
    (build-system cargo-build-system)
    (arguments
     (list
      #:install-source? #f
      ;; Only the CLI crate produces a binary; the others are its libraries.
      #:cargo-build-flags '("--release" "-p" "vacuum-cli")
      ;; TODO(release): populate from `guix import crate -r vacuum'.
      #:cargo-inputs '()
      #:cargo-development-inputs '()
      #:phases
      #~(modify-phases %standard-phases
          ;; Standard section 8.6: the Texinfo manual is generated, not
          ;; committed, so it is built here and registered with install-info.
          (add-after 'build 'build-info-manual
            (lambda _
              (invoke "makeinfo" "--no-split"
                      "doc/vacuum.texi" "-o" "doc/vacuum.info")))
          (add-after 'install 'install-info-manual
            (lambda* (#:key outputs #:allow-other-keys)
              (let* ((out  (assoc-ref outputs "out"))
                     (info (string-append out "/share/info")))
                (mkdir-p info)
                (install-file "doc/vacuum.info" info))))
          (add-after 'install 'install-docs
            (lambda* (#:key outputs #:allow-other-keys)
              (let* ((out (assoc-ref outputs "out"))
                     (doc (string-append out "/share/doc/vacuum")))
                (mkdir-p doc)
                (install-file "README.md" doc)
                (install-file "NOTICE.md" doc)
                (install-file "LICENSE"   doc)))))))
    ;; No linked system libraries: the dependency tree needs only a C compiler
    ;; (for mimalloc's -sys crate), which cargo-build-system already provides.
    (native-inputs (list texinfo))
    (home-page "https://Vacuum.SpacecraftSoftware.org/")
    (synopsis "Fast, safe disk-space recovery for the terminal (CLI + TUI)")
    (description
     "Vacuum scans disk usage in parallel, groups reclaimable space into clear
categories (regenerable build artifacts, package-manager garbage, application
caches, stale temporary files, and large files), and removes what you choose.
It is dry-run by default and sends deletions to the trash unless permanent
removal is requested.  Operations needing root, such as a system-wide Nix
garbage collection, are printed as @command{sudo} lines for you to run rather
than escalated automatically.")
    (license license:gpl3+)))

vacuum
