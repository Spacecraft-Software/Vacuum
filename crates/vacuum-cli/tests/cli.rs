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
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--json", "--category", "build-artifacts", "--root"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("\"tool\": \"vacuum\""))
        .stdout(predicate::str::contains(
            "\"category\": \"build-artifacts\"",
        ))
        .stdout(predicate::str::contains("\"kind\": \"path\""));
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
fn unknown_category_fails() {
    Command::cargo_bin("vacuum")
        .unwrap()
        .args(["list", "--category", "not-a-category"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown category"));
}
