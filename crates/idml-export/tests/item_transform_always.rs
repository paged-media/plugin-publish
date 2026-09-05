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

//! Every page item, spread and page is written WITH an `ItemTransform`,
//! `1 0 0 1 0 0` when the model's is identity. Measured on InDesign
//! 20.0.1: an item without the attribute is not read as identity — it
//! lands one page width to the left on a facing spread (a book's 1438
//! transform-less items put whole versos on the pasteboard); the same
//! file with the identity spelled on each placed every item correctly.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, FrameRef, Paragraph, Story};

const IDENTITY: &str = r#"ItemTransform="1 0 0 1 0 0""#;

/// The one-spread fixture with a checkpoint-era rectangle and text
/// frame that carry NO `ItemTransform` (the exporter used to keep an
/// identity transform absent), and a group without one.
const SPREAD_NO_TRANSFORMS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400"/>
<Rectangle Self="r1" GeometricBounds="450 100 600 300" FillColor="Color/Black"/>
<Group Self="g1"><Oval Self="o1" GeometricBounds="10 10 20 20"/></Group>
</Spread></idPkg:Spread>"#;

/// The fixture spread plus one rectangle, every transform spelled (as
/// every InDesign package spells them).
const SPREAD_WITH_TRANSFORMS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1" ItemTransform="1 0 0 1 0 0">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
<Rectangle Self="r1" GeometricBounds="450 100 600 300" ItemTransform="1 0 0 1 0 0" FillColor="Color/Black"/>
</Spread></idPkg:Spread>"#;

fn source(spread: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

#[test]
fn a_minted_frame_with_an_identity_transform_exports_with_the_attribute() {
    let src = source(SPREAD_WITH_TRANSFORMS);
    let mut doc = pkg::open(&src);
    // A minted text frame (its story placed) and a minted rectangle,
    // both with the model's `None` transform.
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                runs: vec![CharacterRun {
                    text: "minted".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u9", "Story/u9");
    let spread = &mut doc.spreads[0].spread;
    spread.text_frames.last_mut().unwrap().item_transform = None;
    let mut r = spread.rectangles[0].clone();
    r.self_id = Some("r_new".into());
    r.bounds = idml_import::Bounds {
        top: 10.0,
        left: 10.0,
        bottom: 50.0,
        right: 90.0,
    };
    r.item_transform = None;
    spread.rectangles.push(r);
    let idx = spread.rectangles.len() - 1;
    spread.frames_in_order.push(FrameRef::Rectangle(idx));
    doc.rebuild_indexes();

    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    for id in ["tf_u9", "r_new"] {
        let at = pkg::pos(&xml, &format!(r#"Self="{id}""#));
        let tag_end = xml[at..].find('>').map(|e| at + e).unwrap();
        assert!(
            xml[at..tag_end].contains(IDENTITY),
            "{id} must spell the identity transform:\n{}",
            &xml[at..tag_end]
        );
    }
    let re = pkg::open(&out);
    let tf = re.spreads[0]
        .spread
        .text_frames
        .iter()
        .find(|f| f.self_id.as_deref() == Some("tf_u9"))
        .expect("tf_u9");
    assert_eq!(tf.item_transform, Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
}

#[test]
fn a_source_item_without_a_transform_gains_the_identity_on_an_unmutated_save() {
    let src = source(SPREAD_NO_TRANSFORMS);
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<Rectangle Self="r1" GeometricBounds="450 100 600 300" FillColor="Color/Black" ItemTransform="1 0 0 1 0 0"/>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<Group Self="g1" ItemTransform="1 0 0 1 0 0"><Oval Self="o1" GeometricBounds="10 10 20 20" ItemTransform="1 0 0 1 0 0"/></Group>"#),
        "{xml}"
    );
    assert_eq!(xml.matches(IDENTITY).count(), 5, "page + 4 items:\n{xml}");
    // Everything else is untouched, and the fixed part is stable.
    for name in pkg::names(&src) {
        if name != "Spreads/Spread_s1.xml" {
            assert_eq!(pkg::entry(&src, &name), pkg::entry(&out, &name), "{name}");
        }
    }
    let re = pkg::open(&out);
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

#[test]
fn a_source_that_spells_every_transform_round_trips_byte_identically() {
    // The fixture spread carries `ItemTransform="1 0 0 1 0 0"` on its
    // page and frame, as every InDesign package does.
    let src = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let doc = pkg::open(&src);
    assert_eq!(write_idml(&doc, &src).expect("write"), src);
}

#[test]
fn a_minted_spread_spells_the_transform_on_spread_page_and_items() {
    let src = source(SPREAD_WITH_TRANSFORMS);
    let mut doc = pkg::open(&src);
    let mut spread = doc.spreads[0].spread.clone();
    spread.self_id = Some("s2".into());
    spread.item_transform = None;
    spread.text_frames.clear();
    spread.frames_in_order.clear();
    spread.pages[0].self_id = Some("p2".into());
    spread.pages[0].item_transform = None;
    let mut r = spread.rectangles[0].clone();
    r.self_id = Some("r2".into());
    r.item_transform = None;
    spread.rectangles = vec![r];
    spread.frames_in_order.push(FrameRef::Rectangle(0));
    doc.spreads.push(paged_scene::ParsedSpread {
        src: "Spreads/Spread_s2.xml".into(),
        spread,
    });
    doc.designmap.spreads.push(idml_import::SpreadRef {
        src: "Spreads/Spread_s2.xml".into(),
    });
    doc.rebuild_indexes();
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s2.xml").expect("minted spread");
    assert!(
        xml.contains(r#"<Spread Self="s2" ItemTransform="1 0 0 1 0 0""#),
        "{xml}"
    );
    assert!(
        xml.contains(
            r#"<Page Self="p2" Name="1" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 792 612""#
        ),
        "{xml}"
    );
    let at = pkg::pos(&xml, r#"Self="r2""#);
    let tag_end = xml[at..].find('>').map(|e| at + e).unwrap();
    assert!(xml[at..tag_end].contains(IDENTITY), "{}", &xml[at..tag_end]);
}
