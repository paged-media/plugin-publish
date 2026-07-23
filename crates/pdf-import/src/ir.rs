/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * This file is part of paged (https://paged.media) and is additionally
 * available under the Paged Media Enterprise License (PMEL). Full
 * copyright and license information is available in LICENSE.md which is
 * distributed with this source code.
 *
 *  @copyright  Copyright (c) And The Next GmbH
 *  @license    MPL-2.0 OR Paged Media Enterprise License (PMEL)
 */

//! The **Document IR** — the wasm-input contract, twin of the TypeScript
//! `DocumentIr` the bundle's `reconstruct.ts` emits.
//!
//! Everything here is already **reading-ordered**, in **points**, top-left
//! origin (y-down), and style-resolved. This crate is deliberately
//! **PDF-blind**: all pdf.js knowledge and every reconstruction heuristic
//! (glyph clustering, space recovery, paragraph/column detection, confidence
//! gating) live in TypeScript next to pdf.js and vitest. The IR is the clean
//! seam between "understand the PDF" (TS) and "build the native model"
//! (Rust) — so a model-shape change is a Rust compile error, never silent
//! data loss.

use serde::Deserialize;

/// The whole reconstructed document: pages in reading order.
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentIr {
    pub pages: Vec<PageIr>,
}

/// One page: size in points plus its frames in reading order. An optional
/// full-page raster (base64 PNG) is kept as a locked background beneath the
/// recovered frames; low-confidence regions emit no text and let the raster
/// show through (Phase 1 honest degradation).
#[derive(Debug, Clone, Deserialize)]
pub struct PageIr {
    pub width_pt: f32,
    pub height_pt: f32,
    #[serde(default)]
    pub background_png_b64: Option<String>,
    #[serde(default)]
    pub frames: Vec<FrameIr>,
}

/// A frame on the page. Tagged by `kind` in JSON so the TS twin and serde
/// agree explicitly (`{"kind":"text",...}` / `{"kind":"image",...}`).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FrameIr {
    Text(TextFrameIr),
    Image(ImageFrameIr),
}

/// A rectangle in point coordinates, top-left origin (y grows downward).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct RectIr {
    pub x_pt: f32,
    pub y_pt: f32,
    pub width_pt: f32,
    pub height_pt: f32,
}

/// An editable text frame: geometry + its paragraphs (each a run list).
#[derive(Debug, Clone, Deserialize)]
pub struct TextFrameIr {
    #[serde(flatten)]
    pub rect: RectIr,
    #[serde(default)]
    pub paragraphs: Vec<ParagraphIr>,
}

/// A paragraph = a sequence of styled runs.
#[derive(Debug, Clone, Deserialize)]
pub struct ParagraphIr {
    #[serde(default)]
    pub runs: Vec<RunIr>,
}

/// A styled text run — the smallest unit carrying uniform character attrs.
#[derive(Debug, Clone, Deserialize)]
pub struct RunIr {
    pub text: String,
    #[serde(default = "default_font_size")]
    pub font_size_pt: f32,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    /// sRGB fill, each channel 0.0..=1.0. Mapped to a swatch in Phase 2;
    /// carried through now so runs already record their colour.
    #[serde(default)]
    pub color_rgb: Option<[f32; 3]>,
}

fn default_font_size() -> f32 {
    12.0
}

/// An image frame carrying an inline PNG (base64). A full-page raster is just
/// an image frame spanning the page.
#[derive(Debug, Clone, Deserialize)]
pub struct ImageFrameIr {
    #[serde(flatten)]
    pub rect: RectIr,
    pub png_b64: String,
}
