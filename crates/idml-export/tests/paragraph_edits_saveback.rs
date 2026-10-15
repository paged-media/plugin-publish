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

//! The most ordinary edits to a story the part already holds reach the
//! export: typing into an empty paragraph, a new paragraph between two,
//! Enter in the middle of a run, two paragraphs joined.
//!
//! The story is InDesign's shape for paragraphs of different styles: one
//! `<ParagraphStyleRange>` each, an empty one holding only its mark
//! (`<CharacterStyleRange><Br/></CharacterStyleRange>`), the last empty
//! one a bare `<CharacterStyleRange/>`. An empty paragraph produces no
//! run, so no range of it resolved to the text typed into it — and the
//! text was dropped. A paragraph the engine added shifted every source
//! paragraph after it onto the wrong model paragraph — text of one
//! written into another, the tail of the story lost.
//!
//! The model edits are the engine's: typing into an empty paragraph adds
//! a run with no attributes (or fills its one empty run); Enter splits a
//! paragraph into two that share its style; deleting the mark joins two.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story};

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]"/></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]"/><ParagraphStyle Self="ParagraphStyle/Body" Name="Body"/><ParagraphStyle Self="ParagraphStyle/Heading" Name="Heading"/><ParagraphStyle Self="ParagraphStyle/Quote" Name="Quote"/></RootParagraphStyleGroup>
</idPkg:Styles>"#;

/// Heading (empty) · Body "One" · Quote (empty) · Body "Three" ·
/// Body "Four" (14 pt) · Heading (empty, last).
const STORY: &str = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Heading"><CharacterStyleRange><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange><Content>One</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Quote"><CharacterStyleRange><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange><Content>Three</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange PointSize="14"><Content>Four</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Heading"><CharacterStyleRange/></ParagraphStyleRange>"#;

/// `(style, text)` per paragraph.
fn shape(story: &Story) -> Vec<(String, String)> {
    story
        .paragraphs
        .iter()
        .map(|p| {
            (
                p.paragraph_style
                    .as_deref()
                    .unwrap_or("")
                    .trim_start_matches("ParagraphStyle/")
                    .to_string(),
                p.runs.iter().map(|r| r.text.as_str()).collect(),
            )
        })
        .collect()
}

fn typed(text: &str) -> CharacterRun {
    CharacterRun {
        text: text.into(),
        ..Default::default()
    }
}

/// Export the edited model over its source and read it back: the story
/// must come back as the model holds it, style and text, and a second
/// save of what came back must change nothing.
fn exported(edit: impl FnOnce(&mut Vec<Paragraph>)) -> (Vec<(String, String)>, Story, String) {
    let source = pkg::simple("", STORY, STYLES);
    let mut doc = pkg::open(&source);
    edit(&mut doc.stories[0].story.paragraphs);
    let want = shape(&doc.stories[0].story);
    let out = write_idml(&doc, &source).expect("export");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    let re = pkg::open(&out);
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
    (want, re.stories[0].story.clone(), xml)
}

fn assert_round_trip(label: &str, edit: impl FnOnce(&mut Vec<Paragraph>)) {
    let (want, back, xml) = exported(edit);
    assert_eq!(shape(&back), want, "{label}:\n{xml}");
}

#[test]
fn the_source_story_reads_as_six_paragraphs() {
    let doc = pkg::open(&pkg::simple("", STORY, STYLES));
    let s = shape(&doc.stories[0].story);
    assert_eq!(s.len(), 6, "{s:?}");
    assert!(doc.stories[0].story.paragraphs[2].runs.is_empty());
}

#[test]
fn text_typed_into_an_empty_paragraph_between_two_is_exported_in_its_style() {
    assert_round_trip("middle", |p| p[2].runs.push(typed("Middle")));
}

#[test]
fn text_typed_into_an_empty_first_paragraph_is_exported() {
    assert_round_trip("first", |p| p[0].runs.push(typed("Start")));
}

#[test]
fn text_typed_into_an_empty_last_paragraph_is_exported() {
    assert_round_trip("last", |p| p[5].runs.push(typed("End")));
}

/// A paragraph saved from the native model can hold one EMPTY run; the
/// engine types into it rather than adding one.
#[test]
fn text_typed_into_an_empty_run_of_an_empty_paragraph_is_exported() {
    assert_round_trip("empty run", |p| {
        p[2].runs.push(typed(""));
        p[2].runs[0].text = "Filled".into();
    });
}

/// More than one run lands in the empty paragraph (a style applied to
/// part of what was typed): every run, the mark after the last.
#[test]
fn several_runs_typed_into_an_empty_paragraph_are_exported_in_order() {
    let (want, back, xml) = exported(|p| {
        p[2].runs.push(typed("Plain, "));
        p[2].runs.push(CharacterRun {
            text: "bold".into(),
            font_style: Some("Bold".into()),
            ..Default::default()
        });
    });
    assert_eq!(shape(&back), want, "{xml}");
    assert_eq!(
        back.paragraphs[2].runs[1].font_style.as_deref(),
        Some("Bold")
    );
}

/// Enter after "One", then text: a new Body paragraph before the Quote.
#[test]
fn a_paragraph_inserted_between_two_source_paragraphs_is_exported() {
    assert_round_trip("inserted", |p| {
        p.insert(
            2,
            Paragraph {
                paragraph_style: p[1].paragraph_style.clone(),
                runs: vec![typed("Inserted")],
                ..Default::default()
            },
        );
    });
}

/// Enter before everything: a paragraph ahead of the first source one.
#[test]
fn a_paragraph_inserted_before_the_first_source_paragraph_is_exported() {
    assert_round_trip("before first", |p| {
        p.insert(
            0,
            Paragraph {
                paragraph_style: Some("ParagraphStyle/Body".into()),
                runs: vec![typed("Lead")],
                ..Default::default()
            },
        );
    });
}

/// Enter in the middle of "Four" (a 14 pt run): both halves keep it.
#[test]
fn a_paragraph_split_by_enter_in_the_middle_of_a_run_is_exported() {
    let (want, back, xml) = exported(|p| {
        let mut tail = p[4].clone();
        p[4].runs[0].text = "Fo".into();
        tail.runs[0].text = "ur".into();
        p.insert(5, tail);
    });
    assert_eq!(shape(&back), want, "{xml}");
    assert_eq!(back.paragraphs[4].runs[0].point_size, Some(14.0));
    assert_eq!(back.paragraphs[5].runs[0].point_size, Some(14.0));
}

/// Deleting the mark between "Three" and "Four": one paragraph, both runs.
#[test]
fn two_source_paragraphs_joined_by_deleting_the_mark_are_exported() {
    assert_round_trip("joined", |p| {
        let four = p.remove(4);
        p[3].runs.extend(four.runs);
    });
}

/// Every edit at once, in different places.
#[test]
fn edits_in_several_places_are_all_exported() {
    assert_round_trip("all", |p| {
        p[0].runs.push(typed("Start"));
        p[2].runs.push(typed("Middle"));
        p[5].runs.push(typed("End"));
        p.insert(
            2,
            Paragraph {
                paragraph_style: Some("ParagraphStyle/Body".into()),
                runs: vec![typed("Inserted")],
                ..Default::default()
            },
        );
    });
}

/// An empty cell of a table the part holds (InDesign's
/// `<ParagraphStyleRange><CharacterStyleRange/></ParagraphStyleRange>`),
/// typed into.
#[test]
fn text_typed_into_an_empty_cell_of_a_source_table_is_exported() {
    let story = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange><Table Self="t1" HeaderRowCount="0" FooterRowCount="0" BodyRowCount="1" ColumnCount="2"><Row Self="t1Row0" Name="0" SingleRowHeight="20"/><Column Self="t1Column0" Name="0" SingleColumnWidth="80"/><Column Self="t1Column1" Name="1" SingleColumnWidth="80"/><Cell Self="t1i0" Name="0:0" RowSpan="1" ColumnSpan="1"><ParagraphStyleRange><CharacterStyleRange><Content>Full</Content></CharacterStyleRange></ParagraphStyleRange></Cell><Cell Self="t1i1" Name="1:0" RowSpan="1" ColumnSpan="1"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Quote"><CharacterStyleRange/></ParagraphStyleRange></Cell></Table></CharacterStyleRange></ParagraphStyleRange>"#;
    let source = pkg::simple("", story, STYLES);
    let mut doc = pkg::open(&source);
    let table = doc.stories[0].story.paragraphs[0]
        .table
        .as_mut()
        .expect("table");
    assert!(table.cells[1].paragraphs[0].runs.is_empty());
    table.cells[1].paragraphs[0].runs.push(typed("Typed"));
    let out = write_idml(&doc, &source).expect("export");
    let re = pkg::open(&out);
    let t = re.stories[0].story.paragraphs[0].table.as_ref().unwrap();
    assert_eq!(
        shape(&Story {
            paragraphs: t.cells[1].paragraphs.clone(),
            ..Default::default()
        }),
        vec![("Quote".to_string(), "Typed".to_string())],
        "{}",
        pkg::entry(&out, "Stories/Story_st1.xml").unwrap()
    );
    assert_eq!(t.cells[0].paragraphs[0].runs[0].text, "Full");
}
