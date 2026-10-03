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

//! C-62 — a pen path's stroke end cap and line ends survive IDML.
//!
//! A pen or pencil path is a `<Polygon>` whose contour is open. IDML
//! writes `EndCap` and the line-end vocabulary (`LeftLineEnd`,
//! `RightLineEnd`, `*ArrowHeadScale`) on every page item, and the corpus
//! uses it: 109 polygons spell `EndCap`, 78 the line ends, 128 lines
//! spell `RoundEndCap`. The reader took `EndCap` on `<Rectangle>` only
//! and line ends on `<GraphicLine>` only, so a pen path lost both.
//!
//! What this file pins, per lane:
//! * read — the four kinds parse what the source spells;
//! * patch — an unmutated save is byte-identical (including the explicit
//!   `LeftLineEnd="None"` InDesign writes on polygons, which the line-end
//!   patch used to delete), and an edit is written;
//! * minted — an item the source never carried is emitted with its cap,
//!   line ends and non-default scales, and reopens to the same model.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{ArrowheadType, FrameRef};

/// An open three-point path — what InDesign's Pen tool leaves.
const OPEN_PATH: &str = r#"<Properties><PathGeometry><GeometryPathType PathOpen="true"><PathPointArray><PathPointType Anchor="100 100" LeftDirection="100 100" RightDirection="100 100"/><PathPointType Anchor="200 160" LeftDirection="200 160" RightDirection="200 160"/><PathPointType Anchor="300 100" LeftDirection="300 100" RightDirection="300 100"/></PathPointArray></GeometryPathType></PathGeometry></Properties>"#;

fn spread() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="500 100 600 400" ItemTransform="1 0 0 1 0 0"/>
<Polygon Self="pen1" ItemTransform="1 0 0 1 0 0" StrokeColor="Color/Black" StrokeWeight="4" EndCap="RoundEndCap" LeftLineEnd="CircleSolidArrowHead" RightLineEnd="TriangleArrowHead" LeftArrowHeadScale="150" RightArrowHeadScale="100">{OPEN_PATH}</Polygon>
<Polygon Self="pen2" ItemTransform="1 0 0 1 0 200" StrokeColor="Color/Black" StrokeWeight="1" EndCap="ButtEndCap" LeftLineEnd="None" RightLineEnd="None" LeftArrowHeadScale="100" RightArrowHeadScale="100">{OPEN_PATH}</Polygon>
<GraphicLine Self="gl1" ItemTransform="1 0 0 1 0 0" GeometricBounds="400 100 400 300" StrokeColor="Color/Black" StrokeWeight="2" EndCap="RoundEndCap" RightLineEnd="BarbedArrowHead"/>
<Oval Self="ov1" ItemTransform="1 0 0 1 0 0" GeometricBounds="420 320 480 380" StrokeColor="Color/Black" StrokeWeight="3" EndCap="ButtEndCap" LeftLineEnd="None" RightLineEnd="None"/>
</Spread></idPkg:Spread>"#
    )
}

fn source() -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread()),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

fn polygon<'a>(doc: &'a paged_scene::Document, id: &str) -> &'a idml_import::Polygon {
    doc.spreads[0]
        .spread
        .polygons
        .iter()
        .find(|p| p.self_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("polygon {id}"))
}

/// The start tag of the element whose `Self` is `id`.
fn start_tag<'a>(xml: &'a str, id: &str) -> &'a str {
    let at = pkg::pos(xml, &format!(r#"Self="{id}""#));
    let open = xml[..at].rfind('<').expect("tag open");
    let close = at + xml[at..].find('>').expect("tag close");
    &xml[open..=close]
}

#[test]
fn every_kind_reads_its_cap_and_a_polygon_its_line_ends() {
    let doc = pkg::open(&source());
    let pen1 = polygon(&doc, "pen1");
    assert_eq!(pen1.end_cap.as_deref(), Some("RoundEndCap"));
    assert_eq!(pen1.start_arrow, ArrowheadType::CircleSolid);
    assert_eq!(pen1.end_arrow, ArrowheadType::Triangle);
    assert_eq!(pen1.start_arrow_scale, 150.0);
    assert_eq!(pen1.end_arrow_scale, 100.0);
    let pen2 = polygon(&doc, "pen2");
    assert_eq!(pen2.end_cap.as_deref(), Some("ButtEndCap"));
    assert_eq!(pen2.start_arrow, ArrowheadType::None);
    assert_eq!(pen2.end_arrow, ArrowheadType::None);

    let spread = &doc.spreads[0].spread;
    assert_eq!(
        spread.graphic_lines[0].end_cap.as_deref(),
        Some("RoundEndCap")
    );
    assert_eq!(spread.graphic_lines[0].end_arrow, ArrowheadType::Barbed);
    assert_eq!(spread.ovals[0].end_cap.as_deref(), Some("ButtEndCap"));
}

/// Byte-exact where the source had the attributes — including the
/// `LeftLineEnd="None"` / `RightLineEnd="None"` pair InDesign writes on
/// an arrowless polygon, which a model `None` used to REMOVE.
#[test]
fn an_unmutated_save_keeps_every_byte() {
    let src = source();
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    pkg::assert_same_package(&src, &out);
}

#[test]
fn an_edited_cap_and_line_end_are_written_and_reopen() {
    let src = source();
    let mut doc = pkg::open(&src);
    {
        let spread = &mut doc.spreads[0].spread;
        let pen1 = spread
            .polygons
            .iter_mut()
            .find(|p| p.self_id.as_deref() == Some("pen1"))
            .unwrap();
        pen1.end_cap = Some("ProjectingEndCap".to_string());
        pen1.end_arrow = ArrowheadType::Barbed;
        let pen2 = spread
            .polygons
            .iter_mut()
            .find(|p| p.self_id.as_deref() == Some("pen2"))
            .unwrap();
        pen2.start_arrow = ArrowheadType::Simple;
        pen2.end_cap = None;
        spread.graphic_lines[0].end_cap = Some("ButtEndCap".to_string());
        spread.ovals[0].end_cap = Some("RoundEndCap".to_string());
    }
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");

    let pen1 = start_tag(&xml, "pen1");
    assert!(pen1.contains(r#"EndCap="ProjectingEndCap""#), "{pen1}");
    assert!(pen1.contains(r#"RightLineEnd="BarbedArrowHead""#), "{pen1}");
    // Untouched neighbours keep their source spelling.
    assert!(
        pen1.contains(r#"LeftLineEnd="CircleSolidArrowHead""#),
        "{pen1}"
    );
    assert!(pen1.contains(r#"LeftArrowHeadScale="150""#), "{pen1}");
    let pen2 = start_tag(&xml, "pen2");
    assert!(pen2.contains(r#"LeftLineEnd="SimpleArrowHead""#), "{pen2}");
    assert!(pen2.contains(r#"RightLineEnd="None""#), "{pen2}");
    assert!(!pen2.contains("EndCap="), "a cleared cap goes: {pen2}");
    let gl1 = start_tag(&xml, "gl1");
    assert!(gl1.contains(r#"EndCap="ButtEndCap""#), "{gl1}");
    let ov1 = start_tag(&xml, "ov1");
    assert!(ov1.contains(r#"EndCap="RoundEndCap""#), "{ov1}");

    let re = pkg::open(&out);
    assert_eq!(
        polygon(&re, "pen1").end_cap.as_deref(),
        Some("ProjectingEndCap")
    );
    assert_eq!(polygon(&re, "pen1").end_arrow, ArrowheadType::Barbed);
    assert_eq!(polygon(&re, "pen2").start_arrow, ArrowheadType::Simple);
    assert_eq!(polygon(&re, "pen2").end_cap, None);
    let twice = write_idml(&re, &out).expect("write again");
    pkg::assert_same_package(&out, &twice);
}

/// A pen path the source never carried — the editor's freshly drawn one,
/// or a duplicate — is written with its cap, its line ends and a scale
/// that is not InDesign's default, and reopens to the same values.
#[test]
fn a_minted_pen_path_is_written_with_its_cap_and_line_ends() {
    let src = source();
    let mut doc = pkg::open(&src);
    {
        let spread = &mut doc.spreads[0].spread;
        let mut minted = polygon_clone(spread, "pen1", "pen_new");
        minted.end_cap = Some("RoundEndCap".to_string());
        minted.start_arrow = ArrowheadType::CircleSolid;
        minted.end_arrow = ArrowheadType::Triangle;
        minted.start_arrow_scale = 150.0;
        minted.end_arrow_scale = 100.0;
        spread.polygons.push(minted);
        let idx = spread.polygons.len() - 1;
        spread.frames_in_order.push(FrameRef::Polygon(idx));

        let mut line = spread.graphic_lines[0].clone();
        line.self_id = Some("gl_new".to_string());
        line.end_cap = Some("RoundEndCap".to_string());
        line.end_arrow_scale = 80.0;
        spread.graphic_lines.push(line);
        let idx = spread.graphic_lines.len() - 1;
        spread.frames_in_order.push(FrameRef::GraphicLine(idx));
    }
    doc.rebuild_indexes();
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");

    let tag = start_tag(&xml, "pen_new");
    assert!(tag.starts_with("<Polygon "), "{tag}");
    for want in [
        r#"EndCap="RoundEndCap""#,
        r#"LeftLineEnd="CircleSolidArrowHead""#,
        r#"LeftArrowHeadScale="150""#,
        r#"RightLineEnd="TriangleArrowHead""#,
    ] {
        assert!(tag.contains(want), "{want} missing: {tag}");
    }
    assert!(
        !tag.contains("RightArrowHeadScale"),
        "a 100 % scale is the default and is not spelled: {tag}"
    );
    let line = start_tag(&xml, "gl_new");
    assert!(line.contains(r#"EndCap="RoundEndCap""#), "{line}");
    assert!(line.contains(r#"RightLineEnd="BarbedArrowHead""#), "{line}");
    assert!(line.contains(r#"RightArrowHeadScale="80""#), "{line}");

    let re = pkg::open(&out);
    let p = polygon(&re, "pen_new");
    assert_eq!(p.end_cap.as_deref(), Some("RoundEndCap"));
    assert_eq!(p.start_arrow, ArrowheadType::CircleSolid);
    assert_eq!(p.end_arrow, ArrowheadType::Triangle);
    assert_eq!(p.start_arrow_scale, 150.0);
    assert_eq!(p.end_arrow_scale, 100.0);
    assert_eq!(p.subpath_open, vec![true], "the contour stays open");
    let gl = re.spreads[0]
        .spread
        .graphic_lines
        .iter()
        .find(|l| l.self_id.as_deref() == Some("gl_new"))
        .expect("gl_new");
    assert_eq!(gl.end_cap.as_deref(), Some("RoundEndCap"));
    assert_eq!(gl.end_arrow_scale, 80.0);
    let twice = write_idml(&re, &out).expect("write again");
    pkg::assert_same_package(&out, &twice);
}

fn polygon_clone(spread: &idml_import::Spread, from: &str, id: &str) -> idml_import::Polygon {
    let mut p = spread
        .polygons
        .iter()
        .find(|p| p.self_id.as_deref() == Some(from))
        .expect("source polygon")
        .clone();
    p.self_id = Some(id.to_string());
    p
}
