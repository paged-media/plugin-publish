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

//! `<Guide>` save-back — ruler guides inserted, moved or deleted by the
//! model (`InsertGuide` / `MoveGuide` / `DeleteGuide`).
//!
//! A 134-page book authored through the engine carried one runtime
//! guide; its export carried none, because neither the spread rewrite
//! nor the minted-spread emitter had a `<Guide>` lane. This is that
//! lane, as its own streaming pass over a spread part so the (already
//! dense) page-item rewrite stays untouched: it runs BEFORE
//! `rewrite_spread`, which then streams the guide-patched bytes and
//! derives its provenance from those same bytes.
//!
//! # Identity
//!
//! A [`RulerGuide`] has no `Self`; the model addresses guides by
//! POSITION within the spread (`Guide/<spread>/<n>`), and that is the
//! matching rule here too: the n-th source `<Guide>` the PARSER accepts
//! (Orientation + Location present and parseable — the rule is mirrored
//! from `idml_import::parse_spread`) patches against the n-th model
//! guide; source guides beyond the model's count are DROPPED (deleted
//! guides), model guides beyond the source's count are NEW. A source
//! `<Guide>` the parser would not accept passes through verbatim and is
//! not counted, so it can neither be moved nor deleted by accident.
//!
//! # Placement of new guides
//!
//! InDesign writes a page's ruler guides as children of that `<Page>`
//! (measured on real exports: `<Page …><Properties>…</Properties><Guide
//! …>…</Guide></Page>`), so a new guide goes inside the page its
//! `page_index` names (clamped to the last page); a spread with no
//! pages takes them just before `</Spread>`. A self-closing `<Page/>`
//! that gains guides is expanded into an open element.
//!
//! # Spelling
//!
//! Mirrors an InDesign 2025 export: `Self OverriddenPageItemProps=""
//! Orientation Location FitToPage="true" ViewThreshold="5"
//! Locked="false" ItemLayer PageIndex GuideType="Ruler" GuideZone="1"`
//! with `<Properties><GuideColor type="enumeration">Cyan</GuideColor>
//! </Properties>`. `ItemLayer` is the document's first layer when the
//! caller knows one, and omitted otherwise (InDesign defaults it) — an
//! unmeasured corner, flagged in the work report.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use idml_import::{GuideOrientation, RulerGuide};

use crate::rewrite::{
    attr_value, emit_start_with_attrs, format_f32, patch_start, preserving_f32_patch, Patch,
};

/// Whether the parser would model this `<Guide>` (see the module doc).
fn is_modelled_guide(e: &BytesStart) -> bool {
    let orientation_ok = matches!(
        attr_value(e, b"Orientation").as_deref(),
        Some("Vertical") | Some("Horizontal")
    );
    let location_ok = attr_value(e, b"Location")
        .and_then(|s| s.parse::<f32>().ok())
        .is_some();
    orientation_ok && location_ok
}

fn orientation_idml(o: GuideOrientation) -> &'static str {
    match o {
        GuideOrientation::Vertical => "Vertical",
        GuideOrientation::Horizontal => "Horizontal",
    }
}

/// Patch a source `<Guide>` start tag against its model guide, keeping
/// the on-disk bytes of every attribute that already agrees — and the
/// whole original tag (its `/>` spacing included) when nothing changed.
fn patch_guide(e: &BytesStart, g: &RulerGuide) -> Result<BytesStart<'static>, quick_xml::Error> {
    let rebuilt = patch_guide_rebuilt(e, g)?;
    Ok(
        if e.as_ref().trim_ascii_end() == rebuilt.as_ref().trim_ascii_end() {
            e.clone().into_owned()
        } else {
            rebuilt
        },
    )
}

fn patch_guide_rebuilt(
    e: &BytesStart,
    g: &RulerGuide,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let extras: Vec<(&str, String)> = vec![
        ("Orientation", orientation_idml(g.orientation).to_string()),
        ("Location", format_f32(g.location)),
        ("PageIndex", g.page_index.to_string()),
    ];
    patch_start(
        e,
        |k, raw| {
            let raw = std::str::from_utf8(raw).ok();
            match k {
                b"Orientation" => Some(if raw == Some(orientation_idml(g.orientation)) {
                    Patch::Keep
                } else {
                    Patch::Set(orientation_idml(g.orientation).to_string())
                }),
                b"Location" => Some(preserving_f32_patch(raw, Some(g.location))),
                b"PageIndex" => Some(
                    if raw.and_then(|s| s.parse::<u32>().ok()) == Some(g.page_index) {
                        Patch::Keep
                    } else {
                        Patch::Set(g.page_index.to_string())
                    },
                ),
                _ => None,
            }
        },
        &extras,
    )
}

/// Emit one NEW `<Guide>` in InDesign's spelling. `self_id` must be
/// unique in the document; `layer` is the `ItemLayer` to bind, if known.
pub(crate) fn write_guide(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    g: &RulerGuide,
    self_id: &str,
    layer: Option<&str>,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![
        ("Self", self_id.to_string()),
        ("OverriddenPageItemProps", String::new()),
        ("Orientation", orientation_idml(g.orientation).to_string()),
        ("Location", format_f32(g.location)),
        ("FitToPage", "true".to_string()),
        ("ViewThreshold", "5".to_string()),
        ("Locked", "false".to_string()),
    ];
    if let Some(layer) = layer {
        attrs.push(("ItemLayer", layer.to_string()));
    }
    attrs.push(("PageIndex", g.page_index.to_string()));
    attrs.push(("GuideType", "Ruler".to_string()));
    attrs.push(("GuideZone", "1".to_string()));
    emit_start_with_attrs(writer, "Guide", &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    let mut color = BytesStart::new("GuideColor");
    color.push_attribute(("type", "enumeration"));
    writer.write_event(Event::Start(color))?;
    writer.write_event(Event::Text(BytesText::new("Cyan")))?;
    writer.write_event(Event::End(BytesEnd::new("GuideColor")))?;
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    writer.write_event(Event::End(BytesEnd::new("Guide")))?;
    Ok(())
}

/// The `Self` a new guide takes: the model's own guide address
/// (`Guide/<spread>/<n>`), which is unique per spread and never collides
/// with an InDesign `u…` id.
pub(crate) fn guide_self_id(spread_self: Option<&str>, index: usize) -> String {
    format!("Guide/{}/{}", spread_self.unwrap_or_default(), index)
}

/// Emit the new guides destined for page `page_index` (or, with
/// `page_index == None`, every new guide not yet placed).
fn write_new_guides_for(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    guides: &[RulerGuide],
    first_new: usize,
    placed: &mut [bool],
    page_index: Option<u32>,
    spread_self: Option<&str>,
    layer: Option<&str>,
) -> Result<(), quick_xml::Error> {
    for (i, g) in guides.iter().enumerate().skip(first_new) {
        if placed[i] {
            continue;
        }
        if let Some(target) = page_index {
            if g.page_index != target {
                continue;
            }
        }
        write_guide(writer, g, &guide_self_id(spread_self, i), layer)?;
        placed[i] = true;
    }
    Ok(())
}

/// Bring a spread part's `<Guide>` elements in line with `guides` (see
/// the module doc). Byte-identical to `original` when the source guides
/// already match the model.
pub fn rewrite_guides(
    original: &[u8],
    guides: &[RulerGuide],
    spread_self: Option<&str>,
    layer: Option<&str>,
) -> Result<Vec<u8>, quick_xml::Error> {
    // Pre-pass: how many source guides the parser models, and how many
    // pages there are — so new guides can be routed to their page in the
    // single streaming pass below.
    let (source_modelled, page_count) = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut guides_n = 0usize;
        let mut pages_n = 0usize;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(ref e) | Event::Empty(ref e) => match e.name().as_ref() {
                    b"Guide" if is_modelled_guide(e) => guides_n += 1,
                    b"Page" => pages_n += 1,
                    _ => {}
                },
                _ => {}
            }
            buf.clear();
        }
        (guides_n, pages_n)
    };
    let first_new = source_modelled.min(guides.len());
    if source_modelled == guides.len() && guides.is_empty() {
        // Nothing to patch, drop or add.
        return Ok(original.to_vec());
    }
    // The page a new guide lands in: its own index, clamped to the last
    // page. Resolved up front so the streaming pass can compare.
    let target_page = |g: &RulerGuide| -> Option<u32> {
        (page_count > 0).then(|| (g.page_index as usize).min(page_count - 1) as u32)
    };
    let mut placed = vec![false; guides.len()];

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    let mut seen = 0usize;
    let mut depth = 0usize;
    // Depth of a `<Guide>` subtree being dropped; events vanish until
    // its End at this depth.
    let mut skip_depth: Option<usize> = None;
    // The open `<Page>` (its depth and zero-based index) — new guides
    // for it go in before its End.
    let mut open_page: Option<(usize, u32)> = None;
    let mut pages_seen: u32 = 0;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                if skip_depth.is_some() {
                    buf.clear();
                    continue;
                }
                match e.name().as_ref() {
                    b"Guide" if is_modelled_guide(&e) => {
                        let i = seen;
                        seen += 1;
                        if let Some(g) = guides.get(i) {
                            placed[i] = true;
                            writer.write_event(Event::Start(patch_guide(&e, g)?))?;
                        } else {
                            skip_depth = Some(depth);
                        }
                    }
                    b"Page" => {
                        let idx = pages_seen;
                        pages_seen += 1;
                        open_page = Some((depth, idx));
                        writer.write_event(Event::Start(e.into_owned()))?;
                    }
                    _ => writer.write_event(Event::Start(e.into_owned()))?,
                }
            }
            Event::Empty(e) => {
                if skip_depth.is_some() {
                    buf.clear();
                    continue;
                }
                match e.name().as_ref() {
                    b"Guide" if is_modelled_guide(&e) => {
                        let i = seen;
                        seen += 1;
                        if let Some(g) = guides.get(i) {
                            placed[i] = true;
                            writer.write_event(Event::Empty(patch_guide(&e, g)?))?;
                        }
                        // else: a deleted guide — dropped.
                    }
                    b"Page" => {
                        let idx = pages_seen;
                        pages_seen += 1;
                        let wants = guides
                            .iter()
                            .enumerate()
                            .skip(first_new)
                            .any(|(i, g)| !placed[i] && target_page(g) == Some(idx));
                        if wants {
                            // Expand the self-closing page around its
                            // new guides.
                            writer.write_event(Event::Start(e.borrow()))?;
                            let mut sub = guides.to_vec();
                            for g in sub.iter_mut() {
                                if target_page(g) == Some(idx) {
                                    g.page_index = idx;
                                }
                            }
                            write_new_guides_for(
                                &mut writer,
                                &sub,
                                first_new,
                                &mut placed,
                                Some(idx),
                                spread_self,
                                layer,
                            )?;
                            writer.write_event(Event::End(BytesEnd::new("Page")))?;
                        } else {
                            writer.write_event(Event::Empty(e.into_owned()))?;
                        }
                    }
                    _ => writer.write_event(Event::Empty(e.into_owned()))?,
                }
            }
            Event::End(e) => {
                if let Some(d) = skip_depth {
                    if d == depth {
                        skip_depth = None;
                    }
                    depth = depth.saturating_sub(1);
                    buf.clear();
                    continue;
                }
                match e.name().as_ref() {
                    b"Page" if open_page.map(|(d, _)| d) == Some(depth) => {
                        let (_, idx) = open_page.take().expect("checked");
                        let mut sub = guides.to_vec();
                        for g in sub.iter_mut() {
                            if target_page(g) == Some(idx) {
                                g.page_index = idx;
                            }
                        }
                        write_new_guides_for(
                            &mut writer,
                            &sub,
                            first_new,
                            &mut placed,
                            Some(idx),
                            spread_self,
                            layer,
                        )?;
                        writer.write_event(Event::End(e))?;
                    }
                    b"Spread" | b"MasterSpread" => {
                        // Whatever found no page (a spread without pages)
                        // lands at spread level.
                        write_new_guides_for(
                            &mut writer,
                            guides,
                            first_new,
                            &mut placed,
                            None,
                            spread_self,
                            layer,
                        )?;
                        writer.write_event(Event::End(e))?;
                    }
                    _ => writer.write_event(Event::End(e))?,
                }
                depth = depth.saturating_sub(1);
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}
