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

//! `Visible` / `Locked` follow the model. A page item hidden or locked in
//! paged (`element.visible = false`, `element.locked = true`) is written
//! `Visible="false"` / `Locked="true"` on its spread element — both for an
//! item the source carried (the attribute patcher) and for one minted since
//! load (the write-new lane). Before this the source attribute passed
//! through, so InDesign showed a hidden item and let a locked one move.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::FrameRef;

const SPREAD_SPELLED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" Visible="true" Locked="false" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
<Rectangle Self="r1" Visible="false" Locked="true" GeometricBounds="500 100 600 400" ItemTransform="1 0 0 1 0 0"/>
</Spread></idPkg:Spread>"#;

fn source(spread: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", spread),
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
fn a_hidden_locked_item_is_written_when_the_source_never_said() {
    let src = source(pkg::SPREAD);
    let mut doc = pkg::open(&src);
    let f = &mut doc.spreads[0].spread.text_frames[0];
    assert!(f.visible && !f.locked);
    f.visible = false;
    f.locked = true;
    let out = write_idml(&doc, &src).expect("write");
    let spread = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let tag = start_tag(&spread, "tf1");
    assert!(tag.contains(r#"Visible="false""#), "{tag}");
    assert!(tag.contains(r#"Locked="true""#), "{tag}");
    // Reopened, the model reads what it wrote; a second save is a no-op.
    let doc2 = pkg::open(&out);
    let f2 = &doc2.spreads[0].spread.text_frames[0];
    assert!(!f2.visible && f2.locked);
    let twice = write_idml(&doc2, &out).expect("write again");
    pkg::assert_same_package(&out, &twice);
}

#[test]
fn a_spelled_state_is_patched_in_place_and_an_unchanged_one_kept() {
    let src = source(SPREAD_SPELLED);
    let doc = pkg::open(&src);
    // Unmutated: byte-identical, including the explicit defaults.
    let same = write_idml(&doc, &src).expect("write");
    pkg::assert_same_package(&src, &same);

    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.text_frames[0].visible = false;
    let r = &mut doc.spreads[0].spread.rectangles[0];
    assert!(!r.visible && r.locked);
    r.visible = true;
    r.locked = false;
    let out = write_idml(&doc, &src).expect("write");
    let spread = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let tf = start_tag(&spread, "tf1");
    assert!(
        tf.contains(r#"ParentStory="st1" Visible="false" Locked="false""#),
        "patched in place, the explicit Locked kept: {tf}"
    );
    let r1 = start_tag(&spread, "r1");
    assert!(
        !r1.contains("Visible=") && !r1.contains("Locked="),
        "back to the defaults, the non-default spellings go: {r1}"
    );
    let doc2 = pkg::open(&out);
    let r2 = &doc2.spreads[0].spread.rectangles[0];
    assert!(r2.visible && !r2.locked);
}

#[test]
fn a_minted_hidden_locked_item_carries_its_state() {
    let src = source(SPREAD_SPELLED);
    let mut doc = pkg::open(&src);
    {
        let spread = &mut doc.spreads[0].spread;
        let mut r = spread.rectangles[0].clone();
        r.self_id = Some("minted".to_string());
        r.visible = false;
        r.locked = true;
        spread.rectangles.push(r);
        let idx = spread.rectangles.len() - 1;
        spread.frames_in_order.push(FrameRef::Rectangle(idx));
    }
    doc.rebuild_indexes();
    let out = write_idml(&doc, &src).expect("write");
    let spread = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let tag = start_tag(&spread, "minted");
    assert!(tag.contains(r#"Visible="false""#), "{tag}");
    assert!(tag.contains(r#"Locked="true""#), "{tag}");
    let doc2 = pkg::open(&out);
    let r2 = doc2.spreads[0]
        .spread
        .rectangles
        .iter()
        .find(|r| r.self_id.as_deref() == Some("minted"))
        .expect("minted reread");
    assert!(!r2.visible && r2.locked);
}
