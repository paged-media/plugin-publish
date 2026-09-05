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

//! `<TextFramePreference>` auto-sizing save-back for EXISTING frames.
//!
//! Measured on InDesign 20.0.1: `<TextFramePreference
//! AutoSizingType="HeightOnly" AutoSizingReferencePoint="TopCenterPoint"/>`
//! inside a `<TextFrame>` is honoured; a frame the model auto-sizes but
//! whose source element carries no such preference is reported overset.
//! `rewrite::write_new_text_frame` writes it for inserted frames; this
//! pass brings a SOURCE frame's element in line with the model — patched
//! in place when present, added before `</TextFrame>` when absent. Its
//! own streaming pass (like `guides`) so the page-item rewrite stays
//! untouched; byte-identical when nothing differs.

use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{Spread, TextFrame};

use crate::rewrite::{
    attr_value, auto_sizing_idml, auto_sizing_reference_point_idml, format_f32, patch_start,
    preserving_f32_patch, write_text_frame_preference, Patch,
};

fn patch_preference(
    e: &BytesStart,
    f: &TextFrame,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let Some(kind) = f.auto_sizing else {
        return Ok(e.clone().into_owned());
    };
    let kind = auto_sizing_idml(kind);
    let point = f
        .auto_sizing_reference_point
        .map(auto_sizing_reference_point_idml);
    let mut extras: Vec<(&str, String)> = vec![("AutoSizingType", kind.to_string())];
    if let Some(p) = point {
        extras.push(("AutoSizingReferencePoint", p.to_string()));
    }
    if let Some(v) = f.minimum_width_for_auto_sizing {
        extras.push(("MinimumWidthForAutoSizing", format_f32(v)));
    }
    if let Some(v) = f.minimum_height_for_auto_sizing {
        extras.push(("MinimumHeightForAutoSizing", format_f32(v)));
    }
    if let Some(v) = f.use_minimum_height_for_auto_sizing {
        extras.push(("UseMinimumHeightForAutoSizing", v.to_string()));
    }
    let rebuilt = patch_start(
        e,
        |k, raw| {
            let raw = std::str::from_utf8(raw).ok();
            match k {
                b"AutoSizingType" => Some(if raw == Some(kind) {
                    Patch::Keep
                } else {
                    Patch::Set(kind.to_string())
                }),
                b"AutoSizingReferencePoint" => Some(match point {
                    Some(p) if raw == Some(p) => Patch::Keep,
                    Some(p) => Patch::Set(p.to_string()),
                    None => Patch::Keep,
                }),
                b"MinimumWidthForAutoSizing" => f
                    .minimum_width_for_auto_sizing
                    .map(|v| preserving_f32_patch(raw, Some(v))),
                b"MinimumHeightForAutoSizing" => f
                    .minimum_height_for_auto_sizing
                    .map(|v| preserving_f32_patch(raw, Some(v))),
                b"UseMinimumHeightForAutoSizing" => f.use_minimum_height_for_auto_sizing.map(|v| {
                    if raw == Some(if v { "true" } else { "false" }) {
                        Patch::Keep
                    } else {
                        Patch::Set(v.to_string())
                    }
                }),
                _ => None,
            }
        },
        &extras,
    )?;
    Ok(
        if e.as_ref().trim_ascii_end() == rebuilt.as_ref().trim_ascii_end() {
            e.clone().into_owned()
        } else {
            rebuilt
        },
    )
}

/// Bring every source `<TextFrame>`'s `<TextFramePreference>` auto-sizing
/// in line with the model frame of the same `Self`. Frames the model does
/// not auto-size are left exactly as they are.
pub fn rewrite_text_frame_prefs(
    original: &[u8],
    spread: &Spread,
) -> Result<Vec<u8>, quick_xml::Error> {
    let frames: std::collections::HashMap<&str, &TextFrame> = spread
        .text_frames
        .iter()
        .filter(|f| f.auto_sizing.is_some())
        .filter_map(|f| f.self_id.as_deref().map(|id| (id, f)))
        .collect();
    if frames.is_empty() {
        return Ok(original.to_vec());
    }
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut depth = 0usize;
    // The open model-matched `<TextFrame>`: (depth, frame, preference seen).
    let mut open: Option<(usize, &TextFrame, bool)> = None;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                match e.name().as_ref() {
                    b"TextFrame" if open.is_none() => {
                        let f =
                            attr_value(&e, b"Self").and_then(|id| frames.get(id.as_str()).copied());
                        if let Some(f) = f {
                            open = Some((depth, f, false));
                        }
                        writer.write_event(Event::Start(e.into_owned()))?;
                    }
                    b"TextFramePreference" if open.is_some_and(|(d, _, _)| d + 1 == depth) => {
                        let (d, f, _) = open.expect("checked");
                        open = Some((d, f, true));
                        writer.write_event(Event::Start(patch_preference(&e, f)?))?;
                    }
                    _ => writer.write_event(Event::Start(e.into_owned()))?,
                }
            }
            Event::Empty(e) => match e.name().as_ref() {
                b"TextFramePreference" if open.is_some_and(|(d, _, _)| d + 1 == depth + 1) => {
                    let (d, f, _) = open.expect("checked");
                    open = Some((d, f, true));
                    writer.write_event(Event::Empty(patch_preference(&e, f)?))?;
                }
                // A self-closing frame the model auto-sizes: expand it
                // around the preference it needs.
                b"TextFrame" if open.is_none() => {
                    let f = attr_value(&e, b"Self").and_then(|id| frames.get(id.as_str()).copied());
                    match f {
                        Some(f) => {
                            writer.write_event(Event::Start(e.borrow()))?;
                            write_text_frame_preference(&mut writer, f)?;
                            writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                                "TextFrame",
                            )))?;
                        }
                        None => writer.write_event(Event::Empty(e.into_owned()))?,
                    }
                }
                _ => writer.write_event(Event::Empty(e.into_owned()))?,
            },
            Event::End(e) => {
                if let Some((d, f, seen)) = open {
                    if d == depth && e.name().as_ref() == b"TextFrame" {
                        if !seen {
                            write_text_frame_preference(&mut writer, f)?;
                        }
                        open = None;
                    }
                }
                depth = depth.saturating_sub(1);
                writer.write_event(Event::End(e))?;
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}
