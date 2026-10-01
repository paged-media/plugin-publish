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

//! A forced line break (U+2028, Shift+Enter) is a LINE break inside ONE
//! paragraph, through import, re-save and minted export.
//!
//! InDesign spells it as U+2028 inside `<Content>`; its paragraph break
//! is the `<Br/>` mark. The importer used to fold U+2028 into `\n`, which
//! the engine treats as a paragraph break, so every imported forced
//! break composed as a new paragraph: the first-line indent and the
//! bullet repeated after it, and the line before it was not justified.
//! The stories below are core's `forced-line-break` paged-gen fixture's
//! own spelling (InDesign exported its reference PDF from them).

use idml_export::rewrite::rewrite_story;
use idml_import::{parse_story, CharacterRun, Paragraph, Story};

const FLB: char = '\u{2028}';

/// One paragraph with two forced breaks, followed by a real paragraph —
/// the fixture's "first-line indent" case.
fn two_breaks_story() -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<idPkg:Story xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\" DOMVersion=\"20.0\">\
<Story Self=\"udab533\">\
<ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/$ID/[No paragraph style]\" FirstLineIndent=\"24\">\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\" PointSize=\"10\">\
<Content>Alpha one{FLB}Bravo two{FLB}Charlie three</Content><Br/>\
</CharacterStyleRange></ParagraphStyleRange>\
<ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/$ID/[No paragraph style]\" FirstLineIndent=\"24\">\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\" PointSize=\"10\">\
<Content>Delta four</Content>\
</CharacterStyleRange></ParagraphStyleRange>\
</Story></idPkg:Story>"
    )
    .into_bytes()
}

fn texts(story: &Story) -> Vec<String> {
    story
        .paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .collect()
}

#[test]
fn import_keeps_a_forced_break_inside_its_paragraph() {
    let story = parse_story(&two_breaks_story()).expect("parse");
    assert_eq!(
        texts(&story),
        vec![
            format!("Alpha one{FLB}Bravo two{FLB}Charlie three"),
            "Delta four".to_string(),
        ],
        "two forced breaks stay U+2028 in ONE paragraph; a `\\n` here is a \
         paragraph break to the engine"
    );
}

#[test]
fn import_reads_the_character_reference_spelling_too() {
    let xml = String::from_utf8(two_breaks_story())
        .unwrap()
        .replace(FLB, "&#x2028;");
    let story = parse_story(xml.as_bytes()).expect("parse");
    assert_eq!(
        texts(&story)[0],
        format!("Alpha one{FLB}Bravo two{FLB}Charlie three")
    );
}

#[test]
fn a_paragraph_separator_still_reads_as_a_paragraph_break() {
    let xml = String::from_utf8(two_breaks_story())
        .unwrap()
        .replacen(FLB, "\u{2029}", 1);
    let story = parse_story(xml.as_bytes()).expect("parse");
    assert_eq!(
        texts(&story)[0],
        format!("Alpha one\nBravo two{FLB}Charlie three")
    );
}

#[test]
fn an_unmutated_save_is_byte_identical() {
    let src = two_breaks_story();
    let story = parse_story(&src).expect("parse");
    let out = rewrite_story(&src, &story).expect("rewrite");
    assert_eq!(
        String::from_utf8(out).unwrap(),
        String::from_utf8(src).unwrap()
    );
}

#[test]
fn an_edited_run_writes_the_break_verbatim_not_as_a_mark() {
    let src = two_breaks_story();
    let mut story = parse_story(&src).expect("parse");
    story.paragraphs[0].runs[0].text = format!("Alpha one{FLB}Bravo two{FLB}Charlie three!");
    let out = rewrite_story(&src, &story).expect("rewrite");
    let xml = String::from_utf8(out).unwrap();
    assert!(
        xml.contains(&format!(
            "<Content>Alpha one{FLB}Bravo two{FLB}Charlie three!</Content>"
        )),
        "the forced breaks stay inside <Content>:\n{xml}"
    );
    assert_eq!(
        xml.matches("<Br").count(),
        1,
        "only the paragraph mark is a <Br/>:\n{xml}"
    );
    let back = parse_story(xml.as_bytes()).expect("re-parse");
    assert_eq!(texts(&back), texts(&story));
}

#[test]
fn a_minted_story_round_trips_the_break() {
    let story = Story {
        paragraphs: vec![
            Paragraph {
                runs: vec![CharacterRun {
                    text: format!("Kilo eleven{FLB}{FLB}Lima twelve"),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Paragraph {
                runs: vec![CharacterRun {
                    text: "Mike thirteen".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let xml = idml_export::emit_story_part_for_test("u6b27fd", &story).expect("story_part");
    let s = String::from_utf8(xml.clone()).unwrap();
    assert_eq!(s.matches("<Br").count(), 1, "one paragraph mark:\n{s}");
    let back = parse_story(&xml).expect("re-parse");
    assert_eq!(texts(&back), texts(&story));
}
