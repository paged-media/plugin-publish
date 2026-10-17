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

//! A story's table REPLACED since its part was written is exported as the
//! table the story holds now.
//!
//! The patch path matched a story-level `<Table>` by position only: a
//! source range that carried a table counted as "has its table", so the
//! part's old `<Table>` passed through verbatim and the model's new one
//! was never written. A document reopened from `.paged`, its table
//! deleted and a different one inserted (or the table swapped on its
//! paragraph by a plugin refreshing it), exported the deleted table and
//! dropped the new one.
//!
//! The model edits below are the engine's own: `RemoveNode` on a table
//! drops its host paragraph, `InsertTable` puts the table on the story's
//! last paragraph when that one is empty and on a fresh paragraph at the
//! end otherwise.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story, Table, TableCell, TableColumn, TableRow};

/// A `cols`×`rows` table whose cells say `<tag> c<col>r<row>`.
fn table(id: &str, cols: u32, rows: u32, tag: &str) -> Table {
    let mut cells = Vec::new();
    for c in 0..cols {
        for r in 0..rows {
            cells.push(TableCell {
                name: Some(format!("{c}:{r}")),
                row_span: 1,
                column_span: 1,
                paragraphs: vec![Paragraph {
                    runs: vec![CharacterRun {
                        text: format!("{tag} c{c}r{r}"),
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
        body_row_count: rows,
        column_count: cols,
        rows: (0..rows)
            .map(|r| TableRow {
                name: Some(r.to_string()),
                single_row_height: Some(15.0),
                ..Default::default()
            })
            .collect(),
        columns: (0..cols)
            .map(|c| TableColumn {
                name: Some(c.to_string()),
                single_column_width: Some(90.0),
                ..Default::default()
            })
            .collect(),
        cells,
        ..Default::default()
    }
}

fn text_paragraph(text: &str) -> Paragraph {
    Paragraph {
        runs: vec![CharacterRun {
            text: text.into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn table_paragraph(t: Table) -> Paragraph {
    Paragraph {
        table: Some(t),
        ..Default::default()
    }
}

/// Every table of a story with its shape and first cell's text.
fn tables_of(story: &Story) -> Vec<(String, usize, usize, String)> {
    story
        .paragraphs
        .iter()
        .filter_map(|p| p.table.as_ref())
        .map(|t| {
            (
                t.self_id.clone().unwrap_or_default(),
                t.rows.len(),
                t.columns.len(),
                t.cells
                    .first()
                    .and_then(|c| c.paragraphs.first())
                    .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

fn story<'a>(doc: &'a paged_scene::Document, id: &str) -> &'a Story {
    &doc.stories
        .iter()
        .find(|s| s.self_id == id)
        .unwrap_or_else(|| panic!("story {id}"))
        .story
}

/// Write a package whose story `Story/u849` holds `paragraphs` — the
/// document as it was saved, reopened through the importer.
fn saved_with(paragraphs: Vec<Paragraph>) -> (Vec<u8>, paged_scene::Document) {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u849".into(),
        story: Story {
            paragraphs,
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u849", "Story/u849");
    let saved = write_idml(&doc, &source).expect("save");
    let reopened = pkg::open(&saved);
    (saved, reopened)
}

/// Delete the story's table and insert a different one, the way the
/// engine does both.
fn replace_table(paragraphs: &mut Vec<Paragraph>, old: &str, new: Table) {
    let at = paragraphs
        .iter()
        .position(|p| p.table.as_ref().and_then(|t| t.self_id.as_deref()) == Some(old))
        .expect("the old table");
    paragraphs.remove(at);
    match paragraphs.last_mut() {
        Some(last) if last.runs.iter().all(|r| r.text.is_empty()) && last.table.is_none() => {
            last.table = Some(new)
        }
        _ => paragraphs.push(table_paragraph(new)),
    }
}

/// The source story carries text then a table; the table is deleted and
/// a different one inserted after the document was reopened.
#[test]
fn a_table_replaced_after_reopening_is_the_one_exported() {
    let (saved, mut doc) = saved_with(vec![
        text_paragraph("Day book"),
        table_paragraph(table("ufe0bc2", 3, 4, "old")),
    ]);
    let st = doc
        .stories
        .iter_mut()
        .find(|s| s.self_id == "Story_u849")
        .unwrap();
    assert!(!st.src.is_empty(), "reopened from its own part");
    replace_table(
        &mut st.story.paragraphs,
        "ufe0bc2",
        table("ufe0bd7", 3, 6, "new"),
    );

    let out = write_idml(&doc, &saved).expect("export");
    let xml = pkg::entry(&out, "Stories/Story_Story_u849.xml").expect("part");
    assert!(xml.contains(r#"<Table Self="ufe0bd7""#), "{xml}");
    assert!(!xml.contains(r#"<Table Self="ufe0bc2""#), "{xml}");
    let re = pkg::open(&out);
    let back = story(&re, "Story_u849");
    assert_eq!(
        tables_of(back),
        vec![("ufe0bd7".to_string(), 6, 3, "new c0r0".to_string())]
    );
    assert_eq!(back.paragraphs[0].runs[0].text, "Day book");
    // The export now carries the table: a second save is byte-identical.
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

/// The day-book shape: a minted story whose only paragraph holds the
/// table (behind an empty run), reloaded from the native model
/// (`src: ""`) over the checkpoint's part, the table swapped on its
/// paragraph.
#[test]
fn a_table_swapped_on_its_paragraph_in_a_checkpointed_minted_story_is_the_one_exported() {
    let mut first = table_paragraph(table("ufe0bc2", 3, 24, "old"));
    first.runs = vec![CharacterRun::default()];
    let (checkpoint, _) = saved_with(vec![first.clone()]);
    assert!(pkg::entry(&checkpoint, "Stories/Story_Story_u849.xml")
        .unwrap()
        .contains(r#"<Table Self="ufe0bc2""#));

    // The native model: the story as minted, `src` still empty.
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut model = pkg::open(&source);
    first.table = Some(table("ufe0bd7", 3, 24, "new"));
    model.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u849".into(),
        story: Story {
            paragraphs: vec![first],
            ..Default::default()
        },
    });
    pkg::place_story(&mut model, "tf_u849", "Story/u849");

    let out = write_idml(&model, &checkpoint).expect("export over the checkpoint");
    let xml = pkg::entry(&out, "Stories/Story_Story_u849.xml").expect("part");
    assert_eq!(xml.matches("<Table ").count(), 1, "{xml}");
    assert!(xml.contains(r#"<Table Self="ufe0bd7""#), "{xml}");
    let re = pkg::open(&out);
    assert_eq!(
        tables_of(story(&re, "Story_u849")),
        vec![("ufe0bd7".to_string(), 24, 3, "new c0r0".to_string())]
    );
}

/// A table taken off its paragraph (the paragraph stays — an undone
/// `InsertTable` on an empty last paragraph restores it without one) is
/// not exported.
#[test]
fn a_table_taken_off_its_paragraph_is_not_exported() {
    let (saved, mut doc) = saved_with(vec![
        text_paragraph("Day book"),
        table_paragraph(table("ufe0bc2", 2, 2, "old")),
        text_paragraph("After"),
    ]);
    let st = doc
        .stories
        .iter_mut()
        .find(|s| s.self_id == "Story_u849")
        .unwrap();
    st.story.paragraphs[1].table = None;

    let out = write_idml(&doc, &saved).expect("export");
    let xml = pkg::entry(&out, "Stories/Story_Story_u849.xml").expect("part");
    assert!(!xml.contains("<Table"), "{xml}");
    let re = pkg::open(&out);
    let back = story(&re, "Story_u849");
    assert!(tables_of(back).is_empty());
    let texts: Vec<String> = back
        .paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .collect();
    // The paragraph stays, an empty line where the table was.
    assert_eq!(texts, vec!["Day book", "", "After"]);
}

/// An unchanged table still passes through untouched (the part's bytes,
/// not a re-serialisation).
#[test]
fn an_unchanged_table_is_carried_through_verbatim() {
    let (saved, doc) = saved_with(vec![
        text_paragraph("Day book"),
        table_paragraph(table("ufe0bc2", 2, 2, "old")),
    ]);
    pkg::assert_same_package(&saved, &write_idml(&doc, &saved).expect("export"));
}

/// The same table, its cells re-poured after the document was reopened
/// from its native model: the engine mints cells without a `Self`, the
/// part names them after the table (`<table>i<n>`), so no cell of the
/// part matched one of the model and every cell replayed the
/// checkpoint's text — silently, the table's id and shape being right.
#[test]
fn cell_text_poured_into_a_minted_table_after_reopening_is_exported() {
    let mut first = table_paragraph(table("ufe0bc2", 3, 4, "old"));
    for cell in first.table.as_mut().unwrap().cells.iter_mut() {
        cell.self_id = None; // as `InsertTable` mints them
    }
    let (checkpoint, _) = saved_with(vec![text_paragraph("Day book"), first.clone()]);

    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut model = pkg::open(&source);
    model.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u849".into(),
        story: Story {
            paragraphs: vec![text_paragraph("Day book"), first],
            ..Default::default()
        },
    });
    pkg::place_story(&mut model, "tf_u849", "Story/u849");

    // Unchanged, the part comes out as the checkpoint wrote it.
    let same = write_idml(&model, &checkpoint).expect("export");
    assert_eq!(
        pkg::entry(&same, "Stories/Story_Story_u849.xml"),
        pkg::entry(&checkpoint, "Stories/Story_Story_u849.xml")
    );

    let st = model
        .stories
        .iter_mut()
        .find(|s| s.self_id == "Story/u849")
        .unwrap();
    for cell in st.story.paragraphs[1]
        .table
        .as_mut()
        .unwrap()
        .cells
        .iter_mut()
    {
        cell.paragraphs[0].runs[0].text = cell.paragraphs[0].runs[0].text.replace("old", "new");
    }
    let out = write_idml(&model, &checkpoint).expect("export over the checkpoint");
    let xml = pkg::entry(&out, "Stories/Story_Story_u849.xml").expect("part");
    assert!(!xml.contains("old c"), "{xml}");
    let re = pkg::open(&out);
    let back = story(&re, "Story_u849");
    let t = back.paragraphs[1].table.as_ref().expect("table");
    let texts: Vec<String> = t
        .cells
        .iter()
        .map(|c| c.paragraphs[0].runs[0].text.clone())
        .collect();
    assert_eq!(texts.len(), 12);
    assert!(texts.iter().all(|t| t.starts_with("new ")), "{texts:?}");
    assert_eq!(texts[5], "new c1r1");
}
