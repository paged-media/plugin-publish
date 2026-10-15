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

//! What a minted story holds at a character survives an export:
//! anchored objects at their characters, and a table where the model
//! stands it.
//!
//! The minted-story writer wrote no anchored object at all — an inline
//! rectangle's gap closed and an anchored picture frame vanished from
//! every story the engine made — and set every table straight after its
//! paragraph's text, as a character of that paragraph, whatever the
//! model said.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{
    AnchoredFrame, AnchoredFrameKind, AnchoredObjectSetting, Bounds, CharacterRun, Paragraph,
    Story, Table, TableCell, TableColumn, TableRow,
};

fn run(text: &str) -> CharacterRun {
    CharacterRun {
        text: text.into(),
        ..Default::default()
    }
}

fn para(texts: &[&str]) -> Paragraph {
    Paragraph {
        runs: texts.iter().map(|t| run(t)).collect(),
        ..Default::default()
    }
}

fn rectangle(id: &str, w: f32, h: f32) -> AnchoredFrame {
    AnchoredFrame {
        frame_kind: AnchoredFrameKind::Rectangle,
        self_id: Some(id.into()),
        bounds: Some(Bounds {
            top: 0.0,
            left: 0.0,
            bottom: h,
            right: w,
        }),
        item_transform: None,
        parent_story: None,
        setting: None,
        fill_color: None,
        stroke_color: None,
        stroke_weight: None,
        fill_tint: None,
        gradient_fill_angle: None,
        applied_object_style: None,
        image_link: None,
        image_item_transform: None,
        children: Vec::new(),
    }
}

/// Export a document holding `story` as a minted, framed story and read
/// the story back.
fn round_trip(story: Story) -> (Story, String) {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story,
    });
    pkg::place_story(&mut doc, "tf_u9", "Story/u9");
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_Story_u9.xml").expect("minted story part");
    let back = pkg::open(&out);
    let story = back
        .stories
        .iter()
        .find(|s| s.self_id.ends_with("u9"))
        .expect("the story reads back")
        .story
        .clone();
    (story, xml)
}

fn text(p: &Paragraph) -> String {
    p.runs.iter().map(|r| r.text.as_str()).collect()
}

/// Two objects at one character of a run: an anchored custom-positioned
/// frame and an inline one. Both come back at that character, in order,
/// with their boxes and settings.
#[test]
fn objects_inside_a_run_come_back_at_their_character() {
    let mut p = para(&["An anchored rectangle entered just here", " over the wire."]);
    let mut custom = rectangle("ra", 40.0, 14.0);
    custom.setting = Some(AnchoredObjectSetting {
        anchored_position: Some("Anchored".into()),
        anchor_x_offset: 26.0,
        anchor_y_offset: -6.0,
        anchor_point: Some("TopLeftAnchor".into()),
        lock_position: true,
        horizontal_reference_point: Some("TextFrame".into()),
        horizontal_alignment: Some("RightAlign".into()),
        vertical_reference_point: Some("LineBaseline".into()),
        vertical_alignment: Some("TopAlign".into()),
        ..Default::default()
    });
    p.anchored_frames = vec![custom, rectangle("rb", 9.0, 9.0)];
    p.anchored_frame_offsets = vec![21, 21];
    let (back, xml) = round_trip(Story {
        paragraphs: vec![p.clone(), para(&["The next paragraph."])],
        ..Default::default()
    });
    assert!(
        xml.contains(r#"<Content>An anchored rectangle</Content><Rectangle Self="ra""#),
        "the object among the run's contents:\n{xml}"
    );
    let q = &back.paragraphs[0];
    assert_eq!(text(q), text(&p), "the text is untouched");
    assert_eq!(q.anchored_frame_offsets, vec![21, 21]);
    let ids: Vec<_> = q
        .anchored_frames
        .iter()
        .map(|f| f.self_id.clone())
        .collect();
    assert_eq!(ids, vec![Some("ra".to_string()), Some("rb".to_string())]);
    assert_eq!(q.anchored_frames[1].bounds, p.anchored_frames[1].bounds);
    let s = q.anchored_frames[0].setting.as_ref().expect("its setting");
    assert_eq!(s.anchored_position.as_deref(), Some("Anchored"));
    assert_eq!((s.anchor_x_offset, s.anchor_y_offset), (26.0, -6.0));
    assert!(s.lock_position);
    assert_eq!(s.horizontal_alignment.as_deref(), Some("RightAlign"));
    assert_eq!(s.vertical_reference_point.as_deref(), Some("LineBaseline"));
    assert!(q.anchored_frames[1].setting.is_none());
    assert_eq!(text(&back.paragraphs[1]), "The next paragraph.");
}

/// A picture frame alone in an empty paragraph between two others —
/// the "plate mark" under a table — keeps its paragraph, its place and
/// its link.
#[test]
fn a_picture_frame_alone_in_its_paragraph_keeps_it() {
    let mut plate = rectangle("plate", 120.0, 80.0);
    plate.image_link = Some("data:image/png;base64,iVBORw0KGgo=".into());
    let lone = Paragraph {
        anchored_frames: vec![plate],
        anchored_frame_offsets: vec![0],
        ..Default::default()
    };
    let (back, _) = round_trip(Story {
        paragraphs: vec![para(&["Before."]), lone, para(&["After."])],
        ..Default::default()
    });
    assert_eq!(back.paragraphs.len(), 3, "{:#?}", back.paragraphs);
    assert_eq!(text(&back.paragraphs[0]), "Before.");
    let q = &back.paragraphs[1];
    assert_eq!(text(q), "");
    assert_eq!(q.anchored_frame_offsets, vec![0]);
    assert_eq!(
        q.anchored_frames[0].image_link.as_deref(),
        Some("data:image/png;base64,iVBORw0KGgo=")
    );
    assert_eq!(
        q.anchored_frames[0].bounds.map(|b| (b.right, b.bottom)),
        Some((120.0, 80.0))
    );
    assert_eq!(text(&back.paragraphs[2]), "After.");
}

/// An object at the very end of the text, and one in the last paragraph.
#[test]
fn an_object_after_the_last_character_stays_there() {
    let mut p = para(&["Ends with a box"]);
    p.anchored_frames = vec![rectangle("end", 6.0, 6.0)];
    p.anchored_frame_offsets = vec![15];
    let (back, _) = round_trip(Story {
        paragraphs: vec![p],
        ..Default::default()
    });
    assert_eq!(back.paragraphs.len(), 1);
    assert_eq!(text(&back.paragraphs[0]), "Ends with a box");
    assert_eq!(back.paragraphs[0].anchored_frame_offsets, vec![15]);
}

fn table() -> Table {
    Table {
        self_id: Some("t1".into()),
        column_count: 1,
        body_row_count: 1,
        rows: vec![TableRow {
            single_row_height: Some(20.0),
            ..Default::default()
        }],
        columns: vec![TableColumn {
            single_column_width: Some(100.0),
            ..Default::default()
        }],
        cells: vec![TableCell {
            name: Some("0:0".into()),
            row_span: 1,
            column_span: 1,
            paragraphs: vec![para(&["cell"])],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// A table with no anchor after a paragraph's text: the engine reads the
/// text as a paragraph of its own and the table after it, so the text's
/// paragraph mark stands between them — the reading InDesign gives
/// `text<Br/><Table>` too.
#[test]
fn a_table_with_no_anchor_follows_its_text_as_a_paragraph() {
    let mut p = para(&["Circulation by region."]);
    p.table = Some(table());
    p.table_anchor = None;
    let (back, xml) = round_trip(Story {
        paragraphs: vec![p, para(&["After the table."])],
        ..Default::default()
    });
    assert!(
        xml.contains(r#"<Content>Circulation by region.</Content><Br/></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Table"#),
        "{xml}"
    );
    let q = &back.paragraphs[0];
    assert_eq!(
        text(q),
        "Circulation by region.\n",
        "the text ends its paragraph"
    );
    assert_eq!(q.table_anchor, Some(23), "the table stands after that mark");
    assert_eq!(text(&back.paragraphs[1]), "After the table.");
}

/// A table the model stands INSIDE the text is written there, and one at
/// the end of the text stays a character of the paragraph.
#[test]
fn a_table_inside_the_text_is_written_at_its_character() {
    let mut inside = para(&["before", " after"]);
    inside.table = Some(table());
    inside.table_anchor = Some(6);
    let mut at_end = para(&["text then table"]);
    at_end.table = Some(table());
    at_end.table_anchor = Some(15);
    let (back, _) = round_trip(Story {
        paragraphs: vec![inside, at_end],
        ..Default::default()
    });
    assert_eq!(text(&back.paragraphs[0]), "before after");
    assert_eq!(back.paragraphs[0].table_anchor, Some(6));
    assert_eq!(text(&back.paragraphs[1]), "text then table");
    assert_eq!(back.paragraphs[1].table_anchor, Some(15));
}

// ---------------------------------------------------------------------
// A minted story whose part an earlier save already wrote
// ---------------------------------------------------------------------

/// The engine reloads a `.paged` from its native model, where a minted
/// story still has no source path; its part, written by the last save,
/// is the source the next export patches. Save `story` once with
/// `first`, then again with `second` over that save.
fn resave(first: Story, second: Story) -> (Vec<u8>, Vec<u8>, paged_scene::Document) {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story: first,
    });
    pkg::place_story(&mut doc, "tf_u9", "Story/u9");
    let saved = write_idml(&doc, &source).expect("first save");
    let story = doc
        .stories
        .iter_mut()
        .find(|s| s.self_id == "Story/u9")
        .unwrap();
    story.story = second;
    let again = write_idml(&doc, &saved).expect("second save");
    (saved, again, doc)
}

fn reread(package: &[u8]) -> Story {
    pkg::open(package)
        .stories
        .iter()
        .find(|s| s.self_id.ends_with("u9"))
        .expect("the story")
        .story
        .clone()
}

/// An object the model gained after the part was written reaches the
/// export — the part's patch lane adds no object, so the part is written
/// fresh — and a third save of the same model changes nothing.
#[test]
fn an_object_added_after_a_save_reaches_the_next_one() {
    let plain = Story {
        paragraphs: vec![para(&["Just here over the wire."])],
        ..Default::default()
    };
    let mut with_box = plain.clone();
    with_box.paragraphs[0].anchored_frames = vec![rectangle("rb", 9.0, 9.0)];
    with_box.paragraphs[0].anchored_frame_offsets = vec![9];
    let (_, again, doc) = resave(plain, with_box);
    let back = reread(&again);
    assert_eq!(back.paragraphs[0].anchored_frame_offsets, vec![9]);
    assert_eq!(text(&back.paragraphs[0]), "Just here over the wire.");
    let third = write_idml(&doc, &again).expect("third save");
    pkg::assert_same_entries(&again, &third);
}

/// The table with no anchor, saved over a part an older export wrote
/// with the table as a character of the text's own paragraph.
#[test]
fn a_table_after_its_text_is_corrected_in_an_older_part() {
    let mut p = para(&["Circulation by region."]);
    p.table = Some(table());
    let story = Story {
        paragraphs: vec![p, para(&["After."])],
        ..Default::default()
    };
    // What the older writer produced: the table straight after the text.
    let mut older = story.clone();
    older.paragraphs[0].table_anchor = Some(22);
    let (saved, again, doc) = resave(older, story);
    assert_eq!(
        text(&reread(&saved).paragraphs[0]),
        "Circulation by region."
    );
    let back = reread(&again);
    assert_eq!(text(&back.paragraphs[0]), "Circulation by region.\n");
    assert_eq!(back.paragraphs[0].table_anchor, Some(23));
    let third = write_idml(&doc, &again).expect("third save");
    pkg::assert_same_entries(&again, &third);
}

/// A paragraph holding an anchored object AND a next / previous page
/// number: the objects' path cuts the run's ranges at the markers too, so
/// each flavour reads back as itself, and the object stays at its
/// character.
#[test]
fn page_number_flavours_survive_beside_an_anchored_object() {
    use idml_import::{NEXT_PAGE_NUMBER_MARKER as NEXT, PREVIOUS_PAGE_NUMBER_MARKER as PREV};
    let line = format!("Box here, continued on page {NEXT} from {PREV}.");
    let mut p = para(&[line.as_str()]);
    p.anchored_frames = vec![rectangle("rb", 9.0, 9.0)];
    p.anchored_frame_offsets = vec![8];
    let (back, xml) = round_trip(Story {
        paragraphs: vec![p.clone(), para(&["After."])],
        ..Default::default()
    });
    assert!(
        xml.contains(r#"PageNumberType="NextPageNumber""#)
            && xml.contains(r#"PageNumberType="PreviousPageNumber""#),
        "each flavour in a range that names it:\n{xml}"
    );
    let q = &back.paragraphs[0];
    assert_eq!(
        text(q),
        text(&p),
        "every marker reads back as its own flavour"
    );
    assert_eq!(q.anchored_frame_offsets, vec![8]);
    assert_eq!(q.anchored_frames[0].self_id.as_deref(), Some("rb"));
}
