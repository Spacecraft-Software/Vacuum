// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `vacuum` — fast, safe disk-space recovery for the terminal.
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

mod agent;
mod app;
mod cli;
mod config;
mod envelope;
mod error;
mod introspect;
mod trashbin;

use std::process::ExitCode;

use clap::Parser as _;
use mimalloc::MiMalloc;

use crate::cli::{Cli, Command, Format};
use crate::error::{CliError, CliResult};

/// Microsoft Rust Guidelines M-MIMALLOC-APPS: faster allocation on hot paths.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command_line = std::env::args().collect::<Vec<_>>().join(" ");
    let mode = agent::output_mode(&cli.global);

    match run(&cli, mode, &command_line) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => err.emit(mode, &command_line),
    }
}

fn run(cli: &Cli, mode: agent::OutputMode, command_line: &str) -> CliResult<()> {
    // `--format explore` is an interactive request. Honor it for any data
    // command — but never trap an agent in a render loop (the CLI Standard §5): under an
    // AI agent it degrades to the JSON path below with a warning.
    if cli.global.format == Some(Format::Explore) {
        if agent::is_agent() {
            if !cli.global.quiet {
                eprintln!("vacuum: --format explore is interactive; using JSON for agent use");
            }
        } else {
            let fallback = config::fallback_roots(cli.global.config.as_deref())?;
            return app::run_tui(&[], &fallback);
        }
    }

    match &cli.command {
        // Commands that operate within roots resolve the configured/default
        // fallback once (VACUUM_ROOTS → config file → $HOME). It is computed
        // lazily, per arm, so an unrelated command (e.g. `schema`) never fails
        // on a malformed config it does not use.
        Some(Command::Scan(args)) => {
            let fallback = config::fallback_roots(cli.global.config.as_deref())?;
            app::run_scan(args, mode, &cli.global, &fallback, command_line)
        }
        Some(Command::List(args)) => {
            let fallback = config::fallback_roots(cli.global.config.as_deref())?;
            app::run_list(args, mode, &cli.global, &fallback, command_line)
        }
        Some(Command::Clean(args)) => {
            let fallback = config::fallback_roots(cli.global.config.as_deref())?;
            app::run_clean(args, mode, &cli.global, &fallback, command_line)
        }
        Some(Command::EmptyTrash(args)) => {
            app::run_empty_trash(args, mode, &cli.global, command_line)
        }
        Some(Command::Schema) => introspect::run_schema(),
        Some(Command::Describe) => introspect::run_describe(mode, command_line),
        Some(Command::Tui(args)) => {
            let fallback = config::fallback_roots(cli.global.config.as_deref())?;
            app::run_tui(&args.explicit_roots(), &fallback)
        }
        None => {
            if agent::should_launch_tui(&cli.global) {
                let fallback = config::fallback_roots(cli.global.config.as_deref())?;
                app::run_tui(&[], &fallback)
            } else {
                Err(CliError::no_command())
            }
        }
    }
}
