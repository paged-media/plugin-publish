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

//! A MINTED story must carry paragraph marks InDesign can read.
//!
//! # The measurement came first
//!
//! A 134-page document exported from the editor was opened in real
//! InDesign 2026 and re-exported. Every multi-paragraph story came back
//! as ONE paragraph: an eleven-entry table of contents re-exported as
//!
//! ```text
//! <Content>Front matteriPart I divider · Ch.1 Anatomy of This Book1Ch.2 …</Content>
//! ```
//!
//! with `AppliedParagraphStyle="ParagraphStyle/$ID/[No paragraph style]"`.
//! The entries, their tabs and their style were all gone.
//!
//! # Why our own round trip could never catch it
//!
//! `emit::story_part` writes one `<ParagraphStyleRange>` per model
//! paragraph and no paragraph mark, because in our model the break is
//! STRUCTURAL — it is not a character in any run, so `write_run_content`
//! never emits the `<Br/>` that would spell it.
//!
//! Our own importer then rebuilds paragraphs from the
//! `<ParagraphStyleRange>` boundaries, so the file round-trips through
//! us perfectly. It is a private convention that agrees with itself.
//!
//! InDesign does not read it that way, and it is the reference
//! implementation. In IDML a `<ParagraphStyleRange>` is a STYLE run over
//! one or more paragraphs, and the paragraph mark is the `<Br/>`
//! character. A real InDesign-authored file from the corpus spells it
//! exactly so — every paragraph's content ends with `<Br />`, and only
//! the story's last paragraph omits it:
//!
//! ```text
//! <ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Title">
//!   <CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Thin">
//!     <Content>1</Content>
//!     <Br />
//!   </CharacterStyleRange>
//! </ParagraphStyleRange>
//! ```
//!
//! # What this test pins
//!
//! 1. A minted multi-paragraph story emits a `<Br/>` terminator for
//!    every paragraph except the last — the Adobe spelling.
//! 2. Re-importing it yields the SAME paragraph count and the SAME run
//!    text, with no newline smuggled into the text by the mark we just
//!    added. Our reader treats `<Br/>` as a newline inside the current
//!    run, so a terminator that lands in the text would trade an
//!    InDesign bug for a rendering one.

use idml_import::Paragraph;
use idml_import::{parse_story, CharacterRun, Story};

fn para(style: &str, text: &str) -> Paragraph {
    Paragraph {
        paragraph_style: Some(style.to_string()),
        runs: vec![CharacterRun {
            text: text.to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn three_paragraph_story() -> Story {
    Story {
        paragraphs: vec![
            para("ParagraphStyle/TOC Chapter", "Front matter"),
            para("ParagraphStyle/TOC Chapter", "Ch.2 The Letter"),
            para("ParagraphStyle/TOC Chapter", "Ch.3 The Paragraph"),
        ],
        ..Default::default()
    }
}

#[test]
fn a_minted_story_spells_its_paragraph_marks() {
    let xml =
        idml_export::emit_story_part_for_test("u43", &three_paragraph_story()).expect("story_part");
    let s = String::from_utf8(xml).expect("utf8");

    // Two marks for three paragraphs: the last needs none, exactly as
    // InDesign writes it.
    assert_eq!(
        s.matches("<Br").count(),
        2,
        "a three-paragraph story must carry two paragraph marks; \
         without them InDesign reads the whole story as ONE paragraph. \
         Emitted:\n{s}"
    );

    // The mark belongs INSIDE the character run, at the end of the
    // paragraph's content — not between the ParagraphStyleRanges.
    assert!(
        s.contains("<Content>Front matter</Content><Br/></CharacterStyleRange>")
            || s.contains("<Content>Front matter</Content><Br /></CharacterStyleRange>"),
        "the mark must close the paragraph's own character run. Emitted:\n{s}"
    );
}

#[test]
fn the_mark_does_not_leak_into_the_text() {
    let xml =
        idml_export::emit_story_part_for_test("u43", &three_paragraph_story()).expect("story_part");
    let back = parse_story(&xml).expect("re-import the minted story");

    assert_eq!(
        back.paragraphs.len(),
        3,
        "the round trip must preserve the paragraph count"
    );
    let texts: Vec<String> = back
        .paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .collect();
    assert_eq!(
        texts,
        vec![
            "Front matter".to_string(),
            "Ch.2 The Letter".to_string(),
            "Ch.3 The Paragraph".to_string(),
        ],
        "the paragraph mark must not survive as a newline in the run text — \
         that would trade an interchange defect for a composition one"
    );
}
