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

//! A picture frame's `<FrameFittingOption>`: its crops, the fit it
//! names (`FittingOnEmptyFrame`), the point the fit pins
//! (`FittingAlignment`) and `AutoFit`.
//!
//! The importer has always read it and no writer spelled it, so every
//! frame's fitting options were lost on export. The crops are not the
//! model's numbers: InDesign measures a crop from the picture's edge in
//! the picture's own units, the model from the frame's edge in the
//! frame's (see `idml_import::crop_from_idml`). They are converted by the
//! scale of the placed child's transform as THIS export writes it, which
//! is what the reader converts them back by; a frame whose picture is
//! not placed by an axis-aligned transform keeps its crops as spelled on
//! both sides.
//!
//! The element is a direct child of the `<Rectangle>` before its placed
//! child, in InDesign's order. A source element is patched in place and
//! keeps the bytes of every value it already says; a frame without one
//! gains one. Runs after [`crate::images`] (from its pass), so a frame
//! the export minted, and the picture transform the export writes, are
//! both already in the bytes.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{crop_from_idml, crop_scales, crop_to_idml, FrameFittingOption, Spread};

use crate::rewrite::{attr_value, emit_empty_with_attrs, patch_start, Patch};

fn is_placed_name(name: &[u8]) -> bool {
    matches!(name, b"Image" | b"EPS" | b"PDF" | b"ImportedPage")
}

/// A crop at the precision the model holds it (InDesign writes doubles).
fn exact(v: f32) -> String {
    if v == 0.0 {
        "0".to_string()
    } else {
        format!("{v}")
    }
}

/// Each `<Rectangle>`'s placed-child transform scales, by `Self` — the
/// transform this export writes, read back off the bytes.
fn placed_scales(original: &[u8]) -> HashMap<String, (f32, f32)> {
    let mut out = HashMap::new();
    let mut reader = Reader::from_reader(original);
    reader.config_mut().expand_empty_elements = false;
    let mut buf = Vec::new();
    // Open `<Rectangle>`s: (depth, Self).
    let mut hosts: Vec<(usize, Option<String>)> = Vec::new();
    let mut depth = 0usize;
    loop {
        let (e, empty) = match reader.read_event_into(&mut buf) {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => (e.into_owned(), false),
            Ok(Event::Empty(e)) => (e.into_owned(), true),
            Ok(Event::End(_)) => {
                if hosts.last().is_some_and(|(d, _)| *d == depth) {
                    hosts.pop();
                }
                depth = depth.saturating_sub(1);
                buf.clear();
                continue;
            }
            _ => {
                buf.clear();
                continue;
            }
        };
        let name = e.name().as_ref().to_vec();
        if is_placed_name(&name) {
            if let Some((d, Some(id))) = hosts.last() {
                if *d == depth && !out.contains_key(id) {
                    let t = attr_value(&e, b"ItemTransform").and_then(|s| {
                        let v: Vec<f32> = s
                            .split_whitespace()
                            .filter_map(|x| x.parse().ok())
                            .collect();
                        (v.len() == 6).then(|| [v[0], v[1], v[2], v[3], v[4], v[5]])
                    });
                    if let Some(s) = t.as_ref().and_then(crop_scales) {
                        out.insert(id.clone(), s);
                    }
                }
            }
        }
        if !empty {
            depth += 1;
            if name == b"Rectangle" {
                hosts.push((depth, attr_value(&e, b"Self")));
            }
        }
        buf.clear();
    }
    out
}

/// The `(attribute, model value)` pairs in InDesign's order, crops in
/// InDesign's units.
fn idml_values(ff: &FrameFittingOption, scales: Option<(f32, f32)>) -> Vec<(&'static str, String)> {
    let crop = |v: f32, s: Option<f32>| exact(s.map_or(v, |s| crop_to_idml(v, s)));
    let (sx, sy) = (scales.map(|s| s.0), scales.map(|s| s.1));
    let mut out: Vec<(&'static str, String)> = Vec::new();
    if let Some(v) = ff.auto_fit {
        out.push(("AutoFit", v.to_string()));
    }
    for (k, v, s) in [
        ("LeftCrop", ff.left_crop, sx),
        ("TopCrop", ff.top_crop, sy),
        ("RightCrop", ff.right_crop, sx),
        ("BottomCrop", ff.bottom_crop, sy),
    ] {
        if let Some(v) = v {
            out.push((k, crop(v, s)));
        }
    }
    if let Some(v) = &ff.fitting_on_empty_frame {
        out.push(("FittingOnEmptyFrame", v.clone()));
    }
    if let Some(v) = &ff.reference_point {
        out.push(("FittingAlignment", v.clone()));
    }
    out
}

/// A source `<FrameFittingOption>` brought in line with the model: a
/// value the source spells that READS as the model's (the reader's own
/// conversion) keeps its bytes.
fn patch_option(
    e: &BytesStart,
    ff: &FrameFittingOption,
    scales: Option<(f32, f32)>,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let reads_as = |raw: Option<&str>, s: Option<f32>| -> Option<f32> {
        let v = raw?.trim().parse::<f32>().ok()?;
        Some(s.map_or(v, |s| crop_from_idml(v, s)))
    };
    let (sx, sy) = (scales.map(|s| s.0), scales.map(|s| s.1));
    let crop_patch = |raw: Option<&str>, model: Option<f32>, s: Option<f32>| match model {
        Some(m) if reads_as(raw, s) == Some(m) => Patch::Keep,
        Some(m) => Patch::Set(exact(s.map_or(m, |s| crop_to_idml(m, s)))),
        None => Patch::Remove,
    };
    let string_patch = |raw: Option<&str>, model: &Option<String>| match model {
        Some(m) if raw == Some(m.as_str()) => Patch::Keep,
        Some(m) => Patch::Set(m.clone()),
        None => Patch::Remove,
    };
    let extras = idml_values(ff, scales);
    let rebuilt = patch_start(
        e,
        |k, raw| {
            let raw = std::str::from_utf8(raw).ok();
            Some(match k {
                b"LeftCrop" => crop_patch(raw, ff.left_crop, sx),
                b"TopCrop" => crop_patch(raw, ff.top_crop, sy),
                b"RightCrop" => crop_patch(raw, ff.right_crop, sx),
                b"BottomCrop" => crop_patch(raw, ff.bottom_crop, sy),
                b"FittingOnEmptyFrame" => string_patch(raw, &ff.fitting_on_empty_frame),
                b"FittingAlignment" => string_patch(raw, &ff.reference_point),
                b"AutoFit" => match ff.auto_fit {
                    Some(v) if raw.and_then(|s| s.parse::<bool>().ok()) == Some(v) => Patch::Keep,
                    Some(v) => Patch::Set(v.to_string()),
                    None => Patch::Remove,
                },
                _ => return None,
            })
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

/// Bring every `<Rectangle>` the model gives fitting options in line
/// with them. Byte-identical when nothing differs.
pub(crate) fn rewrite_frame_fitting(
    original: &[u8],
    spread: &Spread,
) -> Result<Vec<u8>, quick_xml::Error> {
    let fits: HashMap<&str, &FrameFittingOption> = spread
        .rectangles
        .iter()
        .filter_map(|r| Some((r.self_id.as_deref()?, r.frame_fitting.as_ref()?)))
        .collect();
    if fits.is_empty() {
        return Ok(original.to_vec());
    }
    let scales = placed_scales(original);
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut depth = 0usize;
    // Open `<Rectangle>`s: (depth, the model's options, scales, written).
    type Open<'a> = (
        usize,
        Option<&'a FrameFittingOption>,
        Option<(f32, f32)>,
        bool,
    );
    let mut hosts: Vec<Open> = Vec::new();
    let write_new = |writer: &mut Writer<Cursor<Vec<u8>>>,
                     ff: &FrameFittingOption,
                     s: Option<(f32, f32)>|
     -> Result<(), quick_xml::Error> {
        emit_empty_with_attrs(writer, "FrameFittingOption", &idml_values(ff, s))
    };
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.name().as_ref().to_vec();
                // A direct child of an open frame that still owes its
                // option: the option goes before the placed child.
                if let Some((d, Some(ff), s, written)) = hosts.last_mut() {
                    if *d == depth && !*written {
                        if name == b"FrameFittingOption" {
                            *written = true;
                            let start = patch_option(&e, ff, *s)?;
                            depth += 1;
                            writer.write_event(Event::Start(start))?;
                            buf.clear();
                            continue;
                        }
                        if is_placed_name(&name) {
                            *written = true;
                            write_new(&mut writer, ff, *s)?;
                        }
                    }
                }
                depth += 1;
                if name == b"Rectangle" {
                    let id = attr_value(&e, b"Self");
                    let ff = id.as_deref().and_then(|id| fits.get(id).copied());
                    let s = id.and_then(|id| scales.get(&id).copied());
                    hosts.push((depth, ff, s, false));
                }
                writer.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                if let Some((d, Some(ff), s, written)) = hosts.last_mut() {
                    if *d == depth && *written && name == b"FrameFittingOption" {
                        // One spelled after the placed child, already
                        // written before it: drop the second copy.
                        buf.clear();
                        continue;
                    }
                    if *d == depth && !*written {
                        if name == b"FrameFittingOption" {
                            *written = true;
                            writer.write_event(Event::Empty(patch_option(&e, ff, *s)?))?;
                            buf.clear();
                            continue;
                        }
                        if is_placed_name(&name) {
                            *written = true;
                            write_new(&mut writer, ff, *s)?;
                        }
                    }
                }
                if name == b"Rectangle" {
                    // A self-closing frame the model gives options:
                    // open it around them.
                    let ff = attr_value(&e, b"Self").and_then(|id| fits.get(id.as_str()).copied());
                    if let Some(ff) = ff {
                        writer.write_event(Event::Start(e.into_owned()))?;
                        write_new(&mut writer, ff, None)?;
                        writer.write_event(Event::End(BytesEnd::new("Rectangle")))?;
                        buf.clear();
                        continue;
                    }
                }
                writer.write_event(Event::Empty(e.into_owned()))?;
            }
            Event::End(e) => {
                if hosts.last().is_some_and(|(d, _, _, _)| *d == depth) {
                    let (_, ff, s, written) = hosts.pop().expect("checked");
                    if let (Some(ff), false) = (ff, written) {
                        write_new(&mut writer, ff, s)?;
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
