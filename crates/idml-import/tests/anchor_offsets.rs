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

//! An anchored object is a character of its paragraph: the reader
//! records WHERE (`Paragraph::anchored_frame_offsets`, the character
//! offset of the anchor in the paragraph's text, Unicode scalars counted
//! contiguously over its runs), index-aligned with `anchored_frames`.
//! Before, it recorded nothing and every imported object anchored at its
//! paragraph's start, so a mid-line inline object was drawn at its
//! line's start.

use idml_import::{parse_story, Paragraph, Story};

fn story(body: &str) -> Story {
    let xml = format!(
        "<idPkg:Story xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\">\
         <Story Self=\"u1\">{body}</Story></idPkg:Story>"
    );
    parse_story(xml.as_bytes()).expect("parse")
}

const RECT: &str = "<Rectangle Self=\"r{}\" GeometricBounds=\"0 0 24 30\">\
                    <AnchoredObjectSetting AnchoredPosition=\"InlinePosition\"/></Rectangle>";

fn rect(id: &str) -> String {
    RECT.replace("{}", id)
}

fn text(p: &Paragraph) -> String {
    p.runs.iter().map(|r| r.text.as_str()).collect()
}

/// Each frame's id with its recorded offset.
fn anchors(p: &Paragraph) -> Vec<(String, u32)> {
    assert_eq!(
        p.anchored_frames.len(),
        p.anchored_frame_offsets.len(),
        "offsets are index-aligned with the frames"
    );
    p.anchored_frames
        .iter()
        .zip(&p.anchored_frame_offsets)
        .map(|(f, &o)| (f.self_id.clone().unwrap_or_default(), o))
        .collect()
}

#[test]
fn start_middle_and_end_of_a_paragraph() {
    // InDesign's own spelling: the object sits among the run's
    // `<Content>` elements, at its character.
    let s = story(&format!(
        "<ParagraphStyleRange><CharacterStyleRange>\
           {}<Content>alpha beta</Content>\
         </CharacterStyleRange></ParagraphStyleRange>\
         <ParagraphStyleRange><CharacterStyleRange>\
           <Content>alpha </Content>{}<Content> beta</Content>\
         </CharacterStyleRange></ParagraphStyleRange>\
         <ParagraphStyleRange><CharacterStyleRange>\
           <Content>alpha beta</Content>{}\
         </CharacterStyleRange></ParagraphStyleRange>",
        rect("s"),
        rect("m"),
        rect("e"),
    ));
    assert_eq!(anchors(&s.paragraphs[0]), vec![("rs".into(), 0)]);
    assert_eq!(text(&s.paragraphs[1]), "alpha  beta");
    assert_eq!(anchors(&s.paragraphs[1]), vec![("rm".into(), 6)]);
    assert_eq!(anchors(&s.paragraphs[2]), vec![("re".into(), 10)]);
}

#[test]
fn an_object_opening_its_own_range_counts_the_ranges_before_it() {
    // paged-gen's spelling: every object at the start of the range that
    // carries it, and the object closing a paragraph rides a range with
    // no text (which the reader drops — the offset stays at the end).
    let s = story(&format!(
        "<ParagraphStyleRange>\
           <CharacterStyleRange><Content>b01 b02</Content></CharacterStyleRange>\
           <CharacterStyleRange PointSize=\"12\">{}<Content> b03</Content></CharacterStyleRange>\
           <CharacterStyleRange>{}</CharacterStyleRange>\
         </ParagraphStyleRange>",
        rect("a"),
        rect("b"),
    ));
    let p = &s.paragraphs[0];
    assert_eq!(text(p), "b01 b02 b03");
    assert_eq!(anchors(p), vec![("ra".into(), 7), ("rb".into(), 11)]);
}

#[test]
fn several_objects_in_one_run_and_their_nested_children() {
    // Two objects back to back, then one after more text; a Group's
    // members are its children, not anchors of their own, and a
    // self-closing frame records its anchor too.
    let s = story(
        "<ParagraphStyleRange><CharacterStyleRange>\
           <Content>ab</Content>\
           <Rectangle Self=\"r1\" GeometricBounds=\"0 0 10 10\"/>\
           <Group Self=\"g1\"><Rectangle Self=\"r2\" GeometricBounds=\"0 0 5 5\"/>\
             <TextFrame Self=\"t1\" ParentStory=\"u9\"><Content>not here</Content></TextFrame>\
           </Group>\
           <Content>cdé</Content>\
           <Rectangle Self=\"r3\" GeometricBounds=\"0 0 10 10\"></Rectangle>\
           <Content>f</Content>\
         </CharacterStyleRange></ParagraphStyleRange>",
    );
    let p = &s.paragraphs[0];
    assert_eq!(text(p), "abcdéf");
    // `é` is one character, not two bytes.
    assert_eq!(
        anchors(p),
        vec![("r1".into(), 2), ("g1".into(), 2), ("r3".into(), 5)]
    );
    assert_eq!(p.anchored_frames[1].children.len(), 2);
}

#[test]
fn tabs_breaks_and_variables_are_characters_before_the_anchor() {
    let s = story(&format!(
        "<ParagraphStyleRange><CharacterStyleRange>\
           <Content>a</Content><Tab/><Content>b</Content>\
           <TextVariableInstance ResultText=\"12\" AssociatedTextVariable=\"TextVariable/n\"/>\
           <Content>c</Content>{}<Content>d</Content>\
         </CharacterStyleRange></ParagraphStyleRange>",
        rect("x"),
    ));
    let p = &s.paragraphs[0];
    assert_eq!(text(p), "a\tb12cd");
    assert_eq!(anchors(p), vec![("rx".into(), 6)]);
}

#[test]
fn an_object_after_a_paragraph_mark_belongs_to_the_next_paragraph() {
    // InDesign writes consecutive paragraphs of one style in ONE range,
    // separated by `<Br/>` marks. The model keeps them as one paragraph
    // whose text carries the `\n`, and the anchor counts it.
    let s = story(&format!(
        "<ParagraphStyleRange><CharacterStyleRange>\
           <Content>first</Content><Br/>\
           <Content>sec</Content>{}<Content>ond</Content><Br/>\
           <Content>third</Content>\
         </CharacterStyleRange></ParagraphStyleRange>",
        rect("x"),
    ));
    let p = &s.paragraphs[0];
    assert_eq!(text(p), "first\nsecond\nthird");
    assert_eq!(anchors(p), vec![("rx".into(), 9)]);
}

#[test]
fn an_object_alone_after_the_last_mark_is_a_paragraph_of_its_own() {
    // A picture alone in the range's last paragraph: the held mark before
    // it is interior (the object is that paragraph's content), so it
    // stays a newline and the object anchors after it — not at the end
    // of the paragraph before.
    for tail in [
        // in the same range as the mark
        format!("<Content>caption</Content><Br/>{}", rect("x")),
        // in a range of its own after it
        format!(
            "<Content>caption</Content><Br/></CharacterStyleRange>\
             <CharacterStyleRange>{}",
            rect("x")
        ),
    ] {
        let s = story(&format!(
            "<ParagraphStyleRange><CharacterStyleRange>{tail}\
             </CharacterStyleRange></ParagraphStyleRange>"
        ));
        let p = &s.paragraphs[0];
        assert_eq!(text(p), "caption\n", "{tail}");
        assert_eq!(anchors(p), vec![("rx".into(), 8)], "{tail}");
    }
    // The mark that ENDS the range, with nothing after it, is still the
    // terminator and not text.
    let s = story(&format!(
        "<ParagraphStyleRange><CharacterStyleRange>\
           <Content>cap</Content>{}<Content>tion</Content><Br/>\
         </CharacterStyleRange></ParagraphStyleRange>\
         <ParagraphStyleRange><CharacterStyleRange><Content>next</Content>\
         </CharacterStyleRange></ParagraphStyleRange>",
        rect("y"),
    ));
    assert_eq!(text(&s.paragraphs[0]), "caption");
    assert_eq!(anchors(&s.paragraphs[0]), vec![("ry".into(), 3)]);
    assert!(s.paragraphs[1].anchored_frames.is_empty());
    assert!(s.paragraphs[1].anchored_frame_offsets.is_empty());
}

#[test]
fn objects_in_table_cells_anchor_in_their_cell_paragraph() {
    // The host paragraph's text before the table does not count in a
    // cell, and each cell paragraph counts from its own start.
    let s = story(&format!(
        "<ParagraphStyleRange><CharacterStyleRange>\
           <Content>host</Content>\
           <Table Self=\"t\" BodyRowCount=\"1\" ColumnCount=\"2\">\
             <Row Self=\"r0\" Name=\"0\"/>\
             <Column Self=\"c0\" Name=\"0\"/><Column Self=\"c1\" Name=\"1\"/>\
             <Cell Self=\"a\" Name=\"0:0\"><ParagraphStyleRange><CharacterStyleRange>\
               <Content>xy</Content>{}<Content>z</Content><Br/>\
               <Content>w</Content>{}\
             </CharacterStyleRange></ParagraphStyleRange></Cell>\
             <Cell Self=\"b\" Name=\"1:0\"><ParagraphStyleRange><CharacterStyleRange>\
               {}<Content>q</Content>\
             </CharacterStyleRange></ParagraphStyleRange>\
             <ParagraphStyleRange><CharacterStyleRange>\
               <Content>uv</Content>{}\
             </CharacterStyleRange></ParagraphStyleRange></Cell>\
           </Table>\
         </CharacterStyleRange></ParagraphStyleRange>",
        rect("1"),
        rect("2"),
        rect("3"),
        rect("4"),
    ));
    let host = &s.paragraphs[0];
    assert!(host.anchored_frames.is_empty());
    let cells = &host.table.as_ref().expect("table").cells;
    let a = &cells[0].paragraphs[0];
    assert_eq!(text(a), "xyz\nw");
    assert_eq!(anchors(a), vec![("r1".into(), 2), ("r2".into(), 5)]);
    assert_eq!(anchors(&cells[1].paragraphs[0]), vec![("r3".into(), 0)]);
    assert_eq!(anchors(&cells[1].paragraphs[1]), vec![("r4".into(), 2)]);
}
