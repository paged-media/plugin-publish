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

//! C-82 — a MINTED item (every path paged.draw draws) keeps its stroke
//! join, miter limit, alignment, dash and corners through an IDML export,
//! and the built-ins it references (`Color/Paper`, the Dashed stroke
//! style) are declared in `Graphic.xml`. InDesign 20.0.1 read all of these
//! as their defaults before (plugin-draw's InDesign round-trip lane).

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CornerOption, CornerSpec, FrameRef};

const GRAPHIC: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" Name="Black"/>
</idPkg:Graphic>"#;

const CLOSED: &str = r#"<Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="100 100" LeftDirection="100 100" RightDirection="100 100"/><PathPointType Anchor="200 100" LeftDirection="200 100" RightDirection="200 100"/><PathPointType Anchor="200 200" LeftDirection="200 200" RightDirection="200 200"/><PathPointType Anchor="100 200" LeftDirection="100 200" RightDirection="100 200"/></PathPointArray></GeometryPathType></PathGeometry></Properties>"#;

fn source() -> Vec<u8> {
    let spread = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="500 100 600 400" ItemTransform="1 0 0 1 0 0"/>
<Polygon Self="src1" ItemTransform="1 0 0 1 0 0" StrokeColor="Color/Black" StrokeWeight="2">{CLOSED}</Polygon>
</Spread></idPkg:Spread>"#
    );
    let designmap = pkg::designmap(
        r#"<idPkg:Graphic src="Resources/Graphic.xml"/>
"#,
    );
    pkg::package(&[
        ("designmap.xml", &designmap),
        ("Resources/Graphic.xml", GRAPHIC),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

fn start_tag<'a>(xml: &'a str, id: &str) -> &'a str {
    let at = pkg::pos(xml, &format!(r#"Self="{id}""#));
    let open = xml[..at].rfind('<').expect("tag open");
    let close = at + xml[at..].find('>').expect("tag close");
    &xml[open..=close]
}

fn minted_doc(src: &[u8]) -> paged_scene::Document {
    let mut doc = pkg::open(src);
    {
        let spread = &mut doc.spreads[0].spread;
        let mut p = spread
            .polygons
            .iter()
            .find(|p| p.self_id.as_deref() == Some("src1"))
            .expect("src1")
            .clone();
        p.self_id = Some("drawn".to_string());
        p.fill_color = Some("Color/Paper".to_string());
        p.end_join = Some("RoundEndJoin".to_string());
        p.miter_limit = Some(2.0);
        p.stroke_alignment = Some("InsideAlignment".to_string());
        p.stroke_dash = vec![6.0, 3.0];
        p.corner_option = Some("RoundedCorner".to_string());
        p.corner_radius = Some(12.0);
        p.corners = [CornerSpec {
            option: Some(CornerOption::Rounded),
            radius: Some(12.0),
        }; 4];
        spread.polygons.push(p);
        let idx = spread.polygons.len() - 1;
        spread.frames_in_order.push(FrameRef::Polygon(idx));
    }
    doc.rebuild_indexes();
    doc
}

#[test]
fn a_minted_path_writes_join_miter_alignment_dash_and_corners() {
    let src = source();
    let out = write_idml(&minted_doc(&src), &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let tag = start_tag(&xml, "drawn");
    for want in [
        r#"EndJoin="RoundEndJoin""#,
        r#"MiterLimit="2""#,
        r#"StrokeAlignment="InsideAlignment""#,
        r#"StrokeType="StrokeStyle/$ID/Dashed""#,
        r#"StrokeDashAndGap="6 3""#,
        r#"CornerOption="RoundedCorner""#,
        r#"CornerRadius="12""#,
        r#"TopLeftCornerOption="RoundedCorner""#,
        r#"BottomRightCornerRadius="12""#,
    ] {
        assert!(tag.contains(want), "{want} missing: {tag}");
    }

    let re = pkg::open(&out);
    let p = re.spreads[0]
        .spread
        .polygons
        .iter()
        .find(|p| p.self_id.as_deref() == Some("drawn"))
        .expect("drawn reopens");
    assert_eq!(p.end_join.as_deref(), Some("RoundEndJoin"));
    assert_eq!(p.miter_limit, Some(2.0));
    assert_eq!(p.stroke_alignment.as_deref(), Some("InsideAlignment"));
    assert_eq!(p.stroke_dash, vec![6.0, 3.0]);
    assert_eq!(p.corners[2].option, Some(CornerOption::Rounded));
    assert_eq!(p.corners[2].radius, Some(12.0));

    let twice = write_idml(&re, &out).expect("write again");
    pkg::assert_same_package(&out, &twice);
}

#[test]
fn the_built_ins_a_minted_item_uses_are_declared() {
    let src = source();
    let out = write_idml(&minted_doc(&src), &src).expect("write");
    let graphic = pkg::entry(&out, "Resources/Graphic.xml").expect("graphic");
    assert!(graphic.contains(r#"Self="Color/Paper""#), "{graphic}");
    assert!(
        graphic.contains(r#"Self="StrokeStyle/$ID/Dashed""#),
        "{graphic}"
    );
    assert_eq!(
        graphic.matches(r#"Self="Color/Paper""#).count(),
        1,
        "declared once"
    );
}

#[test]
fn an_unmutated_save_declares_nothing_new() {
    let src = source();
    let out = write_idml(&pkg::open(&src), &src).expect("write");
    pkg::assert_same_package(&src, &out);
}

fn text_path(story: &str) -> idml_import::TextPath {
    idml_import::TextPath {
        self_id: Some(format!("tp_{story}")),
        parent_story: story.to_string(),
        path_alignment: None,
        path_type_alignment: Some("BaselinePathType".to_string()),
        path_effect: Some("RainbowPathEffect".to_string()),
        flip_path_effect: None,
        start_bracket: None,
        end_bracket: None,
    }
}

#[test]
fn type_on_a_path_is_written_for_a_minted_and_a_parsed_host() {
    let src = source();
    let mut doc = minted_doc(&src);
    {
        let spread = &mut doc.spreads[0].spread;
        for p in spread.polygons.iter_mut() {
            p.text_paths = vec![text_path("st1")];
        }
    }
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert_eq!(
        xml.matches("<TextPath ").count(),
        2,
        "one on the minted path, one on the parsed one: {xml}"
    );
    assert!(xml.contains(r#"ParentStory="st1""#), "{xml}");
    assert!(xml.contains(r#"PathEffect="RainbowPathEffect""#), "{xml}");

    let re = pkg::open(&out);
    for p in &re.spreads[0].spread.polygons {
        assert_eq!(p.text_paths.len(), 1, "{:?} keeps its text path", p.self_id);
        assert_eq!(p.text_paths[0].parent_story, "st1");
    }
    assert!(
        pkg::entry(&out, "Stories/Story_st1.xml").is_some(),
        "the story ships with the path"
    );
    let twice = write_idml(&re, &out).expect("write again");
    pkg::assert_same_package(&out, &twice);
}
