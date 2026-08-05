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

//! A style range's attributes stay on the element they came from.
//!
//! # The defect
//!
//! IDML puts no id on a `<ParagraphStyleRange>` / `<CharacterStyleRange>`,
//! so the save-back has to derive the range→model link. It derived it by
//! COUNTING: the nth `<CharacterStyleRange>` element patched against the
//! nth `CharacterRun`. That is only true if the parser keeps one model
//! item per source element, and it does not:
//!
//! * a `<CharacterStyleRange>` whose text came out empty is DROPPED
//!   (`if !run.text.is_empty()`) — a range holding only a `<Table>`, a
//!   self-closing `<CharacterStyleRange/>`;
//! * a `<ParagraphStyleRange>` left with neither a run nor a table is
//!   dropped too;
//! * a `<TextVariableInstance>` SPLITS one range into several runs.
//!
//! Every element after such a range was then patched against the wrong
//! model item. Measured on the corpus, unmutated: `line-sheet`'s
//! table-bearing range came back `PointSize="10"` → `8` with its
//! `FontStyle="Bold"` deleted, `sample.idml` had two
//! `AppliedCharacterStyle`s rewritten, and 99 stories whose every range
//! was empty lost their `AppliedParagraphStyle` outright — the writer
//! resolved "no aligned paragraph" as "the model says no style" and
//! deleted the attribute.
//!
//! # The fix this pins
//!
//! The parser now publishes where each source element landed
//! ([`idml_import::StoryProvenance`], keyed by the element's byte offset)
//! and the rewrite looks it up. The rule that drops and splits stays in
//! one place, on the side that owns it. An element with no entry has no
//! model counterpart and passes through verbatim.
//!
//! Every fixture below asserts its PARSE-SIDE premise first, so none of
//! them can pass for the wrong reason — a fixture whose range happened
//! not to be dropped would make its assertion vacuous.

use idml_export::rewrite::rewrite_story;

/// `samples/line-sheet.idml#Stories/Story_u2b6.xml`, minimised: the
/// paragraph's FIRST character range holds a `<Table>` and no text, so
/// the parser drops it — and the range carries the attributes
/// (`FontStyle`, `PointSize`) that were then overwritten with the next
/// range's.
const TABLE_ONLY_RANGE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="12.0">
<Story Self="u2b6">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/[No paragraph style]">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" FontStyle="Bold" PointSize="10">
<Table Self="t1" BodyRowCount="1" ColumnCount="1">
<Row Self="t1Row0" Name="0" SingleRowHeight="20" />
<Column Self="t1Column0" Name="0" SingleColumnWidth="100" />
<Cell Self="t1i1" Name="0:0" RowSpan="1" ColumnSpan="1">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Blank">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" />
</ParagraphStyleRange>
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle" Justification="CenterAlign">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PointSize="7">
<Content>cell</Content>
</CharacterStyleRange>
</ParagraphStyleRange>
</Cell>
</Table>
</CharacterStyleRange>
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PointSize="8">
<Content>after the table</Content>
</CharacterStyleRange>
</ParagraphStyleRange>
</Story>
</idPkg:Story>"#;

/// `samples/sample.idml`'s TOC shape: a SELF-CLOSING, textless
/// `<CharacterStyleRange/>` ahead of a `<HyperlinkTextSource>` whose own
/// ranges carry the styles. The empty element produced no run, so every
/// range after it patched one position out.
const SELF_CLOSING_EMPTY_RANGE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Story Self="u28e69">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/TOC">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" />
<HyperlinkTextSource Self="u29539" Name=".169273" Hidden="true" AppliedCharacterStyle="n">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Bold Red">
<Content>The Chairman</Content>
</CharacterStyleRange>
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Light 8pt">
<Content>4</Content>
</CharacterStyleRange>
</HyperlinkTextSource>
</ParagraphStyleRange>
</Story>
</idPkg:Story>"#;

/// `ancient-building-magazine`'s empty story: one paragraph range, one
/// character range, no `<Content>` anywhere. The parser drops the run,
/// then drops the runless paragraph, so the model has NO paragraphs at
/// all — and the writer deleted the source's `AppliedParagraphStyle`.
const EMPTY_STORY: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="15.0">
<Story Self="u4d2">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
<Properties>
<AppliedFont type="string">Poppins</AppliedFont>
</Properties>
</CharacterStyleRange>
</ParagraphStyleRange>
</Story>
</idPkg:Story>"#;

/// The other direction: one range becomes THREE runs because the parser
/// gives a `<TextVariableInstance>` its own run and flushes the text
/// around it. Counting elements lags the model here instead of leading
/// it.
const TEXT_VARIABLE_SPLIT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Story Self="uv1">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Header">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Running" PointSize="9">
<Content>page </Content>
<TextVariableInstance Self="tv1" ResultText="12" AssociatedTextVariable="TextVariable/Page" />
<Content> of 40</Content>
</CharacterStyleRange>
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Plain" PointSize="11">
<Content>tail</Content>
</CharacterStyleRange>
</ParagraphStyleRange>
</Story>
</idPkg:Story>"#;

/// Every `KEY="…"` value in document order.
fn values_of<'a>(xml: &'a str, key: &str) -> Vec<&'a str> {
    let needle = format!("{key}=\"");
    xml.match_indices(&needle)
        .map(|(i, _)| {
            let rest = &xml[i + needle.len()..];
            &rest[..rest.find('"').expect("closing quote")]
        })
        .collect()
}

fn round_trip(src: &[u8]) -> String {
    let story = idml_import::parse_story(src).expect("parse");
    let out = rewrite_story(src, &story).expect("rewrite");
    String::from_utf8(out).expect("utf8")
}

// ---------------------------------------------------------------------
// 1. A range holding only a table
// ---------------------------------------------------------------------

/// The premise: the parser really does drop the table-bearing range, so
/// the model has ONE run where the XML has two elements.
#[test]
fn premise_a_textless_table_range_is_dropped_by_the_parser() {
    let story = idml_import::parse_story(TABLE_ONLY_RANGE).expect("parse");
    assert_eq!(story.paragraphs.len(), 1, "one story paragraph");
    assert_eq!(
        story.paragraphs[0].runs.len(),
        1,
        "the table-only range produced no run — the count the writer used \
         to trust is one short of the element count"
    );
    assert_eq!(story.paragraphs[0].runs[0].text, "after the table");
    assert!(
        story.paragraphs[0].table.is_some(),
        "the table itself did land on the paragraph"
    );
}

/// THE DEFECT: the table-bearing range keeps its OWN `PointSize` and
/// `FontStyle`. Before the fix it was patched against the next run and
/// came back `PointSize="8"` with `FontStyle="Bold"` deleted.
#[test]
fn a_table_only_range_keeps_its_own_character_attributes() {
    let xml = round_trip(TABLE_ONLY_RANGE);
    assert_eq!(
        values_of(&xml, "PointSize"),
        ["10", "7", "8"],
        "each range must keep the size it arrived with, in document order:\n{xml}"
    );
    assert_eq!(
        values_of(&xml, "FontStyle"),
        ["Bold"],
        "the table-bearing range's FontStyle must survive:\n{xml}"
    );
    assert_eq!(
        xml.as_bytes(),
        TABLE_ONLY_RANGE,
        "and the whole entry must be byte-identical"
    );
}

/// The same misalignment one level up, INSIDE a cell: the cell's first
/// paragraph range is textless, so the parser drops it and the cell's
/// second range is its FIRST model paragraph. Counting sent that range
/// to `cell.paragraphs[1]`, which does not exist, and the writer resolved
/// the miss as "no style" — deleting `AppliedParagraphStyle`.
#[test]
fn a_cell_paragraph_range_keeps_its_applied_style() {
    let story = idml_import::parse_story(TABLE_ONLY_RANGE).expect("parse");
    let table = story.paragraphs[0].table.as_ref().expect("table");
    assert_eq!(
        table.cells[0].paragraphs.len(),
        1,
        "premise: two range elements in the cell, one model paragraph"
    );

    let xml = round_trip(TABLE_ONLY_RANGE);
    assert_eq!(
        values_of(&xml, "AppliedParagraphStyle"),
        [
            "ParagraphStyle/$ID/[No paragraph style]",
            "ParagraphStyle/Blank",
            "ParagraphStyle/$ID/NormalParagraphStyle"
        ],
        "each cell paragraph must keep its own applied style:\n{xml}"
    );
}

// ---------------------------------------------------------------------
// 2. A self-closing, textless range
// ---------------------------------------------------------------------

#[test]
fn premise_a_self_closing_range_produces_no_run() {
    let story = idml_import::parse_story(SELF_CLOSING_EMPTY_RANGE).expect("parse");
    assert_eq!(story.paragraphs.len(), 1);
    assert_eq!(
        story.paragraphs[0].runs.len(),
        2,
        "three range elements, two runs — the empty one produced nothing"
    );
    assert_eq!(story.paragraphs[0].runs[0].text, "The Chairman");
}

/// THE DEFECT: the two hyperlink ranges keep their own character styles
/// (before the fix the first inherited the second's and the last was
/// patched against nothing), and the empty element is not even
/// re-serialised — its ` />` spacing survives.
#[test]
fn a_self_closing_empty_range_does_not_shift_the_ranges_after_it() {
    let xml = round_trip(SELF_CLOSING_EMPTY_RANGE);
    assert_eq!(
        values_of(&xml, "AppliedCharacterStyle"),
        [
            "CharacterStyle/$ID/[No character style]",
            "n",
            "CharacterStyle/Bold Red",
            "CharacterStyle/Light 8pt"
        ],
        "every range keeps the style it arrived with:\n{xml}"
    );
    assert_eq!(
        xml.as_bytes(),
        SELF_CLOSING_EMPTY_RANGE,
        "an unmutated story must save back as the bytes it arrived as"
    );
}

// ---------------------------------------------------------------------
// 3. A story the parser empties completely
// ---------------------------------------------------------------------

#[test]
fn premise_an_empty_story_parses_to_no_paragraphs_at_all() {
    let story = idml_import::parse_story(EMPTY_STORY).expect("parse");
    assert!(
        story.paragraphs.is_empty(),
        "the runless paragraph is dropped, so there is no model paragraph \
         for the source's range to align with"
    );
}

/// THE DEFECT, at its bluntest: "no model paragraph aligns here" is not
/// "the model says this range has no style". 99 corpus stories had their
/// `AppliedParagraphStyle` deleted on a save that changed nothing.
#[test]
fn an_empty_story_keeps_its_applied_paragraph_style() {
    let xml = round_trip(EMPTY_STORY);
    assert_eq!(
        values_of(&xml, "AppliedParagraphStyle"),
        ["ParagraphStyle/$ID/NormalParagraphStyle"],
        "an unaligned paragraph range passes through verbatim:\n{xml}"
    );
    assert_eq!(xml.as_bytes(), EMPTY_STORY, "and byte-identically");
}

// ---------------------------------------------------------------------
// 4. A range the parser SPLITS
// ---------------------------------------------------------------------

#[test]
fn premise_a_text_variable_splits_one_range_into_three_runs() {
    let story = idml_import::parse_story(TEXT_VARIABLE_SPLIT).expect("parse");
    assert_eq!(
        story.paragraphs[0].runs.len(),
        4,
        "three runs from the first range plus one from the second — the \
         model runs AHEAD of the element count here"
    );
    let texts: Vec<&str> = story.paragraphs[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(texts, ["page ", "12", " of 40", "tail"]);
}

/// THE DEFECT in the other direction: the range AFTER a split one used
/// to be patched against a run three positions early. Both ranges keep
/// their own style and size, and the split range's text — which no single
/// run holds all of — is replayed rather than re-serialised.
#[test]
fn a_split_range_does_not_shift_the_range_after_it() {
    let xml = round_trip(TEXT_VARIABLE_SPLIT);
    assert_eq!(
        values_of(&xml, "PointSize"),
        ["9", "11"],
        "the trailing range keeps its own size:\n{xml}"
    );
    assert_eq!(
        values_of(&xml, "AppliedCharacterStyle"),
        ["CharacterStyle/Running", "CharacterStyle/Plain"],
        "...and its own character style:\n{xml}"
    );
    assert_eq!(
        xml.as_bytes(),
        TEXT_VARIABLE_SPLIT,
        "an unmutated split range must save back verbatim — re-serialising \
         it from one of its three runs would delete the other two"
    );
}

// ---------------------------------------------------------------------
// The map is an alignment, not a freeze
// ---------------------------------------------------------------------

/// A real edit to a correctly-aligned run still saves. The provenance
/// map fixes WHICH element an edit lands on; it must not stop edits
/// landing at all.
#[test]
fn an_edit_to_the_aligned_run_still_saves() {
    let mut story = idml_import::parse_story(TABLE_ONLY_RANGE).expect("parse");
    story.paragraphs[0].runs[0].point_size = Some(12.5);
    let out = rewrite_story(TABLE_ONLY_RANGE, &story).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        values_of(&xml, "PointSize"),
        ["10", "7", "12.5"],
        "the edit must land on the range the run came from, and only there:\n{xml}"
    );
}

/// An edit to a CELL run saves too — the cell scope resolves through the
/// same map.
#[test]
fn an_edit_to_a_cell_run_still_saves() {
    let mut story = idml_import::parse_story(TABLE_ONLY_RANGE).expect("parse");
    let table = story.paragraphs[0].table.as_mut().expect("table");
    table.cells[0].paragraphs[0].runs[0].point_size = Some(6.5);
    let out = rewrite_story(TABLE_ONLY_RANGE, &story).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        values_of(&xml, "PointSize"),
        ["10", "6.5", "8"],
        "the cell edit must land inside the cell:\n{xml}"
    );
}

/// A text edit still saves, and still only to the run that changed.
#[test]
fn a_text_edit_to_the_aligned_run_still_saves() {
    let mut story = idml_import::parse_story(TABLE_ONLY_RANGE).expect("parse");
    story.paragraphs[0].runs[0].text = "AFTER THE TABLE".to_string();
    let out = rewrite_story(TABLE_ONLY_RANGE, &story).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert!(xml.contains("<Content>AFTER THE TABLE</Content>"), "{xml}");
    assert!(xml.contains("<Content>cell</Content>"), "{xml}");
}
