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

//! A run's conditions survive an export: `AppliedConditions` on its
//! `<CharacterStyleRange>`, the ids space-separated, as InDesign writes
//! them. Neither story lane wrote them, so every run lost its conditions
//! — text a hidden condition hides came back showing.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story};

fn conds(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn source_story(doc: &paged_scene::Document) -> &Story {
    &doc.stories
        .iter()
        .find(|s| s.self_id.ends_with("st1"))
        .expect("st1")
        .story
}

/// A story the export mints.
#[test]
fn a_minted_runs_conditions_come_back() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                runs: vec![
                    CharacterRun {
                        text: "plain ".into(),
                        ..Default::default()
                    },
                    CharacterRun {
                        text: "spec note".into(),
                        applied_conditions: conds(&["Condition/Notes", "Condition/Draft"]),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u9", "Story/u9");
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_Story_u9.xml").expect("part");
    assert!(
        xml.contains(r#"AppliedConditions="Condition/Notes Condition/Draft""#),
        "{xml}"
    );
    let back = pkg::open(&out);
    let story = &back
        .stories
        .iter()
        .find(|s| s.self_id.ends_with("u9"))
        .unwrap()
        .story;
    let runs = &story.paragraphs[0].runs;
    assert!(runs[0].applied_conditions.is_empty());
    assert_eq!(
        runs[1].applied_conditions,
        conds(&["Condition/Notes", "Condition/Draft"])
    );
}

/// A source run the model gives a condition gains the attribute; one
/// that already spells the model's keeps its bytes; one cleared loses it.
#[test]
fn a_source_runs_conditions_follow_the_model() {
    let body = r#"<ParagraphStyleRange><CharacterStyleRange><Content>Hello world</Content></CharacterStyleRange></ParagraphStyleRange>"#;
    let source = pkg::simple("", body, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories[0].story.paragraphs[0].runs[0].applied_conditions = conds(&["Condition/Notes"]);
    let gained = write_idml(&doc, &source).expect("write");
    let back = pkg::open(&gained);
    assert_eq!(
        source_story(&back).paragraphs[0].runs[0].applied_conditions,
        conds(&["Condition/Notes"])
    );
    // Unchanged: byte-identical.
    pkg::assert_same_package(&gained, &write_idml(&back, &gained).expect("again"));
    // Cleared.
    let mut cleared = pkg::open(&gained);
    cleared.stories[0].story.paragraphs[0].runs[0]
        .applied_conditions
        .clear();
    let out = write_idml(&cleared, &gained).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(!xml.contains("AppliedConditions"), "{xml}");
}

/// InDesign's own spelling, unedited, is passed through.
#[test]
fn an_unedited_indesign_range_keeps_its_bytes() {
    let body = r#"<ParagraphStyleRange><CharacterStyleRange AppliedConditions="Condition/A  Condition/B"><Content>Hello world</Content></CharacterStyleRange></ParagraphStyleRange>"#;
    let source = pkg::simple("", body, pkg::STYLES);
    let doc = pkg::open(&source);
    assert_eq!(
        source_story(&doc).paragraphs[0].runs[0].applied_conditions,
        conds(&["Condition/A", "Condition/B"])
    );
    pkg::assert_same_package(&source, &write_idml(&doc, &source).expect("write"));
}
