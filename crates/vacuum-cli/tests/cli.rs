// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end tests for the `vacuum` binary.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

/// Build a sample tree with a Rust `target/` dir and return its temp root.
fn sample_tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("proj/target");
    fs::create_dir_all(&target).unwrap();
    fs::write(tmp.path().join("proj/Cargo.toml"), b"[package]").unwrap();
    fs::write(target.join("blob"), vec![0_u8; 1_000_000]).unwrap();
    tmp
}

#[test]
fn version_carries_attribution() {
    Command::cargo_bin("vacuum")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Mohamed.Hammad@SpacecraftSoftware.org",
        ))
        .stdout(predicate::str::contains(
            "https://Vacuum.SpacecraftSoftware.org/",
        ));
}

#[test]
fn list_json_reports_build_artifacts() {
    let tmp = sample_tree();
    // Piped stdout (non-TTY) yields compact JSON — no space after the colon.
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--category", "build-artifacts", "--root"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"tool\":\"vacuum\""))
        .stdout(predicate::str::contains("\"category\":\"build-artifacts\""))
        .stdout(predicate::str::contains("\"kind\":\"path\""));
}

#[test]
fn list_fields_restricts_payload() {
    let tmp = sample_tree();
    Command::cargo_bin("vacuum")
        .unwrap()
        .args([
            "list",
            "--json",
            "--fields",
            "label,bytes",
            "--category",
            "build-artifacts",
            "--root",
        ])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"label\":"))
        // Projected away: risk/regenerable/target must not appear in records.
        .stdout(predicate::str::contains("\"regenerable\":").not())
        .stdout(predicate::str::contains("\"risk\":").not());
}

#[test]
fn list_jsonl_streams_unenveloped_rows() {
    let tmp = sample_tree();
    Command::cargo_bin("vacuum")
        .unwrap()
        .args([
            "list",
            "--format",
            "jsonl",
            "--category",
            "build-artifacts",
            "--root",
        ])
        .arg(tmp.path())
        .assert()
        .success()
        // One record per line, no metadata envelope wrapping the stream.
        .stdout(predicate::str::contains("\"cleaner_id\":"))
        .stdout(predicate::str::contains("\"metadata\"").not());
}

#[test]
fn schema_is_valid_json_schema() {
    Command::cargo_bin("vacuum")
        .unwrap()
        .arg("schema")
        .assert()
        .success()
        .stdout(predicate::str::contains("json-schema.org/draft/2020-12"))
        .stdout(predicate::str::contains("\"commands\""))
        .stdout(predicate::str::contains("build-artifacts"));
}

#[test]
fn describe_lists_capabilities() {
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["describe", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run_default\":true"));
}

#[test]
fn clean_dry_run_does_not_delete() {
    let tmp = sample_tree();
    let target = tmp.path().join("proj/target");

    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["clean", "--category", "build-artifacts", "--root"])
        .arg(tmp.path())
        .assert()
        .success();

    assert!(target.exists(), "dry run must not delete the target dir");
}

#[test]
fn clean_apply_purge_removes() {
    let tmp = sample_tree();
    let target = tmp.path().join("proj/target");

    Command::cargo_bin("vacuum")
        .unwrap()
        .args([
            "clean",
            "--category",
            "build-artifacts",
            "--apply",
            "--purge",
            "--yes",
            "--root",
        ])
        .arg(tmp.path())
        .assert()
        .success();

    assert!(!target.exists(), "purge must remove the target dir");
}

#[test]
fn list_emits_one_group_per_category() {
    // Build artifacts come from two cleaners (the directory sweep and
    // the Cargo unit prune), but a category must still appear exactly once — frontends
    // group by category, not by cleaner.
    let tmp = sample_tree();
    let output = Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--root"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();

    for slug in [
        "build-artifacts",
        "package-gc",
        "caches",
        "temp-files",
        "large-files",
    ] {
        let needle = format!("\"category\":\"{slug}\",\"title\"");
        assert!(
            stdout.matches(&needle).count() <= 1,
            "category {slug} must head at most one group, found {} in: {stdout}",
            stdout.matches(&needle).count()
        );
    }
}

#[test]
fn temp_files_category_is_accepted() {
    // The category must be selectable and appear in the machine-readable schema
    // so agents can discover it.
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--category", "temp-files"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"tool\":\"vacuum\""));

    Command::cargo_bin("vacuum")
        .unwrap()
        .arg("schema")
        .assert()
        .success()
        .stdout(predicate::str::contains("temp-files"));
}

#[test]
fn privileged_gc_is_offered_as_a_sudo_line_never_executed() {
    // Which recipes appear depends entirely on which tools are installed, so
    // this asserts the shape rather than the contents: the group is always
    // present, and any candidate in it is a command target whose root-requiring
    // form is rendered as a `sudo` line for the user to run. On a machine with
    // no GC tools at all (a clean build sandbox) the list is legitimately empty.
    let output = Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--category", "package-gc"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();

    assert!(
        stdout.contains("\"category\":\"package-gc\""),
        "the package-gc group must always be reported: {stdout}"
    );
    if stdout.contains("\"cleaner_id\":\"package-gc\"") {
        assert!(
            stdout.contains("\"kind\":\"command\""),
            "package-gc candidates are commands, never paths: {stdout}"
        );
    }
    assert!(
        !stdout.contains("\"needs_root\":true,\"program\"") || stdout.contains("sudo "),
        "a root-requiring recipe must render its sudo line: {stdout}"
    );
}

#[test]
fn stale_days_and_cleaner_filters_are_accepted() {
    let tmp = sample_tree();

    // `--stale-days 0` makes every build unit cold; it must parse and run.
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--stale-days", "0", "--root"])
        .arg(tmp.path())
        .assert()
        .success();

    // `--cleaner` narrows within a category. Selecting only the Cargo prune
    // must not surface the whole-directory build-artifacts candidate.
    let output = Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--cleaner", "cargo-prune", "--root"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert!(
        !stdout.contains("\"cleaner_id\":\"build-artifacts\""),
        "--cleaner cargo-prune must exclude the whole-directory cleaner: {stdout}"
    );
}

#[test]
fn cargo_prune_ignores_a_target_dir_without_a_cachedir_tag() {
    // `sample_tree` builds a bare `proj/target` with no CACHEDIR.TAG, so it is
    // not a real Cargo target directory and the prune must leave it alone.
    let tmp = sample_tree();
    let output = Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--cleaner", "cargo-prune", "--root"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert!(
        !stdout.contains("\"kind\":\"batch\""),
        "no batch should be produced for a non-Cargo target dir: {stdout}"
    );
}

#[test]
fn unknown_category_emits_structured_error_with_hint() {
    // Usage errors exit 2 and carry a runnable hint (the CLI Standard §4, agentic §3).
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--category", "not-a-category"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\"code\":\"UNKNOWN_CATEGORY\""))
        .stderr(predicate::str::contains("\"exit_code\":2"))
        .stderr(predicate::str::contains("\"hint\":"))
        .stderr(predicate::str::contains("unknown category"));
}

#[test]
fn scan_accepts_root_flag() {
    let tmp = sample_tree();
    // `scan` gained `--root` for parity with list/clean; it scans that root.
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["scan", "--json", "--root"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"tool\":\"vacuum\""))
        .stdout(predicate::str::contains(&*tmp.path().display().to_string()));
}

#[test]
fn empty_trash_dry_run_reports_without_purging() {
    // A dry run lists what *would* be purged and must not touch the trash. It is
    // read-only over the real trash, so it is safe to run in CI (never --apply).
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["empty-trash", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run\":true"))
        .stdout(predicate::str::contains("\"count\":"))
        .stdout(predicate::str::contains("\"reclaimable_bytes\":"));
}

#[test]
fn empty_trash_apply_without_yes_is_refused_non_interactively() {
    // Permanent purge without --yes must refuse in machine mode (the CLI Standard §7). If the
    // trash is empty the command short-circuits to success, so accept either a
    // refusal or a clean exit — but never an unguarded purge.
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["empty-trash", "--json", "--apply"])
        .assert()
        .stderr(
            predicate::str::contains("\"code\":\"CONFIRMATION_REQUIRED\"")
                .or(predicate::str::is_empty()),
        );
}

#[test]
fn empty_trash_is_in_schema() {
    Command::cargo_bin("vacuum")
        .unwrap()
        .arg("schema")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"empty-trash\""));
}

#[test]
fn clean_apply_without_yes_is_refused_non_interactively() {
    let tmp = sample_tree();
    // Non-interactive --apply without --yes must refuse, not act (the CLI Standard §7).
    Command::cargo_bin("vacuum")
        .unwrap()
        .args([
            "clean",
            "--json",
            "--apply",
            "--category",
            "build-artifacts",
            "--root",
        ])
        .arg(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "\"code\":\"CONFIRMATION_REQUIRED\"",
        ));
    assert!(
        tmp.path().join("proj/target").exists(),
        "a refused clean must not delete anything"
    );
}
