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

//! An anchored object's place in its paragraph survives a save.
//!
//! The reader records each object's character offset
//! (`Paragraph::anchored_frame_offsets`); the writer never writes that
//! number — the anchor IS the object element's place in the run's
//! content stream, and the rewrite passes the element through where it
//! stands. So the contract is: an unmutated save is byte-identical, and
//! after an edit that moves text (with the model's anchors kept on their
//! characters by `Paragraph::shift_anchors`, as the engine's text edits
//! do) re-reading the save finds every object at the offset the model
//! says.

use idml_export::rewrite::rewrite_story;
use idml_import::{parse_story, Paragraph, Story};

fn rect(id: &str) -> String {
    format!(
        "<Rectangle Self=\"{id}\" GeometricBounds=\"0 0 24 30\">\
         <AnchoredObjectSetting AnchoredPosition=\"InlinePosition\"/></Rectangle>"
    )
}

/// Every anchor case in one story: InDesign's spelling (the object among
/// a run's `<Content>`s), paged-gen's (the object opening its range), an
/// object after a paragraph mark inside one range, an object alone in a
/// range's last paragraph, and objects in a table cell.
fn source() -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<idPkg:Story xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\" DOMVersion=\"20.0\">\
<Story Self=\"u1\">\
<ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/$ID/[No paragraph style]\">\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\" PointSize=\"10\">\
<Content>lead in </Content></CharacterStyleRange>\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\" PointSize=\"12\">\
<Content>alpha </Content>{}<Content> beta</Content><Br/>\
<Content>gamma</Content>{}<Content> delta</Content><Br/>\
</CharacterStyleRange></ParagraphStyleRange>\
<ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/$ID/[No paragraph style]\">\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\">\
<Content>b01 b02</Content></CharacterStyleRange>\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\" PointSize=\"11\">\
{}<Content> b03</Content></CharacterStyleRange>\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\">\
<Content>caption</Content><Br/></CharacterStyleRange>\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\">\
{}<Br/></CharacterStyleRange></ParagraphStyleRange>\
<ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/$ID/[No paragraph style]\">\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\">\
<Table Self=\"t\" BodyRowCount=\"1\" ColumnCount=\"1\">\
<Row Self=\"t0\" Name=\"0\"/><Column Self=\"tc0\" Name=\"0\"/>\
<Cell Self=\"c\" Name=\"0:0\"><ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/$ID/[No paragraph style]\">\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\">\
<Content>cell </Content></CharacterStyleRange>\
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\" PointSize=\"9\">\
{}<Content> text</Content></CharacterStyleRange></ParagraphStyleRange></Cell>\
</Table></CharacterStyleRange></ParagraphStyleRange>\
</Story></idPkg:Story>",
        rect("ra"),
        rect("rb"),
        rect("rc"),
        rect("rd"),
        rect("re"),
    )
    .into_bytes()
}

fn text(p: &Paragraph) -> String {
    p.runs.iter().map(|r| r.text.as_str()).collect()
}

/// Every anchor of the story, story paragraphs then cell paragraphs, as
/// `(id, offset, paragraph text)`.
fn anchors(s: &Story) -> Vec<(String, u32, String)> {
    let mut out = Vec::new();
    let mut visit = |p: &Paragraph| {
        assert_eq!(p.anchored_frames.len(), p.anchored_frame_offsets.len());
        for (f, &o) in p.anchored_frames.iter().zip(&p.anchored_frame_offsets) {
            out.push((f.self_id.clone().unwrap_or_default(), o, text(p)));
        }
    };
    for p in &s.paragraphs {
        visit(p);
    }
    for p in &s.paragraphs {
        if let Some(t) = &p.table {
            for c in &t.cells {
                for cp in &c.paragraphs {
                    visit(cp);
                }
            }
        }
    }
    out
}

fn a(id: &str, at: u32, text: &str) -> (String, u32, String) {
    (id.to_string(), at, text.to_string())
}

#[test]
fn the_reader_records_every_anchor() {
    let s = parse_story(&source()).expect("parse");
    assert_eq!(
        anchors(&s),
        vec![
            a("ra", 14, "lead in alpha  beta\ngamma delta"),
            a("rb", 25, "lead in alpha  beta\ngamma delta"),
            a("rc", 7, "b01 b02 b03caption\n"),
            a("rd", 19, "b01 b02 b03caption\n"),
            a("re", 5, "cell  text"),
        ]
    );
}

#[test]
fn an_unmutated_save_is_byte_identical() {
    let src = source();
    let s = parse_story(&src).expect("parse");
    let out = rewrite_story(&src, &s).expect("rewrite");
    assert_eq!(
        String::from_utf8(out).unwrap(),
        String::from_utf8(src).unwrap()
    );
}

/// Replace run `run` of paragraph `para` with `text`, keeping the
/// paragraph's anchors on their characters the way the engine's text
/// edits do.
fn edit(s: &mut Story, para: usize, run: usize, text: &str) {
    let p = &mut s.paragraphs[para];
    let at: u32 = p.runs[..run]
        .iter()
        .map(|r| r.text.chars().count() as u32)
        .sum();
    let old = p.runs[run].text.chars().count() as i64;
    let new = text.chars().count() as i64;
    p.runs[run].text = text.to_string();
    // The run's common length stays; the rest is typed at, or removed
    // from, its end — so an object standing right after the run moves
    // with it either way.
    p.shift_anchors(at + old.min(new) as u32, new - old);
}

fn resave(s: &Story) -> Story {
    let out = rewrite_story(&source(), s).expect("rewrite");
    parse_story(&out).expect("re-parse")
}

#[test]
fn text_edited_before_an_anchor_moves_it_with_its_character() {
    let mut s = parse_story(&source()).expect("parse");
    // The range before the objects' range grows by 9 characters.
    edit(&mut s, 0, 0, "a longer lead in ");
    // The run before paged-gen's object shrinks by 4.
    edit(&mut s, 1, 0, "b01");
    let back = resave(&s);
    assert_eq!(anchors(&back), anchors(&s));
    assert_eq!(
        anchors(&back),
        vec![
            a("ra", 23, "a longer lead in alpha  beta\ngamma delta"),
            a("rb", 34, "a longer lead in alpha  beta\ngamma delta"),
            a("rc", 3, "b01 b03caption\n"),
            a("rd", 15, "b01 b03caption\n"),
            a("re", 5, "cell  text"),
        ]
    );
}

#[test]
fn text_edited_after_an_anchor_leaves_it() {
    let mut s = parse_story(&source()).expect("parse");
    // The run paged-gen's object opens: the object stands before its
    // text, so editing that text does not move it.
    edit(&mut s, 1, 1, " b03 b04");
    // The caption before the lone picture: the picture stays in the
    // paragraph of its own after the mark.
    edit(&mut s, 1, 2, "a caption");
    let back = resave(&s);
    assert_eq!(anchors(&back), anchors(&s));
    assert_eq!(
        anchors(&back)[2..4],
        [
            a("rc", 7, "b01 b02 b03 b04a caption\n"),
            a("rd", 25, "b01 b02 b03 b04a caption\n"),
        ]
    );
}
