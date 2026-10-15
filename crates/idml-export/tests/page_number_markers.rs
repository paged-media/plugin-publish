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

//! The next and previous page numbers survive an export.
//!
//! InDesign 20.0.1 spells all three page numbers as the same `<?ACE 18?>`
//! and names the flavour on the RANGE: a next page number sits in a
//! `<CharacterStyleRange PageNumberType="NextPageNumber">` of its own, a
//! previous one in `"PreviousPageNumber"` (see `text_variable_roundtrip.rs`
//! for its own file). The writer wrote every marker as a bare `<?ACE 18?>`
//! in whatever range the run had, so a next page number came back as the
//! current one; and a part written that way was replayed verbatim on
//! every later export, because a run carrying a marker was never
//! re-serialised.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::rewrite::rewrite_story;
use idml_export::write_idml;
use idml_import::{
    parse_story, CharacterRun, Paragraph, Story, AUTO_PAGE_NUMBER_MARKER, NEXT_PAGE_NUMBER_MARKER,
    PREVIOUS_PAGE_NUMBER_MARKER, SECTION_MARKER,
};

const NEXT: char = NEXT_PAGE_NUMBER_MARKER;
const PREV: char = PREVIOUS_PAGE_NUMBER_MARKER;
const AUTO: char = AUTO_PAGE_NUMBER_MARKER;

fn text_of(story: &Story) -> Vec<String> {
    story
        .paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .collect()
}

fn story_xml(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><Story Self="u242">{body}</Story></idPkg:Story>"#
    )
}

/// A story minted in the editor: every flavour of page number in one run.
#[test]
fn a_minted_story_writes_next_and_previous_page_numbers_in_their_own_ranges() {
    let text = format!(
        "Continued on page {NEXT}, from page {PREV}; this is page {AUTO} of {SECTION_MARKER}."
    );
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u242".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                runs: vec![CharacterRun {
                    text: text.clone(),
                    point_size: Some(9.0),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u242", "Story/u242");

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_Story_u242.xml").expect("minted story");
    assert!(
        xml.contains(r#"<CharacterStyleRange PointSize="9" PageNumberType="NextPageNumber"><Content><?ACE 18?></Content></CharacterStyleRange>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<CharacterStyleRange PointSize="9" PageNumberType="PreviousPageNumber"><Content><?ACE 18?></Content></CharacterStyleRange>"#),
        "{xml}"
    );
    assert!(
        xml.contains(
            "<Content>; this is page <?ACE 18?></Content><Content> of <?ACE 19?></Content>"
        ),
        "the current page number and the section marker stay in plain ranges:\n{xml}"
    );
    let re = pkg::open(&out);
    let story = re
        .stories
        .iter()
        .find(|s| s.self_id == "Story_u242")
        .expect("story back");
    assert_eq!(text_of(&story.story), vec![text]);
    assert!(story.story.paragraphs[0]
        .runs
        .iter()
        .all(|r| r.point_size == Some(9.0)));
    // A second save of the reopened package is byte-identical.
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

/// The editor inserts a next page number into a run the part already
/// has: the run is re-serialised and the marker gets its range.
#[test]
fn a_next_page_number_typed_into_a_source_run_is_written_as_one() {
    let xml = story_xml(
        r#"<ParagraphStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" Tracking="5"><Content>Continued on page </Content></CharacterStyleRange></ParagraphStyleRange>"#,
    );
    let mut story = parse_story(xml.as_bytes()).expect("parse");
    story.paragraphs[0].runs[0].text = format!("Continued on page {NEXT}, see there");
    let want = text_of(&story);
    let out = String::from_utf8(rewrite_story(xml.as_bytes(), &story).expect("rewrite")).unwrap();
    assert!(
        out.contains(r#"<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" Tracking="5"><Content>Continued on page </Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" Tracking="5" PageNumberType="NextPageNumber"><Content><?ACE 18?></Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" Tracking="5"><Content>, see there</Content></CharacterStyleRange>"#),
        "{out}"
    );
    assert_eq!(
        text_of(&parse_story(out.as_bytes()).expect("re-parse")),
        want
    );
}

/// A part an older writer left with a bare `<?ACE 18?>` where the model
/// holds a next page number (a `.paged` checkpoint reloaded from its
/// native model): the run is re-serialised from the model instead of
/// replayed, so the export says what the model says.
#[test]
fn a_bare_marker_an_older_export_wrote_is_corrected_from_the_model() {
    let stale = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body Small"><CharacterStyleRange><Content>Continued on page <?ACE 18?></Content><Content> — said every jump line ever printed; this one is set on page <?ACE 18?></Content><Content>.</Content></CharacterStyleRange></ParagraphStyleRange>"#;
    let source = pkg::simple("", stale, pkg::STYLES);
    let mut doc = pkg::open(&source);
    let run = &mut doc.stories[0].story.paragraphs[0].runs[0];
    assert_eq!(run.text.matches(AUTO).count(), 2, "{:?}", run.text);
    // The model's own account: the first is the NEXT page number.
    run.text = run.text.replacen(AUTO, &NEXT.to_string(), 1);
    let want = text_of(&doc.stories[0].story);

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(
        xml.contains(r#"<CharacterStyleRange PageNumberType="NextPageNumber"><Content><?ACE 18?></Content></CharacterStyleRange>"#),
        "{xml}"
    );
    let re = pkg::open(&out);
    assert_eq!(text_of(&re.stories[0].story), want);
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

/// InDesign's own range, the marker changed in the model: the range
/// keeps its attributes and the flavour follows the model — to the other
/// flavour, and to the current page number.
#[test]
fn indesigns_next_page_range_follows_the_model_when_the_marker_changes() {
    let xml = story_xml(
        r#"<ParagraphStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Content>next=[</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PageNumberType="NextPageNumber"><Content><?ACE 18?></Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Content>]</Content></CharacterStyleRange></ParagraphStyleRange>"#,
    );
    let parsed = parse_story(xml.as_bytes()).expect("parse");
    assert_eq!(text_of(&parsed), vec![format!("next=[{NEXT}]")]);
    // Unchanged: byte-identical.
    assert_eq!(
        String::from_utf8(rewrite_story(xml.as_bytes(), &parsed).unwrap()).unwrap(),
        xml
    );
    for (marker, label) in [(PREV, "previous"), (AUTO, "current")] {
        let mut story = parsed.clone();
        story.paragraphs[0].runs[1].text = marker.to_string();
        let want = text_of(&story);
        let out = rewrite_story(xml.as_bytes(), &story).expect("rewrite");
        let back = parse_story(&out).expect("re-parse");
        assert_eq!(
            text_of(&back),
            want,
            "a {label} page number:\n{}",
            String::from_utf8_lossy(&out)
        );
    }
    // A next page number with text typed after it: the text does not
    // stay in the next-page range.
    let mut story = parsed.clone();
    story.paragraphs[0].runs[1].text = format!("{NEXT} (over)");
    let out = String::from_utf8(rewrite_story(xml.as_bytes(), &story).unwrap()).unwrap();
    assert!(
        out.contains(r#"PageNumberType="NextPageNumber"><Content><?ACE 18?></Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Content> (over)</Content></CharacterStyleRange>"#),
        "{out}"
    );
    assert_eq!(
        text_of(&parse_story(out.as_bytes()).unwrap()),
        text_of(&story)
    );
}

/// A page number opening a paragraph after a mark, inside one range: the
/// mark stays before it.
#[test]
fn a_next_page_number_after_a_paragraph_mark_stays_in_its_paragraph() {
    let xml = story_xml(
        r#"<ParagraphStyleRange><CharacterStyleRange><Content>Jump</Content></CharacterStyleRange></ParagraphStyleRange>"#,
    );
    let mut story = parse_story(xml.as_bytes()).expect("parse");
    story.paragraphs[0].runs[0].text = format!("Jump\n{NEXT}\tcontinued");
    let want = text_of(&story);
    let out = rewrite_story(xml.as_bytes(), &story).expect("rewrite");
    assert_eq!(
        text_of(&parse_story(&out).expect("re-parse")),
        want,
        "{}",
        String::from_utf8_lossy(&out)
    );
}
