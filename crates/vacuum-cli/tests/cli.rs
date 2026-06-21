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
fn unknown_category_emits_structured_error_with_hint() {
    // Usage errors exit 2 and carry a runnable hint (SFRS §4, agentic §3).
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
fn clean_apply_without_yes_is_refused_non_interactively() {
    let tmp = sample_tree();
    // Non-interactive --apply without --yes must refuse, not act (SFRS §7).
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
