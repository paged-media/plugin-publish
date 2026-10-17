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

//! A page item's `OverprintFill` / `OverprintStroke` survive an export.
//!
//! The importer reads both and the renderer composites them, but no
//! writer spelled them: a frame set to overprint came back knocking out
//! of the plate beneath it — on an item the source carried and on one
//! minted since load alike.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::FrameRef;

const SPREAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
<Rectangle Self="r1" GeometricBounds="500 100 600 400" ItemTransform="1 0 0 1 0 0" FillColor="Color/Black"/>
<Oval Self="o1" OverprintFill="false" OverprintStroke="false" GeometricBounds="500 420 600 520" ItemTransform="1 0 0 1 0 0"/>
</Spread></idPkg:Spread>"#;

fn source() -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

fn start_tag<'a>(xml: &'a str, id: &str) -> &'a str {
    let at = pkg::pos(xml, &format!(r#"Self="{id}""#));
    let open = xml[..at].rfind('<').expect("tag open");
    let close = at + xml[at..].find('>').expect("tag close");
    &xml[open..=close]
}

#[test]
fn a_source_item_set_to_overprint_is_written_and_reads_back() {
    let src = source();
    let mut doc = pkg::open(&src);
    {
        let s = &mut doc.spreads[0].spread;
        s.rectangles[0].overprint_fill = true;
        s.rectangles[0].overprint_stroke = true;
        s.ovals[0].overprint_fill = true;
        s.text_frames[0].overprint_stroke = true;
    }
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let r1 = start_tag(&xml, "r1");
    assert!(
        r1.contains(r#"OverprintFill="true""#) && r1.contains(r#"OverprintStroke="true""#),
        "{r1}"
    );
    let o1 = start_tag(&xml, "o1");
    assert!(
        o1.contains(r#"OverprintFill="true" OverprintStroke="false""#),
        "patched in place, the unchanged spelling kept: {o1}"
    );
    let back = pkg::open(&out);
    let s = &back.spreads[0].spread;
    assert!(s.rectangles[0].overprint_fill && s.rectangles[0].overprint_stroke);
    assert!(s.ovals[0].overprint_fill && !s.ovals[0].overprint_stroke);
    assert!(!s.text_frames[0].overprint_fill && s.text_frames[0].overprint_stroke);
    // A second save of what was written changes nothing.
    pkg::assert_same_package(&out, &write_idml(&back, &out).expect("again"));
}

#[test]
fn turning_overprint_off_drops_the_attribute() {
    let src = source();
    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.rectangles[0].overprint_fill = true;
    let on = write_idml(&doc, &src).expect("write");
    let mut doc = pkg::open(&on);
    assert!(doc.spreads[0].spread.rectangles[0].overprint_fill);
    doc.spreads[0].spread.rectangles[0].overprint_fill = false;
    let off = write_idml(&doc, &on).expect("write");
    let xml = pkg::entry(&off, "Spreads/Spread_s1.xml").expect("spread");
    assert!(!start_tag(&xml, "r1").contains("Overprint"), "{xml}");
    assert!(!pkg::open(&off).spreads[0].spread.rectangles[0].overprint_fill);
}

#[test]
fn a_minted_item_carries_its_overprint() {
    let src = source();
    let mut doc = pkg::open(&src);
    {
        let spread = &mut doc.spreads[0].spread;
        let mut r = spread.rectangles[0].clone();
        r.self_id = Some("minted".to_string());
        r.overprint_fill = true;
        r.overprint_stroke = true;
        spread.rectangles.push(r);
        let idx = spread.rectangles.len() - 1;
        spread.frames_in_order.push(FrameRef::Rectangle(idx));
    }
    doc.rebuild_indexes();
    let out = write_idml(&doc, &src).expect("write");
    let back = pkg::open(&out);
    let r = back.spreads[0]
        .spread
        .rectangles
        .iter()
        .find(|r| r.self_id.as_deref() == Some("minted"))
        .expect("minted rectangle");
    assert!(r.overprint_fill && r.overprint_stroke);
}

#[test]
fn an_unmutated_package_is_byte_identical() {
    let src = source();
    let doc = pkg::open(&src);
    pkg::assert_same_package(&src, &write_idml(&doc, &src).expect("write"));
}
