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

//! `ItemLayer` follows the model: an item the engine put on a layer is
//! written there even when its source element never said (every
//! engine-minted item — measured 2026-09-05 on the annual: 0 of 1932
//! items carried the attribute while 1519 had a layer in the model, so
//! InDesign opened the whole book on one layer), a layer the source
//! spells is kept, and a layer is never removed.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const SPREAD_WITH_LAYER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" ItemLayer="ub6" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
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
fn a_layer_the_model_names_is_written_when_the_source_never_said() {
    let src = source(pkg::SPREAD);
    let mut doc = pkg::open(&src);
    assert_eq!(doc.spreads[0].spread.text_frames[0].item_layer, None);
    doc.spreads[0].spread.text_frames[0].item_layer = Some("ub6".to_string());
    let out = write_idml(&doc, &src).expect("write");
    let spread = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread part");
    assert!(
        spread.contains(r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0" ItemLayer="ub6"/>"#),
        "{spread}"
    );
    // Now the source's own word: reopening reads it, and a second save
    // changes nothing.
    let doc2 = pkg::open(&out);
    assert_eq!(
        doc2.spreads[0].spread.text_frames[0].item_layer.as_deref(),
        Some("ub6")
    );
    let twice = write_idml(&doc2, &out).expect("write again");
    pkg::assert_same_package(&out, &twice);
}

#[test]
fn a_layer_the_source_spells_is_kept_and_never_removed() {
    let src = source(SPREAD_WITH_LAYER);
    let mut doc = pkg::open(&src);
    assert_eq!(
        doc.spreads[0].spread.text_frames[0].item_layer.as_deref(),
        Some("ub6")
    );
    let same = write_idml(&doc, &src).expect("write");
    pkg::assert_same_package(&src, &same);
    // A model that names no layer does not unset the source's.
    doc.spreads[0].spread.text_frames[0].item_layer = None;
    let kept = write_idml(&doc, &src).expect("write");
    pkg::assert_same_package(&src, &kept);
}

#[test]
fn a_layer_the_model_moved_patches_in_place() {
    let src = source(SPREAD_WITH_LAYER);
    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.text_frames[0].item_layer = Some("u203189".to_string());
    let out = write_idml(&doc, &src).expect("write");
    let spread = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread part");
    assert!(
        spread.contains(r#"<TextFrame Self="tf1" ParentStory="st1" ItemLayer="u203189" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#),
        "{spread}"
    );
}
