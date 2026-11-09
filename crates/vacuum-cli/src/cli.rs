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
///
/// Several independent on/off switches are inherent to a CLI flag surface, so
/// the "excessive bools" lint does not apply here.
#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct GlobalArgs {
    /// Emit machine-readable JSON (shorthand for `--format json`).
    #[arg(long, global = true)]
    pub json: bool,

    /// Output format. Defaults to human in a terminal, JSON for agents/pipes.
    #[arg(long, global = true, value_enum)]
    pub format: Option<Format>,

    /// Restrict JSON records to these fields (comma-separated). Trims token cost.
    #[arg(long, global = true, value_name = "F1,F2,...", value_delimiter = ',')]
    pub fields: Vec<String>,

    /// Color policy.
    #[arg(long, global = true, value_enum)]
    pub color: Option<ColorMode>,

    /// Disable color (equivalent to `--color never`).
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Diagnostic output to stderr.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,

    /// Suppress non-error stderr.
    #[arg(long, short = 'q', global = true)]
    pub quiet: bool,

    /// Read default scan roots from this TOML config file. Defaults to
    /// `$XDG_CONFIG_HOME/vacuum/config.toml`. Overridden by explicit paths/roots
    /// on the command line and by `VACUUM_ROOTS`.
    #[arg(long, global = true, value_name = "PATH", env = "VACUUM_CONFIG")]
    pub config: Option<PathBuf>,

    /// Accessible mode (Standard §18): no animation, progress as text. Also
    /// `SPACECRAFT_A11Y=1` or `[accessibility] enabled = true` in the config.
    #[arg(long, global = true, overrides_with = "no_accessible")]
    pub accessible: bool,

    /// Force accessible mode off, overriding the environment, the config and
    /// the `NO_COLOR` / `TERM=dumb` hints.
    #[arg(long, global = true, overrides_with = "accessible")]
    pub no_accessible: bool,

    /// Terminal protocol overrides for the TUI, e.g. `-all` or
    /// `-keyboard,+progress`. Names: keyboard, graphics, animation, clipboard,
    /// clipboard-rich, progress, pointer, all. Beats
    /// `SPACECRAFT_TERM_PROTOCOLS` name by name.
    #[arg(long, global = true, value_name = "LIST", allow_hyphen_values = true)]
    pub term_protocols: Option<String>,
}

/// Output format selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Human-readable text.
    Human,
    /// Machine-readable JSON envelope (single document).
    Json,
    /// Newline-delimited JSON: one record per line, for streaming consumers.
    Jsonl,
    /// Launch the interactive TUI (falls back to JSON under an AI agent).
    Explore,
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
    /// Permanently empty the OS trash (dry-run unless `--apply`).
    EmptyTrash(EmptyTrashArgs),
    /// Launch the interactive TUI over the given roots (defaults to `$HOME`).
    Tui(TuiArgs),
    /// Emit the JSON Schema of the command surface (for LLM function-calling).
    Schema,
    /// Emit a compact capability manifest.
    Describe,
}

/// Arguments for `vacuum scan`.
#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Paths to scan. Defaults to your home directory.
    pub paths: Vec<PathBuf>,

    /// Additional roots to scan (repeatable). Merged with the positional paths;
    /// accepted for parity with `list`/`clean`.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,

    /// How many top consumers to show per scanned path.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

impl ScanArgs {
    /// The explicit roots to scan: positional paths plus any `--root` flags.
    pub fn explicit_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.paths.clone();
        roots.extend(self.roots.iter().cloned());
        roots
    }
}

/// Arguments for `vacuum tui`.
#[derive(Debug, Args)]
pub struct TuiArgs {
    /// Roots to scan. Defaults to the configured/default roots (`VACUUM_ROOTS`,
    /// then the config file, then `$HOME`).
    pub paths: Vec<PathBuf>,

    /// Additional roots to scan (repeatable). Merged with the positional paths;
    /// accepted for parity with `scan`/`list`/`clean`.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,
}

impl TuiArgs {
    /// The explicit roots to scan: positional paths plus any `--root` flags.
    pub fn explicit_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.paths.clone();
        roots.extend(self.roots.iter().cloned());
        roots
    }
}

/// Category and root selection shared by `list` and `clean`.
#[derive(Debug, Args)]
pub struct SelectArgs {
    /// Restrict to these categories (repeatable). Defaults to all.
    #[arg(long = "category", value_name = "CATEGORY")]
    pub categories: Vec<String>,

    /// Restrict to these cleaners by id (repeatable). Defaults to all.
    ///
    /// Finer-grained than `--category`: several cleaners can share a category,
    /// and their candidates may deliberately overlap. `--cleaner cargo-prune`
    /// takes only the per-unit Cargo prune, leaving the whole-directory
    /// build-artifacts candidate alone. `vacuum describe` lists the ids.
    #[arg(long = "cleaner", value_name = "ID")]
    pub cleaners: Vec<String>,

    /// Roots to operate within (repeatable). Defaults to your home directory.
    #[arg(long = "root", value_name = "PATH")]
    pub roots: Vec<PathBuf>,

    /// Days of disuse after which a build artifact counts as stale (default 30).
    #[arg(long = "stale-days", value_name = "DAYS")]
    pub stale_days: Option<u64>,
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
    #[arg(long, short = 'y', visible_alias = "force")]
    pub yes: bool,
}

/// Arguments for `vacuum empty-trash`.
#[derive(Debug, Args)]
pub struct EmptyTrashArgs {
    /// Actually purge the trash. Without this, it is a dry run that only lists
    /// what would be permanently removed.
    #[arg(long)]
    pub apply: bool,

    /// Skip the confirmation prompt (required for non-interactive `--apply`).
    #[arg(long, short = 'y', visible_alias = "force")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_merges_positional_and_root_flags() {
        let cli = Cli::try_parse_from(["vacuum", "tui", "/a", "--root", "/b"])
            .expect("tui args should parse");
        let Some(Command::Tui(args)) = cli.command else {
            panic!("expected the tui subcommand");
        };
        assert_eq!(
            args.explicit_roots(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn term_protocols_accepts_a_leading_minus() {
        let cli = Cli::try_parse_from(["vacuum", "tui", "--term-protocols", "-all,+keyboard"])
            .expect("a list starting with - is a value, not a flag");
        assert_eq!(cli.global.term_protocols.as_deref(), Some("-all,+keyboard"));
    }

    #[test]
    fn last_accessibility_flag_wins() {
        let cli = Cli::try_parse_from(["vacuum", "--accessible", "--no-accessible", "tui"])
            .expect("both flags parse");
        assert!(!cli.global.accessible);
        assert!(cli.global.no_accessible);
    }

    #[test]
    fn tui_without_roots_is_empty() {
        let cli = Cli::try_parse_from(["vacuum", "tui"]).expect("bare tui should parse");
        let Some(Command::Tui(args)) = cli.command else {
            panic!("expected the tui subcommand");
        };
        assert!(args.explicit_roots().is_empty());
    }
}
