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

//! A story no frame references is an ORPHAN: InDesign 20.0.1 discards
//! it on open (measured), so the writer drops its part and its designmap
//! reference rather than shipping a story the reader will not show. A
//! source whose every story is placed is untouched; a story reachable
//! only through an anchored frame inside a placed story is placed.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::{placed_story_ids, story_is_placed, write_idml};
use idml_import::{CharacterRun, Paragraph, Story};

#[test]
fn a_source_whose_every_story_is_placed_round_trips_byte_identically() {
    let src = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let doc = pkg::open(&src);
    assert_eq!(write_idml(&doc, &src).expect("write"), src);
}

#[test]
fn a_minted_story_whose_frame_was_deleted_reaches_no_part_and_no_reference() {
    let src = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&src);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                runs: vec![CharacterRun {
                    text: "kept after its frame went".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    // Placed: the part is written and referenced.
    pkg::place_story(&mut doc, "tf_u9", "Story/u9");
    let placed = write_idml(&doc, &src).expect("write");
    assert!(pkg::entry(&placed, "Stories/Story_Story_u9.xml").is_some());
    assert!(pkg::entry(&placed, "designmap.xml")
        .unwrap()
        .contains(r#"<idPkg:Story src="Stories/Story_Story_u9.xml"/>"#));

    // The frame is deleted; the engine keeps the story.
    let spread = &mut doc.spreads[0].spread;
    let idx = spread.text_frames.len() - 1;
    spread.text_frames.remove(idx);
    spread
        .frames_in_order
        .retain(|f| !matches!(f, idml_import::FrameRef::TextFrame(i) if *i == idx));
    doc.rebuild_indexes();
    let ids = placed_story_ids(&doc);
    assert!(story_is_placed(&ids, "st1"));
    assert!(!story_is_placed(&ids, "Story/u9"));

    let out = write_idml(&doc, &src).expect("write");
    assert!(
        pkg::entry(&out, "Stories/Story_Story_u9.xml").is_none(),
        "no part for the orphan: {:?}",
        pkg::names(&out)
    );
    assert!(
        !pkg::entry(&out, "designmap.xml")
            .unwrap()
            .contains("Story_Story_u9"),
        "no designmap reference either"
    );
    // The rest of the package is the source: byte-identical.
    pkg::assert_same_package(&src, &out);
}

#[test]
fn a_source_story_whose_frame_was_deleted_is_dropped_with_its_reference() {
    let src = pkg::package(&[
        (
            "designmap.xml",
            &pkg::designmap(
                r#"<idPkg:Story src="Stories/Story_st2.xml"/>
"#,
            ),
        ),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
        (
            "Stories/Story_st2.xml",
            &pkg::story(pkg::STORY_BODY).replace("Self=\"st1\"", "Self=\"st2\""),
        ),
    ]);
    let doc = pkg::open(&src);
    assert_eq!(doc.stories.len(), 2);
    let out = write_idml(&doc, &src).expect("write");
    assert!(pkg::entry(&out, "Stories/Story_st2.xml").is_none());
    let dm = pkg::entry(&out, "designmap.xml").unwrap();
    assert!(dm.contains(r#"<idPkg:Story src="Stories/Story_st1.xml"/>"#));
    assert!(!dm.contains("Story_st2"), "{dm}");
    assert_eq!(pkg::open(&out).stories.len(), 1);
}

#[test]
fn a_story_hosted_by_an_anchored_frame_in_a_placed_story_is_placed() {
    let body = r#"<ParagraphStyleRange><CharacterStyleRange><Content>Host </Content></CharacterStyleRange><CharacterStyleRange><TextFrame Self="atf" ParentStory="st2" GeometricBounds="0 0 50 100" ItemTransform="1 0 0 1 0 0"><AnchoredObjectSetting AnchoredPosition="InlinePosition"/></TextFrame></CharacterStyleRange></ParagraphStyleRange>"#;
    let src = pkg::package(&[
        (
            "designmap.xml",
            &pkg::designmap(
                r#"<idPkg:Story src="Stories/Story_st2.xml"/>
"#,
            ),
        ),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(body)),
        (
            "Stories/Story_st2.xml",
            &pkg::story(pkg::STORY_BODY).replace("Self=\"st1\"", "Self=\"st2\""),
        ),
    ]);
    let doc = pkg::open(&src);
    let host = doc.stories.iter().find(|s| s.self_id == "st1").unwrap();
    assert_eq!(host.story.paragraphs[0].anchored_frames.len(), 1);
    let ids = placed_story_ids(&doc);
    assert!(story_is_placed(&ids, "st2"), "{ids:?}");
    assert_eq!(
        write_idml(&doc, &src).expect("write"),
        src,
        "byte-identical"
    );
}
