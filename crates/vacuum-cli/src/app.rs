// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Subcommand implementations: `scan`, `list`, `clean`, `empty-trash`, and `tui`.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use serde_json::{Value, json};
use vacuum_cleaners::cleaners_for;
use vacuum_core::{
    Action, Candidate, Category, DeleteMode, Deleter, Outcome, ScanContext, Usage, human_bytes,
    scan, total_bytes,
};

use vacuum_tui::{ProtocolOverrides, protocols};

use crate::a11y;
use crate::agent::{self, OutputMode};
use crate::cli::{CleanArgs, EmptyTrashArgs, GlobalArgs, ScanArgs, SelectArgs};
use crate::config::Settings;
use crate::envelope;
use crate::error::{CliError, CliResult};
use crate::trashbin;

/// Resolve explicit roots, falling back to the configured/default roots.
///
/// `fallback` is the precedence-resolved default (`VACUUM_ROOTS` → config file →
/// `$HOME`) computed once per run; explicit command-line roots always win.
fn resolve_roots(explicit: &[PathBuf], fallback: &[PathBuf]) -> Vec<PathBuf> {
    if explicit.is_empty() {
        fallback.to_vec()
    } else {
        explicit.to_vec()
    }
}

/// Run `vacuum tui`: launch the interactive frontend over the resolved roots.
///
/// Explicit command-line roots win; otherwise the precedence-resolved `fallback`
/// (`VACUUM_ROOTS` → config file → `$HOME`) is used. The roots bound both the
/// scan and every deletion the frontend performs (via [`vacuum_core::safety`]),
/// exactly as they do for `list`/`clean`.
///
/// # Errors
///
/// Returns a [`CliError`] if the frontend fails to scan or to drive the terminal.
pub fn run_tui(explicit: &[PathBuf], settings: &Settings, global: &GlobalArgs) -> CliResult<()> {
    let roots = resolve_roots(explicit, &settings.roots);
    let accessibility = a11y::resolve_for(global, settings.accessible);
    let options = vacuum_tui::TuiOptions {
        accessible: accessibility.enabled,
        agent: agent::is_agent(),
        protocols: term_protocols(global)?,
        ..vacuum_tui::TuiOptions::default()
    };
    let report = vacuum_tui::run(roots, &options).map_err(|err| CliError::tui(&err))?;
    // Printed after the TUI has left the alternate screen, so it stays visible.
    if global.verbose && !global.quiet {
        eprintln!("[INFO] accessible mode: {accessibility}");
        eprintln!("[INFO] terminal protocols: {report}");
    }
    Ok(())
}

/// The terminal-protocol overrides: `--term-protocols` layered over
/// `SPACECRAFT_TERM_PROTOCOLS`, the flag winning name by name.
fn term_protocols(global: &GlobalArgs) -> CliResult<ProtocolOverrides> {
    let env = match std::env::var(protocols::OVERRIDE_ENV) {
        Ok(list) => ProtocolOverrides::parse(&list).map_err(|err| {
            CliError::usage(
                "INVALID_TERM_PROTOCOLS",
                format!("{}: {err}", protocols::OVERRIDE_ENV),
                format!("env -u {} vacuum tui", protocols::OVERRIDE_ENV),
            )
        })?,
        Err(_) => ProtocolOverrides::default(),
    };
    let flag = match global.term_protocols.as_deref() {
        Some(list) => ProtocolOverrides::parse(list).map_err(|err| {
            CliError::usage(
                "INVALID_TERM_PROTOCOLS",
                format!("--term-protocols: {err}"),
                "vacuum tui --term-protocols=-all",
            )
        })?,
        None => ProtocolOverrides::default(),
    };
    Ok(flag.over(env))
}

/// Whether an outcome's bytes count toward reclaimed space.
fn counts_bytes(action: Action) -> bool {
    matches!(
        action,
        Action::Trashed | Action::Purged | Action::WouldTrash | Action::WouldPurge
    )
}

/// Project a record onto `fields`, dropping the rest. An empty field list
/// returns the record whole. Non-object records pass through untouched
/// (token-economy hygiene, the CLI Standard §8).
fn project<T: Serialize>(record: &T, fields: &[String]) -> Value {
    let value = serde_json::to_value(record).unwrap_or(Value::Null);
    if fields.is_empty() {
        return value;
    }
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for field in fields {
                if let Some(found) = map.get(field) {
                    out.insert(field.clone(), found.clone());
                }
            }
            Value::Object(out)
        }
        other => other,
    }
}

/// Print one compact JSON object per line (newline-delimited JSON).
fn emit_jsonl<T: Serialize>(rows: &[T]) -> CliResult<()> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    for row in rows {
        let line = serde_json::to_string(row)
            .map_err(|err| CliError::internal(&anyhow::Error::from(err)))?;
        writeln!(lock, "{line}").map_err(|err| CliError::internal(&anyhow::Error::from(err)))?;
    }
    Ok(())
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
pub fn run_scan(
    args: &ScanArgs,
    mode: OutputMode,
    global: &GlobalArgs,
    settings: &Settings,
    command_line: &str,
) -> CliResult<()> {
    let roots = resolve_roots(&args.explicit_roots(), &settings.roots);
    let mut report = ScanReport { roots: Vec::new() };

    for root in roots {
        let entries = scan::top_consumers(&root, args.limit)?;
        report.roots.push(RootUsage {
            root: root.display().to_string(),
            entries,
        });
    }

    match mode {
        OutputMode::Json => {
            let data = json!({
                "roots": report.roots.iter().map(|root| json!({
                    "root": root.root,
                    "entries": root.entries.iter()
                        .map(|entry| project(entry, &global.fields)).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            });
            envelope::emit(command_line, &data).map_err(|err| CliError::internal(&err))?;
        }
        OutputMode::Jsonl => {
            let rows: Vec<Value> = report
                .roots
                .iter()
                .flat_map(|root| {
                    root.entries.iter().map(move |entry| {
                        let mut row = project(entry, &global.fields);
                        if global.fields.is_empty() {
                            if let Value::Object(map) = &mut row {
                                map.insert("root".to_owned(), json!(root.root));
                            }
                        }
                        row
                    })
                })
                .collect();
            emit_jsonl(&rows)?;
        }
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

#[derive(Debug)]
struct CategoryGroup {
    category: &'static str,
    title: &'static str,
    total_bytes: u64,
    candidates: Vec<Candidate>,
}

/// What a scan produced: candidates grouped by category, plus the roots any
/// resulting deletion must be allowed to touch.
#[derive(Debug)]
struct Gathered {
    groups: Vec<CategoryGroup>,
    /// The scan roots widened by the [`Cleaner::extra_roots`] of the cleaners
    /// that actually ran — see [`run_clean`].
    ///
    /// [`Cleaner::extra_roots`]: vacuum_core::Cleaner::extra_roots
    delete_roots: Vec<PathBuf>,
}

/// Gather candidates for the selected categories within the selected roots.
///
/// Several cleaners may share a category (build artifacts, for instance, come
/// from both the directory sweep and the Cargo unit prune), so their candidates are
/// merged into one group per category rather than one per cleaner. Groups are
/// emitted in `Category::ALL` order, independent of catalog order.
fn gather(select: &SelectArgs, settings: &Settings) -> CliResult<Gathered> {
    let categories = select
        .selected_categories()
        .map_err(CliError::unknown_category)?;
    let roots = resolve_roots(&select.roots, &settings.roots);
    let ctx = ScanContext {
        roots: roots.clone(),
        // The flag wins over the config file, which wins over the default.
        stale_days: select.stale_days.unwrap_or(settings.stale_days),
    };

    let mut by_category: Vec<(Category, Vec<Candidate>)> = Vec::new();
    let mut delete_roots = roots;
    for cleaner in cleaners_for(&categories) {
        // `--cleaner` narrows within a category. Several cleaners can share one,
        // and their candidates may overlap on purpose (the whole-directory and
        // per-unit Cargo candidates cover the same tree), so a caller that wants
        // exactly one of them needs to say so.
        if !select.cleaners.is_empty() && !select.cleaners.iter().any(|id| id == cleaner.id()) {
            continue;
        }
        let candidates = cleaner.scan(&ctx)?;
        for root in cleaner.extra_roots() {
            if !delete_roots.contains(&root) {
                delete_roots.push(root);
            }
        }

        let category = cleaner.category();
        if let Some((_, existing)) = by_category.iter_mut().find(|(seen, _)| *seen == category) {
            existing.extend(candidates);
        } else {
            by_category.push((category, candidates));
        }
    }

    // Report in canonical category order, not the order cleaners happen to run.
    by_category.sort_by_key(|(category, _)| {
        Category::ALL
            .iter()
            .position(|known| known == category)
            .unwrap_or(usize::MAX)
    });

    let groups = by_category
        .into_iter()
        .map(|(category, candidates)| CategoryGroup {
            category: category.slug(),
            title: category.title(),
            total_bytes: total_bytes(&candidates),
            candidates,
        })
        .collect();

    Ok(Gathered {
        groups,
        delete_roots,
    })
}

/// Run `vacuum list`.
pub fn run_list(
    args: &SelectArgs,
    mode: OutputMode,
    global: &GlobalArgs,
    settings: &Settings,
    command_line: &str,
) -> CliResult<()> {
    let groups = gather(args, settings)?.groups;
    let grand_total: u64 = groups.iter().map(|group| group.total_bytes).sum();

    match mode {
        OutputMode::Json => {
            let data = json!({
                "categories": groups.iter().map(|group| json!({
                    "category": group.category,
                    "title": group.title,
                    "total_bytes": group.total_bytes,
                    "candidates": group.candidates.iter()
                        .map(|candidate| project(candidate, &global.fields)).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "total_bytes": grand_total,
            });
            envelope::emit(command_line, &data).map_err(|err| CliError::internal(&err))?;
        }
        OutputMode::Jsonl => {
            let rows: Vec<Value> = groups
                .iter()
                .flat_map(|group| group.candidates.iter())
                .map(|candidate| project(candidate, &global.fields))
                .collect();
            emit_jsonl(&rows)?;
        }
        OutputMode::Human => {
            for group in &groups {
                println!("\n{} — {}", group.title, human_bytes(group.total_bytes));
                for candidate in &group.candidates {
                    println!("  {:>10}  {}", size_label(candidate), candidate.label);
                }
            }
            println!("\nTotal reclaimable: {}", human_bytes(grand_total));
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

/// Build a steady-tick spinner for the deletion loop, or `None` when progress
/// must stay silent (machine modes, `--quiet`, dry runs, or no work to do).
///
/// indicatif draws to stderr and auto-hides when stderr is not a terminal, so
/// stdout stays data-only and pipes never see control characters (the CLI Standard §7). The
/// template is uncolored, which trivially satisfies `NO_COLOR`.
fn clean_spinner(len: usize, enabled: bool) -> Option<ProgressBar> {
    if !enabled || len == 0 {
        return None;
    }
    let style = ProgressStyle::with_template("{spinner} [{pos}/{len}] {wide_msg}")
        .expect("static progress template is valid");
    let bar = ProgressBar::new(len as u64).with_style(style);
    bar.enable_steady_tick(Duration::from_millis(100));
    Some(bar)
}

/// Act on every candidate, returning the outcomes and total reclaimed bytes.
///
/// When `show_progress` is set, a live spinner (above) reports the item being
/// removed so a long deletion does not look frozen. Each candidate is a single
/// blocking removal, so the steady tick — not a percent bar — is what proves the
/// process is alive. Per-item skip warnings are printed above the bar via
/// [`ProgressBar::suspend`] so the spinner line is never corrupted.
fn execute_candidates(
    deleter: &Deleter,
    candidates: &[Candidate],
    delete_mode: DeleteMode,
    show_progress: bool,
    quiet: bool,
) -> (Vec<Outcome>, u64) {
    let verb = if delete_mode == DeleteMode::Purge {
        "Removing"
    } else {
        "Trashing"
    };
    let progress = clean_spinner(candidates.len(), show_progress);
    let mut outcomes = Vec::new();
    let mut reclaimed = 0_u64;

    for candidate in candidates {
        if let Some(bar) = &progress {
            bar.set_message(format!("{verb} {}", candidate.label));
        }
        match deleter.execute(candidate) {
            Ok(outcome) => {
                if counts_bytes(outcome.action) {
                    reclaimed += outcome.bytes;
                }
                outcomes.push(outcome);
            }
            Err(err) => {
                if !quiet {
                    let warn = format!("vacuum: skipping {}: {err}", candidate.label);
                    match &progress {
                        Some(bar) => bar.suspend(|| eprintln!("{warn}")),
                        None => eprintln!("{warn}"),
                    }
                }
            }
        }
        if let Some(bar) = &progress {
            bar.inc(1);
        }
    }

    if let Some(bar) = &progress {
        bar.finish_and_clear();
    }
    (outcomes, reclaimed)
}

/// Run `vacuum clean`.
pub fn run_clean(
    args: &CleanArgs,
    mode: OutputMode,
    global: &GlobalArgs,
    settings: &Settings,
    command_line: &str,
) -> CliResult<()> {
    let gathered = gather(&args.select, settings)?;
    let candidates: Vec<Candidate> = gathered
        .groups
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
        if mode.is_machine() || !confirm(&prompt) {
            return Err(CliError::confirmation_required(
                verb,
                "vacuum clean --apply --yes  # add --purge for permanent deletion",
            ));
        }
    }

    // The delete allowlist is the scan roots plus whatever the selected cleaners
    // declared they need (e.g. `/tmp` for the temp-file cleaner). The
    // protected-prefix and symlink guards still apply to every path.
    let deleter = Deleter::new(delete_mode, dry_run, gathered.delete_roots);
    // Show a live spinner only for real, interactive work (the CLI Standard §7).
    let show_progress = mode == OutputMode::Human && !global.quiet && !dry_run;
    let (outcomes, reclaimed) = execute_candidates(
        &deleter,
        &candidates,
        delete_mode,
        show_progress,
        global.quiet,
    );

    let report = CleanReport {
        dry_run,
        mode: delete_mode,
        reclaimed_bytes: reclaimed,
        outcomes,
    };

    match mode {
        OutputMode::Json => {
            let data = json!({
                "dry_run": report.dry_run,
                "mode": report.mode,
                "reclaimed_bytes": report.reclaimed_bytes,
                "outcomes": report.outcomes.iter()
                    .map(|outcome| project(outcome, &global.fields)).collect::<Vec<_>>(),
            });
            envelope::emit(command_line, &data).map_err(|err| CliError::internal(&err))?;
        }
        OutputMode::Jsonl => {
            let rows: Vec<Value> = report
                .outcomes
                .iter()
                .map(|outcome| project(outcome, &global.fields))
                .collect();
            emit_jsonl(&rows)?;
        }
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

// --- empty-trash ---------------------------------------------------------

/// A trashed entry as rendered in `empty-trash` output.
#[derive(Debug, Serialize)]
struct TrashRecord {
    name: String,
    original_path: String,
    bytes: Option<u64>,
    deleted_at: Option<String>,
}

impl From<&trashbin::Entry> for TrashRecord {
    fn from(entry: &trashbin::Entry) -> Self {
        Self {
            name: entry.name.clone(),
            original_path: entry.original_path.clone(),
            bytes: entry.bytes,
            deleted_at: entry.deleted_at.clone(),
        }
    }
}

/// Permanently purge every entry, returning the purged subset and the reclaimed
/// byte total.
///
/// Mirrors [`execute_candidates`]: one removal at a time so the spinner reports
/// progress and a single backend failure is isolated (skipped with a warning)
/// rather than aborting the whole sweep.
fn purge_entries(
    entries: &[trashbin::Entry],
    show_progress: bool,
    quiet: bool,
) -> (Vec<&trashbin::Entry>, u64) {
    let progress = clean_spinner(entries.len(), show_progress);
    let mut purged = Vec::new();
    let mut reclaimed = 0_u64;

    for entry in entries {
        if let Some(bar) = &progress {
            bar.set_message(format!("Removing {}", entry.name));
        }
        match trashbin::purge(entry) {
            Ok(()) => {
                reclaimed += entry.bytes.unwrap_or(0);
                purged.push(entry);
            }
            Err(err) => {
                if !quiet {
                    let warn = format!("vacuum: skipping {}: {}", entry.name, err.message());
                    match &progress {
                        Some(bar) => bar.suspend(|| eprintln!("{warn}")),
                        None => eprintln!("{warn}"),
                    }
                }
            }
        }
        if let Some(bar) = &progress {
            bar.inc(1);
        }
    }

    if let Some(bar) = &progress {
        bar.finish_and_clear();
    }
    (purged, reclaimed)
}

/// Run `vacuum empty-trash`.
pub fn run_empty_trash(
    args: &EmptyTrashArgs,
    mode: OutputMode,
    global: &GlobalArgs,
    command_line: &str,
) -> CliResult<()> {
    let entries = trashbin::list()?;
    let dry_run = !args.apply;
    // Best-effort: directories report an entry count rather than a byte size,
    // so they contribute to the item count but not to the byte total.
    let reclaimable_bytes: u64 = entries.iter().filter_map(|entry| entry.bytes).sum();

    // Emptying the trash is permanent. Guard real purges behind confirmation;
    // non-interactive callers must pass --yes (machine mode cannot prompt).
    if !dry_run && !args.yes && !entries.is_empty() {
        let prompt = format!(
            "About to permanently delete {} item(s) (~{}) from the trash. Continue?",
            entries.len(),
            human_bytes(reclaimable_bytes)
        );
        if mode.is_machine() || !confirm(&prompt) {
            return Err(CliError::confirmation_required(
                "empty the trash",
                "vacuum empty-trash --apply --yes",
            ));
        }
    }

    // Purge one entry at a time so the spinner can report progress and a single
    // failure does not abort the rest of the sweep.
    let show_progress = mode == OutputMode::Human && !global.quiet && !dry_run;
    let (purged, reclaimed) = if dry_run {
        (Vec::new(), 0)
    } else {
        purge_entries(&entries, show_progress, global.quiet)
    };

    // A dry run reports what *would* be purged; after --apply, what actually was.
    let records: Vec<TrashRecord> = if dry_run {
        entries.iter().map(TrashRecord::from).collect()
    } else {
        purged.iter().copied().map(TrashRecord::from).collect()
    };
    let report_bytes = if dry_run {
        reclaimable_bytes
    } else {
        reclaimed
    };

    match mode {
        OutputMode::Json => {
            let data = json!({
                "dry_run": dry_run,
                "count": records.len(),
                "reclaimable_bytes": report_bytes,
                "items": records.iter()
                    .map(|record| project(record, &global.fields)).collect::<Vec<_>>(),
            });
            envelope::emit(command_line, &data).map_err(|err| CliError::internal(&err))?;
        }
        OutputMode::Jsonl => {
            let rows: Vec<Value> = records
                .iter()
                .map(|record| project(record, &global.fields))
                .collect();
            emit_jsonl(&rows)?;
        }
        OutputMode::Human => {
            if records.is_empty() {
                println!("Trash is already empty.");
            } else {
                for record in &records {
                    let size = match record.bytes {
                        Some(0) | None => "  unknown".to_owned(),
                        Some(bytes) => human_bytes(bytes),
                    };
                    println!("  {size:>10}  {}", record.name);
                }
                let suffix = if dry_run { " (dry run)" } else { "" };
                // Directories report no byte size, so the total is a lower bound
                // whenever any are present — say so rather than imply it is exact.
                let dir_items = records.iter().filter(|r| r.bytes.is_none()).count();
                let qualifier = if dir_items > 0 { "at least " } else { "" };
                println!(
                    "\n{} reclaimable: {qualifier}{}{suffix}",
                    if dry_run { "Would free" } else { "Freed" },
                    human_bytes(report_bytes)
                );
                if dir_items > 0 {
                    println!(
                        "({dir_items} item(s) are directories whose size the trash does not report.)"
                    );
                }
                if dry_run {
                    println!("Re-run with --apply to permanently empty the trash.");
                }
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
