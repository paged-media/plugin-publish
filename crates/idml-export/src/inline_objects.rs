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

//! The body of a minted paragraph that holds something besides text at
//! a character: anchored objects, or a table that does not simply follow
//! all of the paragraph's text.
//!
//! An anchored object is content of its paragraph — InDesign writes the
//! object's element among its range's `<Content>`s, at its character
//! (`…<Content>just here</Content><Rectangle …>…</Rectangle><Content>
//! over the wire</Content>…`), and the reader records the offset it finds
//! it at (`Paragraph::anchored_frame_offsets`). The minted-story writer
//! wrote no object at all: every anchored frame of a story the engine
//! made was lost on export, with the gap or the picture it set.
//!
//! A table stands at a character too (`Paragraph::table_anchor`). A
//! paragraph whose table carries no anchor is the engine's older reading
//! — "its text is a paragraph of its own and the table follows it" — and
//! is written that way, with the paragraph mark between them; the writer
//! used to set the table straight after the text as a character of the
//! SAME paragraph, which closes no paragraph and so dropped the space
//! after the text.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Writer;

use idml_import::{
    AnchoredFrame, AnchoredFrameKind, AnchoredObjectSetting, CharacterRun, Paragraph,
};

use crate::emit::{
    character_run_attrs, emit_applied_font, page_number_pieces, sanitize_id, write_run_body,
    write_table, NO_CHARACTER_STYLE,
};
use crate::rewrite::{emit_empty_with_attrs, emit_start_with_attrs, format_f32, write_run_content};

type W = Writer<Cursor<Vec<u8>>>;

fn text_len(p: &Paragraph) -> usize {
    p.runs.iter().map(|r| r.text.chars().count()).sum()
}

/// Whether `p`'s body is [`write_body`]'s to write: it anchors objects,
/// or its table does not stand after all of its text.
pub(crate) fn needs(p: &Paragraph) -> bool {
    if !p.anchored_frames.is_empty() {
        return true;
    }
    if p.table.is_none() {
        return false;
    }
    let len = text_len(p);
    len > 0
        && match p.table_anchor {
            None => true,
            Some(a) => (a as usize) < len,
        }
}

/// Whether a story part the SOURCE carries must be written fresh from
/// the model (`rewrite::fresh_story_part`) rather than patched, because
/// the patch lane cannot add what the model holds at a character:
///
/// * a story-level paragraph whose anchored objects (by `Self`, in order)
///   are not the ones its source range carries — an object the model
///   gained since the part was written (the patch lane passes objects
///   through where they stand, and adds none), or one it lost;
/// * a paragraph whose table has no anchor after text (see the module
///   docs): its spelling puts a paragraph mark in the source the model's
///   text does not hold, which the patch lane would take back out.
///
/// A minted story's part is the engine's own earlier export, so writing
/// it fresh loses nothing. A story with footnotes keeps the patch lane
/// (the fresh writer does not serialise footnotes yet).
pub(crate) fn source_cannot_hold(original: &[u8], story: &idml_import::Story) -> bool {
    if story.paragraphs.iter().any(|p| !p.footnotes.is_empty()) {
        return false;
    }
    if story
        .paragraphs
        .iter()
        .any(|p| p.table.is_some() && p.table_anchor.is_none() && text_len(p) > 0)
    {
        return true;
    }
    if story
        .paragraphs
        .iter()
        .all(|p| p.anchored_frames.is_empty())
    {
        // Nothing the model anchors: whatever the source anchors passes
        // through as before.
        return false;
    }
    let Ok(parsed) = idml_import::parse_story(original) else {
        return false;
    };
    let ids = |p: &Paragraph| -> Vec<Option<String>> {
        p.anchored_frames
            .iter()
            .map(|f| f.self_id.clone())
            .collect()
    };
    story.paragraphs.iter().enumerate().any(|(i, m)| {
        parsed
            .paragraphs
            .get(i)
            .map_or(!m.anchored_frames.is_empty(), |s| ids(m) != ids(s))
    })
}

/// What stands at a character of the paragraph.
enum Inline<'a> {
    Frame(&'a AnchoredFrame),
    Table,
}

/// Write the body of paragraph `p` — everything inside its
/// `<ParagraphStyleRange>` after its `<Properties>`: its runs, cut at
/// each object's and the table's character, the objects and the table
/// between the pieces, and the paragraph mark at the end when `mark`.
pub(crate) fn write_body(
    writer: &mut W,
    p: &Paragraph,
    mark: bool,
    host_width: Option<f32>,
) -> Result<(), quick_xml::Error> {
    let len = text_len(p);
    // `(offset, inline)`, in text order; a frame with no recorded offset
    // anchors at the paragraph's start, one past the text at its end.
    let mut inlines: Vec<(usize, Inline)> = p
        .anchored_frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let at = p.anchored_frame_offsets.get(i).copied().unwrap_or(0) as usize;
            (at.min(len), Inline::Frame(f))
        })
        .collect();
    // A table at the end of the text is written after the runs, as the
    // writer always has; one with NO anchor follows the text as a
    // paragraph of its own, so the text's mark comes first (see the
    // module docs). Only a table inside the text cuts a run.
    let table_after_mark = p.table.is_some() && p.table_anchor.is_none() && len > 0;
    let table_at_end = p.table.is_some()
        && (len == 0
            || match p.table_anchor {
                None => true,
                Some(a) => a as usize >= len,
            });
    if let (Some(_), Some(a)) = (&p.table, p.table_anchor) {
        if (a as usize) < len {
            inlines.push((a as usize, Inline::Table));
        }
    }
    // Stable: objects keep their model order at one character; a table
    // at the same character as an object follows it.
    inlines.sort_by_key(|(at, _)| *at);

    let mut next = 0usize;
    let mut pos = 0usize;
    let runs = flavour_pieces(p);
    let last_run = runs.len().saturating_sub(1);
    for (ri, (r, flavour)) in runs.iter().enumerate() {
        let chars: Vec<char> = r.text.chars().collect();
        let end = pos + chars.len();
        let is_last = ri == last_run;
        // The inlines this run holds: those before its end, and on the
        // last run those at the very end of the text too.
        let mut here: Vec<(usize, &Inline)> = Vec::new();
        while next < inlines.len() && (inlines[next].0 < end || (is_last && inlines[next].0 <= end))
        {
            here.push((inlines[next].0 - pos, &inlines[next].1));
            next += 1;
        }
        let mut attrs = character_run_attrs(r);
        if let Some(flavour) = flavour {
            attrs.push(("PageNumberType", flavour.to_string()));
        }
        let range = Range {
            attrs,
            run: r,
            source: r.hyperlink_source.as_deref(),
        };
        range.open(writer)?;
        if here.is_empty() || r.text_variable.is_some() {
            // Nothing to cut, or a run that cannot be cut (a text
            // variable is one character): its own body, its inlines after.
            write_run_body(writer, r, true)?;
            for (_, inline) in &here {
                write_inline(writer, inline, &range, p, host_width, false)?;
            }
        } else {
            range.open_source(writer)?;
            let mut cut = 0usize;
            for (local, inline) in &here {
                if *local > cut {
                    write_run_content(writer, &chars[cut..*local].iter().collect::<String>())?;
                }
                cut = *local;
                write_inline(writer, inline, &range, p, host_width, true)?;
            }
            if cut < chars.len() {
                write_run_content(writer, &chars[cut..].iter().collect::<String>())?;
            }
            range.close_source(writer)?;
        }
        if is_last && (table_after_mark || (mark && !table_at_end)) {
            // The text's own mark: the paragraph's, or the one that ends
            // the text before a table that follows it as a paragraph.
            writer.write_event(Event::Empty(BytesStart::new("Br")))?;
        }
        writer.write_event(Event::End(BytesEnd::new("CharacterStyleRange")))?;
        pos = end;
    }
    if p.runs.is_empty() {
        // No text: the objects in a range of their own.
        emit_start_with_attrs(writer, "CharacterStyleRange", &[])?;
        for (_, inline) in &inlines {
            if let Inline::Frame(f) = inline {
                write_anchored_frame(writer, f)?;
            }
        }
        if mark && !table_at_end {
            writer.write_event(Event::Empty(BytesStart::new("Br")))?;
        }
        writer.write_event(Event::End(BytesEnd::new("CharacterStyleRange")))?;
    }
    if table_at_end {
        write_table_range(writer, p, host_width, mark)?;
    }
    Ok(())
}

/// `p`'s runs cut where InDesign cuts ranges at a next or previous page
/// number (`emit::page_number_pieces`), each piece with the flavour its
/// range names — the same cut the plain paragraph writer makes
/// (`emit::write_run_ranges`), so a paragraph that also holds an object
/// keeps its page numbers' flavours. A run's hyperlink source and
/// placeholder stay on its first piece; a text variable is never cut.
fn flavour_pieces(p: &Paragraph) -> Vec<(CharacterRun, Option<&'static str>)> {
    let mut out = Vec::with_capacity(p.runs.len());
    for r in &p.runs {
        let pieces = if r.text_variable.is_some() {
            Vec::new()
        } else {
            page_number_pieces(&r.text)
        };
        if pieces.iter().all(|(kind, _)| kind.is_none()) {
            out.push((r.clone(), None));
            continue;
        }
        for (i, (kind, text)) in pieces.into_iter().enumerate() {
            let mut piece = r.clone();
            piece.text = text.to_string();
            if i > 0 {
                piece.hyperlink_source = None;
                piece.placeholder = None;
            }
            out.push((piece, kind));
        }
    }
    out
}

/// The `<CharacterStyleRange>` a run is written in — reopened after a
/// table cuts it.
struct Range<'a> {
    attrs: Vec<(&'static str, String)>,
    run: &'a CharacterRun,
    source: Option<&'a str>,
}

impl Range<'_> {
    fn open(&self, writer: &mut W) -> Result<(), quick_xml::Error> {
        emit_start_with_attrs(writer, "CharacterStyleRange", &self.attrs)?;
        emit_applied_font(writer, &self.run.font)
    }

    /// The run's hyperlink source, around its pieces and the objects
    /// among them (one source, one `Self`).
    fn open_source(&self, writer: &mut W) -> Result<(), quick_xml::Error> {
        if let Some(source) = self.source {
            emit_start_with_attrs(
                writer,
                "HyperlinkTextSource",
                &[
                    ("Self", source.to_string()),
                    (
                        "Name",
                        source.rsplit('/').next().unwrap_or(source).to_string(),
                    ),
                    ("Hidden", "false".to_string()),
                ],
            )?;
        }
        Ok(())
    }

    fn close_source(&self, writer: &mut W) -> Result<(), quick_xml::Error> {
        if self.source.is_some() {
            writer.write_event(Event::End(BytesEnd::new("HyperlinkTextSource")))?;
        }
        Ok(())
    }
}

/// One inline inside a run's range. A table is a range of its own: the
/// run's range (and its source, when `in_source`) closes around it and
/// reopens for the rest of the run.
fn write_inline(
    writer: &mut W,
    inline: &Inline,
    range: &Range,
    p: &Paragraph,
    host_width: Option<f32>,
    in_source: bool,
) -> Result<(), quick_xml::Error> {
    match inline {
        Inline::Frame(f) => write_anchored_frame(writer, f),
        Inline::Table => {
            if in_source {
                range.close_source(writer)?;
            }
            writer.write_event(Event::End(BytesEnd::new("CharacterStyleRange")))?;
            write_table_range(writer, p, host_width, false)?;
            range.open(writer)?;
            if in_source {
                range.open_source(writer)?;
            }
            Ok(())
        }
    }
}

/// The paragraph's table in a range of its own, the paragraph mark after
/// it when `mark` — the spelling the minted-story writer always used.
fn write_table_range(
    writer: &mut W,
    p: &Paragraph,
    host_width: Option<f32>,
    mark: bool,
) -> Result<(), quick_xml::Error> {
    let Some(table) = &p.table else {
        return Ok(());
    };
    emit_start_with_attrs(
        writer,
        "CharacterStyleRange",
        &[("AppliedCharacterStyle", NO_CHARACTER_STYLE.to_string())],
    )?;
    write_table(writer, table, host_width)?;
    if mark {
        writer.write_event(Event::Empty(BytesStart::new("Br")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("CharacterStyleRange")))?;
    Ok(())
}

/// One anchored object in InDesign's spelling: the frame element with
/// its `ItemTransform`, its box as `<PathGeometry>` (in its own
/// coordinates), its `<AnchoredObjectSetting>`, and — a picture frame —
/// its `<Image>` with the `<Link>` the model names; a group's members
/// inside it. What the model leaves unset is not written, so the
/// object's style decides it on both sides.
pub(crate) fn write_anchored_frame(
    writer: &mut W,
    f: &AnchoredFrame,
) -> Result<(), quick_xml::Error> {
    let name = match f.frame_kind {
        AnchoredFrameKind::TextFrame => "TextFrame",
        AnchoredFrameKind::Rectangle => "Rectangle",
        AnchoredFrameKind::Group => "Group",
    };
    let mut attrs: Vec<(&str, String)> = Vec::new();
    if let Some(id) = &f.self_id {
        attrs.push(("Self", id.clone()));
    }
    if matches!(f.frame_kind, AnchoredFrameKind::TextFrame) {
        if let Some(story) = &f.parent_story {
            attrs.push(("ParentStory", sanitize_id(story)));
        }
        attrs.push(("ContentType", "TextType".to_string()));
    } else if f.image_link.is_some() {
        attrs.push(("ContentType", "GraphicType".to_string()));
    }
    if let Some(s) = &f.applied_object_style {
        attrs.push(("AppliedObjectStyle", s.clone()));
    }
    attrs.push((
        "ItemTransform",
        crate::rewrite::format_matrix(&f.item_transform.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])),
    ));
    if let Some(c) = &f.fill_color {
        attrs.push(("FillColor", c.clone()));
    }
    if let Some(t) = f.fill_tint {
        attrs.push(("FillTint", format_f32(t)));
    }
    if let Some(c) = &f.stroke_color {
        attrs.push(("StrokeColor", c.clone()));
    }
    if let Some(w) = f.stroke_weight {
        attrs.push(("StrokeWeight", format_f32(w)));
    }
    if let Some(a) = f.gradient_fill_angle {
        attrs.push(("GradientFillAngle", format_f32(a)));
    }
    emit_start_with_attrs(writer, name, &attrs)?;
    if let (Some(b), false) = (f.bounds, matches!(f.frame_kind, AnchoredFrameKind::Group)) {
        writer.write_event(Event::Start(BytesStart::new("Properties")))?;
        write_box_geometry(writer, b)?;
        writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    }
    if let Some(s) = &f.setting {
        write_anchored_object_setting(writer, s)?;
    }
    if let (Some(uri), AnchoredFrameKind::Rectangle) = (&f.image_link, f.frame_kind) {
        let id = f.self_id.as_deref().unwrap_or("anchored");
        let mut image: Vec<(&str, String)> = vec![("Self", format!("{id}Image"))];
        if let Some(t) = f.image_item_transform {
            image.push(("ItemTransform", crate::images::format_image_transform(&t)));
        }
        emit_start_with_attrs(writer, "Image", &image)?;
        emit_empty_with_attrs(
            writer,
            "Link",
            &[
                ("Self", format!("{id}Link")),
                ("LinkResourceURI", uri.clone()),
                ("StoredState", "Normal".to_string()),
            ],
        )?;
        writer.write_event(Event::End(BytesEnd::new("Image")))?;
    }
    for child in &f.children {
        write_anchored_frame(writer, child)?;
    }
    writer.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

/// A closed four-corner `<PathGeometry>` for `b` (top-left, bottom-left,
/// bottom-right, top-right — the order InDesign and the page-item writer
/// use); the reader takes the object's box back off the anchors.
fn write_box_geometry(writer: &mut W, b: idml_import::Bounds) -> Result<(), quick_xml::Error> {
    writer.write_event(Event::Start(BytesStart::new("PathGeometry")))?;
    emit_start_with_attrs(
        writer,
        "GeometryPathType",
        &[("PathOpen", "false".to_string())],
    )?;
    writer.write_event(Event::Start(BytesStart::new("PathPointArray")))?;
    for (x, y) in [
        (b.left, b.top),
        (b.left, b.bottom),
        (b.right, b.bottom),
        (b.right, b.top),
    ] {
        let p = format!("{} {}", format_f32(x), format_f32(y));
        emit_empty_with_attrs(
            writer,
            "PathPointType",
            &[
                ("Anchor", p.clone()),
                ("LeftDirection", p.clone()),
                ("RightDirection", p),
            ],
        )?;
    }
    writer.write_event(Event::End(BytesEnd::new("PathPointArray")))?;
    writer.write_event(Event::End(BytesEnd::new("GeometryPathType")))?;
    writer.write_event(Event::End(BytesEnd::new("PathGeometry")))?;
    Ok(())
}

/// `<AnchoredObjectSetting>` in InDesign's attribute order. The four
/// plain values are written when the setting states them (a setting made
/// in paged states all of them — `stated: None`), the rest when set.
fn write_anchored_object_setting(
    writer: &mut W,
    s: &AnchoredObjectSetting,
) -> Result<(), quick_xml::Error> {
    let stated = s.stated.unwrap_or(paged_model::AnchoredStated {
        spine_relative: true,
        anchor_x_offset: true,
        anchor_y_offset: true,
        lock_position: true,
    });
    let mut a: Vec<(&str, String)> = Vec::new();
    if let Some(v) = &s.anchored_position {
        a.push(("AnchoredPosition", v.clone()));
    }
    if stated.spine_relative {
        a.push(("SpineRelative", s.spine_relative.to_string()));
    }
    if stated.lock_position {
        a.push(("LockPosition", s.lock_position.to_string()));
    }
    for (k, v) in [
        ("AnchorPoint", &s.anchor_point),
        ("HorizontalAlignment", &s.horizontal_alignment),
        ("HorizontalReferencePoint", &s.horizontal_reference_point),
        ("VerticalAlignment", &s.vertical_alignment),
        ("VerticalReferencePoint", &s.vertical_reference_point),
    ] {
        if let Some(v) = v {
            a.push((k, v.clone()));
        }
    }
    if stated.anchor_x_offset {
        a.push(("AnchorXoffset", format_f32(s.anchor_x_offset)));
    }
    if stated.anchor_y_offset {
        a.push(("AnchorYoffset", format_f32(s.anchor_y_offset)));
    }
    emit_empty_with_attrs(writer, "AnchoredObjectSetting", &a)
}
