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

//! Text wrap save-back: every page item the model wraps text around
//! carries InDesign's `<TextWrapPreference>` in the export.
//!
//! The engine composes around a wrap set by an op (`frameTextWrapMode`
//! — bounding box, contour, jump, next column, the inverted "text
//! inside the outline" form), but an engine-set wrap was never
//! written: the annual's wrap exhibits opened in InDesign with no wrap
//! at all, and the paragraph the engine set INSIDE an inverted wrap
//! composed full-measure there (measured 2026-09-06 — the one story
//! the engine overset and InDesign did not). This pass writes the
//! block, in InDesign's spelling, on every host that models a wrap and
//! whose source element carries none; a source that already spells its
//! wrap passes through byte-identical (its block is the source's word,
//! see `rewrite`).

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{ContourOptionType, Spread, TextWrap, TextWrapMode};

use crate::rewrite::{attr_value, emit_empty_with_attrs, emit_start_with_attrs, format_f32};

const TEXT_WRAP_PREFERENCE: &[u8] = b"TextWrapPreference";

fn is_host_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"Rectangle" | b"Oval" | b"Polygon" | b"TextFrame" | b"GraphicLine"
    )
}

fn mode_idml(mode: TextWrapMode) -> Option<&'static str> {
    Some(match mode {
        TextWrapMode::None => "None",
        TextWrapMode::BoundingBoxTextWrap => "BoundingBoxTextWrap",
        TextWrapMode::ContourTextWrap => "ContourTextWrap",
        TextWrapMode::JumpObjectTextWrap => "JumpObjectTextWrap",
        TextWrapMode::NextColumnTextWrap => "NextColumnTextWrap",
        TextWrapMode::Other => return None,
    })
}

fn contour_idml(kind: ContourOptionType) -> Option<&'static str> {
    Some(match kind {
        ContourOptionType::SameAsClipping => "SameAsClipping",
        ContourOptionType::GraphicFrame => "GraphicFrame",
        ContourOptionType::DetectEdges => "DetectEdges",
        ContourOptionType::AlphaChannel => "AlphaChannel",
        ContourOptionType::PhotoshopPath => "PhotoshopPath",
        _ => return None,
    })
}

/// A wrap worth writing: one that changes composition. `None` mode
/// with nothing inverted is InDesign's default and is not spelled.
fn wraps(w: &TextWrap) -> bool {
    w.mode != TextWrapMode::None || w.invert == Some(true)
}

/// Every wrapping host in `spread`, keyed by `Self`.
fn wraps_of(spread: &Spread) -> HashMap<String, TextWrap> {
    let mut out: HashMap<String, TextWrap> = HashMap::new();
    let mut add = |id: Option<&str>, w: Option<&TextWrap>| {
        if let (Some(id), Some(w)) = (id, w) {
            if wraps(w) {
                out.insert(id.to_string(), w.clone());
            }
        }
    };
    for r in &spread.rectangles {
        add(r.self_id.as_deref(), r.text_wrap.as_ref());
    }
    for o in &spread.ovals {
        add(o.self_id.as_deref(), o.text_wrap.as_ref());
    }
    for p in &spread.polygons {
        add(p.self_id.as_deref(), p.text_wrap.as_ref());
    }
    for t in &spread.text_frames {
        add(t.self_id.as_deref(), t.text_wrap.as_ref());
    }
    for l in &spread.graphic_lines {
        add(l.self_id.as_deref(), l.text_wrap.as_ref());
    }
    out
}

/// `<TextWrapPreference …>` in InDesign's spelling: the mode and the
/// inverted flag on the element, the four offsets as a typed
/// `<Properties><TextWrapOffset>` child, a `<ContourOption>` for a
/// contour wrap.
fn write_text_wrap(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    w: &TextWrap,
) -> Result<(), quick_xml::Error> {
    let mode = mode_idml(w.mode).unwrap_or("BoundingBoxTextWrap");
    emit_start_with_attrs(
        writer,
        "TextWrapPreference",
        &[
            ("Inverse", w.invert.unwrap_or(false).to_string()),
            ("ApplyToMasterPageOnly", "false".to_string()),
            ("TextWrapSide", "BothSides".to_string()),
            ("TextWrapMode", mode.to_string()),
        ],
    )?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    emit_empty_with_attrs(
        writer,
        "TextWrapOffset",
        &[
            ("Top", format_f32(w.offsets[0])),
            ("Left", format_f32(w.offsets[1])),
            ("Bottom", format_f32(w.offsets[2])),
            ("Right", format_f32(w.offsets[3])),
        ],
    )?;
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    if w.mode == TextWrapMode::ContourTextWrap {
        emit_empty_with_attrs(
            writer,
            "ContourOption",
            &[
                (
                    "ContourType",
                    w.contour_type
                        .and_then(contour_idml)
                        .unwrap_or("GraphicFrame")
                        .to_string(),
                ),
                (
                    "IncludeInsideEdges",
                    w.include_inside_edges.unwrap_or(false).to_string(),
                ),
                ("ContourPathName", "$ID/".to_string()),
            ],
        )?;
    }
    writer.write_event(Event::End(BytesEnd::new("TextWrapPreference")))?;
    Ok(())
}

/// Write a `<TextWrapPreference>` into every wrapping host of `spread`
/// whose element has none — right after the host's own `<Properties>`
/// block when it has one, else first inside the element (a self-closing
/// host opens up around it). Byte-identical when nothing is missing.
pub fn rewrite_text_wrap(original: &[u8], spread: &Spread) -> Result<Vec<u8>, quick_xml::Error> {
    rewrite_with(original, &wraps_of(spread))
}

fn rewrite_with(
    original: &[u8],
    wraps: &HashMap<String, TextWrap>,
) -> Result<Vec<u8>, quick_xml::Error> {
    if wraps.is_empty() {
        return Ok(original.to_vec());
    }
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut depth = 0usize;
    // The open wrapping host: (depth, its wrap, written yet, inside its
    // own `<Properties>` block).
    let mut open: Option<(usize, TextWrap, bool, bool)> = None;
    let mut changed = false;
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Eof => break,
            Event::Start(ref e) => {
                depth += 1;
                let name = e.name().as_ref().to_vec();
                if open.is_none() && is_host_name(&name) {
                    if let Some(w) = attr_value(e, b"Self").and_then(|id| wraps.get(&id).cloned()) {
                        open = Some((depth, w, false, false));
                    }
                    writer.write_event(ev.borrow())?;
                } else if let Some((d, _, written, in_props)) = open.as_mut() {
                    if depth == *d + 1 && name == TEXT_WRAP_PREFERENCE {
                        // The source spells its own wrap: nothing to add.
                        *written = true;
                    } else if depth == *d + 1 && name == b"Properties" {
                        *in_props = true;
                    }
                    writer.write_event(ev.borrow())?;
                } else {
                    writer.write_event(ev.borrow())?;
                }
            }
            Event::Empty(ref e) => {
                let name = e.name().as_ref().to_vec();
                if open.is_none() && is_host_name(&name) {
                    match attr_value(e, b"Self").and_then(|id| wraps.get(&id).cloned()) {
                        Some(w) => {
                            writer.write_event(Event::Start(e.to_owned()))?;
                            write_text_wrap(&mut writer, &w)?;
                            writer.write_event(Event::End(BytesEnd::new(
                                String::from_utf8_lossy(&name).into_owned(),
                            )))?;
                            changed = true;
                        }
                        None => writer.write_event(ev.borrow())?,
                    }
                } else if let Some((d, _, written, _)) = open.as_mut() {
                    if depth + 1 == *d + 1 && name == TEXT_WRAP_PREFERENCE {
                        *written = true;
                    }
                    writer.write_event(ev.borrow())?;
                } else {
                    writer.write_event(ev.borrow())?;
                }
            }
            Event::End(ref e) => {
                let name = e.name().as_ref().to_vec();
                if let Some((d, w, written, in_props)) = open.as_mut() {
                    // The host's own Properties block closes: the wrap
                    // goes right after it.
                    if depth == *d + 1 && name == b"Properties" && *in_props {
                        writer.write_event(ev.borrow())?;
                        if !*written {
                            write_text_wrap(&mut writer, w)?;
                            *written = true;
                            changed = true;
                        }
                        *in_props = false;
                        depth = depth.saturating_sub(1);
                        buf.clear();
                        continue;
                    }
                    if depth == *d && is_host_name(&name) {
                        if !*written {
                            write_text_wrap(&mut writer, w)?;
                            changed = true;
                        }
                        open = None;
                    }
                }
                depth = depth.saturating_sub(1);
                writer.write_event(ev.borrow())?;
            }
            other => writer.write_event(other.borrow())?,
        }
        buf.clear();
    }
    Ok(if changed {
        writer.into_inner().into_inner()
    } else {
        original.to_vec()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wraps_with(id: &str, w: TextWrap) -> HashMap<String, TextWrap> {
        HashMap::from([(id.to_string(), w)])
    }

    #[test]
    fn a_wrapping_host_without_the_block_gains_it_after_its_properties() {
        let wraps = wraps_with(
            "r1",
            TextWrap {
                mode: TextWrapMode::BoundingBoxTextWrap,
                offsets: [4.0, 4.0, 4.0, 4.0],
                invert: Some(true),
                contour_type: None,
                include_inside_edges: None,
            },
        );
        let src = r#"<Spread Self="s1"><Rectangle Self="r1" ItemTransform="1 0 0 1 0 0"><Properties><PathGeometry/></Properties></Rectangle><Rectangle Self="r2"><Properties><PathGeometry/></Properties></Rectangle><Rectangle Self="r3" ItemTransform="1 0 0 1 0 0"/></Spread>"#;
        let out = String::from_utf8(rewrite_with(src.as_bytes(), &wraps).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<Spread Self="s1"><Rectangle Self="r1" ItemTransform="1 0 0 1 0 0"><Properties><PathGeometry/></Properties><TextWrapPreference Inverse="true" ApplyToMasterPageOnly="false" TextWrapSide="BothSides" TextWrapMode="BoundingBoxTextWrap"><Properties><TextWrapOffset Top="4" Left="4" Bottom="4" Right="4"/></Properties></TextWrapPreference></Rectangle><Rectangle Self="r2"><Properties><PathGeometry/></Properties></Rectangle><Rectangle Self="r3" ItemTransform="1 0 0 1 0 0"/></Spread>"#
        );
    }

    #[test]
    fn a_contour_wrap_carries_its_contour_option_and_a_spelled_wrap_is_kept() {
        let wraps = wraps_with(
            "p1",
            TextWrap {
                mode: TextWrapMode::ContourTextWrap,
                offsets: [8.0, 8.0, 8.0, 8.0],
                invert: None,
                contour_type: Some(ContourOptionType::SameAsClipping),
                include_inside_edges: Some(true),
            },
        );
        let src = r#"<Spread Self="s1"><Rectangle Self="p1"/></Spread>"#;
        let out = String::from_utf8(rewrite_with(src.as_bytes(), &wraps).unwrap()).unwrap();
        assert!(out.contains(r#"<Rectangle Self="p1"><TextWrapPreference Inverse="false" ApplyToMasterPageOnly="false" TextWrapSide="BothSides" TextWrapMode="ContourTextWrap"><Properties><TextWrapOffset Top="8" Left="8" Bottom="8" Right="8"/></Properties><ContourOption ContourType="SameAsClipping" IncludeInsideEdges="true" ContourPathName="$ID/"/></TextWrapPreference></Rectangle>"#), "{out}");
        let spelled = r#"<Spread Self="s1"><Rectangle Self="p1"><Properties/><TextWrapPreference Inverse="false" TextWrapMode="ContourTextWrap"><Properties><TextWrapOffset Top="8" Left="8" Bottom="8" Right="8"/></Properties></TextWrapPreference></Rectangle></Spread>"#;
        assert_eq!(
            rewrite_with(spelled.as_bytes(), &wraps).unwrap(),
            spelled.as_bytes()
        );
    }
}
