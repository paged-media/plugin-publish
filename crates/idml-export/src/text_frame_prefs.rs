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

//! `<TextFramePreference>` save-back for EXISTING frames: auto-sizing,
//! insets, and the frame's layout options (columns, balance, first
//! baseline, vertical justification, ignore-wrap — [`layout_attrs`]).
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

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{FirstBaselineOffset, Spread, TextFrame, VerticalJustification};

use crate::rewrite::{
    attr_value, auto_sizing_idml, auto_sizing_reference_point_idml, format_f32, patch_start,
    preserving_f32_patch, write_inset_spacing, write_text_frame_preference, Patch,
};

/// The insets a SOURCE `<TextFrame>` already carries, keyed by `Self`.
///
/// Needed because "the model changed the insets" is not a question the
/// parsed document can answer on its own — its value IS the source's
/// unless something mutated it. A frame whose insets still agree with
/// its source is left completely untouched, so an unmutated package
/// stays byte-identical.
fn source_insets(original: &[u8]) -> std::collections::HashMap<String, [f32; 4]> {
    let mut out = std::collections::HashMap::new();
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut buf = Vec::new();
    // (frame Self, inside its own TextFramePreference, list form, values, text)
    let mut frame: Option<String> = None;
    let mut in_pref = false;
    let mut open: Option<(bool, Vec<f32>)> = None;
    let mut text = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"TextFrame" => frame = attr_value(&e, b"Self"),
                b"TextFramePreference" if frame.is_some() => in_pref = true,
                b"InsetSpacing" if in_pref => {
                    open = Some((
                        attr_value(&e, b"type").as_deref() == Some("list"),
                        Vec::new(),
                    ));
                    text.clear();
                }
                b"ListItem" if open.is_some() => text.clear(),
                _ => {}
            },
            Ok(Event::Text(t)) if open.is_some() => {
                text.push_str(&String::from_utf8_lossy(t.as_ref()));
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"TextFrame" => frame = None,
                b"TextFramePreference" => in_pref = false,
                b"ListItem" => {
                    if let Some((_, vals)) = open.as_mut() {
                        if let Ok(v) = text.trim().parse::<f32>() {
                            vals.push(v);
                        }
                        text.clear();
                    }
                }
                b"InsetSpacing" => {
                    if let (Some((list, vals)), Some(id)) = (open.take(), frame.as_ref()) {
                        let insets = if list {
                            (vals.len() == 4).then(|| [vals[0], vals[1], vals[2], vals[3]])
                        } else {
                            text.trim().parse::<f32>().ok().map(|v| [v; 4])
                        };
                        if let Some(insets) = insets {
                            out.insert(id.clone(), insets);
                        }
                    }
                    text.clear();
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    out
}

fn patch_preference(
    e: &BytesStart,
    f: &TextFrame,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let kind = f.auto_sizing.map(auto_sizing_idml);
    let point = f
        .auto_sizing_reference_point
        .map(auto_sizing_reference_point_idml);
    // The layout attributes first, in InDesign's order, then the
    // auto-sizing ones (only for a frame the model auto-sizes, as
    // before).
    let mut extras: Vec<(&str, String)> = layout_attrs(f);
    if let Some(kind) = kind {
        extras.push(("AutoSizingType", kind.to_string()));
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
    }
    let rebuilt = patch_start(
        e,
        |k, raw| {
            let raw = std::str::from_utf8(raw).ok();
            if let Some(patch) = layout_patch(k, raw, f) {
                return Some(patch);
            }
            let kind = kind?;
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

/// InDesign's `VerticalJustification` value.
fn vertical_justification_idml(v: VerticalJustification) -> &'static str {
    match v {
        VerticalJustification::Top => "TopAlign",
        VerticalJustification::Center => "CenterAlign",
        VerticalJustification::Bottom => "BottomAlign",
        VerticalJustification::Justify => "JustifyAlign",
    }
}

/// InDesign's `FirstBaselineOffset` value. The em box is `EmboxHeight`
/// in its own export (InDesign reads `EmBoxHeight`, the wire's name, as
/// `AscentOffset` — see `paged_model::FirstBaselineOffset::from_idml`).
fn first_baseline_offset_idml(v: FirstBaselineOffset) -> &'static str {
    match v {
        FirstBaselineOffset::AscentOffset => "AscentOffset",
        FirstBaselineOffset::CapHeight => "CapHeight",
        FirstBaselineOffset::XHeight => "XHeight",
        FirstBaselineOffset::EmBoxHeight => "EmboxHeight",
        FirstBaselineOffset::LeadingOffset => "LeadingOffset",
        FirstBaselineOffset::FixedHeight => "FixedHeight",
    }
}

/// The text frame's own layout options the model carries — columns,
/// their balance, the first-baseline policy, vertical justification and
/// the frame's wrap opt-out — as `<TextFramePreference>` attributes, in
/// the order InDesign writes them. Only what the model states: an unset
/// option is InDesign's default, and writing nothing keeps it so.
///
/// The importer has always read these and the engine lays them out, but
/// no writer spelled them: a two-column frame, a balanced one, a
/// vertically justified one and every first-baseline policy came back
/// from an export as one top-aligned column set from the ascent.
pub(crate) fn layout_attrs(f: &TextFrame) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    if let Some(n) = f.column_count {
        out.push(("TextColumnCount", n.to_string()));
    }
    if let Some(g) = f.column_gutter {
        out.push(("TextColumnGutter", format_f32(g)));
    }
    if let Some(v) = f.first_baseline_offset {
        out.push((
            "FirstBaselineOffset",
            first_baseline_offset_idml(v).to_string(),
        ));
    }
    if let Some(v) = f.minimum_first_baseline_offset {
        out.push(("MinimumFirstBaselineOffset", format_f32(v)));
    }
    if let Some(v) = f.vertical_justification {
        out.push((
            "VerticalJustification",
            vertical_justification_idml(v).to_string(),
        ));
    }
    if let Some(v) = f.ignore_wrap {
        out.push(("IgnoreWrap", v.to_string()));
    }
    if let Some(v) = f.column_balance {
        out.push(("VerticalBalanceColumns", v.to_string()));
    }
    out
}

/// Whether the source preference attribute `k` (one of
/// [`layout_attrs`]'s) already says what the model holds. A value the
/// source spells another way that reads the same (`TextColumnGutter="12.0"`)
/// keeps its bytes; an enumerator is compared by its spelling, so a
/// value only our older exports wrote (`EmBoxHeight`) is rewritten in
/// InDesign's.
fn layout_patch(k: &[u8], raw: Option<&str>, f: &TextFrame) -> Option<Patch> {
    fn spelled(raw: Option<&str>, want: Option<String>) -> Patch {
        match want {
            Some(w) if raw == Some(w.as_str()) => Patch::Keep,
            Some(w) => Patch::Set(w),
            None => Patch::Remove,
        }
    }
    fn parsed<T: std::str::FromStr + PartialEq + ToString>(
        raw: Option<&str>,
        want: Option<T>,
    ) -> Patch {
        match want {
            Some(w) if raw.and_then(|s| s.trim().parse::<T>().ok()).as_ref() == Some(&w) => {
                Patch::Keep
            }
            Some(w) => Patch::Set(w.to_string()),
            None => Patch::Remove,
        }
    }
    Some(match k {
        b"TextColumnCount" => parsed(raw, f.column_count),
        b"TextColumnGutter" => match f.column_gutter {
            Some(v) => preserving_f32_patch(raw, Some(v)),
            None => Patch::Remove,
        },
        b"FirstBaselineOffset" => spelled(
            raw,
            f.first_baseline_offset
                .map(|v| first_baseline_offset_idml(v).to_string()),
        ),
        b"MinimumFirstBaselineOffset" => match f.minimum_first_baseline_offset {
            Some(v) => preserving_f32_patch(raw, Some(v)),
            None => Patch::Remove,
        },
        b"VerticalJustification" => spelled(
            raw,
            f.vertical_justification
                .map(|v| vertical_justification_idml(v).to_string()),
        ),
        b"IgnoreWrap" => parsed(raw, f.ignore_wrap),
        b"VerticalBalanceColumns" => parsed(raw, f.column_balance),
        _ => return None,
    })
}

/// Whether the model states any of [`layout_attrs`].
fn has_layout(f: &TextFrame) -> bool {
    f.column_count.is_some()
        || f.column_gutter.is_some()
        || f.first_baseline_offset.is_some()
        || f.minimum_first_baseline_offset.is_some()
        || f.vertical_justification.is_some()
        || f.ignore_wrap.is_some()
        || f.column_balance.is_some()
}

/// The `Self` of every source `<TextFrame>` whose own
/// `<TextFramePreference>` spells one of [`layout_attrs`] — so an option
/// the model has since CLEARED is removed rather than passed through.
fn source_layout_frames(original: &[u8]) -> std::collections::HashSet<String> {
    const KEYS: [&[u8]; 7] = [
        b"TextColumnCount",
        b"TextColumnGutter",
        b"FirstBaselineOffset",
        b"MinimumFirstBaselineOffset",
        b"VerticalJustification",
        b"IgnoreWrap",
        b"VerticalBalanceColumns",
    ];
    let mut out = std::collections::HashSet::new();
    let mut reader = Reader::from_reader(original);
    reader.config_mut().expand_empty_elements = false;
    let mut buf = Vec::new();
    let mut frame: Option<String> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"TextFrame" => frame = attr_value(&e, b"Self"),
                b"TextFramePreference" => {
                    if let Some(id) = frame.as_ref() {
                        if e.attributes()
                            .flatten()
                            .any(|a| KEYS.contains(&a.key.as_ref()))
                        {
                            out.insert(id.clone());
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) if e.name().as_ref() == b"TextFrame" => frame = None,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Bring every source `<TextFrame>`'s `<TextFramePreference>` in line
/// with the model frame of the same `Self` — auto-sizing, and the text
/// insets when the model has changed them. A frame the model neither
/// auto-sizes nor re-insets is left exactly as it is.
pub fn rewrite_text_frame_prefs(
    original: &[u8],
    spread: &Spread,
) -> Result<Vec<u8>, quick_xml::Error> {
    // Insets are only rewritten where the model DISAGREES with the
    // source, so an unmutated package is passed through untouched
    // rather than reformatted into our own spelling.
    let source = source_insets(original);
    let moved_insets: std::collections::HashSet<&str> = spread
        .text_frames
        .iter()
        .filter_map(|f| {
            let id = f.self_id.as_deref()?;
            let model = f.inset_spacing.unwrap_or([0.0; 4]);
            let src = source.get(id).copied().unwrap_or([0.0; 4]);
            (model != src).then_some(id)
        })
        .collect();
    let layout_in_source = source_layout_frames(original);
    let frames: std::collections::HashMap<&str, &TextFrame> = spread
        .text_frames
        .iter()
        .filter_map(|f| f.self_id.as_deref().map(|id| (id, f)))
        .filter(|(id, f)| {
            f.auto_sizing.is_some()
                || moved_insets.contains(id)
                || has_layout(f)
                || layout_in_source.contains(*id)
        })
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
    // The open `<TextFramePreference>` whose insets must be rewritten:
    // (its depth, the values, whether they have been written, whether a
    // `<Properties>` child has been seen). `skip` holds the depth of an
    // existing `<InsetSpacing>` subtree being dropped in favour of it.
    let mut pref: Option<(usize, [f32; 4], bool, bool)> = None;
    let mut skip: Option<usize> = None;
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
                        if moved_insets.contains(f.self_id.as_deref().unwrap_or_default()) {
                            pref = Some((depth, f.inset_spacing.unwrap_or([0.0; 4]), false, false));
                        }
                    }
                    // The model's insets replace whatever the source
                    // carried; the rest of `<Properties>` is untouched.
                    b"Properties" if pref.is_some_and(|(d, _, _, _)| d + 1 == depth) => {
                        writer.write_event(Event::Start(e.into_owned()))?;
                        let (d, insets, _, _) = pref.expect("checked");
                        write_inset_spacing(&mut writer, insets)?;
                        pref = Some((d, insets, true, true));
                    }
                    b"InsetSpacing" if pref.is_some_and(|(_, _, w, _)| w) && skip.is_none() => {
                        skip = Some(depth);
                    }
                    _ if skip.is_some() => {}
                    _ => writer.write_event(Event::Start(e.into_owned()))?,
                }
            }
            Event::Empty(e) => match e.name().as_ref() {
                b"TextFramePreference" if open.is_some_and(|(d, _, _)| d + 1 == depth + 1) => {
                    let (d, f, _) = open.expect("checked");
                    open = Some((d, f, true));
                    let start = patch_preference(&e, f)?;
                    if moved_insets.contains(f.self_id.as_deref().unwrap_or_default()) {
                        // A self-closing preference has nowhere to put
                        // the insets — open it around a `<Properties>`.
                        writer.write_event(Event::Start(start))?;
                        writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                        write_inset_spacing(&mut writer, f.inset_spacing.unwrap_or([0.0; 4]))?;
                        writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                        writer.write_event(Event::End(BytesEnd::new("TextFramePreference")))?;
                    } else {
                        writer.write_event(Event::Empty(start))?;
                    }
                }
                _ if skip.is_some() => {}
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
                if let Some(d) = skip {
                    // Inside the source `<InsetSpacing>` we replaced.
                    if d == depth && e.name().as_ref() == b"InsetSpacing" {
                        skip = None;
                    }
                    depth = depth.saturating_sub(1);
                    buf.clear();
                    continue;
                }
                if let Some((d, insets, written, saw_props)) = pref {
                    if d == depth && e.name().as_ref() == b"TextFramePreference" {
                        if !saw_props {
                            // No `<Properties>` at all: give it one.
                            writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                            write_inset_spacing(&mut writer, insets)?;
                            writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                        }
                        let _ = written;
                        pref = None;
                    }
                }
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
            other if skip.is_none() => writer.write_event(other)?,
            _ => {}
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}
