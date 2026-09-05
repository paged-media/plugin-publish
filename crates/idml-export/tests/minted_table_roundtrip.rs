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

//! A `<Table>` inside a MINTED story reaches the story part — rows,
//! columns, cells with content, header count, spans, styles, widths,
//! heights — and reads back. A 134-page book authored through the
//! engine had 7 runtime `insertTable`s; its export had `<Table>` 0.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story, Table, TableCell, TableColumn, TableRow};

fn cell(c: u32, r: u32, text: &str) -> TableCell {
    TableCell {
        name: Some(format!("{c}:{r}")),
        row_span: 1,
        column_span: 1,
        text_top_inset: 4.0,
        text_left_inset: 4.0,
        text_bottom_inset: 4.0,
        text_right_inset: 4.0,
        applied_cell_style: Some("CellStyle/$ID/[None]".into()),
        paragraphs: vec![Paragraph {
            runs: vec![CharacterRun {
                text: text.into(),
                point_size: Some(9.0),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn table() -> Table {
    let mut cells = Vec::new();
    // Column-major: every row of column 0, then column 1, then column 2.
    for c in 0..3 {
        for r in 0..2 {
            cells.push(cell(c, r, &format!("c{c}r{r}")));
        }
    }
    cells[0].column_span = 2; // header cell spanning two columns
    cells[5].paragraphs.clear(); // an EMPTY cell
    Table {
        self_id: Some("u7".into()),
        header_row_count: 1,
        footer_row_count: 0,
        body_row_count: 1,
        column_count: 3,
        applied_table_style: Some("TableStyle/$ID/[Basic Table]".into()),
        rows: vec![
            TableRow {
                name: Some("0".into()),
                single_row_height: Some(22.0),
                ..Default::default()
            },
            TableRow {
                name: Some("1".into()),
                single_row_height: Some(18.5),
                ..Default::default()
            },
        ],
        columns: (0..3)
            .map(|c| TableColumn {
                name: Some(c.to_string()),
                single_column_width: Some(60.0 + c as f32),
                ..Default::default()
            })
            .collect(),
        cells,
        ..Default::default()
    }
}

#[test]
fn a_table_in_a_minted_story_is_written_and_reads_back() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u1".into(),
        story: Story {
            paragraphs: vec![
                Paragraph {
                    runs: vec![CharacterRun {
                        text: "Table below".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Paragraph {
                    table: Some(table()),
                    ..Default::default()
                },
                Paragraph {
                    runs: vec![CharacterRun {
                        text: "After".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u1", "Story/u1");

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_Story_u1.xml").expect("minted story");
    assert!(xml.contains(r#"<Table Self="u7" HeaderRowCount="1" FooterRowCount="0" BodyRowCount="1" ColumnCount="3" AppliedTableStyle="TableStyle/$ID/[Basic Table]" TableDirection="LeftToRightDirection">"#), "{xml}");
    assert!(
        xml.contains(r#"<Row Self="u7Row0" Name="0" SingleRowHeight="22"/>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<Column Self="u7Column2" Name="2" SingleColumnWidth="62"/>"#),
        "{xml}"
    );
    assert!(xml.contains(r#"<Cell Self="u7i0" Name="0:0" RowSpan="1" ColumnSpan="2" CellType="TextTypeCell" TextTopInset="4" TextLeftInset="4" TextBottomInset="4" TextRightInset="4" AppliedCellStyle="CellStyle/$ID/[None]">"#), "{xml}");
    // The table sits in its own range inside the paragraph, the mark after it.
    assert!(xml.contains(r#"<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Table "#), "{xml}");
    assert!(xml.contains("</Table><Br/></CharacterStyleRange>"), "{xml}");

    let re = pkg::open(&out);
    let story = re
        .stories
        .iter()
        .find(|s| s.self_id == "Story_u1")
        .expect("minted story back");
    assert_eq!(
        story.story.paragraphs.len(),
        3,
        "{:#?}",
        story.story.paragraphs
    );
    let t = story.story.paragraphs[1]
        .table
        .as_ref()
        .expect("table back");
    assert_eq!(t.self_id.as_deref(), Some("u7"));
    assert_eq!((t.rows.len(), t.columns.len()), (2, 3));
    assert_eq!((t.header_row_count, t.body_row_count), (1, 1));
    assert_eq!(t.cells.len(), 6);
    assert_eq!(t.cells[0].column_span, 2);
    assert_eq!(t.cells[0].coords(), Some((0, 0)));
    assert_eq!(t.cells[5].coords(), Some((2, 1)));
    assert_eq!(t.cells[4].paragraphs[0].runs[0].text, "c2r0");
    assert_eq!(t.cells[4].paragraphs[0].runs[0].point_size, Some(9.0));
    // The empty cell carries InDesign's empty paragraph, and reads back empty.
    assert!(
        xml.contains(r#"Name="2:1" RowSpan="1" ColumnSpan="1" CellType="TextTypeCell" TextTopInset="4" TextLeftInset="4" TextBottomInset="4" TextRightInset="4" AppliedCellStyle="CellStyle/$ID/[None]"><ParagraphStyleRange><CharacterStyleRange/></ParagraphStyleRange></Cell>"#),
        "{xml}"
    );
    assert!(t.cells[5].paragraphs.is_empty());
    assert_eq!(t.cells[1].text_top_inset, 4.0);
    assert_eq!(t.columns[1].single_column_width, Some(61.0));
    assert_eq!(t.rows[1].single_row_height, Some(18.5));
    assert_eq!(story.story.paragraphs[2].runs[0].text, "After");
    // A second save of the reopened package is byte-identical.
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

/// A bare `insertTable` (no sizing op) has rows and columns with no
/// size; InDesign 20.0.1 dropped exactly that table. Both attributes are
/// always spelled — 24 pt rows, 72 pt columns when no host frame width
/// is known (the frame here has no usable inner width).
#[test]
fn an_unsized_table_gets_the_fallback_row_height_and_column_width() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    let mut t = table();
    for r in t.rows.iter_mut() {
        r.single_row_height = None;
    }
    t.columns.clear(); // not even Column entries — only ColumnCount
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u2".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                table: Some(t),
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    // A story must be framed to reach the package (an orphan is
    // dropped); the fallback fires when the frame yields no usable
    // inner width — here a zero-width frame.
    pkg::place_story(&mut doc, "tf_u2", "Story/u2");
    let frame = doc.spreads[0].spread.text_frames.last_mut().unwrap();
    frame.bounds.right = frame.bounds.left;
    doc.rebuild_indexes();
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_Story_u2.xml").expect("minted story");
    assert_eq!(xml.matches(r#"SingleRowHeight="24""#).count(), 2, "{xml}");
    assert_eq!(xml.matches(r#"SingleColumnWidth="72""#).count(), 3, "{xml}");
    let re = pkg::open(&out);
    let story = re.stories.iter().find(|s| s.self_id == "Story_u2").unwrap();
    let t = story.story.paragraphs[0].table.as_ref().unwrap();
    assert!(t.rows.iter().all(|r| r.single_row_height == Some(24.0)));
    assert_eq!(t.columns.len(), 3);
    assert!(t
        .columns
        .iter()
        .all(|c| c.single_column_width == Some(72.0)));
}
