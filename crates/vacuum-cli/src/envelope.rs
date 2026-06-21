// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The SFRS JSON output envelope (`{ metadata, data }`).

use jiff::Timestamp;
use serde::Serialize;

/// Maintainer attribution embedded in every JSON envelope (Standard §15.2).
const MAINTAINER: &str = "Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>";
/// Project website embedded in every JSON envelope (Standard §15.2).
const WEBSITE: &str = "https://Vacuum.SpacecraftSoftware.org/";

/// The metadata block common to every JSON response.
#[derive(Debug, Serialize)]
struct Metadata {
    tool: &'static str,
    version: &'static str,
    command: String,
    timestamp: String,
    maintainer: &'static str,
    website: &'static str,
}

/// A complete JSON response: metadata plus a command-specific data payload.
#[derive(Debug, Serialize)]
struct Envelope<T: Serialize> {
    metadata: Metadata,
    data: T,
}

/// Serialize `data` inside the standard envelope and print it to stdout.
///
/// The timestamp is ISO 8601 UTC with a `Z` suffix (Standard §14).
///
/// # Errors
///
/// Returns an error if serialization fails.
pub fn emit<T: Serialize>(command: &str, data: T) -> anyhow::Result<()> {
    let envelope = Envelope {
        metadata: Metadata {
            tool: "vacuum",
            version: env!("CARGO_PKG_VERSION"),
            command: command.to_owned(),
            timestamp: Timestamp::now().to_string(),
            maintainer: MAINTAINER,
            website: WEBSITE,
        },
        data,
    };
    let rendered = serde_json::to_string_pretty(&envelope)?;
    println!("{rendered}");
    Ok(())
}
