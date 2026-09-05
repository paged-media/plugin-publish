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

//! Table cell insets, spelled on every `<Cell>`.
//!
//! An inset InDesign does not find on a `<Cell>` is its application
//! default, 4 pt (measured 2026-09-06: `1.411 mm` on every cell of the
//! annual's tables, whose engine-minted cells spelled only the left and
//! right insets they set). The engine reads an absent inset as 0 pt, so
//! every such row stood 8 pt taller in InDesign than in the engine and
//! six table frames overflowed there alone. InDesign's own files spell
//! all four on every cell; so does the minted-table writer now
//! (`emit::write_table`), and this pass spells the MODEL's value on
//! every source cell that left one out — the number the engine composed
//! with — and InDesign's 4 pt for a cell the model does not know.
//! Byte-identical when every cell spells all four.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{Paragraph, Story};

use crate::rewrite::format_f32;

/// InDesign's cell inset when a `<Cell>` spells none (0.0556 in).
const INDESIGN_DEFAULT_INSET_PT: f32 = 4.0;

/// `[top, left, bottom, right]` insets of every cell in `story`, keyed
/// by the cell's `Self` — nested tables (a cell's paragraphs) and
/// footnotes walked too.
pub fn cell_insets_of(story: &Story) -> HashMap<String, [f32; 4]> {
    fn walk(paragraphs: &[Paragraph], out: &mut HashMap<String, [f32; 4]>) {
        for p in paragraphs {
            if let Some(t) = &p.table {
                for c in &t.cells {
                    if let Some(id) = &c.self_id {
                        out.insert(
                            id.clone(),
                            [
                                c.text_top_inset,
                                c.text_left_inset,
                                c.text_bottom_inset,
                                c.text_right_inset,
                            ],
                        );
                    }
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

const CELL: &[u8] = b"Cell";
const INSETS: [&[u8]; 4] = [
    b"TextTopInset",
    b"TextLeftInset",
    b"TextBottomInset",
    b"TextRightInset",
];

fn missing(e: &BytesStart) -> Vec<&'static [u8]> {
    INSETS
        .iter()
        .copied()
        .filter(|k| !e.attributes().flatten().any(|a| a.key.as_ref() == *k))
        .collect()
}

fn with_insets(
    e: &BytesStart,
    add: &[&[u8]],
    known: &HashMap<String, [f32; 4]>,
) -> BytesStart<'static> {
    let id = e
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == b"Self")
        .and_then(|a| std::str::from_utf8(&a.value).ok().map(|s| s.to_string()));
    let values = id.as_deref().and_then(|id| known.get(id));
    let mut out = e.to_owned();
    for k in add {
        let slot = INSETS.iter().position(|i| i == k).expect("one of the four");
        let v = values.map(|v| v[slot]).unwrap_or(INDESIGN_DEFAULT_INSET_PT);
        out.push_attribute((*k, format_f32(v).as_bytes()));
    }
    out
}

/// Spell every inset a `<Cell>` leaves out: the model's value for that
/// cell (`known`, from [`cell_insets_of`]), InDesign's 4 pt otherwise.
pub fn spell_cell_insets(
    original: &[u8],
    known: &HashMap<String, [f32; 4]>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let any = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut found = false;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(e) | Event::Empty(e)
                    if e.name().as_ref() == CELL && !missing(&e).is_empty() =>
                {
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
            Event::Start(ref e) if e.name().as_ref() == CELL => {
                let add = missing(e);
                if add.is_empty() {
                    writer.write_event(ev.borrow())?;
                } else {
                    writer.write_event(Event::Start(with_insets(e, &add, known)))?;
                }
            }
            Event::Empty(ref e) if e.name().as_ref() == CELL => {
                let add = missing(e);
                if add.is_empty() {
                    writer.write_event(ev.borrow())?;
                } else {
                    writer.write_event(Event::Empty(with_insets(e, &add, known)))?;
                }
            }
            other => writer.write_event(other.borrow())?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_missing_insets_gains_the_models_values_or_indesigns_default() {
        let src = r#"<Story Self="u1"><Table Self="t"><Cell Self="c0" Name="0:0" TextLeftInset="6" TextRightInset="6"><ParagraphStyleRange/></Cell><Cell Self="c1" Name="0:1"/><Cell Self="c2" Name="1:0" TextTopInset="4" TextLeftInset="4" TextBottomInset="4" TextRightInset="4"/></Table></Story>"#;
        // c0 is known to the model (0 top / bottom); c1 is not.
        let known = HashMap::from([("c0".to_string(), [0.0, 6.0, 0.0, 6.0])]);
        let out = String::from_utf8(spell_cell_insets(src.as_bytes(), &known).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<Story Self="u1"><Table Self="t"><Cell Self="c0" Name="0:0" TextLeftInset="6" TextRightInset="6" TextTopInset="0" TextBottomInset="0"><ParagraphStyleRange/></Cell><Cell Self="c1" Name="0:1" TextTopInset="4" TextLeftInset="4" TextBottomInset="4" TextRightInset="4"/><Cell Self="c2" Name="1:0" TextTopInset="4" TextLeftInset="4" TextBottomInset="4" TextRightInset="4"/></Table></Story>"#
        );
    }

    #[test]
    fn cells_spelling_every_inset_are_byte_identical() {
        let src = r#"<Story Self="u1"><Table Self="t"><Cell Self="c2" Name="1:0" TextTopInset="4" TextLeftInset="4" TextBottomInset="4" TextRightInset="4"><Content/></Cell></Table></Story>"#;
        assert_eq!(
            spell_cell_insets(src.as_bytes(), &HashMap::new()).unwrap(),
            src.as_bytes()
        );
    }
}
