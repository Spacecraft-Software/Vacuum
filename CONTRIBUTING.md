<!-- SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org> -->
<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Contributing to Vacuum

Thank you for your interest. Vacuum is a Personal / Hobby project (Steelbore Standard §5);
contributions are welcome but accepted at the maintainer's sole discretion. Rejection reflects
fit with the project's direction, not the quality of your work.

## Before you start

- Open an issue describing the change before investing significant effort.
- For anything touching the **deletion path**, expect extra scrutiny — safety is the priority.

## Development workflow

This is a Cargo workspace (edition 2024). Run the full gate before opening a PR:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
reuse lint
```

## Coding conventions

- `#![forbid(unsafe_code)]` in every crate.
- Every source file and manifest carries the REUSE two-tag SPDX header:
  ```
  // SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
  // SPDX-License-Identifier: GPL-3.0-or-later
  ```
- Errors: `thiserror` in libraries, `anyhow` in the binary. No `unwrap`/`expect` on fallible input.
- Time: `jiff`, UTC `Z` only.
- Follow the Microsoft Pragmatic Rust Guidelines and The Steelbore Standard.

## Sign-off and signing

- Sign off your commits (`git commit -s`) to certify the Developer Certificate of Origin.
- All commits to this repository must be **cryptographically signed** and show "Verified" on
  GitHub (Steelbore Standard §6.3).

## Contributions license

By contributing, you agree your contributions are licensed under `GPL-3.0-or-later`.

## Security reporting

Report security-sensitive issues privately to `Mohamed.Hammad [at] SpacecraftSoftware.org`
rather than opening a public issue.
