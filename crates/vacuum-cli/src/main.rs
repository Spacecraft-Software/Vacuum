// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! `vacuum` — fast, safe disk-space recovery for the terminal.
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

mod agent;
mod app;
mod cli;
mod envelope;

use std::process::ExitCode;

use clap::Parser as _;
use mimalloc::MiMalloc;

use crate::cli::{Cli, Command};

/// Microsoft Rust Guidelines M-MIMALLOC-APPS: faster allocation on hot paths.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let command_line = std::env::args().collect::<Vec<_>>().join(" ");

    match run(&cli, &command_line) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("vacuum: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli, command_line: &str) -> anyhow::Result<ExitCode> {
    let mode = agent::output_mode(&cli.global);

    match &cli.command {
        Some(Command::Scan(args)) => app::run_scan(args, mode, command_line)?,
        Some(Command::List(args)) => app::run_list(args, mode, command_line)?,
        Some(Command::Clean(args)) => app::run_clean(args, mode, command_line)?,
        Some(Command::Tui) => vacuum_tui::run()?,
        None => {
            if agent::should_launch_tui(&cli.global) {
                vacuum_tui::run()?;
            } else {
                eprintln!(
                    "vacuum: no subcommand given. Try `vacuum --help`, `vacuum scan`, or `vacuum list`."
                );
                return Ok(ExitCode::from(2));
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}
