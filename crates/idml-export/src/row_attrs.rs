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

//! A source table row's `AutoGrow`, `KeepWithNextRow` and `StartRow`,
//! patched back.
//!
//! All three decide where InDesign puts a row (core `tables-rows` pages
//! 10-11 and `table-start-row`, measured on InDesign 20.0.1):
//! `AutoGrow="false"` keeps a row at its `SingleRowHeight` and oversets
//! what does not fit, `KeepWithNextRow="true"` moves a run of kept rows to
//! the next frame together, and `StartRow` moves a row and the rows after
//! it to the next column, frame or page. They are model-owned
//! (`TableRow::auto_grow` / `keep_with_next_row` / `start_row`), so a
//! source `<Row>` whose model row says something else is patched: a
//! changed value in place, a new one appended, a cleared one dropped
//! (absent = InDesign's default: `AutoGrow` true, `KeepWithNextRow` false,
//! `StartRow` Anywhere). A row whose model still says what its source
//! spells keeps its bytes, so an unmutated save is byte-identical. Rows
//! the model does not know (no `Self` match) pass through.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{Paragraph, Story};
use paged_model::StartParagraph;

use crate::rewrite::{patch_start, Patch};

/// The model-owned placement attributes of one row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RowAttrs {
    pub auto_grow: Option<bool>,
    pub keep_with_next_row: Option<bool>,
    pub start_row: Option<StartParagraph>,
}

/// The [`RowAttrs`] of every row in `story`, keyed by the row's `Self` —
/// nested tables (a cell's paragraphs) and footnotes walked too.
pub fn row_attrs_of(story: &Story) -> HashMap<String, RowAttrs> {
    fn walk(paragraphs: &[Paragraph], out: &mut HashMap<String, RowAttrs>) {
        for p in paragraphs {
            if let Some(t) = &p.table {
                for r in &t.rows {
                    if let Some(id) = &r.self_id {
                        out.insert(
                            id.clone(),
                            RowAttrs {
                                auto_grow: r.auto_grow,
                                keep_with_next_row: r.keep_with_next_row,
                                start_row: r.start_row,
                            },
                        );
                    }
                }
                for c in &t.cells {
                    walk(&c.paragraphs, out);
                }
            }
            for f in &p.footnotes {
                walk(&f.paragraphs, out);
            }
        }
    }
    let mut out = HashMap::new();
    walk(&story.paragraphs, &mut out);
    out
}

const ROW: &[u8] = b"Row";
const AUTO_GROW: &[u8] = b"AutoGrow";
const KEEP_WITH_NEXT_ROW: &[u8] = b"KeepWithNextRow";
const START_ROW: &[u8] = b"StartRow";

fn self_of(e: &BytesStart) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == b"Self")
        .and_then(|a| std::str::from_utf8(&a.value).ok().map(|s| s.to_string()))
}

fn raw_of(e: &BytesStart, key: &[u8]) -> Option<Vec<u8>> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| a.value.to_vec())
}

/// What one model-owned boolean asks of the source attribute `raw`.
fn bool_patch(raw: &[u8], model: Option<bool>) -> Patch {
    match model {
        Some(v) if std::str::from_utf8(raw).ok().and_then(|s| s.parse().ok()) == Some(v) => {
            Patch::Keep
        }
        Some(v) => Patch::Set(v.to_string()),
        None => Patch::Remove,
    }
}

fn read_start(raw: &[u8]) -> Option<StartParagraph> {
    StartParagraph::from_idml(std::str::from_utf8(raw).ok()?)
}

/// What the model's start rule asks of the source `StartRow` `raw`.
fn start_patch(raw: &[u8], model: Option<StartParagraph>) -> Patch {
    match model {
        Some(v) if read_start(raw) == Some(v) => Patch::Keep,
        Some(v) => Patch::Set(v.as_idml().to_string()),
        None => Patch::Remove,
    }
}

/// Whether `e` (a `<Row>`) spells something other than its model row.
/// A source value that does not parse read as `None`, so the comparison
/// is against what the reader made of it.
fn differs(e: &BytesStart, model: RowAttrs) -> bool {
    let read =
        |k| raw_of(e, k).and_then(|raw| std::str::from_utf8(&raw).ok()?.parse::<bool>().ok());
    read(AUTO_GROW) != model.auto_grow
        || read(KEEP_WITH_NEXT_ROW) != model.keep_with_next_row
        || raw_of(e, START_ROW).and_then(|raw| read_start(&raw)) != model.start_row
}

fn patched(e: &BytesStart, model: RowAttrs) -> Result<BytesStart<'static>, quick_xml::Error> {
    let mut extras: Vec<(&str, String)> = Vec::new();
    if let Some(v) = model.auto_grow {
        extras.push(("AutoGrow", v.to_string()));
    }
    if let Some(v) = model.keep_with_next_row {
        extras.push(("KeepWithNextRow", v.to_string()));
    }
    if let Some(v) = model.start_row {
        extras.push(("StartRow", v.as_idml().to_string()));
    }
    patch_start(
        e,
        |k, raw| match k {
            AUTO_GROW => Some(bool_patch(raw, model.auto_grow)),
            KEEP_WITH_NEXT_ROW => Some(bool_patch(raw, model.keep_with_next_row)),
            START_ROW => Some(start_patch(raw, model.start_row)),
            _ => None,
        },
        &extras,
    )
}

/// Patch every source `<Row>` whose model row (`known`, from
/// [`row_attrs_of`]) spells a different `AutoGrow` / `KeepWithNextRow` /
/// `StartRow`. Byte-identical when none does.
pub fn patch_row_attrs(
    original: &[u8],
    known: &HashMap<String, RowAttrs>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let model_of = |e: &BytesStart| -> Option<RowAttrs> {
        if e.name().as_ref() != ROW {
            return None;
        }
        let m = *known.get(&self_of(e)?)?;
        differs(e, m).then_some(m)
    };
    let any = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut found = false;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(e) | Event::Empty(e) if model_of(&e).is_some() => {
                    found = true;
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        found
    };
    if !any {
        return Ok(original.to_vec());
    }
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Eof => break,
            Event::Start(ref e) => match model_of(e) {
                Some(m) => writer.write_event(Event::Start(patched(e, m)?))?,
                None => writer.write_event(ev.borrow())?,
            },
            Event::Empty(ref e) => match model_of(e) {
                Some(m) => writer.write_event(Event::Empty(patched(e, m)?))?,
                None => writer.write_event(ev.borrow())?,
            },
            other => writer.write_event(other.borrow())?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"<Story Self="u1"><Table Self="t"><Row Self="r0" Name="0" SingleRowHeight="20" AutoGrow="false"/><Row Self="r1" Name="1" SingleRowHeight="20" KeepWithNextRow="true"/><Row Self="r2" Name="2" SingleRowHeight="20"/><Row Self="r3" Name="3" SingleRowHeight="20" StartRow="NextFrame"/></Table></Story>"#;

    /// `(Self, AutoGrow, KeepWithNextRow, StartRow)` of a model row.
    type Row<'a> = (&'a str, Option<bool>, Option<bool>, Option<StartParagraph>);

    fn known(rows: &[Row]) -> HashMap<String, RowAttrs> {
        rows.iter()
            .map(|(id, a, k, s)| {
                (
                    id.to_string(),
                    RowAttrs {
                        auto_grow: *a,
                        keep_with_next_row: *k,
                        start_row: *s,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn rows_the_model_still_agrees_with_are_byte_identical() {
        let k = known(&[
            ("r0", Some(false), None, None),
            ("r1", None, Some(true), None),
            ("r2", None, None, None),
            ("r3", None, None, Some(StartParagraph::NextFrame)),
        ]);
        assert_eq!(patch_row_attrs(SRC.as_bytes(), &k).unwrap(), SRC.as_bytes());
    }

    #[test]
    fn a_changed_value_patches_in_place_a_new_one_appends_a_cleared_one_drops() {
        let k = known(&[
            ("r0", Some(true), None, None),
            ("r1", None, None, Some(StartParagraph::NextOddPage)),
            ("r2", Some(false), Some(true), None),
            ("r3", None, None, Some(StartParagraph::NextColumn)),
        ]);
        let out = String::from_utf8(patch_row_attrs(SRC.as_bytes(), &k).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<Story Self="u1"><Table Self="t"><Row Self="r0" Name="0" SingleRowHeight="20" AutoGrow="true"/><Row Self="r1" Name="1" SingleRowHeight="20" StartRow="NextOddPage"/><Row Self="r2" Name="2" SingleRowHeight="20" AutoGrow="false" KeepWithNextRow="true"/><Row Self="r3" Name="3" SingleRowHeight="20" StartRow="NextColumn"/></Table></Story>"#
        );
    }

    #[test]
    fn a_cleared_start_row_drops_its_attribute() {
        let k = known(&[("r3", None, None, None)]);
        let out = String::from_utf8(patch_row_attrs(SRC.as_bytes(), &k).unwrap()).unwrap();
        assert!(
            out.contains(r#"<Row Self="r3" Name="3" SingleRowHeight="20"/>"#),
            "{out}"
        );
    }

    #[test]
    fn rows_the_model_does_not_know_pass_through() {
        assert_eq!(
            patch_row_attrs(SRC.as_bytes(), &HashMap::new()).unwrap(),
            SRC.as_bytes()
        );
    }
}
