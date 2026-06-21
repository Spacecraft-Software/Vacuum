// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Subcommand implementations: `scan`, `list`, and `clean`.

use std::io::Write as _;
use std::path::PathBuf;

use serde::Serialize;
use vacuum_cleaners::cleaners_for;
use vacuum_core::{
    Action, Candidate, DeleteMode, Deleter, Outcome, ScanContext, Usage, human_bytes, safety, scan,
    total_bytes,
};

use crate::agent::OutputMode;
use crate::cli::{CleanArgs, ScanArgs, SelectArgs};
use crate::envelope;

/// Resolve explicit roots, falling back to the user's home directory.
fn resolve_roots(explicit: &[PathBuf]) -> Vec<PathBuf> {
    if explicit.is_empty() {
        safety::default_roots()
    } else {
        explicit.to_vec()
    }
}

/// Whether an outcome's bytes count toward reclaimed space.
fn counts_bytes(action: Action) -> bool {
    matches!(
        action,
        Action::Trashed | Action::Purged | Action::WouldTrash | Action::WouldPurge
    )
}

// --- scan ----------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RootUsage {
    root: String,
    entries: Vec<Usage>,
}

#[derive(Debug, Serialize)]
struct ScanReport {
    roots: Vec<RootUsage>,
}

/// Run `vacuum scan`.
pub fn run_scan(args: &ScanArgs, mode: OutputMode, command_line: &str) -> anyhow::Result<()> {
    let roots = resolve_roots(&args.paths);
    let mut report = ScanReport { roots: Vec::new() };

    for root in roots {
        let entries = scan::top_consumers(&root, args.limit)?;
        report.roots.push(RootUsage {
            root: root.display().to_string(),
            entries,
        });
    }

    match mode {
        OutputMode::Json => envelope::emit(command_line, &report)?,
        OutputMode::Human => {
            for root in &report.roots {
                println!("\n{}", root.root);
                for entry in &root.entries {
                    let marker = if entry.is_dir { "/" } else { "" };
                    println!(
                        "  {:>10}  {}{marker}",
                        human_bytes(entry.bytes),
                        entry.path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

// --- list ----------------------------------------------------------------

#[derive(Debug, Serialize)]
struct CategoryGroup {
    category: &'static str,
    title: &'static str,
    total_bytes: u64,
    candidates: Vec<Candidate>,
}

#[derive(Debug, Serialize)]
struct ListReport {
    categories: Vec<CategoryGroup>,
    total_bytes: u64,
}

/// Gather candidates for the selected categories within the selected roots.
fn gather(select: &SelectArgs) -> anyhow::Result<Vec<CategoryGroup>> {
    let categories = select.selected_categories().map_err(anyhow::Error::msg)?;
    let ctx = ScanContext {
        roots: resolve_roots(&select.roots),
    };

    let mut groups = Vec::new();
    for cleaner in cleaners_for(&categories) {
        let candidates = cleaner.scan(&ctx)?;
        groups.push(CategoryGroup {
            category: cleaner.category().slug(),
            title: cleaner.category().title(),
            total_bytes: total_bytes(&candidates),
            candidates,
        });
    }
    Ok(groups)
}

/// Run `vacuum list`.
pub fn run_list(args: &SelectArgs, mode: OutputMode, command_line: &str) -> anyhow::Result<()> {
    let groups = gather(args)?;
    let grand_total = groups.iter().map(|group| group.total_bytes).sum();
    let report = ListReport {
        categories: groups,
        total_bytes: grand_total,
    };

    match mode {
        OutputMode::Json => envelope::emit(command_line, &report)?,
        OutputMode::Human => {
            for group in &report.categories {
                println!("\n{} — {}", group.title, human_bytes(group.total_bytes));
                for candidate in &group.candidates {
                    println!("  {:>10}  {}", size_label(candidate), candidate.label);
                }
            }
            println!("\nTotal reclaimable: {}", human_bytes(report.total_bytes));
        }
    }
    Ok(())
}

/// Render a candidate's size, or "unknown" when it cannot be measured ahead.
fn size_label(candidate: &Candidate) -> String {
    if candidate.bytes == 0 {
        "  unknown".to_owned()
    } else {
        human_bytes(candidate.bytes)
    }
}

// --- clean ---------------------------------------------------------------

#[derive(Debug, Serialize)]
struct CleanReport {
    dry_run: bool,
    mode: DeleteMode,
    reclaimed_bytes: u64,
    outcomes: Vec<Outcome>,
}

/// Run `vacuum clean`.
pub fn run_clean(args: &CleanArgs, mode: OutputMode, command_line: &str) -> anyhow::Result<()> {
    let groups = gather(&args.select)?;
    let candidates: Vec<Candidate> = groups
        .into_iter()
        .flat_map(|group| group.candidates)
        .collect();

    let dry_run = !args.apply;
    let delete_mode = args.delete_mode();

    // Guard real deletions behind confirmation. Non-interactive callers must
    // pass --yes; interactive callers are prompted.
    if !dry_run && !args.yes {
        let projected: u64 = candidates
            .iter()
            .filter(|c| matches!(c.target, vacuum_core::Target::Path { .. }))
            .map(|c| c.bytes)
            .sum();
        let verb = if delete_mode == DeleteMode::Purge {
            "permanently delete"
        } else {
            "move to trash"
        };
        let prompt = format!(
            "About to {verb} {} across {} item(s). Continue?",
            human_bytes(projected),
            candidates.len()
        );
        if mode == OutputMode::Json || !confirm(&prompt) {
            anyhow::bail!("aborted: pass --yes to apply non-interactively");
        }
    }

    let deleter = Deleter::new(delete_mode, dry_run, resolve_roots(&args.select.roots));
    let mut outcomes = Vec::new();
    let mut reclaimed = 0_u64;

    for candidate in &candidates {
        match deleter.execute(candidate) {
            Ok(outcome) => {
                if counts_bytes(outcome.action) {
                    reclaimed += outcome.bytes;
                }
                outcomes.push(outcome);
            }
            Err(err) => eprintln!("vacuum: skipping {}: {err}", candidate.label),
        }
    }

    let report = CleanReport {
        dry_run,
        mode: delete_mode,
        reclaimed_bytes: reclaimed,
        outcomes,
    };

    match mode {
        OutputMode::Json => envelope::emit(command_line, &report)?,
        OutputMode::Human => {
            for outcome in &report.outcomes {
                let size = if outcome.bytes == 0 {
                    "  unknown".to_owned()
                } else {
                    human_bytes(outcome.bytes)
                };
                print!("  {size:>10}  {:?}  {}", outcome.action, outcome.label);
                if let Some(note) = &outcome.note {
                    print!("  ({note})");
                }
                println!();
            }
            let suffix = if report.dry_run { " (dry run)" } else { "" };
            println!(
                "\n{} reclaimable: {}{suffix}",
                if report.dry_run {
                    "Would free"
                } else {
                    "Freed"
                },
                human_bytes(report.reclaimed_bytes)
            );
            if report.dry_run {
                println!("Re-run with --apply to act (add --purge for permanent deletion).");
            }
        }
    }
    Ok(())
}

/// Prompt the user for confirmation on stdin; defaults to no.
fn confirm(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    if std::io::stdout().flush().is_err() {
        return false;
    }
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
