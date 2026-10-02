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

//! A table row's `AutoGrow` and `KeepWithNextRow` reach the model and
//! come back out.
//!
//! Both decide where InDesign 20.0.1 puts a row (core's `tables-rows`
//! fixture, pages 10-11): a fixed row (`AutoGrow="false"`) keeps its
//! `SingleRowHeight` and oversets what does not fit, and a run of
//! `KeepWithNextRow="true"` rows moves to the next frame together. The
//! reader dropped both, so every row grew and every row broke alone.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story, Table, TableCell, TableRow};

/// A one-column table whose three rows spell `rows[i]` after their
/// height (InDesign writes `AutoGrow` there; `KeepWithNextRow` it writes
/// only when set). Cells spell all four insets, as InDesign's do, so the
/// inset pass (`cell_insets`) leaves them alone.
fn package_with(rows: [&str; 3]) -> Vec<u8> {
    let row_xml: String = rows
        .iter()
        .enumerate()
        .map(|(i, extra)| {
            format!(
                r#"<Row Self="tRow{i}" Name="{i}" SingleRowHeight="20" MinimumHeight="20"{extra}/>"#
            )
        })
        .collect();
    let cells: String = (0..3)
        .map(|i| {
            format!(
                r#"<Cell Self="ti{i}" Name="0:{i}" RowSpan="1" ColumnSpan="1" TextTopInset="4" TextLeftInset="4" TextBottomInset="4" TextRightInset="4"><ParagraphStyleRange><CharacterStyleRange><Content>r{i}</Content></CharacterStyleRange></ParagraphStyleRange></Cell>"#
            )
        })
        .collect();
    let body = format!(
        r#"<ParagraphStyleRange><CharacterStyleRange><Table Self="t" HeaderRowCount="0" FooterRowCount="0" BodyRowCount="3" ColumnCount="1">{row_xml}<Column Self="tColumn0" Name="0" SingleColumnWidth="100"/>{cells}</Table></CharacterStyleRange></ParagraphStyleRange>"#
    );
    pkg::simple("", &body, pkg::STYLES)
}

const SET: [&str; 3] = [
    r#" AutoGrow="false""#,
    r#" AutoGrow="true" KeepWithNextRow="true""#,
    "",
];

fn rows(doc: &paged_scene::Document) -> &[TableRow] {
    let story = doc
        .stories
        .iter()
        .find(|s| s.src == "Stories/Story_st1.xml")
        .expect("story");
    &story
        .story
        .paragraphs
        .iter()
        .find_map(|p| p.table.as_ref())
        .expect("table")
        .rows
}

fn rows_mut(doc: &mut paged_scene::Document) -> &mut Vec<TableRow> {
    let story = doc
        .stories
        .iter_mut()
        .find(|s| s.src == "Stories/Story_st1.xml")
        .expect("story");
    &mut story
        .story
        .paragraphs
        .iter_mut()
        .find_map(|p| p.table.as_mut())
        .expect("table")
        .rows
}

#[test]
fn each_row_reads_its_auto_grow_and_keep_with_next_row() {
    let doc = pkg::open(&package_with(SET));
    let got: Vec<_> = rows(&doc)
        .iter()
        .map(|r| (r.auto_grow, r.keep_with_next_row))
        .collect();
    assert_eq!(
        got,
        vec![(Some(false), None), (Some(true), Some(true)), (None, None)]
    );
}

#[test]
fn an_unmutated_save_is_byte_identical() {
    for set in [
        SET,
        ["", "", ""],
        [r#" KeepWithNextRow="false""#, r#" AutoGrow="false""#, ""],
    ] {
        let source = package_with(set);
        let doc = pkg::open(&source);
        let out = write_idml(&doc, &source).expect("write");
        pkg::assert_same_package(&source, &out);
    }
}

#[test]
fn a_changed_value_is_written_in_place_a_new_one_appended_a_cleared_one_dropped() {
    let source = package_with(SET);
    let mut doc = pkg::open(&source);
    {
        let rows = rows_mut(&mut doc);
        rows[0].auto_grow = Some(true); // changed
        rows[1].keep_with_next_row = None; // cleared
        rows[2].auto_grow = Some(false); // new
        rows[2].keep_with_next_row = Some(true); // new
    }
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(
        xml.contains(
            r#"<Row Self="tRow0" Name="0" SingleRowHeight="20" MinimumHeight="20" AutoGrow="true"/>"#
        ),
        "patched in place:\n{xml}"
    );
    assert!(
        xml.contains(
            r#"<Row Self="tRow1" Name="1" SingleRowHeight="20" MinimumHeight="20" AutoGrow="true"/>"#
        ),
        "dropped:\n{xml}"
    );
    assert!(
        xml.contains(
            r#"<Row Self="tRow2" Name="2" SingleRowHeight="20" MinimumHeight="20" AutoGrow="false" KeepWithNextRow="true"/>"#
        ),
        "appended:\n{xml}"
    );
    let re = pkg::open(&out);
    let got: Vec<_> = rows(&re)
        .iter()
        .map(|r| (r.auto_grow, r.keep_with_next_row))
        .collect();
    assert_eq!(
        got,
        vec![
            (Some(true), None),
            (Some(true), None),
            (Some(false), Some(true))
        ]
    );
}

#[test]
fn a_minted_table_writes_its_fixed_and_kept_rows() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    let row = |r: u32, auto_grow, keep| TableRow {
        name: Some(r.to_string()),
        single_row_height: Some(20.0),
        auto_grow,
        keep_with_next_row: keep,
        ..Default::default()
    };
    let cell = |r: u32| TableCell {
        name: Some(format!("0:{r}")),
        row_span: 1,
        column_span: 1,
        paragraphs: vec![Paragraph {
            runs: vec![CharacterRun {
                text: format!("r{r}"),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let table = Table {
        self_id: Some("u7".into()),
        body_row_count: 3,
        column_count: 1,
        rows: vec![
            row(0, Some(false), None),
            row(1, None, Some(true)),
            row(2, None, None),
        ],
        cells: (0..3).map(cell).collect(),
        ..Default::default()
    };
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u1".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                table: Some(table),
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u1", "Story/u1");
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_Story_u1.xml").expect("minted story");
    assert!(
        xml.contains(r#"<Row Self="u7Row0" Name="0" SingleRowHeight="20" MinimumHeight="20" AutoGrow="false"/>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<Row Self="u7Row1" Name="1" SingleRowHeight="20" MinimumHeight="20" AutoGrow="true" KeepWithNextRow="true"/>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<Row Self="u7Row2" Name="2" SingleRowHeight="20" MinimumHeight="20" AutoGrow="true"/>"#),
        "{xml}"
    );
    let re = pkg::open(&out);
    let minted = re
        .stories
        .iter()
        .find(|s| s.src.contains("u1"))
        .expect("minted story");
    let got: Vec<_> = minted
        .story
        .paragraphs
        .iter()
        .find_map(|p| p.table.as_ref())
        .expect("table")
        .rows
        .iter()
        .map(|r| (r.auto_grow, r.keep_with_next_row))
        .collect();
    assert_eq!(
        got,
        vec![
            (Some(false), None),
            (Some(true), Some(true)),
            (Some(true), None)
        ]
    );
}
