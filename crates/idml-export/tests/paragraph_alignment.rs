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

//! An empty range is a paragraph: InDesign shows a blank line for
//! `<ParagraphStyleRange><CharacterStyleRange><Br/></…>`, so the parser
//! keeps it (it used to drop it, and a model from the native part that
//! kept it then patched every range after it against the wrong
//! paragraph — the annual's DOCX-lowered story shifted its whole tail by
//! one on save). Parse space is model space now; a model with FEWER
//! paragraphs than its part maps is written fresh.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::Paragraph;

const BODY: &str = r#"<ParagraphStyleRange><CharacterStyleRange><Content>Alpha</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange><Content>Gamma</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange><Content>Delta</Content></CharacterStyleRange></ParagraphStyleRange>"#;

#[test]
fn an_empty_range_is_a_paragraph_and_the_ranges_after_it_stay_aligned() {
    let source = pkg::simple("", BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    assert_eq!(doc.stories[0].story.paragraphs.len(), 4);
    assert!(doc.stories[0].story.paragraphs[1].runs.is_empty());
    doc.stories[0].story.paragraphs[3].runs[0].text = "Delta edited".into();

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(xml.contains("<Content>Alpha</Content>"), "{xml}");
    assert!(
        xml.contains("<Content>Gamma</Content>"),
        "Gamma stays where it was:\n{xml}"
    );
    assert!(
        xml.contains("<Content>Delta edited</Content>"),
        "the edit lands on the last range:\n{xml}"
    );
    let re = pkg::open(&out);
    let texts: Vec<String> = re.stories[0]
        .story
        .paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .collect();
    assert_eq!(texts, vec!["Alpha", "", "Gamma", "Delta edited"]);
}

#[test]
fn the_unmutated_story_with_an_empty_range_is_byte_identical() {
    let source = pkg::simple("", BODY, pkg::STYLES);
    let doc = pkg::open(&source);
    pkg::assert_same_package(&source, &write_idml(&doc, &source).expect("write"));
}

#[test]
fn a_model_that_lost_a_paragraph_rewrites_its_part_from_the_model() {
    let source = pkg::simple("", BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories[0].story.paragraphs.remove(1);
    let out = write_idml(&doc, &source).expect("write");
    let re = pkg::open(&out);
    let texts: Vec<String> = re.stories[0]
        .story
        .paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .collect();
    assert_eq!(texts, vec!["Alpha", "Gamma", "Delta"]);
    let _ = Paragraph::default();
}
