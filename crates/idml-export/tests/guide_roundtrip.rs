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

//! `<Guide>` save-back: inserted, moved and deleted ruler guides reach
//! the spread part (inside their `<Page>`, InDesign's placement) and read
//! back; a spread whose guides match the model round-trips byte-for-byte.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{GuideOrientation, RulerGuide};

fn guides_of(doc: &paged_scene::Document) -> Vec<(GuideOrientation, f32, u32)> {
    doc.spreads[0]
        .spread
        .guides
        .iter()
        .map(|g| (g.orientation, g.location, g.page_index))
        .collect()
}

#[test]
fn an_inserted_guide_lands_inside_its_page_and_reparses() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    assert!(doc.spreads[0].spread.guides.is_empty());
    doc.spreads[0].spread.guides.push(RulerGuide {
        orientation: GuideOrientation::Vertical,
        location: 100.0,
        page_index: 0,
    });
    doc.spreads[0].spread.guides.push(RulerGuide {
        orientation: GuideOrientation::Horizontal,
        location: 240.5,
        page_index: 0,
    });

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    // The self-closing <Page/> was expanded around its guides.
    let page_open = pkg::pos(&xml, "<Page Self=\"p1\"");
    let page_close = pkg::pos(&xml, "</Page>");
    let g1 = pkg::pos(&xml, "<Guide Self=\"Guide/s1/0\"");
    let g2 = pkg::pos(&xml, "<Guide Self=\"Guide/s1/1\"");
    assert!(page_open < g1 && g1 < g2 && g2 < page_close, "{xml}");
    assert!(
        xml.contains(r#"Orientation="Vertical" Location="100""#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"Orientation="Horizontal" Location="240.5""#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"ItemLayer="ub6""#),
        "bound to the first layer:\n{xml}"
    );
    assert!(xml.contains(r#"<GuideColor type="enumeration">Cyan</GuideColor>"#));

    let re = pkg::open(&out);
    assert_eq!(
        guides_of(&re),
        vec![
            (GuideOrientation::Vertical, 100.0, 0),
            (GuideOrientation::Horizontal, 240.5, 0)
        ]
    );
    // Second save of the reopened document: byte-identical.
    assert_eq!(out, write_idml(&re, &out).expect("re-write"));
}

const SPREAD_WITH_GUIDES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0">
<Guide Self="ue9" OverriddenPageItemProps="" Orientation="Vertical" Location="612" FitToPage="true" ViewThreshold="5" Locked="false" ItemLayer="ub6" PageIndex="0" GuideType="Ruler" GuideZone="1"><Properties><GuideColor type="enumeration">Cyan</GuideColor></Properties></Guide>
<Guide Self="ue7" Orientation="Horizontal" Location="50" PageIndex="0"/>
<Guide Self="bogus" Orientation="Diagonal" Location="1"/>
</Page>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
</Spread></idPkg:Spread>"#;

fn source_with_guides() -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", SPREAD_WITH_GUIDES),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

#[test]
fn matching_guides_round_trip_byte_identically() {
    let source = source_with_guides();
    let doc = pkg::open(&source);
    assert_eq!(
        doc.spreads[0].spread.guides.len(),
        2,
        "the bogus one is not modelled"
    );
    assert_eq!(source, write_idml(&doc, &source).expect("write"));
}

#[test]
fn a_moved_guide_is_patched_in_place_and_a_deleted_one_dropped() {
    let source = source_with_guides();
    let mut doc = pkg::open(&source);
    // Move the first, delete the second (the model addresses by position).
    doc.spreads[0].spread.guides[0].location = 300.25;
    doc.spreads[0].spread.guides.truncate(1);

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(
            r#"Self="ue9" OverriddenPageItemProps="" Orientation="Vertical" Location="300.25""#
        ),
        "patched in place, other attributes kept:\n{xml}"
    );
    assert!(
        !xml.contains("Self=\"ue7\""),
        "deleted guide dropped:\n{xml}"
    );
    assert!(
        xml.contains("Self=\"bogus\""),
        "an unmodelled guide is never touched:\n{xml}"
    );
    assert_eq!(
        guides_of(&pkg::open(&out)),
        vec![(GuideOrientation::Vertical, 300.25, 0)]
    );
}

#[test]
fn a_minted_spread_carries_its_guides() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    let mut spread = idml_import::Spread {
        self_id: Some("s2".into()),
        ..Default::default()
    };
    spread.pages.push(idml_import::Page {
        self_id: Some("p2".into()),
        bounds: idml_import::Bounds {
            top: 0.0,
            left: 0.0,
            bottom: 792.0,
            right: 612.0,
        },
        applied_master: None,
        item_transform: None,
        master_page_transform: None,
        override_list: Vec::new(),
        name: None,
        show_master_items: None,
    });
    spread.guides.push(RulerGuide {
        orientation: GuideOrientation::Vertical,
        location: 72.0,
        page_index: 0,
    });
    doc.spreads.push(paged_scene::ParsedSpread {
        src: "Spreads/Spread_s2.xml".into(),
        spread,
    });

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s2.xml").expect("minted spread");
    assert!(
        xml.contains("<Page Self=\"p2\"") && xml.contains("</Page>"),
        "{xml}"
    );
    assert!(xml.contains(r#"<Guide Self="Guide/s2/0" OverriddenPageItemProps="" Orientation="Vertical" Location="72""#), "{xml}");
    let re = pkg::open(&out);
    assert_eq!(re.spreads.len(), 2);
    assert_eq!(re.spreads[1].spread.guides.len(), 1);
    assert_eq!(re.spreads[1].spread.guides[0].location, 72.0);
}
