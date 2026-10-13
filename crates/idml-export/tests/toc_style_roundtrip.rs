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

//! A story's table-of-contents style: InDesign writes `AppliedTOCStyle`
//! on the `<Story>` (`"n"` on an ordinary story). It is read into the
//! model, survives an edit of the story's text — in the lane that patches
//! the part and in the one that writes it fresh — and a story minted with
//! one names it.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story};

const TOC_STORY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
	<Story Self="st1" UserText="true" IsEndnoteStory="false" AppliedTOCStyle="TOCStyle/TOC Style" TrackChanges="false" StoryTitle="$ID/" AppliedNamedGrid="n">
		<StoryPreference OpticalMarginAlignment="false" OpticalMarginSize="12" FrameType="TextFrameType" StoryOrientation="Horizontal" StoryDirection="LeftToRightDirection" />
		<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/[No paragraph style]">
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>Contents</Content>
				<Br />
			</CharacterStyleRange>
		</ParagraphStyleRange>
		<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/[No paragraph style]">
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>Chapter one</Content>
			</CharacterStyleRange>
		</ParagraphStyleRange>
	</Story>
</idPkg:Story>"#;

const STORY_TAG: &str = r#"<Story Self="st1" UserText="true" IsEndnoteStory="false" AppliedTOCStyle="TOCStyle/TOC Style" TrackChanges="false" StoryTitle="$ID/" AppliedNamedGrid="n">"#;

fn source(story: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", story),
    ])
}

fn story_part(package: &[u8]) -> String {
    pkg::entry(package, "Stories/Story_st1.xml").expect("story part")
}

#[test]
fn the_story_s_toc_style_is_read() {
    let doc = pkg::open(&source(TOC_STORY));
    assert_eq!(
        doc.stories[0].story.applied_toc_style.as_deref(),
        Some("TOCStyle/TOC Style")
    );
    let plain = TOC_STORY.replace(
        r#"AppliedTOCStyle="TOCStyle/TOC Style""#,
        r#"AppliedTOCStyle="n""#,
    );
    assert_eq!(
        pkg::open(&source(&plain)).stories[0]
            .story
            .applied_toc_style,
        None
    );
}

#[test]
fn an_unchanged_toc_story_round_trips_byte_identically() {
    let src = source(TOC_STORY);
    let doc = pkg::open(&src);
    pkg::assert_same_entries(&src, &write_idml(&doc, &src).expect("write"));
}

#[test]
fn the_toc_style_survives_an_edit_of_the_story_s_text() {
    let src = source(TOC_STORY);
    let mut doc = pkg::open(&src);
    doc.stories[0].story.paragraphs[1].runs[0].text = "Chapter one, revised".into();
    let out = write_idml(&doc, &src).expect("write");
    let part = story_part(&out);
    assert!(part.contains("Chapter one, revised"), "{part}");
    assert!(
        part.contains(STORY_TAG),
        "the <Story> tag is kept as is: {part}"
    );
    let back = pkg::open(&out);
    assert_eq!(
        back.stories[0].story.applied_toc_style.as_deref(),
        Some("TOCStyle/TOC Style")
    );
}

/// Deleting a paragraph writes the part fresh from the model; the style
/// comes from the model then.
#[test]
fn the_toc_style_survives_a_story_written_fresh() {
    let src = source(TOC_STORY);
    let mut doc = pkg::open(&src);
    doc.stories[0].story.paragraphs.remove(0);
    let out = write_idml(&doc, &src).expect("write");
    let part = story_part(&out);
    assert!(!part.contains("Contents"), "the paragraph went: {part}");
    assert!(
        part.contains(r#"<Story Self="st1" AppliedTOCStyle="TOCStyle/TOC Style">"#),
        "{part}"
    );
    let back = pkg::open(&out);
    assert_eq!(
        back.stories[0].story.applied_toc_style.as_deref(),
        Some("TOCStyle/TOC Style")
    );
}

#[test]
fn a_changed_toc_style_is_written_onto_the_story() {
    let src = source(TOC_STORY);
    let mut doc = pkg::open(&src);
    doc.stories[0].story.applied_toc_style = Some("TOCStyle/Figures".into());
    let out = write_idml(&doc, &src).expect("write");
    assert!(
        story_part(&out).contains(
            r#"IsEndnoteStory="false" AppliedTOCStyle="TOCStyle/Figures" TrackChanges="false""#
        ),
        "in place, the other attributes untouched: {}",
        story_part(&out)
    );
    // Cleared: InDesign's spelling of none.
    doc.stories[0].story.applied_toc_style = None;
    let out = write_idml(&doc, &src).expect("write");
    assert!(
        story_part(&out).contains(r#"AppliedTOCStyle="n""#),
        "{}",
        story_part(&out)
    );
    assert_eq!(pkg::open(&out).stories[0].story.applied_toc_style, None);
    // A story without the attribute gains it.
    let bare = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&bare);
    doc.stories[0].story.applied_toc_style = Some("TOCStyle/TOC Style".into());
    let out = write_idml(&doc, &bare).expect("write");
    assert!(
        story_part(&out).contains(r#"<Story Self="st1" AppliedTOCStyle="TOCStyle/TOC Style">"#),
        "{}",
        story_part(&out)
    );
}

#[test]
fn a_minted_toc_story_names_its_style() {
    let src = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&src);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "u9".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                runs: vec![CharacterRun {
                    text: "Contents".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            applied_toc_style: Some("TOCStyle/TOC Style".into()),
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u9", "u9");
    let out = write_idml(&doc, &src).expect("write");
    let part = pkg::entry(&out, "Stories/Story_u9.xml").expect("minted part");
    assert!(
        part.contains(r#"<Story Self="u9" AppliedTOCStyle="TOCStyle/TOC Style">"#),
        "{part}"
    );
    let back = pkg::open(&out);
    let minted = back
        .stories
        .iter()
        .find(|s| s.self_id == "u9")
        .expect("minted story reads back");
    assert_eq!(
        minted.story.applied_toc_style.as_deref(),
        Some("TOCStyle/TOC Style")
    );
    // An ordinary minted story writes no attribute.
    assert!(!story_part(&out).contains("AppliedTOCStyle"));
}
