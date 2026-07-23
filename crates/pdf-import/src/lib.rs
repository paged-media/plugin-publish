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

//! `pdf-import` — the **PDF-blind** mapper from a reading-ordered Document IR
//! (produced by the `@paged-media/pdf` bundle's pdf.js reconstruction) into a
//! native paged document.
//!
//! Pipeline: [`ir::DocumentIr`] JSON → [`build::build_document`] →
//! `paged_scene::Document` → [`ocf::wrap_document`] → `.paged` OCF bytes that
//! the editor opens through the existing `host.nativeDocument.open` door with
//! **no IDML parse** (the model rides inside `paged/core/model/document.pgm`).
//!
//! Deliberately tiny: serde + zip + the paged model. It knows nothing about
//! PDF — pdf.js and every reconstruction heuristic live in the TS bundle, so
//! the wasm stays small and rebuilds rarely, and a model-shape change is a
//! compile error here rather than silent data loss in hand-rolled JSON.

pub mod build;
pub mod ir;
pub mod ocf;

/// Errors mapping a Document IR into `.paged` bytes.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid Document IR json: {0}")]
    Json(#[from] serde_json::Error),
    /// pgm encode of the built model (distinct from IR parse so drift is
    /// attributable). Constructed by [`ocf::wrap_document`].
    #[error("native pgm encode failed: {0}")]
    Pgm(serde_json::Error),
    #[error("base64 image decode: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("model build: {0}")]
    Build(String),
}

/// Map a Document-IR JSON string to `.paged` OCF bytes. The single entry the
/// wasm binding and the native tests share.
pub fn pdf_ir_to_paged(ir_json: &str) -> Result<Vec<u8>, Error> {
    let ir: ir::DocumentIr = serde_json::from_str(ir_json)?;
    let doc = build::build_document(&ir)?;
    // The OCF fallback skeleton (only parsed if the pgm ever fails to decode)
    // takes the first page's size so a drift-degraded open is at least the
    // right paper size; Letter if the document has no pages.
    let (w, h) = ir
        .pages
        .first()
        .map(|p| (p.width_pt, p.height_pt))
        .unwrap_or((612.0, 792.0));
    ocf::wrap_document(&doc, w, h)
}

/// wasm-bindgen surface consumed by the bundle's `engine-loader.ts`. Returns
/// the `.paged` bytes or a JS error carrying the mapper's message.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn pdf_ir_to_paged_wasm(ir_json: &str) -> Result<Vec<u8>, wasm_bindgen::JsError> {
    pdf_ir_to_paged(ir_json).map_err(|e| wasm_bindgen::JsError::new(&e.to_string()))
}
