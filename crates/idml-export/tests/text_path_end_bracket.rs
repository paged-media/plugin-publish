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

//! A `<TextPath>` whose end bracket the model leaves unset is written with
//! `EndBracket` = the host path's arc length. The model reads
//! `end_bracket: None` as "to the end of the path"; InDesign reads an
//! absent `EndBracket` as 0 — no text on the path, the story overset
//! (plugin-draw's InDesign round-trip). The lengths below are InDesign
//! 20.0.1's own `EndBracket` for the same geometry (measured 2026-10-06:
//! a 200 pt line 200, this cubic 221.00078291246408, a 100 x 150
//! rectangle 500).

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

fn geometry(open: bool, points: &[((f32, f32), (f32, f32), (f32, f32))]) -> String {
    let pts: String = points
        .iter()
        .map(|(l, a, r)| {
            format!(
                r#"<PathPointType Anchor="{} {}" LeftDirection="{} {}" RightDirection="{} {}"/>"#,
                a.0, a.1, l.0, l.1, r.0, r.1
            )
        })
        .collect();
    format!(
        r#"<Properties><PathGeometry><GeometryPathType PathOpen="{open}"><PathPointArray>{pts}</PathPointArray></GeometryPathType></PathGeometry></Properties>"#
    )
}

fn source() -> Vec<u8> {
    let line = geometry(
        true,
        &[
            ((100.0, 100.0), (100.0, 100.0), (100.0, 100.0)),
            ((300.0, 100.0), (300.0, 100.0), (300.0, 100.0)),
        ],
    );
    let curve = geometry(
        true,
        &[
            ((100.0, 500.0), (100.0, 500.0), (100.0, 450.0)),
            ((300.0, 500.0), (300.0, 450.0), (300.0, 500.0)),
        ],
    );
    let rect = geometry(
        false,
        &[
            ((100.0, 200.0), (100.0, 200.0), (100.0, 200.0)),
            ((250.0, 200.0), (250.0, 200.0), (250.0, 200.0)),
            ((250.0, 300.0), (250.0, 300.0), (250.0, 300.0)),
            ((100.0, 300.0), (100.0, 300.0), (100.0, 300.0)),
        ],
    );
    let spread = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="600 100 700 400" ItemTransform="1 0 0 1 0 0"/>
<GraphicLine Self="line" ItemTransform="1 0 0 1 0 0">{line}</GraphicLine>
<GraphicLine Self="curve" ItemTransform="1 0 0 1 0 0">{curve}</GraphicLine>
<Rectangle Self="rect" ItemTransform="1 0 0 1 0 0">{rect}</Rectangle>
</Spread></idPkg:Spread>"#
    );
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

fn text_path(id: &str, start: Option<f32>, end: Option<f32>) -> idml_import::TextPath {
    idml_import::TextPath {
        self_id: Some(id.to_string()),
        parent_story: "st1".to_string(),
        path_alignment: None,
        path_type_alignment: None,
        path_effect: None,
        flip_path_effect: None,
        start_bracket: start,
        end_bracket: end,
    }
}

fn text_path_tag<'a>(xml: &'a str, id: &str) -> &'a str {
    let at = pkg::pos(xml, &format!(r#"<TextPath Self="{id}""#));
    &xml[at..at + xml[at..].find("/>").expect("tag end") + 2]
}

#[test]
fn an_unset_end_bracket_is_the_path_length() {
    let src = source();
    let mut doc = pkg::open(&src);
    {
        let s = &mut doc.spreads[0].spread;
        let line = s
            .graphic_lines
            .iter_mut()
            .find(|l| l.self_id.as_deref() == Some("line"))
            .unwrap();
        line.text_paths = vec![text_path("tpline", Some(10.0), None)];
        let curve = s
            .graphic_lines
            .iter_mut()
            .find(|l| l.self_id.as_deref() == Some("curve"))
            .unwrap();
        curve.text_paths = vec![text_path("tpcurve", None, None)];
        s.rectangles[0].text_paths = vec![text_path("tprect", None, None)];
    }
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let line = text_path_tag(&xml, "tpline");
    assert!(
        line.contains(r#"StartBracket="10" EndBracket="200""#),
        "{line}"
    );
    let curve = text_path_tag(&xml, "tpcurve");
    // The exact arc length is 221.0026; InDesign's own estimate is
    // 0.0018 pt shorter. Within a hundredth of a point either way.
    let end: f32 = curve
        .split(r#"EndBracket=""#)
        .nth(1)
        .and_then(|r| r.split('"').next())
        .and_then(|v| v.parse().ok())
        .expect("EndBracket written");
    assert!((end - 221.000_78).abs() < 0.01, "{curve}");
    let rect = text_path_tag(&xml, "tprect");
    assert!(rect.contains(r#"EndBracket="500""#), "{rect}");

    // Reopened, the bracket is explicit, and saves unchanged.
    let doc2 = pkg::open(&out);
    let twice = write_idml(&doc2, &out).expect("write again");
    pkg::assert_same_package(&out, &twice);
}

#[test]
fn a_set_end_bracket_is_written_as_set() {
    let src = source();
    let mut doc = pkg::open(&src);
    let line = doc.spreads[0]
        .spread
        .graphic_lines
        .iter_mut()
        .find(|l| l.self_id.as_deref() == Some("line"))
        .unwrap();
    line.text_paths = vec![text_path("tpline", Some(10.0), Some(120.0))];
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let tag = text_path_tag(&xml, "tpline");
    assert!(tag.contains(r#"EndBracket="120""#), "{tag}");
}
