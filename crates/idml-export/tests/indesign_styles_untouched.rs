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

//! A text edit leaves an InDesign-written `Resources/Styles.xml` as it
//! came.
//!
//! The fixture is the part InDesign 20.0.1 wrote for a one-page document
//! (45 paragraph styles, every one in InDesign's own spelling). Editing a
//! headline and saving used to rewrite it: the paragraph style "docx
//! direct format 9" lost its `<TabList>` (a right tab at 216 pt with a
//! "." leader). The style reader only knew the generator's `<TabStop
//! Position=…/>` spelling, so the model's style had no stops, and the
//! export settled the source child against that empty list — and dropped
//! it. An untouched part must go back byte for byte.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const STYLES: &str = include_str!("fixtures/indesign-20.0.1-styles.xml");

/// A story in one of the fixture's own styles.
const BODY: &str = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Chapter Title"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Content>The Picture</Content></CharacterStyleRange></ParagraphStyleRange>"#;

fn source() -> Vec<u8> {
    pkg::simple("", BODY, STYLES)
}

#[test]
fn the_model_reads_the_style_s_tab_stops() {
    let doc = pkg::open(&source());
    let style = &doc.styles.paragraph_styles["ParagraphStyle/docx direct format 9"];
    assert_eq!(style.tab_list.len(), 1, "{:?}", style.tab_list);
    let stop = &style.tab_list[0];
    assert_eq!(stop.position, 216.0);
    assert_eq!(stop.alignment.as_deref(), Some("RightAlign"));
    assert_eq!(stop.leader.as_deref(), Some("."));
}

#[test]
fn an_unmutated_save_leaves_the_styles_part_byte_identical() {
    let src = source();
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    let styles = pkg::entry(&out, "Resources/Styles.xml").expect("styles part");
    assert!(styles == STYLES, "Resources/Styles.xml was rewritten");
}

#[test]
fn a_text_edit_rewrites_the_story_and_leaves_the_styles_part_untouched() {
    let src = source();
    let mut doc = pkg::open(&src);
    doc.stories[0].story.paragraphs[0].runs[0].text = "The Photograph".to_string();
    let out = write_idml(&doc, &src).expect("write");

    let story = pkg::entry(&out, "Stories/Story_st1.xml").expect("story part");
    assert!(
        story.contains("<Content>The Photograph</Content>"),
        "{story}"
    );

    let styles = pkg::entry(&out, "Resources/Styles.xml").expect("styles part");
    assert!(
        styles.contains(
            "<TabList type=\"list\">\n\t\t\t\t\t<ListItem type=\"record\">\n\t\t\t\t\t\t<Alignment type=\"enumeration\">RightAlign</Alignment>\n\t\t\t\t\t\t<AlignmentCharacter type=\"string\">.</AlignmentCharacter>\n\t\t\t\t\t\t<Leader type=\"string\">.</Leader>\n\t\t\t\t\t\t<Position type=\"unit\">216</Position>"
        ),
        "the style's TabList was lost"
    );
    assert!(styles == STYLES, "Resources/Styles.xml was rewritten");
}
