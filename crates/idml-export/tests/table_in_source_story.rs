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

//! A table the MODEL holds on a story whose SOURCE part does not carry it
//! reaches the export through the patch path. The 134-page annual was
//! authored chapter by chapter, each chapter saving a `.paged` checkpoint
//! with an exporter that had no table lane; on the final export every
//! story had a (table-less) source part and all 16 tables were lost.
//!
//! Two shapes, both from that book:
//! * the story keeps its `src` and the model appended a table paragraph
//!   after the part was written (`InsertTable` always appends);
//! * the story is a MINTED one (`src: ""` in the reloaded model) whose
//!   derived entry the checkpoint already wrote — that used to be read as
//!   a name collision and the story was skipped wholesale, stale text
//!   included.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story, Table, TableCell, TableColumn, TableRow};

fn table(id: &str) -> Table {
    let mut cells = Vec::new();
    for c in 0..2 {
        for r in 0..2 {
            cells.push(TableCell {
                name: Some(format!("{c}:{r}")),
                row_span: 1,
                column_span: 1,
                paragraphs: vec![Paragraph {
                    runs: vec![CharacterRun {
                        text: format!("c{c}r{r}"),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            });
        }
    }
    Table {
        self_id: Some(id.into()),
        header_row_count: 1,
        body_row_count: 1,
        column_count: 2,
        rows: (0..2)
            .map(|r| TableRow {
                name: Some(r.to_string()),
                single_row_height: Some(20.0),
                ..Default::default()
            })
            .collect(),
        columns: (0..2)
            .map(|c| TableColumn {
                name: Some(c.to_string()),
                single_column_width: Some(80.0),
                ..Default::default()
            })
            .collect(),
        cells,
        ..Default::default()
    }
}

fn table_paragraph(id: &str) -> Paragraph {
    Paragraph {
        table: Some(table(id)),
        ..Default::default()
    }
}

fn tables_of(story: &Story) -> Vec<String> {
    story
        .paragraphs
        .iter()
        .filter_map(|p| p.table.as_ref().and_then(|t| t.self_id.clone()))
        .collect()
}

#[test]
fn a_table_appended_to_a_source_story_is_written_and_reads_back() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    assert_eq!(doc.stories[0].src, "Stories/Story_st1.xml");
    doc.stories[0].story.paragraphs.push(table_paragraph("u7"));

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    // The source's last paragraph gained its mark, then the table paragraph.
    assert!(
        xml.contains(r#"<Content>Hello world</Content></CharacterStyleRange><CharacterStyleRange><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Table Self="u7" HeaderRowCount="1""#),
        "{xml}"
    );
    let re = pkg::open(&out);
    assert_eq!(
        re.stories[0].story.paragraphs.len(),
        2,
        "{:#?}",
        re.stories[0].story.paragraphs
    );
    assert_eq!(
        re.stories[0].story.paragraphs[0].runs[0].text,
        "Hello world"
    );
    assert_eq!(tables_of(&re.stories[0].story), vec!["u7".to_string()]);
    let t = re.stories[0].story.paragraphs[1].table.as_ref().unwrap();
    assert_eq!((t.rows.len(), t.columns.len(), t.cells.len()), (2, 2, 4));
    assert_eq!(t.cells[3].paragraphs[0].runs[0].text, "c1r1");
    // Now the source carries it: a second save is byte-identical.
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

#[test]
fn a_table_dropped_from_the_part_by_hand_comes_back_on_export() {
    // First export mints the story with its table.
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story: Story {
            paragraphs: vec![
                Paragraph {
                    runs: vec![CharacterRun {
                        text: "Figures".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                table_paragraph("u8"),
            ],
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u9", "Story/u9");
    let with_table = write_idml(&doc, &source).expect("first export");
    let reloaded = pkg::open(&with_table);
    let minted = reloaded
        .stories
        .iter()
        .find(|s| s.self_id == "Story_u9")
        .expect("minted story");
    assert_eq!(tables_of(&minted.story), vec!["u8".to_string()]);

    // Drop the table from the part by hand — the checkpoint written by an
    // exporter that had no table lane.
    let part = pkg::entry(&with_table, "Stories/Story_Story_u9.xml").expect("part");
    let a = pkg::pos(&part, "<Table ");
    let b = pkg::pos(&part, "</Table>") + "</Table>".len();
    let stale_part = format!("{}{}", &part[..a], &part[b..]);
    assert!(!stale_part.contains("<Table"));
    let stale = pkg::package(&[
        (
            "designmap.xml",
            &pkg::entry(&with_table, "designmap.xml").unwrap(),
        ),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
        ("Stories/Story_Story_u9.xml", &stale_part),
    ]);
    assert!(tables_of(
        &pkg::open(&stale)
            .stories
            .iter()
            .find(|s| s.self_id == "Story_u9")
            .unwrap()
            .story
    )
    .is_empty());

    // The model still holds the table; export against the stale package.
    let out = write_idml(&reloaded, &stale).expect("export over stale part");
    let xml = pkg::entry(&out, "Stories/Story_Story_u9.xml").expect("part");
    assert!(
        xml.contains(r#"<Table Self="u8""#),
        "the table is back:\n{xml}"
    );
    let re = pkg::open(&out);
    let story = re.stories.iter().find(|s| s.self_id == "Story_u9").unwrap();
    assert_eq!(tables_of(&story.story), vec!["u8".to_string()]);
    assert_eq!(story.story.paragraphs[0].runs[0].text, "Figures");
}

#[test]
fn a_minted_story_whose_part_a_checkpoint_already_wrote_is_patched_not_skipped() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                runs: vec![CharacterRun {
                    text: "Q2Q2The chart wall".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u9b", "Story/u9");
    let checkpoint = write_idml(&doc, &source).expect("checkpoint");
    assert!(pkg::entry(&checkpoint, "Stories/Story_Story_u9.xml").is_some());

    // The reloaded model keeps `src: ""` (a `.pgm` reload), the text was
    // edited and a table added after the checkpoint.
    let mut model = doc;
    let st = model
        .stories
        .iter_mut()
        .find(|s| s.self_id == "Story/u9")
        .unwrap();
    assert!(st.src.is_empty());
    st.story.paragraphs[0].runs[0].text = "The chart wall".into();
    st.story.paragraphs.push(table_paragraph("u8"));

    let out = write_idml(&model, &checkpoint).expect("export over the checkpoint");
    let xml = pkg::entry(&out, "Stories/Story_Story_u9.xml").expect("part");
    assert!(
        xml.contains("<Content>The chart wall</Content>"),
        "text patched:\n{xml}"
    );
    assert!(!xml.contains("Q2Q2"), "stale text gone:\n{xml}");
    assert!(xml.contains(r#"<Table Self="u8""#), "table added:\n{xml}");
    let re = pkg::open(&out);
    let story = re.stories.iter().find(|s| s.self_id == "Story_u9").unwrap();
    assert_eq!(story.story.paragraphs[0].runs[0].text, "The chart wall");
    assert_eq!(tables_of(&story.story), vec!["u8".to_string()]);
    // And the designmap did not gain a second reference to the part.
    let dm = pkg::entry(&out, "designmap.xml").unwrap();
    assert_eq!(dm.matches("Stories/Story_Story_u9.xml").count(), 1, "{dm}");
}

/// The column fallback is the host frame's inner width shared out:
/// `tf1` is 300 pt wide with no insets and one text column, so an
/// unsized 2-column table appended to its story gets 150 pt columns.
#[test]
fn an_unsized_table_in_a_framed_story_shares_the_frames_width() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    let mut t = table("u7");
    for c in t.columns.iter_mut() {
        c.single_column_width = None;
    }
    for r in t.rows.iter_mut() {
        r.single_row_height = None;
    }
    doc.stories[0].story.paragraphs.push(Paragraph {
        table: Some(t),
        ..Default::default()
    });
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert_eq!(
        xml.matches(r#"SingleColumnWidth="150""#).count(),
        2,
        "{xml}"
    );
    assert_eq!(xml.matches(r#"SingleRowHeight="24""#).count(), 2, "{xml}");
    let re = pkg::open(&out);
    let t = re.stories[0].story.paragraphs[1].table.as_ref().unwrap();
    assert!(t
        .columns
        .iter()
        .all(|c| c.single_column_width == Some(150.0)));
}
