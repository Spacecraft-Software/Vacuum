// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The **Steelbore** theme — the canonical Spacecraft Software color contract
//! (The Steelbore Standard §11 / §11.1).
//!
//! UI code references these named tokens rather than bare hex literals, so an
//! end user can swap the theme without touching application logic. Void Navy is
//! the mandatory background for every Spacecraft Software surface.
//!
//! Rust guideline compliant 2026-05-18

#![forbid(unsafe_code)]

use ratatui::style::Color;

/// The Steelbore theme: named tokens mapping to the Standard §11 palette.
#[derive(Debug, Clone, Copy)]
pub struct Steelbore;

impl Steelbore {
    /// Void Navy `#000027` — the mandatory background / canvas.
    pub const BACKGROUND: Color = Color::Rgb(0, 0, 39);
    /// Molten Amber `#D98E32` — primary text / active readout.
    pub const FOREGROUND: Color = Color::Rgb(217, 142, 50);
    /// Steel Blue `#4B7EB0` — primary accent / structural.
    pub const ACCENT: Color = Color::Rgb(75, 126, 176);
    /// Radium Green `#50FA7B` — success / safe status.
    pub const SUCCESS: Color = Color::Rgb(80, 250, 123);
    /// Red Oxide `#FF5C5C` — warning / error status.
    pub const ERROR: Color = Color::Rgb(255, 92, 92);
    /// Liquid Coolant `#8BE9FD` — info / links.
    pub const INFO: Color = Color::Rgb(139, 233, 253);
}
