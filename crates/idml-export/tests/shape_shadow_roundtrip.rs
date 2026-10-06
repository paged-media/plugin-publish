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

//! A drop shadow on a polygon, an oval and a line survives the trip.
//!
//! The model carries a shadow on every shape. The importer read one only
//! off text frames, rectangles and ovals; the exporter wrote one only for
//! those kinds when they came from the source, and for none of the
//! inserted ovals, polygons or lines. A pen path's shadow, a rule's
//! shadow and a drawn ellipse's shadow were dropped on save.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{DropShadowSetting, FrameRef};

const SPREAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
<Polygon Self="pg1" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red"><Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="100 450" LeftDirection="100 450" RightDirection="100 450"/><PathPointType Anchor="300 450" LeftDirection="300 450" RightDirection="300 450"/><PathPointType Anchor="200 600" LeftDirection="200 600" RightDirection="200 600"/></PathPointArray></GeometryPathType></PathGeometry></Properties></Polygon>
<GraphicLine Self="gl1" ItemTransform="1 0 0 1 0 0" StrokeColor="Color/Black" StrokeWeight="2"><Properties><PathGeometry><GeometryPathType PathOpen="true"><PathPointArray><PathPointType Anchor="100 650" LeftDirection="100 650" RightDirection="100 650"/><PathPointType Anchor="400 650" LeftDirection="400 650" RightDirection="400 650"/></PathPointArray></GeometryPathType></PathGeometry></Properties></GraphicLine>
</Spread></idPkg:Spread>"#;

fn source(spread: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

fn shadow(x: f32) -> DropShadowSetting {
    DropShadowSetting {
        mode: "Drop".into(),
        x_offset: x,
        y_offset: 3.0,
        size: 5.0,
        opacity_pct: 60.0,
        effect_color: Some("Color/Black".into()),
    }
}

fn offsets(s: &Option<DropShadowSetting>) -> Option<(f32, f32, f32, f32)> {
    s.as_ref()
        .map(|s| (s.x_offset, s.y_offset, s.size, s.opacity_pct))
}

#[test]
fn polygon_and_line_shadows_are_read() {
    let with_shadows = SPREAD
        .replace(
            r#"FillColor="Color/Red"><Properties>"#,
            r#"FillColor="Color/Red"><Properties><TransparencySetting><DropShadowSetting Mode="Drop" XOffset="7" YOffset="3" Size="5" Opacity="60" EffectColor="Color/Black"/></TransparencySetting>"#,
        )
        .replace(
            r#"StrokeWeight="2"><Properties>"#,
            r#"StrokeWeight="2"><Properties><TransparencySetting><DropShadowSetting Mode="Drop" XOffset="8" YOffset="3" Size="5" Opacity="60" EffectColor="Color/Black"/></TransparencySetting>"#,
        );
    let doc = pkg::open(&source(&with_shadows));
    let s = &doc.spreads[0].spread;
    assert_eq!(
        offsets(&s.polygons[0].drop_shadow),
        Some((7.0, 3.0, 5.0, 60.0))
    );
    assert_eq!(
        offsets(&s.graphic_lines[0].drop_shadow),
        Some((8.0, 3.0, 5.0, 60.0))
    );
}

#[test]
fn a_shadow_added_to_a_source_polygon_and_line_is_written() {
    let src = source(SPREAD);
    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.polygons[0].drop_shadow = Some(shadow(7.0));
    doc.spreads[0].spread.graphic_lines[0].drop_shadow = Some(shadow(8.0));
    let out = write_idml(&doc, &src).expect("write");
    let re = pkg::open(&out);
    let s = &re.spreads[0].spread;
    assert_eq!(
        offsets(&s.polygons[0].drop_shadow),
        Some((7.0, 3.0, 5.0, 60.0))
    );
    assert_eq!(
        offsets(&s.graphic_lines[0].drop_shadow),
        Some((8.0, 3.0, 5.0, 60.0))
    );

    // And an unchanged document re-saves its shadows untouched.
    let again = write_idml(&re, &out).expect("re-write");
    assert_eq!(
        pkg::entry(&again, "Spreads/Spread_s1.xml"),
        pkg::entry(&out, "Spreads/Spread_s1.xml"),
        "a model that agrees with the part leaves it byte-identical"
    );
}

#[test]
fn inserted_ovals_polygons_and_lines_carry_their_shadows() {
    let src = source(SPREAD);
    let mut doc = pkg::open(&src);
    let spread = &mut doc.spreads[0].spread;

    let mut poly = spread.polygons[0].clone();
    poly.self_id = Some("pg2".into());
    poly.drop_shadow = Some(shadow(4.0));
    spread.polygons.push(poly);
    spread.frames_in_order.push(FrameRef::Polygon(1));

    let mut line = spread.graphic_lines[0].clone();
    line.self_id = Some("gl2".into());
    line.drop_shadow = Some(shadow(5.0));
    spread.graphic_lines.push(line);
    spread.frames_in_order.push(FrameRef::GraphicLine(1));

    let mut oval = idml_import::Oval::new(
        "ov2",
        idml_import::Bounds {
            top: 100.0,
            left: 450.0,
            bottom: 200.0,
            right: 550.0,
        },
    );
    oval.fill_color = Some("Color/Red".into());
    oval.drop_shadow = Some(shadow(6.0));
    spread.ovals.push(oval);
    let oi = spread.ovals.len() - 1;
    spread.frames_in_order.push(FrameRef::Oval(oi));

    let out = write_idml(&doc, &src).expect("write");
    let re = pkg::open(&out);
    let s = &re.spreads[0].spread;
    let find = |id: &str| -> Option<(f32, f32, f32, f32)> {
        s.polygons
            .iter()
            .find(|p| p.self_id.as_deref() == Some(id))
            .map(|p| offsets(&p.drop_shadow))
            .or_else(|| {
                s.graphic_lines
                    .iter()
                    .find(|p| p.self_id.as_deref() == Some(id))
                    .map(|p| offsets(&p.drop_shadow))
            })
            .or_else(|| {
                s.ovals
                    .iter()
                    .find(|p| p.self_id.as_deref() == Some(id))
                    .map(|p| offsets(&p.drop_shadow))
            })
            .flatten()
    };
    assert_eq!(find("pg2"), Some((4.0, 3.0, 5.0, 60.0)), "inserted polygon");
    assert_eq!(find("gl2"), Some((5.0, 3.0, 5.0, 60.0)), "inserted line");
    assert_eq!(find("ov2"), Some((6.0, 3.0, 5.0, 60.0)), "inserted oval");
}
