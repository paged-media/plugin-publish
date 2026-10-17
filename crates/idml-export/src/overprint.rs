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

//! A page item's `OverprintFill` / `OverprintStroke`.
//!
//! The importer reads both off every page item (absent ⇒ `false`, the
//! IDML default) and the renderer honours them, but no writer spelled
//! them: an item set to overprint knocked out of the plate beneath it
//! in the export and in InDesign. InDesign writes them as attributes of
//! the item's own element (`<Polygon … OverprintFill="true">`, 24 of the
//! corpus's real packages' polygons; the rest spell `"false"`).

use quick_xml::events::BytesStart;

use idml_import::Spread;

use crate::rewrite::{attr_value, patch_start, Patch};

/// The two attributes for an item the export mints: only a `true` is
/// written, absence being InDesign's default.
pub(crate) fn push_attrs(attrs: &mut Vec<(&'static str, String)>, fill: bool, stroke: bool) {
    if fill {
        attrs.push(("OverprintFill", "true".to_string()));
    }
    if stroke {
        attrs.push(("OverprintStroke", "true".to_string()));
    }
}

/// [`push_attrs`] for the model item `id` of `spread`, when there is one.
pub(crate) fn push_item_attrs(attrs: &mut Vec<(&'static str, String)>, spread: &Spread, id: &str) {
    if let Some((fill, stroke)) = item_overprint(spread, id) {
        push_attrs(attrs, fill, stroke);
    }
}

/// The model's `(overprint_fill, overprint_stroke)` for the page item
/// `id`, across the four kinds the model carries them on (a graphic
/// line models neither, so its source attributes pass through).
fn item_overprint(spread: &Spread, id: &str) -> Option<(bool, bool)> {
    let is = |s: &Option<String>| s.as_deref() == Some(id);
    if let Some(f) = spread.text_frames.iter().find(|f| is(&f.self_id)) {
        return Some((f.overprint_fill, f.overprint_stroke));
    }
    if let Some(r) = spread.rectangles.iter().find(|r| is(&r.self_id)) {
        return Some((r.overprint_fill, r.overprint_stroke));
    }
    if let Some(o) = spread.ovals.iter().find(|o| is(&o.self_id)) {
        return Some((o.overprint_fill, o.overprint_stroke));
    }
    spread
        .polygons
        .iter()
        .find(|p| is(&p.self_id))
        .map(|p| (p.overprint_fill, p.overprint_stroke))
}

/// A source page item's start tag with both attributes brought in line
/// with the model. A value the source already says — `"false"` spelled
/// out or left absent — keeps its bytes; a changed one is set; a return
/// to the default drops a source `"true"`.
pub(crate) fn patch_item(
    start: BytesStart<'static>,
    spread: &Spread,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let Some(self_id) = attr_value(&start, b"Self") else {
        return Ok(start);
    };
    let Some((fill, stroke)) = item_overprint(spread, &self_id) else {
        return Ok(start);
    };
    let reads = |k: &[u8]| {
        attr_value(&start, k)
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(false)
    };
    if reads(b"OverprintFill") == fill && reads(b"OverprintStroke") == stroke {
        return Ok(start);
    }
    let patch = |raw: &[u8], v: bool| {
        let source = std::str::from_utf8(raw)
            .ok()
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(false);
        if source == v {
            Patch::Keep
        } else if v {
            Patch::Set("true".to_string())
        } else {
            Patch::Remove
        }
    };
    let mut extras = Vec::new();
    push_attrs(&mut extras, fill, stroke);
    patch_start(
        &start,
        |k, raw| match k {
            b"OverprintFill" => Some(patch(raw, fill)),
            b"OverprintStroke" => Some(patch(raw, stroke)),
            _ => None,
        },
        &extras,
    )
}
