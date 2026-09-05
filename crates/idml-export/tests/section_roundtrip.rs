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

//! `<Section>` save-back: a section the model added, edited or deleted
//! reaches `designmap.xml` in InDesign's spelling (measured on a 20.0.1
//! export: after the spread includes, `PageNumberStyle` as a typed
//! Properties child), and a canonical source round-trips byte-for-byte.
//!
//! A 134-page book authored through the engine had 3 runtime sections
//! (folios i–x); its export had `<Section>` 0.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{NumberingStyle, Section};

const CANONICAL_SECTION: &str = r#"<Section Self="sec1" Length="1" Name="" ContinueNumbering="false" IncludeSectionPrefix="true" PageNumberStart="3" Marker="" PageStart="p1" SectionPrefix="A-"><Properties><PageNumberStyle type="enumeration">LowerRoman</PageNumberStyle></Properties></Section>
"#;

#[test]
fn a_canonical_section_round_trips_byte_identically() {
    let source = pkg::simple(CANONICAL_SECTION, pkg::STORY_BODY, pkg::STYLES);
    let doc = pkg::open(&source);
    let sec = &doc.designmap.sections[0];
    assert_eq!(
        sec.numbering_style,
        NumberingStyle::LowerRoman,
        "the typed child is read"
    );
    assert_eq!(sec.start_at, Some(3));
    assert_eq!(sec.section_prefix.as_deref(), Some("A-"));
    assert!(sec.include_prefix);
    let out = write_idml(&doc, &source).expect("write");
    assert_eq!(
        source, out,
        "unmutated canonical source must be byte-identical"
    );
}

#[test]
fn an_inserted_section_is_written_after_the_spread_includes_and_reparses() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    assert!(doc.designmap.sections.is_empty());
    doc.designmap.sections.push(Section {
        self_id: "Section/u0".into(),
        page_start: Some("p1".into()),
        continue_numbering: false,
        start_at: Some(3),
        numbering_style: NumberingStyle::LowerRoman,
        section_prefix: Some("A-".into()),
        marker: None,
        include_prefix: true,
    });

    let out = write_idml(&doc, &source).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    let spread_ref = pkg::pos(&dm, "<idPkg:Spread ");
    let section = pkg::pos(&dm, "<Section Self=\"Section/u0\"");
    let story_ref = pkg::pos(&dm, "<idPkg:Story ");
    assert!(
        spread_ref < section && section < story_ref,
        "the section sits after the spread includes:\n{dm}"
    );
    assert!(
        dm.contains(r#"<Properties><PageNumberStyle type="enumeration">LowerRoman</PageNumberStyle></Properties>"#),
        "the numbering style is a typed Properties child:\n{dm}"
    );
    assert!(dm.contains(r#"PageStart="p1""#) && dm.contains(r#"SectionPrefix="A-""#));
    assert!(
        dm.contains(r#"Length="1""#),
        "one page in the section:\n{dm}"
    );

    let re = pkg::open(&out);
    let sec = &re.designmap.sections[0];
    assert_eq!(sec.self_id, "Section/u0");
    assert_eq!(sec.page_start.as_deref(), Some("p1"));
    assert_eq!(sec.numbering_style, NumberingStyle::LowerRoman);
    assert_eq!(sec.start_at, Some(3));
    assert_eq!(sec.section_prefix.as_deref(), Some("A-"));
    assert!(sec.include_prefix);
}

#[test]
fn an_edited_section_is_patched_in_place_and_a_deleted_one_dropped() {
    let two = format!(
        "{CANONICAL_SECTION}<Section Self=\"sec2\" Length=\"1\" Name=\"\" ContinueNumbering=\"true\" IncludeSectionPrefix=\"false\" Marker=\"\" PageStart=\"p1\" SectionPrefix=\"\"><Properties><PageNumberStyle type=\"enumeration\">Arabic</PageNumberStyle></Properties></Section>\n"
    );
    let source = pkg::simple(&two, pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    assert_eq!(doc.designmap.sections.len(), 2);
    // Edit sec1: Arabic from 7, no prefix. Delete sec2.
    {
        let s = &mut doc.designmap.sections[0];
        s.numbering_style = NumberingStyle::Arabic;
        s.start_at = Some(7);
        s.section_prefix = None;
        s.include_prefix = false;
    }
    doc.designmap.sections.truncate(1);

    let out = write_idml(&doc, &source).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert!(!dm.contains("sec2"), "the deleted section is gone:\n{dm}");
    assert!(
        dm.contains(r#"<PageNumberStyle type="enumeration">Arabic</PageNumberStyle>"#),
        "the child text is patched:\n{dm}"
    );
    assert!(dm.contains(r#"PageNumberStart="7""#));
    let re = pkg::open(&out);
    assert_eq!(re.designmap.sections.len(), 1);
    let sec = &re.designmap.sections[0];
    assert_eq!(sec.numbering_style, NumberingStyle::Arabic);
    assert_eq!(sec.start_at, Some(7));
    assert!(!sec.include_prefix);
    assert_eq!(sec.section_prefix.as_deref(), Some(""));
}

#[test]
fn the_attribute_spelling_of_the_style_still_reads_and_patches() {
    // The engine's pre-2026-09 fixtures spell the style as an attribute.
    let old = r#"<Section Self="sec1" PageStart="p1" PageNumberStyle="UpperRoman" PageNumberStart="1"/>
"#;
    let source = pkg::simple(old, pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    assert_eq!(
        doc.designmap.sections[0].numbering_style,
        NumberingStyle::UpperRoman
    );
    // Unmutated: byte-identical (the attribute form is tolerated in place).
    assert_eq!(source, write_idml(&doc, &source).expect("write"));
    doc.designmap.sections[0].numbering_style = NumberingStyle::LowerAlpha;
    let out = write_idml(&doc, &source).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert!(dm.contains(r#"PageNumberStyle="LowerLetters""#), "{dm}");
    assert_eq!(
        pkg::open(&out).designmap.sections[0].numbering_style,
        NumberingStyle::LowerAlpha
    );
}
