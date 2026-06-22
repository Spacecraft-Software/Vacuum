// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Self-documenting introspection: `vacuum schema` and `vacuum describe`
//! (SFRS §2 Rule 4).
//!
//! `schema` emits a JSON Schema (Draft 2020-12) of the command surface — the
//! per-command `parameters` objects drop directly into the Anthropic / MCP
//! function-calling APIs (agentic-cli §5). `describe` emits a compact
//! capability manifest for quick agent orientation.

use std::io::IsTerminal as _;

use serde_json::{Value, json};
use vacuum_core::Category;

use crate::agent::OutputMode;
use crate::envelope;
use crate::error::{CliError, CliResult};

/// The canonical exit-code map (SFRS §4), surfaced through introspection.
fn exit_codes() -> Value {
    json!({
        "0": "success",
        "1": "general failure",
        "2": "usage error (bad arguments) — fix invocation, do not retry",
        "3": "resource not found",
        "4": "permission denied / refused by safety guard",
        "5": "conflict (already exists)",
        "127": "required external tool not found — install it"
    })
}

/// All category slugs, derived from the core enum so the two never drift.
fn category_slugs() -> Vec<&'static str> {
    Category::ALL.iter().map(|c| c.slug()).collect()
}

/// The shared object schema for the `--category` / `--root` selectors.
fn selection_properties() -> Value {
    json!({
        "category": {
            "type": "array",
            "items": { "type": "string", "enum": category_slugs() },
            "description": "Restrict to these categories. Defaults to all."
        },
        "root": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Roots to operate within. Defaults to $HOME."
        }
    })
}

/// The `clean` command's schema entry (extracted to keep `command_schema` lean).
fn clean_command() -> Value {
    json!({
        "name": "clean",
        "summary": "Remove reclaimable space. Dry-run unless --apply; trash unless --purge.",
        "parameters": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "category": selection_properties()["category"].clone(),
                "root": selection_properties()["root"].clone(),
                "apply": {
                    "type": "boolean",
                    "default": false,
                    "description": "Actually act. Without it, the run is a dry run."
                },
                "purge": {
                    "type": "boolean",
                    "default": false,
                    "description": "Delete permanently instead of moving to trash."
                },
                "yes": {
                    "type": "boolean",
                    "default": false,
                    "description": "Skip confirmation (required for non-interactive --apply)."
                }
            }
        },
        "examples": [
            "vacuum clean --category build-artifacts --json",
            "vacuum clean --category caches --apply --yes --json"
        ]
    })
}

/// The `empty-trash` command's schema entry.
fn empty_trash_command() -> Value {
    json!({
        "name": "empty-trash",
        "summary": "Permanently empty the OS trash. Dry-run unless --apply.",
        "parameters": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "apply": {
                    "type": "boolean",
                    "default": false,
                    "description": "Actually purge. Without it, the run is a dry run."
                },
                "yes": {
                    "type": "boolean",
                    "default": false,
                    "description": "Skip confirmation (required for non-interactive --apply)."
                }
            }
        },
        "examples": [
            "vacuum empty-trash --json",
            "vacuum empty-trash --apply --yes --json"
        ]
    })
}

/// Build the full command-surface schema (raw JSON Schema, Anthropic/MCP-ready).
fn command_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "vacuum",
        "description": "Fast, safe disk-space recovery for the terminal (CLI + TUI).",
        "version": env!("CARGO_PKG_VERSION"),
        "categories": category_slugs(),
        "exit_codes": exit_codes(),
        "commands": [
            {
                "name": "scan",
                "summary": "Report the largest disk-space consumers under each path (read-only).",
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Paths to scan. Defaults to $HOME."
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "default": 20,
                            "description": "Top consumers to show per path."
                        }
                    }
                },
                "examples": [
                    "vacuum scan --json",
                    "vacuum scan ~/Downloads --limit 10 --json"
                ]
            },
            {
                "name": "list",
                "summary": "List reclaimable candidates by category (read-only).",
                "parameters": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": selection_properties()
                },
                "examples": [
                    "vacuum list --json",
                    "vacuum list --category build-artifacts --json --fields label,bytes"
                ]
            },
            clean_command(),
            empty_trash_command(),
            {
                "name": "schema",
                "summary": "Emit this JSON Schema of the command surface.",
                "parameters": { "type": "object", "additionalProperties": false, "properties": {} },
                "examples": [ "vacuum schema" ]
            },
            {
                "name": "describe",
                "summary": "Emit a compact capability manifest.",
                "parameters": { "type": "object", "additionalProperties": false, "properties": {} },
                "examples": [ "vacuum describe --json" ]
            }
        ]
    })
}

/// Run `vacuum schema`: emit the raw JSON Schema to stdout (never enveloped, so
/// it drops straight into a function-calling `input_schema`).
pub fn run_schema() -> CliResult<()> {
    let schema = command_schema();
    let rendered = if std::io::stdout().is_terminal() {
        serde_json::to_string_pretty(&schema)
    } else {
        serde_json::to_string(&schema)
    }
    .map_err(|err| CliError::internal(&anyhow::Error::from(err)))?;
    println!("{rendered}");
    Ok(())
}

/// Run `vacuum describe`: a capability manifest, enveloped like any data command.
pub fn run_describe(mode: OutputMode, command_line: &str) -> CliResult<()> {
    let manifest = json!({
        "tool": "vacuum",
        "summary": "Fast, safe disk-space recovery for the terminal.",
        "commands": ["scan", "list", "clean", "empty-trash", "schema", "describe", "tui"],
        "categories": category_slugs(),
        "safety": {
            "dry_run_default": true,
            "trash_default": true,
            "purge_flag": "--purge",
            "never_follows_symlinks": true,
            "never_escalates_privileges": true
        }
    });

    match mode {
        OutputMode::Json | OutputMode::Jsonl => {
            envelope::emit(command_line, &manifest).map_err(|err| CliError::internal(&err))?;
        }
        OutputMode::Human => {
            println!("vacuum — fast, safe disk-space recovery for the terminal.");
            println!("\nCommands: scan, list, clean, empty-trash, schema, describe, tui");
            println!("Categories: {}", category_slugs().join(", "));
            println!("\nSafety: dry-run by default, trash unless --purge, never follows symlinks,");
            println!("never escalates privileges. Run `vacuum schema` for the full surface.");
        }
    }
    Ok(())
}
