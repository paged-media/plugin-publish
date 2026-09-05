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

//! New-entry emission (C-8) — full Story / Spread parts for model
//! objects MINTED after parse, which have no original ZIP entry the
//! carry-through writer could patch:
//!
//! * a story minted by `InsertNode { NodeSpec::TextFrame { parent_story:
//!   Some(_) } }` (the wire's InsertTextFrame) carries `src: ""` — text
//!   poured into a fresh frame was silently dropped on export;
//! * a spread minted by `InsertPage` carries a fresh
//!   `Spreads/Spread_<id>.xml` src the source archive doesn't contain.
//!
//! Both are serialised here from the in-memory model, in the same
//! vocabulary [`crate::rewrite`] writes when patching (its emit helpers
//! are reused directly), wrapped in the standard `idPkg` part envelope
//! our own parser reads back (`idml_import::Story::parse` /
//! `Spread::parse`). The new entries are then REFERENCED by a minimal
//! `designmap.xml` insertion ([`patch_designmap`]) — an unmutated
//! document never reaches this module, so its designmap round-trips
//! byte-identically.

use std::io::Cursor;

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{CharacterRun, Spread, Story};

use crate::rewrite;

const PKG_NS: &str = "http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging";
const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Sanitize a model `Self` id into an entry-name stem: `/` → `_`.
/// Wire-minted stories are named `Story/u<n>`; an entry path can't
/// carry the slash, and `paged_scene::derive_story_id` re-derives the
/// story id from the entry stem on reopen — so the sanitized form is
/// the id the document carries after a save→reopen round-trip (the
/// frame's `ParentStory` is written sanitized too, see
/// `rewrite::write_new_text_frame`).
pub(crate) fn sanitize_id(id: &str) -> String {
    id.replace('/', "_")
}

/// Entry path for a minted story: `Stories/Story_<sanitized-id>.xml`.
/// `derive_story_id` strips exactly the `Story_` prefix added here, so
/// the reopened story's `self_id` equals [`sanitize_id`] of the minted
/// id.
pub(crate) fn story_src_for(self_id: &str) -> String {
    format!("Stories/Story_{}.xml", sanitize_id(self_id))
}

fn new_part_writer() -> Result<Writer<Cursor<Vec<u8>>>, quick_xml::Error> {
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    writer.write_event(Event::Decl(BytesDecl::new(
        "1.0",
        Some("UTF-8"),
        Some("yes"),
    )))?;
    Ok(writer)
}

fn open_pkg_root(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    name: &str,
    dom_version: &str,
) -> Result<(), quick_xml::Error> {
    let mut root = BytesStart::new(name);
    root.push_attribute(("xmlns:idPkg", PKG_NS));
    root.push_attribute(("DOMVersion", dom_version));
    writer.write_event(Event::Start(root))?;
    Ok(())
}

/// Serialise a full `Stories/Story_*.xml` part from the in-memory model.
///
/// The body vocabulary is exactly what `rewrite::rewrite_story` owns
/// when patching: one `<ParagraphStyleRange>` per model paragraph
/// (`AppliedParagraphStyle` when set), one `<CharacterStyleRange>` per
/// run carrying the patchable character attributes, run text split
/// across `<Content>` / `<Br/>` / `<Tab/>` via
/// `rewrite::write_run_content`. Attributes the model doesn't set are
/// omitted, so a save→reopen reproduces the model (`None` stays `None`).
///
/// On top of the text-pour lane this now serialises what InDesign 2025
/// was measured to lose from a minted story:
///
/// * a run tagged `hyperlink_source` is written in InDesign's own
///   nesting — the `<HyperlinkTextSource>` INSIDE the range, around its
///   `<Content>` (a source wrapping the range is what the engine's
///   fixtures wrote; both read back, this is what InDesign writes);
/// * `anchors` — the text destinations (`HyperlinkDestinationKind::
///   TextAnchor`) targeting this story — are written as inline
///   `<HyperlinkTextDestination/>` markers in a range of their own at the
///   head of the first paragraph (measured: the designmap spelling binds
///   nothing, the inline marker binds);
/// * a paragraph's `table` is written in full (`write_table`): rows,
///   columns, cells with their paragraphs, header / footer counts,
///   spans, applied styles, insets, edge strokes — the vocabulary
///   `idml_import::parse_story` reads and `rewrite_story` patches.
///
/// Footnotes inside a minted story are still not serialised (no
/// consumer mints them).
pub(crate) fn story_part(
    self_id: &str,
    story: &Story,
    dom_version: &str,
    anchors: &[(String, Option<String>)],
    host_width: Option<f32>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let mut writer = new_part_writer()?;
    open_pkg_root(&mut writer, "idPkg:Story", dom_version)?;
    let mut s = BytesStart::new("Story");
    s.push_attribute(("Self", self_id));
    writer.write_event(Event::Start(s))?;
    if story.paragraphs.is_empty() && !anchors.is_empty() {
        // An anchor still needs a paragraph to sit in.
        rewrite::emit_start_with_attrs(&mut writer, "ParagraphStyleRange", &[])?;
        write_anchor_markers(&mut writer, anchors)?;
        writer.write_event(Event::End(BytesEnd::new("ParagraphStyleRange")))?;
    }
    write_paragraphs(&mut writer, &story.paragraphs, anchors, host_width)?;
    writer.write_event(Event::End(BytesEnd::new("Story")))?;
    writer.write_event(Event::End(BytesEnd::new("idPkg:Story")))?;
    Ok(writer.into_inner().into_inner())
}

pub(crate) const NO_CHARACTER_STYLE: &str = "CharacterStyle/$ID/[No character style]";

/// The inline text-destination markers, each in a range of its own.
fn write_anchor_markers(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    anchors: &[(String, Option<String>)],
) -> Result<(), quick_xml::Error> {
    for (id, name) in anchors {
        rewrite::emit_start_with_attrs(
            writer,
            "CharacterStyleRange",
            &[("AppliedCharacterStyle", NO_CHARACTER_STYLE.to_string())],
        )?;
        rewrite::emit_empty_with_attrs(
            writer,
            "HyperlinkTextDestination",
            &[
                ("Self", id.clone()),
                (
                    "Name",
                    name.clone()
                        .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string()),
                ),
                ("Hidden", "false".to_string()),
            ],
        )?;
        writer.write_event(Event::End(BytesEnd::new("CharacterStyleRange")))?;
    }
    Ok(())
}

/// One `<ParagraphStyleRange>` per paragraph — the story body and,
/// recursively, every table cell's body. `anchors` go at the head of the
/// FIRST paragraph only.
pub(crate) fn write_paragraphs(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    paragraphs: &[idml_import::Paragraph],
    anchors: &[(String, Option<String>)],
    host_width: Option<f32>,
) -> Result<(), quick_xml::Error> {
    let last_para = paragraphs.len().saturating_sub(1);
    for (pi, p) in paragraphs.iter().enumerate() {
        let mut attrs: Vec<(&str, String)> = Vec::new();
        if let Some(style) = &p.paragraph_style {
            attrs.push(("AppliedParagraphStyle", style.clone()));
        }
        rewrite::emit_start_with_attrs(writer, "ParagraphStyleRange", &attrs)?;
        if pi == 0 {
            write_anchor_markers(writer, anchors)?;
        }
        // The paragraph MARK. In IDML a `<ParagraphStyleRange>` is a
        // style run over one or more paragraphs, and what separates
        // paragraphs is the `<Br/>` character — an InDesign-authored
        // file ends every paragraph's content with one and omits it only
        // on the story's last. We wrote none, because in our model the
        // break is structural rather than a character in any run, and
        // our own reader rebuilds paragraphs from the range boundaries.
        // That private convention agreed with itself and with nobody
        // else: opened in InDesign, an eleven-entry table of contents
        // came back as ONE paragraph with its style dropped.
        let mark = pi < last_para;
        // A table takes the mark (it is the paragraph's last child);
        // otherwise the last run does.
        let mark_on_runs = mark && p.table.is_none();
        let last_run = p.runs.len().saturating_sub(1);
        for (ri, r) in p.runs.iter().enumerate() {
            rewrite::emit_start_with_attrs(writer, "CharacterStyleRange", &character_run_attrs(r))?;
            emit_applied_font(writer, &r.font)?;
            if let Some(source) = &r.hyperlink_source {
                rewrite::emit_start_with_attrs(
                    writer,
                    "HyperlinkTextSource",
                    &[
                        ("Self", source.clone()),
                        (
                            "Name",
                            source.rsplit('/').next().unwrap_or(source).to_string(),
                        ),
                        ("Hidden", "false".to_string()),
                    ],
                )?;
                rewrite::write_run_content(writer, &r.text)?;
                writer.write_event(Event::End(BytesEnd::new("HyperlinkTextSource")))?;
            } else {
                rewrite::write_run_content(writer, &r.text)?;
            }
            if mark_on_runs && ri == last_run {
                writer.write_event(Event::Empty(BytesStart::new("Br")))?;
            }
            writer.write_event(Event::End(BytesEnd::new("CharacterStyleRange")))?;
        }
        if let Some(table) = &p.table {
            rewrite::emit_start_with_attrs(
                writer,
                "CharacterStyleRange",
                &[("AppliedCharacterStyle", NO_CHARACTER_STYLE.to_string())],
            )?;
            write_table(writer, table, host_width)?;
            if mark {
                writer.write_event(Event::Empty(BytesStart::new("Br")))?;
            }
            writer.write_event(Event::End(BytesEnd::new("CharacterStyleRange")))?;
        } else if mark && p.runs.is_empty() {
            // An empty paragraph still ends somewhere: give the mark a
            // character run to live in, or the break disappears and the
            // paragraph with it.
            rewrite::emit_start_with_attrs(writer, "CharacterStyleRange", &[])?;
            writer.write_event(Event::Empty(BytesStart::new("Br")))?;
            writer.write_event(Event::End(BytesEnd::new("CharacterStyleRange")))?;
        }
        writer.write_event(Event::End(BytesEnd::new("ParagraphStyleRange")))?;
    }
    Ok(())
}

fn push_opt_f32(out: &mut Vec<(&'static str, String)>, k: &'static str, v: Option<f32>) {
    if let Some(v) = v {
        out.push((k, rewrite::format_f32(v)));
    }
}

fn push_opt_str(out: &mut Vec<(&'static str, String)>, k: &'static str, v: &Option<String>) {
    if let Some(v) = v {
        out.push((k, v.clone()));
    }
}

fn push_opt_u32(out: &mut Vec<(&'static str, String)>, k: &'static str, v: Option<u32>) {
    if let Some(v) = v {
        out.push((k, v.to_string()));
    }
}

/// The table-level stroke bags (`TableBorder`, the row / column
/// `TableLineStrokes`) as attributes, only what is set.
fn table_stroke_attrs(t: &idml_import::Table) -> Vec<(&'static str, String)> {
    let mut a: Vec<(&'static str, String)> = Vec::new();
    let b = &t.border;
    push_opt_str(&mut a, "TopBorderStrokeColor", &b.top_color);
    push_opt_str(&mut a, "TopBorderStrokeType", &b.top_type);
    push_opt_f32(&mut a, "TopBorderStrokeWeight", b.top_weight);
    push_opt_f32(&mut a, "TopBorderStrokeTint", b.top_tint);
    push_opt_str(&mut a, "BottomBorderStrokeColor", &b.bottom_color);
    push_opt_str(&mut a, "BottomBorderStrokeType", &b.bottom_type);
    push_opt_f32(&mut a, "BottomBorderStrokeWeight", b.bottom_weight);
    push_opt_f32(&mut a, "BottomBorderStrokeTint", b.bottom_tint);
    push_opt_str(&mut a, "LeftBorderStrokeColor", &b.left_color);
    push_opt_str(&mut a, "LeftBorderStrokeType", &b.left_type);
    push_opt_f32(&mut a, "LeftBorderStrokeWeight", b.left_weight);
    push_opt_f32(&mut a, "LeftBorderStrokeTint", b.left_tint);
    push_opt_str(&mut a, "RightBorderStrokeColor", &b.right_color);
    push_opt_str(&mut a, "RightBorderStrokeType", &b.right_type);
    push_opt_f32(&mut a, "RightBorderStrokeWeight", b.right_weight);
    push_opt_f32(&mut a, "RightBorderStrokeTint", b.right_tint);
    let r = &t.row_strokes;
    push_opt_u32(&mut a, "StartRowStrokeCount", r.start_count);
    push_opt_str(&mut a, "StartRowStrokeColor", &r.start_color);
    push_opt_str(&mut a, "StartRowStrokeType", &r.start_type);
    push_opt_f32(&mut a, "StartRowStrokeWeight", r.start_weight);
    push_opt_f32(&mut a, "StartRowStrokeTint", r.start_tint);
    push_opt_u32(&mut a, "EndRowStrokeCount", r.end_count);
    push_opt_str(&mut a, "EndRowStrokeColor", &r.end_color);
    push_opt_str(&mut a, "EndRowStrokeType", &r.end_type);
    push_opt_f32(&mut a, "EndRowStrokeWeight", r.end_weight);
    push_opt_f32(&mut a, "EndRowStrokeTint", r.end_tint);
    let c = &t.column_strokes;
    push_opt_u32(&mut a, "StartColumnStrokeCount", c.start_count);
    push_opt_str(&mut a, "StartColumnStrokeColor", &c.start_color);
    push_opt_str(&mut a, "StartColumnStrokeType", &c.start_type);
    push_opt_f32(&mut a, "StartColumnStrokeWeight", c.start_weight);
    push_opt_f32(&mut a, "StartColumnStrokeTint", c.start_tint);
    push_opt_u32(&mut a, "EndColumnStrokeCount", c.end_count);
    push_opt_str(&mut a, "EndColumnStrokeColor", &c.end_color);
    push_opt_str(&mut a, "EndColumnStrokeType", &c.end_type);
    push_opt_f32(&mut a, "EndColumnStrokeWeight", c.end_weight);
    push_opt_f32(&mut a, "EndColumnStrokeTint", c.end_tint);
    a
}

/// The row height / column width written when the model has none
/// (a bare `insertTable` with no sizing op): InDesign 20.0.1 was
/// measured to DROP a table whose `<Row>`s carry no `SingleRowHeight`
/// or whose `<Column>`s no `SingleColumnWidth` — Adobe's own files
/// always spell both. Rows fall back to 24 pt; columns share the host
/// frame's inner (text-column) width when the caller knows it, else
/// 72 pt each. Not a loss: the model had no size, this is a size.
const FALLBACK_ROW_HEIGHT: f32 = 24.0;
const FALLBACK_COLUMN_WIDTH: f32 = 72.0;

/// Serialise one `<Table>` in InDesign's spelling (measured on a 2025
/// export: `Table` > `Row`* > `Column`* > `Cell`*, cells column-major,
/// `Self` ids derived from the table's — `<t>Row<r>`, `<t>Column<c>`,
/// `<t>i<n>` — when the model minted none). `host_width` is the inner
/// width of the text column the table sits in, when known (see
/// [`FALLBACK_COLUMN_WIDTH`]).
pub(crate) fn write_table(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    t: &idml_import::Table,
    host_width: Option<f32>,
) -> Result<(), quick_xml::Error> {
    let table_id = t
        .self_id
        .clone()
        .unwrap_or_else(|| "paged-table".to_string());
    let rows = if t.rows.is_empty() {
        (t.header_row_count + t.body_row_count + t.footer_row_count) as usize
    } else {
        t.rows.len()
    };
    let cols = if t.columns.is_empty() {
        t.column_count as usize
    } else {
        t.columns.len()
    };
    let header = (t.header_row_count as usize).min(rows);
    let footer = (t.footer_row_count as usize).min(rows - header);
    let body = rows - header - footer;
    let mut attrs: Vec<(&str, String)> = vec![
        ("Self", table_id.clone()),
        ("HeaderRowCount", header.to_string()),
        ("FooterRowCount", footer.to_string()),
        ("BodyRowCount", body.to_string()),
        ("ColumnCount", cols.to_string()),
    ];
    attrs.push((
        "AppliedTableStyle",
        t.applied_table_style
            .clone()
            .unwrap_or_else(|| "TableStyle/$ID/[No table style]".to_string()),
    ));
    if let Some(v) = t.repeating_header {
        attrs.push(("RepeatingHeader", v.to_string()));
    }
    if let Some(v) = t.repeating_footer {
        attrs.push(("RepeatingFooter", v.to_string()));
    }
    attrs.extend(table_stroke_attrs(t));
    attrs.push(("TableDirection", "LeftToRightDirection".to_string()));
    rewrite::emit_start_with_attrs(writer, "Table", &attrs)?;
    for r in 0..rows {
        let row = t.rows.get(r);
        let mut a: Vec<(&str, String)> = vec![
            (
                "Self",
                row.and_then(|r| r.self_id.clone())
                    .unwrap_or_else(|| format!("{table_id}Row{r}")),
            ),
            (
                "Name",
                row.and_then(|r| r.name.clone())
                    .unwrap_or_else(|| r.to_string()),
            ),
        ];
        a.push((
            "SingleRowHeight",
            rewrite::format_f32(
                row.and_then(|r| r.single_row_height)
                    .unwrap_or(FALLBACK_ROW_HEIGHT),
            ),
        ));
        if let Some(row) = row {
            push_opt_f32(&mut a, "MinimumHeight", row.minimum_height);
            push_opt_f32(&mut a, "MaximumHeight", row.maximum_height);
        }
        rewrite::emit_empty_with_attrs(writer, "Row", &a)?;
    }
    for c in 0..cols {
        let col = t.columns.get(c);
        let mut a: Vec<(&str, String)> = vec![
            (
                "Self",
                col.and_then(|c| c.self_id.clone())
                    .unwrap_or_else(|| format!("{table_id}Column{c}")),
            ),
            (
                "Name",
                col.and_then(|c| c.name.clone())
                    .unwrap_or_else(|| c.to_string()),
            ),
        ];
        let fallback_width = host_width
            .filter(|w| *w > 0.0 && cols > 0)
            .map(|w| w / cols as f32)
            .unwrap_or(FALLBACK_COLUMN_WIDTH);
        a.push((
            "SingleColumnWidth",
            rewrite::format_f32(
                col.and_then(|c| c.single_column_width)
                    .unwrap_or(fallback_width),
            ),
        ));
        rewrite::emit_empty_with_attrs(writer, "Column", &a)?;
    }
    for (i, cell) in t.cells.iter().enumerate() {
        let name = cell.name.clone().unwrap_or_else(|| {
            // Column-major document order.
            let (c, r) = if rows > 0 {
                (i / rows, i % rows)
            } else {
                (0, i)
            };
            format!("{c}:{r}")
        });
        let mut a: Vec<(&str, String)> = vec![
            (
                "Self",
                cell.self_id
                    .clone()
                    .unwrap_or_else(|| format!("{table_id}i{i}")),
            ),
            ("Name", name),
            ("RowSpan", cell.row_span.max(1).to_string()),
            ("ColumnSpan", cell.column_span.max(1).to_string()),
            ("CellType", "TextTypeCell".to_string()),
        ];
        if cell.text_top_inset != 0.0 {
            a.push(("TextTopInset", rewrite::format_f32(cell.text_top_inset)));
        }
        if cell.text_left_inset != 0.0 {
            a.push(("TextLeftInset", rewrite::format_f32(cell.text_left_inset)));
        }
        if cell.text_bottom_inset != 0.0 {
            a.push((
                "TextBottomInset",
                rewrite::format_f32(cell.text_bottom_inset),
            ));
        }
        if cell.text_right_inset != 0.0 {
            a.push(("TextRightInset", rewrite::format_f32(cell.text_right_inset)));
        }
        push_opt_str(&mut a, "AppliedCellStyle", &cell.applied_cell_style);
        push_opt_str(&mut a, "FillColor", &cell.fill_color);
        push_opt_str(&mut a, "TopEdgeStrokeColor", &cell.top_edge_stroke_color);
        push_opt_f32(&mut a, "TopEdgeStrokeWeight", cell.top_edge_stroke_weight);
        push_opt_f32(&mut a, "TopEdgeStrokeTint", cell.top_edge_stroke_tint);
        push_opt_str(
            &mut a,
            "BottomEdgeStrokeColor",
            &cell.bottom_edge_stroke_color,
        );
        push_opt_f32(
            &mut a,
            "BottomEdgeStrokeWeight",
            cell.bottom_edge_stroke_weight,
        );
        push_opt_f32(&mut a, "BottomEdgeStrokeTint", cell.bottom_edge_stroke_tint);
        push_opt_str(&mut a, "LeftEdgeStrokeColor", &cell.left_edge_stroke_color);
        push_opt_f32(&mut a, "LeftEdgeStrokeWeight", cell.left_edge_stroke_weight);
        push_opt_f32(&mut a, "LeftEdgeStrokeTint", cell.left_edge_stroke_tint);
        push_opt_str(
            &mut a,
            "RightEdgeStrokeColor",
            &cell.right_edge_stroke_color,
        );
        push_opt_f32(
            &mut a,
            "RightEdgeStrokeWeight",
            cell.right_edge_stroke_weight,
        );
        push_opt_f32(&mut a, "RightEdgeStrokeTint", cell.right_edge_stroke_tint);
        push_opt_str(
            &mut a,
            "VerticalJustification",
            &cell.vertical_justification,
        );
        push_opt_str(&mut a, "FirstBaselineOffset", &cell.first_baseline_offset);
        push_opt_f32(&mut a, "RotationAngle", cell.rotation_angle);
        rewrite::emit_start_with_attrs(writer, "Cell", &a)?;
        if cell
            .paragraphs
            .iter()
            .all(|p| p.runs.is_empty() && p.table.is_none())
        {
            // An empty cell still holds one (empty) paragraph — InDesign's
            // own spelling (`<ParagraphStyleRange><CharacterStyleRange/>
            // </ParagraphStyleRange>`), and the one InDesign 20.0.1 was
            // measured to REQUIRE: a 2×2 whose four cells were bare
            // `<Cell></Cell>` was the one table of sixteen it dropped.
            rewrite::emit_start_with_attrs(writer, "ParagraphStyleRange", &[])?;
            writer.write_event(Event::Empty(BytesStart::new("CharacterStyleRange")))?;
            writer.write_event(Event::End(BytesEnd::new("ParagraphStyleRange")))?;
        } else {
            write_paragraphs(writer, &cell.paragraphs, &[], host_width)?;
        }
        writer.write_event(Event::End(BytesEnd::new("Cell")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("Table")))?;
    Ok(())
}

/// `<Properties><AppliedFont type="string">NAME</AppliedFont></Properties>`
/// — the ONLY spelling InDesign reads for an applied font. Emitted
/// straight after the range's start tag, where a `<Properties>` block
/// belongs; nothing is written when the run pins no font.
pub(crate) fn emit_applied_font(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    font: &Option<String>,
) -> Result<(), quick_xml::Error> {
    let Some(name) = font else { return Ok(()) };
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    let mut e = BytesStart::new("AppliedFont");
    e.push_attribute(("type", "string"));
    writer.write_event(Event::Start(e))?;
    writer.write_event(Event::Text(quick_xml::events::BytesText::new(name)))?;
    writer.write_event(Event::End(BytesEnd::new("AppliedFont")))?;
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    Ok(())
}

/// The `<CharacterStyleRange>` attributes for one run — the same key
/// set `rewrite::character_attr_patch` owns, emitted only when set.
pub(crate) fn character_run_attrs(r: &CharacterRun) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    // NOT here: `AppliedFont`. InDesign reads the applied font as a
    // typed CHILD of `<Properties>`, never as an attribute — a real
    // InDesign-authored file carries 54 `<AppliedFont type="string">`
    // elements and not one attribute. We wrote the attribute, Adobe
    // ignored it, and a 134-page specimen in twenty faces opened
    // entirely in Minion Pro. `emit_applied_font` writes it now.
    let string_attrs: [(&'static str, &Option<String>); 8] = [
        ("AppliedCharacterStyle", &r.character_style),
        ("FontStyle", &r.font_style),
        ("FillColor", &r.fill_color),
        ("StrokeColor", &r.stroke_color),
        ("Capitalization", &r.capitalization),
        ("Position", &r.position),
        ("KerningMethod", &r.kerning_method),
        ("AppliedLanguage", &r.applied_language),
    ];
    for (k, v) in string_attrs {
        if let Some(v) = v {
            out.push((k, v.clone()));
        }
    }
    let f32_attrs: [(&'static str, Option<f32>); 9] = [
        ("PointSize", r.point_size),
        ("FillTint", r.fill_tint),
        ("StrokeWeight", r.stroke_weight),
        ("Leading", r.leading),
        ("Tracking", r.tracking),
        ("BaselineShift", r.baseline_shift),
        ("HorizontalScale", r.horizontal_scale),
        ("VerticalScale", r.vertical_scale),
        ("Skew", r.skew),
    ];
    for (k, v) in f32_attrs {
        if let Some(v) = v {
            out.push((k, rewrite::format_f32(v)));
        }
    }
    let bool_attrs: [(&'static str, Option<bool>); 3] = [
        ("Underline", r.underline),
        ("StrikeThru", r.strikethru),
        ("Ligatures", r.ligatures_on),
    ];
    for (k, v) in bool_attrs {
        if let Some(v) = v {
            out.push((k, v.to_string()));
        }
    }
    out
}

/// Serialise a full `Spreads/Spread_*.xml` part from the in-memory
/// model: the `<Spread>` start tag, one `<Page>` per model page (with
/// the page's ruler guides inside it — InDesign's placement, see
/// `guides`), then every page item via `rewrite::write_inserted_items`
/// (with an empty seen-set every item is "inserted" — a minted spread's
/// items all arrived through ops). Groups on a minted spread are not
/// serialised (the group-insert lane is a separate, documented defer).
/// `layer` is the `ItemLayer` new guides bind to, when known.
pub(crate) fn spread_part(
    spread: &Spread,
    dom_version: &str,
    layer: Option<&str>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let mut writer = new_part_writer()?;
    open_pkg_root(&mut writer, "idPkg:Spread", dom_version)?;
    let mut attrs: Vec<(&str, String)> = Vec::new();
    if let Some(id) = &spread.self_id {
        attrs.push(("Self", id.clone()));
    }
    // Always spelled, identity included: InDesign 20.0.1 does not read
    // an absent `ItemTransform` as identity (see
    // `rewrite::TransformPlan::extra`); its own packages carry one on
    // every `<Spread>`, `<Page>` and page item.
    attrs.push((
        "ItemTransform",
        rewrite::format_matrix(&spread.item_transform.unwrap_or(IDENTITY)),
    ));
    rewrite::emit_start_with_attrs(&mut writer, "Spread", &attrs)?;
    let page_count = spread.pages.len();
    // A guide lands in the page its index names, clamped to the last.
    let page_of = |g: &idml_import::RulerGuide| -> usize {
        (g.page_index as usize).min(page_count.saturating_sub(1))
    };
    for (pi, p) in spread.pages.iter().enumerate() {
        let mut pa: Vec<(&str, String)> = Vec::new();
        if let Some(id) = &p.self_id {
            pa.push(("Self", id.clone()));
        }
        if let Some(n) = &p.name {
            pa.push(("Name", n.clone()));
        }
        if let Some(m) = &p.applied_master {
            pa.push(("AppliedMaster", m.clone()));
        }
        pa.push((
            "ItemTransform",
            rewrite::format_matrix(&p.item_transform.unwrap_or(IDENTITY)),
        ));
        // GeometricBounds is the one attribute the parser requires to
        // accept a `<Page>` at all.
        pa.push((
            "GeometricBounds",
            format!(
                "{} {} {} {}",
                rewrite::format_f32(p.bounds.top),
                rewrite::format_f32(p.bounds.left),
                rewrite::format_f32(p.bounds.bottom),
                rewrite::format_f32(p.bounds.right),
            ),
        ));
        if let Some(m) = &p.master_page_transform {
            pa.push(("MasterPageTransform", rewrite::format_matrix(m)));
        }
        if !p.override_list.is_empty() {
            pa.push(("OverrideList", p.override_list.join(" ")));
        }
        if let Some(b) = p.show_master_items {
            pa.push(("ShowMasterItems", b.to_string()));
        }
        let guides_here: Vec<(usize, &idml_import::RulerGuide)> = spread
            .guides
            .iter()
            .enumerate()
            .filter(|(_, g)| page_of(g) == pi)
            .collect();
        if guides_here.is_empty() {
            rewrite::emit_empty_with_attrs(&mut writer, "Page", &pa)?;
        } else {
            rewrite::emit_start_with_attrs(&mut writer, "Page", &pa)?;
            for (gi, g) in guides_here {
                let mut g = *g;
                g.page_index = pi as u32;
                crate::guides::write_guide(
                    &mut writer,
                    &g,
                    &crate::guides::guide_self_id(spread.self_id.as_deref(), gi),
                    layer,
                )?;
            }
            writer.write_event(Event::End(BytesEnd::new("Page")))?;
        }
    }
    if page_count == 0 {
        for (gi, g) in spread.guides.iter().enumerate() {
            crate::guides::write_guide(
                &mut writer,
                g,
                &crate::guides::guide_self_id(spread.self_id.as_deref(), gi),
                layer,
            )?;
        }
    }
    rewrite::write_inserted_items(&mut writer, spread, &std::collections::HashSet::new())?;
    writer.write_event(Event::End(BytesEnd::new("Spread")))?;
    writer.write_event(Event::End(BytesEnd::new("idPkg:Spread")))?;
    Ok(writer.into_inner().into_inner())
}

/// Patch `designmap.xml` so the new entries are referenced. Every
/// original event passes through verbatim; only the new `<idPkg:Spread
/// src="..."/>` / `<idPkg:Story src="..."/>` elements are injected:
///
/// * a new spread ref goes immediately AFTER the existing spread ref
///   named by its anchor (`None` ⇒ before the first existing one) —
///   spread manifest order IS page order, so a minted spread must land
///   next to its host;
/// * new story refs are appended after the LAST existing `idPkg:Story`
///   (story manifest order only drives `doc.stories` order, and minted
///   stories were appended there too).
///
/// Anything left unplaced (no existing refs of that kind, an anchor
/// that vanished) is flushed just before `</Document>` — a reference is
/// never silently dropped.
pub(crate) fn patch_designmap(
    original: &[u8],
    new_spreads: &[(Option<String>, String)],
    new_stories: &[String],
    dropped_stories: &[String],
) -> Result<Vec<u8>, quick_xml::Error> {
    // Pass 1 — count the existing story refs so "after the last one" is
    // recognisable in the single forward pass below.
    let story_total = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut n = 0usize;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"idPkg:Story" => {
                    n += 1;
                }
                _ => {}
            }
            buf.clear();
        }
        n
    };

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    let mut spread_placed = vec![false; new_spreads.len()];
    let mut stories_placed = false;
    let mut first_spread_seen = false;
    let mut story_seen = 0usize;

    fn write_ref(
        writer: &mut Writer<Cursor<Vec<u8>>>,
        kind: &str,
        src: &str,
    ) -> Result<(), quick_xml::Error> {
        rewrite::emit_empty_with_attrs(writer, kind, &[("src", src.to_string())])
    }

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Empty(e) if e.name().as_ref() == b"idPkg:Spread" => {
                if !first_spread_seen {
                    first_spread_seen = true;
                    // Anchor-less spreads precede every existing one.
                    for (i, (anchor, src)) in new_spreads.iter().enumerate() {
                        if anchor.is_none() && !spread_placed[i] {
                            write_ref(&mut writer, "idPkg:Spread", src)?;
                            spread_placed[i] = true;
                        }
                    }
                }
                let this_src = attr_value(&e, b"src");
                writer.write_event(Event::Empty(e.into_owned()))?;
                if let Some(this_src) = this_src {
                    for (i, (anchor, src)) in new_spreads.iter().enumerate() {
                        if !spread_placed[i] && anchor.as_deref() == Some(this_src.as_str()) {
                            write_ref(&mut writer, "idPkg:Spread", src)?;
                            spread_placed[i] = true;
                        }
                    }
                }
            }
            Event::Empty(e) if e.name().as_ref() == b"idPkg:Story" => {
                story_seen += 1;
                // An orphan story's part is not written (see
                // `write_package`); its reference goes with it.
                let dropped = attr_value(&e, b"src")
                    .map(|src| dropped_stories.contains(&src))
                    .unwrap_or(false);
                if !dropped {
                    writer.write_event(Event::Empty(e.into_owned()))?;
                }
                if story_seen == story_total && !stories_placed {
                    for src in new_stories {
                        write_ref(&mut writer, "idPkg:Story", src)?;
                    }
                    stories_placed = true;
                }
            }
            Event::End(e) if e.name().as_ref() == b"Document" => {
                // Flush everything still unplaced before the root closes.
                for (i, (_, src)) in new_spreads.iter().enumerate() {
                    if !spread_placed[i] {
                        write_ref(&mut writer, "idPkg:Spread", src)?;
                        spread_placed[i] = true;
                    }
                }
                if !stories_placed {
                    for src in new_stories {
                        write_ref(&mut writer, "idPkg:Story", src)?;
                    }
                    stories_placed = true;
                }
                writer.write_event(Event::End(e))?;
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }

    Ok(writer.into_inner().into_inner())
}

/// Reference `Resources/Fonts.xml` from the designmap when it does not
/// already: the `<idPkg:Fonts>` element goes where InDesign puts it —
/// before `<idPkg:Styles>`, else before the first spread reference,
/// else last. Byte-identical when the reference exists.
pub(crate) fn ensure_fonts_ref(original: &[u8], src: &str) -> Result<Vec<u8>, quick_xml::Error> {
    let has_ref = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut found = false;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"idPkg:Fonts" => {
                    found = true;
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        found
    };
    if has_ref {
        return Ok(original.to_vec());
    }
    let has_styles_ref = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut found = false;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"idPkg:Styles" => {
                    found = true;
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        found
    };
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut placed = false;
    let place = |writer: &mut Writer<Cursor<Vec<u8>>>| -> Result<(), quick_xml::Error> {
        rewrite::emit_empty_with_attrs(writer, "idPkg:Fonts", &[("src", src.to_string())])
    };
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Eof => break,
            Event::Empty(ref e) | Event::Start(ref e)
                if !placed
                    && ((has_styles_ref && e.name().as_ref() == b"idPkg:Styles")
                        || (!has_styles_ref
                            && matches!(
                                e.name().as_ref(),
                                b"idPkg:MasterSpread" | b"idPkg:Spread"
                            ))) =>
            {
                place(&mut writer)?;
                placed = true;
                writer.write_event(ev.borrow())?;
            }
            Event::End(ref e) if !placed && e.name().as_ref() == b"Document" => {
                place(&mut writer)?;
                placed = true;
                writer.write_event(ev.borrow())?;
            }
            _ => writer.write_event(ev.borrow())?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

/// Read an attribute's decoded value off a start tag (local copy of
/// `rewrite::attr_value`, which is private to that module's hot path).
fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| std::str::from_utf8(&a.value).ok().map(|s| s.to_string()))
}
