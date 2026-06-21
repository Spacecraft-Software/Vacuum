# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Convenience targets. The Rust build is driven by Cargo; these targets cover
# the Texinfo manual (Standard §8.2) and the standard quality gate.

CARGO ?= cargo
TEXI  = doc/vacuum.texi

.PHONY: all build test lint fmt check info html pdf doc clean

all: build

build:
	$(CARGO) build --release

test:
	$(CARGO) test --workspace

lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all -- --check

# Full pre-commit gate.
check: fmt lint test

info: $(TEXI)
	makeinfo --no-split $(TEXI) -o doc/vacuum.info

html: $(TEXI)
	makeinfo --html --no-split $(TEXI) -o doc/vacuum.html

pdf: $(TEXI)
	texi2pdf -q -o doc/vacuum.pdf $(TEXI)

doc: info html pdf

clean:
	$(CARGO) clean
	rm -f doc/vacuum.info doc/vacuum.html doc/vacuum.pdf
