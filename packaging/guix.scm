;;; SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
;;; SPDX-License-Identifier: GPL-3.0-or-later
;;;
;;; GNU Guix package definition for Vacuum.
;;; Build with:  guix build -f packaging/guix.scm

(use-modules (guix packages)
             (guix download)
             (guix build-system cargo)
             ((guix licenses) #:prefix license:))

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
     (list #:install-source? #f))
    (native-inputs (list))           ; texinfo added when the manual is installed
    (home-page "https://Vacuum.SpacecraftSoftware.org/")
    (synopsis "Fast, safe disk-space recovery for the terminal (CLI + TUI)")
    (description
     "Vacuum scans disk usage in parallel, groups reclaimable space into clear
categories (regenerable build artifacts, package-manager garbage, application
caches, and large files), and removes what you choose.  It is dry-run by
default and sends deletions to the trash unless permanent removal is requested.")
    (license license:gpl3+)))

vacuum
