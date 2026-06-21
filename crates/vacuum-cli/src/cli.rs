// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Command-line surface for `vacuum` (clap derive).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use vacuum_core::{Category, DeleteMode};

/// Long version string including the Standard §15.2 attribution block.
pub const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\n\nMaintained by Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>",
    "\nCopyright (C) 2026 Mohamed Hammad & Spacecraft Software  |  License: GPL-3.0-or-later",
    "\nhttps://Vacuum.SpacecraftSoftware.org/"
);

/// Fast, safe disk-space recovery for the terminal.
#[derive(Debug, Parser)]
#[command(
    name = "vacuum",
    version,
    long_version = LONG_VERSION,
    about = "Fast, safe disk-space recovery for the terminal (CLI + TUI).",
    after_help = "Maintained by Mohamed Hammad — https://Vacuum.SpacecraftSoftware.org/"
)]
pub struct Cli {
    /// Global, command-agnostic flags.
    #[command(flatten)]
    pub global: GlobalArgs,

    /// The subcommand to run; omitted launches the interactive TUI in a terminal.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Flags accepted by every subcommand.
#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Emit machine-readable JSON (shorthand for `--format json`).
    #[arg(long, global = true)]
    pub json: bool,

    /// Output format. Defaults to human in a terminal, JSON for agents/pipes.
    #[arg(long, global = true, value_enum)]
    pub format: Option<Format>,

    /// Color policy.
    #[arg(long, global = true, value_enum)]
    pub color: Option<ColorMode>,

    /// Disable color (equivalent to `--color never`).
    #[arg(long, global = true)]
    pub no_color: bool,
}

/// Output format selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Human-readable text.
    Human,
    /// Machine-readable JSON envelope.
    Json,
}

/// Color policy selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    /// Color when writing to a terminal.
    Auto,
    /// Always color.
    Always,
    /// Never color.
    Never,
}

/// The `vacuum` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show what is using disk space (parallel `du`/`df`-style report).
    Scan(ScanArgs),
    /// List reclaimable candidates by category (read-only).
    List(SelectArgs),
    /// Remove reclaimable space (dry-run unless `--apply`).
    Clean(CleanArgs),
    /// Launch the interactive TUI.
    Tui,
}

/// Arguments for `vacuum scan`.
#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Paths to scan. Defaults to your home directory.
    pub paths: Vec<PathBuf>,

    /// How many top consumers to show per scanned path.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

/// Category and root selection shared by `list` and `clean`.
#[derive(Debug, Args)]
pub struct SelectArgs {
    /// Restrict to these categories (repeatable). Defaults to all.
    #[arg(long = "category", value_name = "CATEGORY")]
    pub categories: Vec<String>,

    /// Roots to operate within (repeatable). Defaults to your home directory.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,
}

/// Arguments for `vacuum clean`.
#[derive(Debug, Args)]
pub struct CleanArgs {
    /// Category/root selection.
    #[command(flatten)]
    pub select: SelectArgs,

    /// Actually perform the removals. Without this, it is a dry run.
    #[arg(long)]
    pub apply: bool,

    /// Delete permanently instead of moving to the trash.
    #[arg(long)]
    pub purge: bool,

    /// Skip the confirmation prompt (use with care).
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl SelectArgs {
    /// Resolve the selected categories, parsing slugs and defaulting to all.
    ///
    /// # Errors
    ///
    /// Returns an error message naming any unknown category slug.
    pub fn selected_categories(&self) -> Result<Vec<Category>, String> {
        if self.categories.is_empty() {
            return Ok(Category::ALL.to_vec());
        }
        let mut out = Vec::new();
        for slug in &self.categories {
            let category =
                Category::from_slug(slug).ok_or_else(|| format!("unknown category: {slug}"))?;
            out.push(category);
        }
        Ok(out)
    }
}

impl CleanArgs {
    /// The deletion mode implied by `--purge`.
    pub fn delete_mode(&self) -> DeleteMode {
        if self.purge {
            DeleteMode::Purge
        } else {
            DeleteMode::Trash
        }
    }
}
