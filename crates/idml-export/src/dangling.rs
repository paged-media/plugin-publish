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

//! Dangling style references: a paragraph or run naming a style the
//! document does not define composes, in the engine, as if it named
//! none (the cascade resolves nothing — `StyleSheet::resolve_paragraph`
//! on an unknown id is empty). InDesign instead binds such text to its
//! own `[Basic Paragraph]` — the application default, Minion Pro 12 pt
//! (measured 2026-09-05: 54 references to `docx-*` styles a plugin
//! never defined, 135 runs of Minion Pro). Dropping the reference makes
//! InDesign do what the engine does: the text falls to the document's
//! `<TextDefault>` (see [`crate::preferences`]).
//!
//! InDesign's reserved `$ID/…` styles (`[No paragraph style]`,
//! `[No character style]`, `NormalParagraphStyle`) are defined by
//! InDesign itself and are kept whether or not the model lists them.
//! A part whose every reference resolves passes through byte-identical;
//! the re-parse of a dropped reference differs from the model, which is
//! how the loss ledger names it.

use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::StyleSheet;

const PARAGRAPH_RANGE: &[u8] = b"ParagraphStyleRange";
const CHARACTER_RANGE: &[u8] = b"CharacterStyleRange";
const APPLIED_PARAGRAPH: &[u8] = b"AppliedParagraphStyle";
const APPLIED_CHARACTER: &[u8] = b"AppliedCharacterStyle";

fn is_reserved(id: &str) -> bool {
    id.contains("/$ID/")
}

/// Whether the element's style reference (if any) names a style the
/// document defines; `None` when it names none at all.
fn dangling_ref(e: &BytesStart, key: &[u8], styles: &StyleSheet) -> bool {
    let Some(value) = e
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| std::str::from_utf8(&a.value).ok().map(|s| s.to_string()))
    else {
        return false;
    };
    if is_reserved(&value) {
        return false;
    }
    let defined = if key == APPLIED_PARAGRAPH {
        styles.paragraph_styles.contains_key(&value)
    } else {
        styles.character_styles.contains_key(&value)
    };
    !defined
}

fn without(e: &BytesStart, key: &[u8]) -> BytesStart<'static> {
    let mut out = BytesStart::new(String::from_utf8_lossy(e.name().as_ref()).into_owned());
    for a in e.attributes().flatten() {
        if a.key.as_ref() != key {
            out.push_attribute((a.key.as_ref(), a.value.as_ref()));
        }
    }
    out
}

/// Drop every `AppliedParagraphStyle` / `AppliedCharacterStyle` that
/// names a style `styles` does not define (reserved `$ID/` ids kept).
/// Byte-identical when every reference resolves.
pub fn drop_dangling_style_refs(
    original: &[u8],
    styles: &StyleSheet,
) -> Result<Vec<u8>, quick_xml::Error> {
    // A document with no stylesheet at all (a bare fixture, no
    // `Resources/Styles.xml`) defines nothing, so every reference would
    // be dangling — but there is nothing to reconcile the references
    // AGAINST; such a package passes through as it is.
    if styles.paragraph_styles.is_empty() && styles.character_styles.is_empty() {
        return Ok(original.to_vec());
    }
    let key_for = |name: &[u8]| -> Option<&'static [u8]> {
        match name {
            PARAGRAPH_RANGE => Some(APPLIED_PARAGRAPH),
            CHARACTER_RANGE => Some(APPLIED_CHARACTER),
            _ => None,
        }
    };
    let any = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut found = false;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(e) | Event::Empty(e) => {
                    if let Some(key) = key_for(e.name().as_ref()) {
                        if dangling_ref(&e, key, styles) {
                            found = true;
                            break;
                        }
                    }
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
            Event::Start(ref e) => match key_for(e.name().as_ref()) {
                Some(key) if dangling_ref(e, key, styles) => {
                    writer.write_event(Event::Start(without(e, key)))?
                }
                _ => writer.write_event(ev.borrow())?,
            },
            Event::Empty(ref e) => match key_for(e.name().as_ref()) {
                Some(key) if dangling_ref(e, key, styles) => {
                    writer.write_event(Event::Empty(without(e, key)))?
                }
                _ => writer.write_event(ev.borrow())?,
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
    use idml_import::styles::{CharacterStyleDef, ParagraphStyleDef};

    fn sheet() -> StyleSheet {
        let mut s = StyleSheet::default();
        s.paragraph_styles.insert(
            "ParagraphStyle/Body".to_string(),
            ParagraphStyleDef::default(),
        );
        s.character_styles.insert(
            "CharacterStyle/Strong".to_string(),
            CharacterStyleDef::default(),
        );
        s
    }

    #[test]
    fn a_reference_to_an_undefined_style_is_dropped_and_others_kept() {
        let src = r#"<Story Self="u1"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/docx-Default" Justification="LeftAlign"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/docx-auto-c1" PointSize="9"><Content>a</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Strong"><Content>b</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Content>c</Content></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/gone"/></ParagraphStyleRange></Story>"#;
        let out =
            String::from_utf8(drop_dangling_style_refs(src.as_bytes(), &sheet()).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<Story Self="u1"><ParagraphStyleRange Justification="LeftAlign"><CharacterStyleRange PointSize="9"><Content>a</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Strong"><Content>b</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Content>c</Content></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange/></ParagraphStyleRange></Story>"#
        );
    }

    #[test]
    fn a_document_with_no_stylesheet_is_left_alone() {
        let src = r#"<Story Self="u1"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Strong"><Content>a</Content></CharacterStyleRange></ParagraphStyleRange></Story>"#;
        assert_eq!(
            drop_dangling_style_refs(src.as_bytes(), &StyleSheet::default()).unwrap(),
            src.as_bytes()
        );
    }

    #[test]
    fn a_story_whose_references_all_resolve_is_byte_identical() {
        let src = r#"<Story Self="u1"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Strong"><Content>a</Content></CharacterStyleRange><CharacterStyleRange><Content>b</Content></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle"><CharacterStyleRange><Content>c</Content></CharacterStyleRange></ParagraphStyleRange></Story>"#;
        assert_eq!(
            drop_dangling_style_refs(src.as_bytes(), &sheet()).unwrap(),
            src.as_bytes()
        );
    }
}
