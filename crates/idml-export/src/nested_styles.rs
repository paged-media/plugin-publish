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

//! Nested, line and GREP styles in InDesign's spelling: typed lists in a
//! `<Properties>` block (see `idml_import::style_lists` for what InDesign
//! 20.0.1 writes).
//!
//! The engine's older packages spelled a nested style as a `<NestedStyle
//! AppliedCharacterStyle=… Delimiter=… Repetition=… Inclusive=…/>` child
//! of the `<ParagraphStyle>`. InDesign does not read that element: the
//! style opens with no nested styles at all. [`patch_legacy_nested_styles`]
//! rewrites it into the `<AllNestedStyles>` list, inside the style's
//! `<Properties>`; a part without the element passes through
//! byte-identical. A list the source already spells passes through as
//! well (no operation edits these lists yet), and a style the export
//! mints carries the model's lists ([`write_style_lists`]).

use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use idml_import::style_lists::delimiter_to_idml;
use idml_import::{GrepStyle, LineStyle, NestedStyle};

type W = Writer<Cursor<Vec<u8>>>;

fn typed(w: &mut W, name: &str, kind: &str, value: &str) -> Result<(), quick_xml::Error> {
    let mut start = BytesStart::new(name);
    start.push_attribute(("type", kind));
    w.write_event(Event::Start(start))?;
    if !value.is_empty() {
        w.write_event(Event::Text(BytesText::new(value)))?;
    }
    w.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

fn list<T>(
    w: &mut W,
    name: &str,
    items: &[T],
    mut item: impl FnMut(&mut W, &T) -> Result<bool, quick_xml::Error>,
) -> Result<(), quick_xml::Error> {
    let mut start = BytesStart::new(name);
    start.push_attribute(("type", "list"));
    w.write_event(Event::Start(start))?;
    for it in items {
        let mut record = BytesStart::new("ListItem");
        record.push_attribute(("type", "record"));
        // Write into a scratch writer first: an entry with no InDesign
        // spelling is left out whole.
        let mut scratch = Writer::new(Cursor::new(Vec::new()));
        if item(&mut scratch, it)? {
            w.write_event(Event::Start(record))?;
            // Through the cursor, so its position moves past the bytes.
            std::io::Write::write_all(w.get_mut(), &scratch.into_inner().into_inner())
                .map_err(|e| quick_xml::Error::Io(std::sync::Arc::new(e)))?;
            w.write_event(Event::End(BytesEnd::new("ListItem")))?;
        }
    }
    w.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

/// `<AllNestedStyles type="list">…` — nothing when `nested` is empty.
pub(crate) fn write_nested_styles(
    w: &mut W,
    nested: &[NestedStyle],
) -> Result<(), quick_xml::Error> {
    if nested.is_empty() {
        return Ok(());
    }
    list(w, "AllNestedStyles", nested, |w, ns| {
        let Some((enumeration, delimiter)) = delimiter_to_idml(&ns.delimiter) else {
            return Ok(false);
        };
        typed(
            w,
            "AppliedCharacterStyle",
            "object",
            &ns.applied_character_style,
        )?;
        typed(
            w,
            "Delimiter",
            if enumeration { "enumeration" } else { "string" },
            &delimiter,
        )?;
        typed(w, "Repetition", "long", &ns.repetition.to_string())?;
        typed(w, "Inclusive", "boolean", &ns.inclusive.to_string())?;
        Ok(true)
    })
}

/// The three lists, in InDesign's order (nested, GREP, line); each only
/// when it has entries.
pub(crate) fn write_style_lists(
    w: &mut W,
    nested: &[NestedStyle],
    line: &[LineStyle],
    grep: &[GrepStyle],
) -> Result<(), quick_xml::Error> {
    write_nested_styles(w, nested)?;
    if !grep.is_empty() {
        list(w, "AllGREPStyles", grep, |w, g| {
            typed(
                w,
                "AppliedCharacterStyle",
                "object",
                &g.applied_character_style,
            )?;
            typed(w, "GrepExpression", "string", &g.grep_expression)?;
            Ok(true)
        })?;
    }
    if !line.is_empty() {
        list(w, "AllLineStyles", line, |w, l| {
            typed(
                w,
                "AppliedCharacterStyle",
                "object",
                &l.applied_character_style,
            )?;
            typed(w, "LineCount", "long", &l.line_count.to_string())?;
            typed(
                w,
                "RepeatLast",
                "long",
                &l.repeat_last.unwrap_or(-1).to_string(),
            )?;
            Ok(true)
        })?;
    }
    Ok(())
}

fn attr(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| std::str::from_utf8(&a.value).ok().map(str::to_string))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Rewrite every `<NestedStyle …/>` child of a `<ParagraphStyle>` into
/// InDesign's `<Properties><AllNestedStyles>` list (see the module doc).
/// A style that already spells the list keeps it and loses only the
/// legacy elements. Byte-identical when the part has no `<NestedStyle`.
pub fn patch_legacy_nested_styles(original: &[u8]) -> Result<Vec<u8>, quick_xml::Error> {
    if !contains(original, b"<NestedStyle") {
        return Ok(original.to_vec());
    }
    // First pass: each style's legacy entries, and which styles already
    // spell the list.
    let mut legacy: BTreeMap<String, Vec<NestedStyle>> = BTreeMap::new();
    let mut spelled: std::collections::BTreeSet<String> = Default::default();
    {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut open: Option<String> = None;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(e) if e.name().as_ref() == b"ParagraphStyle" => {
                    open = attr(&e, b"Self");
                }
                Event::End(e) if e.name().as_ref() == b"ParagraphStyle" => open = None,
                Event::Start(e) | Event::Empty(e) => {
                    if let Some(id) = &open {
                        match e.name().as_ref() {
                            b"NestedStyle" => {
                                let entry = legacy.entry(id.clone()).or_default();
                                if let Some(ns) = idml_import::styles::parse_legacy_nested_style(&e)
                                {
                                    entry.push(ns);
                                }
                            }
                            b"AllNestedStyles" => {
                                spelled.insert(id.clone());
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            buf.clear();
        }
    }

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(original.len() + 512)));
    let mut buf = Vec::new();
    // The open style: the list it still owes, and the depth below its
    // start tag.
    let mut owed: Option<Vec<NestedStyle>> = None;
    let mut in_style = false;
    let mut depth = 0usize;
    // Inside a legacy `<NestedStyle>…</NestedStyle>` element (dropped).
    let mut skipping = 0usize;
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        if skipping > 0 {
            match &ev {
                Event::Start(_) => skipping += 1,
                Event::End(_) => skipping -= 1,
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
            continue;
        }
        match ev {
            Event::Eof => break,
            Event::Start(ref e) if !in_style && e.name().as_ref() == b"ParagraphStyle" => {
                in_style = true;
                depth = 0;
                owed = attr(e, b"Self").and_then(|id| {
                    let list = legacy.get(&id)?;
                    (!spelled.contains(&id) && !list.is_empty()).then(|| list.clone())
                });
                writer.write_event(ev.borrow())?;
            }
            Event::Start(ref e) if in_style => {
                let name = e.name().as_ref().to_vec();
                if depth == 0 && name == b"NestedStyle" {
                    skipping = 1;
                } else {
                    if depth == 0 && name != b"Properties" {
                        // A child before any `<Properties>`: the block
                        // goes first.
                        write_owed_block(&mut writer, &mut owed)?;
                    }
                    depth += 1;
                    writer.write_event(ev.borrow())?;
                }
            }
            Event::Empty(ref e) if in_style && depth == 0 => {
                match e.name().as_ref() {
                    b"NestedStyle" => {}
                    b"Properties" => match owed.take() {
                        // `<Properties/>`: opened around the list.
                        Some(list) => {
                            writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                            write_nested_styles(&mut writer, &list)?;
                            writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                        }
                        None => writer.write_event(ev.borrow())?,
                    },
                    _ => {
                        write_owed_block(&mut writer, &mut owed)?;
                        writer.write_event(ev.borrow())?;
                    }
                }
            }
            Event::End(ref e) if in_style => {
                if depth == 0 {
                    // The style closes; a list still owed gets a block.
                    write_owed_block(&mut writer, &mut owed)?;
                    in_style = false;
                } else {
                    depth -= 1;
                    if depth == 0 && e.name().as_ref() == b"Properties" {
                        if let Some(list) = owed.take() {
                            write_nested_styles(&mut writer, &list)?;
                        }
                    }
                }
                writer.write_event(ev.borrow())?;
            }
            other => writer.write_event(other.borrow())?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

fn write_owed_block(
    w: &mut W,
    owed: &mut Option<Vec<NestedStyle>>,
) -> Result<(), quick_xml::Error> {
    if let Some(list) = owed.take() {
        w.write_event(Event::Start(BytesStart::new("Properties")))?;
        write_nested_styles(w, &list)?;
        w.write_event(Event::End(BytesEnd::new("Properties")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"<AllNestedStyles type="list"><ListItem type="record"><AppliedCharacterStyle type="object">CharacterStyle/Num</AppliedCharacterStyle><Delimiter type="string">.</Delimiter><Repetition type="long">1</Repetition><Inclusive type="boolean">true</Inclusive></ListItem></AllNestedStyles>"#;

    #[test]
    fn a_legacy_nested_style_becomes_indesigns_list() {
        let src = r#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/Entry" Name="Entry"><Properties><BasedOn type="object">ParagraphStyle/Body</BasedOn></Properties><NestedStyle AppliedCharacterStyle="CharacterStyle/Num" Delimiter="." Repetition="1" Inclusive="true"/></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Bare" Name="Bare"><NestedStyle AppliedCharacterStyle="CharacterStyle/Num" Delimiter="." Repetition="1" Inclusive="true"/></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Plain" Name="Plain"/></RootParagraphStyleGroup></idPkg:Styles>"#;
        let out = String::from_utf8(patch_legacy_nested_styles(src.as_bytes()).unwrap()).unwrap();
        assert!(!out.contains("<NestedStyle"), "{out}");
        assert!(
            out.contains(&format!(
                r#"<ParagraphStyle Self="ParagraphStyle/Entry" Name="Entry"><Properties><BasedOn type="object">ParagraphStyle/Body</BasedOn>{LIST}</Properties></ParagraphStyle>"#
            )),
            "{out}"
        );
        // A style with no `<Properties>` gains one.
        assert!(
            out.contains(&format!(
                r#"<ParagraphStyle Self="ParagraphStyle/Bare" Name="Bare"><Properties>{LIST}</Properties></ParagraphStyle>"#
            )),
            "{out}"
        );
        assert!(out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Plain" Name="Plain"/>"#));
        // And the importer reads the list back.
        let sheet = idml_import::parse_stylesheet(out.as_bytes()).unwrap();
        let entry = &sheet.paragraph_styles["ParagraphStyle/Entry"].nested_styles;
        assert_eq!(entry.len(), 1);
        assert_eq!(entry[0].delimiter, idml_import::NestedDelimiter::Char('.'));
    }

    #[test]
    fn a_style_that_spells_the_list_keeps_it_and_loses_the_legacy_element() {
        let src = format!(
            r#"<idPkg:Styles xmlns:idPkg="x"><ParagraphStyle Self="ParagraphStyle/Both" Name="Both"><Properties>{LIST}</Properties><NestedStyle AppliedCharacterStyle="CharacterStyle/Other" Delimiter="Words" Repetition="2" Inclusive="true"/></ParagraphStyle></idPkg:Styles>"#
        );
        let out = String::from_utf8(patch_legacy_nested_styles(src.as_bytes()).unwrap()).unwrap();
        assert_eq!(
            out,
            format!(
                r#"<idPkg:Styles xmlns:idPkg="x"><ParagraphStyle Self="ParagraphStyle/Both" Name="Both"><Properties>{LIST}</Properties></ParagraphStyle></idPkg:Styles>"#
            )
        );
    }

    #[test]
    fn a_part_without_legacy_nested_styles_is_byte_identical() {
        let src = format!(
            r#"<idPkg:Styles xmlns:idPkg="x"><ParagraphStyle Self="ParagraphStyle/A" Name="A" EmptyNestedStyles="false"><Properties>{LIST}</Properties></ParagraphStyle></idPkg:Styles>"#
        );
        assert_eq!(
            patch_legacy_nested_styles(src.as_bytes()).unwrap(),
            src.as_bytes()
        );
    }

    #[test]
    fn the_lists_are_written_in_indesigns_order_and_spelling() {
        let mut w = Writer::new(Cursor::new(Vec::new()));
        write_style_lists(
            &mut w,
            &[NestedStyle {
                applied_character_style: "CharacterStyle/R".into(),
                delimiter: idml_import::NestedDelimiter::Tab,
                repetition: 1,
                inclusive: false,
            }],
            &[LineStyle {
                applied_character_style: "CharacterStyle/U".into(),
                line_count: 2,
                repeat_last: None,
            }],
            &[GrepStyle {
                applied_character_style: "CharacterStyle/G".into(),
                grep_expression: r"\d+".into(),
            }],
        )
        .unwrap();
        let out = String::from_utf8(w.into_inner().into_inner()).unwrap();
        assert_eq!(
            out,
            concat!(
                r#"<AllNestedStyles type="list"><ListItem type="record"><AppliedCharacterStyle type="object">CharacterStyle/R</AppliedCharacterStyle><Delimiter type="enumeration">Tabs</Delimiter><Repetition type="long">1</Repetition><Inclusive type="boolean">false</Inclusive></ListItem></AllNestedStyles>"#,
                r#"<AllGREPStyles type="list"><ListItem type="record"><AppliedCharacterStyle type="object">CharacterStyle/G</AppliedCharacterStyle><GrepExpression type="string">\d+</GrepExpression></ListItem></AllGREPStyles>"#,
                r#"<AllLineStyles type="list"><ListItem type="record"><AppliedCharacterStyle type="object">CharacterStyle/U</AppliedCharacterStyle><LineCount type="long">2</LineCount><RepeatLast type="long">-1</RepeatLast></ListItem></AllLineStyles>"#,
            )
        );
    }
}
