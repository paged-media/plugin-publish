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

//! Nested, line and GREP styles as InDesign writes them: typed lists
//! inside a `<Properties>` block, on a `<ParagraphStyle>`, a
//! `<ParagraphStyleRange>` and the document's `<TextDefault>`.
//!
//! Asked of InDesign 20.0.1 (2026-10-12, a document built in its DOM and
//! exported as IDML):
//!
//! ```xml
//! <AllNestedStyles type="list">
//!   <ListItem type="record">
//!     <AppliedCharacterStyle type="object">CharacterStyle/Red</AppliedCharacterStyle>
//!     <Delimiter type="enumeration">Tabs</Delimiter>   <!-- or type="string">:</… -->
//!     <Repetition type="long">1</Repetition>
//!     <Inclusive type="boolean">true</Inclusive>
//!   </ListItem>
//! </AllNestedStyles>
//! <AllGREPStyles type="list">
//!   <ListItem type="record">
//!     <AppliedCharacterStyle type="object">CharacterStyle/Green</AppliedCharacterStyle>
//!     <GrepExpression type="string">\d+</GrepExpression>
//!   </ListItem>
//! </AllGREPStyles>
//! <AllLineStyles type="list">
//!   <ListItem type="record">
//!     <AppliedCharacterStyle type="object">CharacterStyle/Under</AppliedCharacterStyle>
//!     <LineCount type="long">2</LineCount>
//!     <RepeatLast type="long">-1</RepeatLast>
//!   </ListItem>
//! </AllLineStyles>
//! ```
//!
//! The delimiter enumerations InDesign writes are `AnyCharacter`,
//! `AnyWord`, `AutoPageNumber`, `Digits`, `Dropcap`, `EmSpace`,
//! `EndNestedStyle`, `EnSpace`, `ForcedLineBreak`, `IndentHereTab`,
//! `InlineGraphic`, `Letters`, `NonbreakingSpace`, `Repeat`,
//! `SectionMarker`, `Sentence` and `Tabs`; a `type="string"` delimiter is
//! the literal characters, any one of which ends a repetition. A drop
//! cap's character style is the FIRST nested style, with the `Dropcap`
//! delimiter.

use std::io::BufRead;

use quick_xml::events::Event;
use quick_xml::Reader;

use paged_model::{GrepStyle, LineStyle, NestedDelimiter, NestedStyle};

use crate::ParseError;

/// One list read off the XML.
#[derive(Debug, Clone, PartialEq)]
pub enum StyleList {
    Nested(Vec<NestedStyle>),
    Line(Vec<LineStyle>),
    Grep(Vec<GrepStyle>),
}

/// The fields of one `<ListItem type="record">`, by child element.
#[derive(Default)]
struct Record {
    applied: Option<String>,
    delimiter: Option<(bool, String)>,
    repetition: Option<String>,
    inclusive: Option<String>,
    line_count: Option<String>,
    repeat_last: Option<String>,
    grep: Option<String>,
}

impl Record {
    fn set(&mut self, field: &[u8], enumeration: bool, value: String) {
        match field {
            b"AppliedCharacterStyle" => self.applied = Some(value),
            b"Delimiter" => self.delimiter = Some((enumeration, value)),
            b"Repetition" => self.repetition = Some(value),
            b"Inclusive" => self.inclusive = Some(value),
            b"LineCount" => self.line_count = Some(value),
            b"RepeatLast" => self.repeat_last = Some(value),
            b"GrepExpression" => self.grep = Some(value),
            _ => {}
        }
    }
}

/// Read the list element whose start tag `reader` just returned, through
/// its end tag. `name` is that element's name (`AllNestedStyles`,
/// `AllLineStyles` or `AllGREPStyles`).
/// Text is read untrimmed, whatever the reader's setting: a delimiter can
/// be a space.
pub(crate) fn read_style_list<R: BufRead>(
    reader: &mut Reader<R>,
    name: &[u8],
) -> Result<StyleList, ParseError> {
    let saved = (
        reader.config().trim_text_start,
        reader.config().trim_text_end,
    );
    reader.config_mut().trim_text(false);
    let result = read_records(reader, name);
    reader.config_mut().trim_text_start = saved.0;
    reader.config_mut().trim_text_end = saved.1;
    let records = result?;
    Ok(match name {
        b"AllLineStyles" => StyleList::Line(records.into_iter().filter_map(line_style).collect()),
        b"AllGREPStyles" => StyleList::Grep(records.into_iter().filter_map(grep_style).collect()),
        _ => StyleList::Nested(records.into_iter().filter_map(nested_style).collect()),
    })
}

fn read_records<R: BufRead>(
    reader: &mut Reader<R>,
    name: &[u8],
) -> Result<Vec<Record>, ParseError> {
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut record: Option<Record> = None;
    // The record field open (`AppliedCharacterStyle`, …), whether its
    // `type` is `enumeration`, and its text so far.
    let mut field: Option<(Vec<u8>, bool, String)> = None;
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                depth += 1;
                if e.name().as_ref() == b"ListItem" {
                    record = Some(Record::default());
                } else if record.is_some() {
                    let enumeration =
                        crate::util::attr(&e, b"type").as_deref() == Some("enumeration");
                    field = Some((e.name().as_ref().to_vec(), enumeration, String::new()));
                }
            }
            Event::Empty(e) => {
                // `<Delimiter type="string"/>`: an empty value.
                if let Some(r) = record.as_mut() {
                    let enumeration =
                        crate::util::attr(&e, b"type").as_deref() == Some("enumeration");
                    r.set(e.name().as_ref(), enumeration, String::new());
                }
            }
            Event::Text(t) => {
                if let Some((_, _, text)) = field.as_mut() {
                    text.push_str(
                        &t.xml_content(quick_xml::XmlVersion::Implicit1_0)
                            .unwrap_or_default(),
                    );
                }
            }
            Event::CData(t) => {
                if let Some((_, _, text)) = field.as_mut() {
                    text.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::GeneralRef(r) => {
                if let Some((_, _, text)) = field.as_mut() {
                    let entity = String::from_utf8_lossy(r.as_ref());
                    if let Ok(v) = quick_xml::escape::unescape(&format!("&{entity};")) {
                        text.push_str(&v);
                    }
                }
            }
            Event::End(e) => {
                if depth == 0 {
                    // The list's own end tag.
                    debug_assert_eq!(e.name().as_ref(), name);
                    break;
                }
                depth -= 1;
                let end = e.name().as_ref().to_vec();
                if end == b"ListItem" {
                    if let Some(r) = record.take() {
                        out.push(r);
                    }
                } else if let Some((f, enumeration, text)) = field.take() {
                    if f == end {
                        if let Some(r) = record.as_mut() {
                            r.set(&f, enumeration, text);
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// A record's character style, `None` when it names none.
fn applied(r: &Record) -> Option<String> {
    r.applied
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn nested_style(r: Record) -> Option<NestedStyle> {
    let applied_character_style = applied(&r)?;
    let delimiter = match &r.delimiter {
        Some((true, v)) => delimiter_from_enumeration(v.trim()),
        Some((false, v)) => delimiter_from_string(v),
        None => NestedDelimiter::Unknown,
    };
    Some(NestedStyle {
        applied_character_style,
        delimiter,
        repetition: r
            .repetition
            .as_deref()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1),
        inclusive: r
            .inclusive
            .as_deref()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(true),
    })
}

fn line_style(r: Record) -> Option<LineStyle> {
    Some(LineStyle {
        applied_character_style: applied(&r)?,
        line_count: r
            .line_count
            .as_deref()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1),
        repeat_last: r.repeat_last.as_deref().and_then(|s| s.trim().parse().ok()),
    })
}

fn grep_style(r: Record) -> Option<GrepStyle> {
    Some(GrepStyle {
        applied_character_style: applied(&r)?,
        grep_expression: r.grep.clone().unwrap_or_default(),
    })
}

/// A `<Delimiter type="enumeration">` value.
pub fn delimiter_from_enumeration(v: &str) -> NestedDelimiter {
    match v {
        "AnyCharacter" => NestedDelimiter::Characters,
        "AnyWord" => NestedDelimiter::Words,
        "Sentence" => NestedDelimiter::Sentences,
        "Letters" => NestedDelimiter::AnyLetter,
        "Digits" => NestedDelimiter::AnyDigit,
        "Tabs" => NestedDelimiter::Tab,
        "ForcedLineBreak" => NestedDelimiter::ForcedLineBreak,
        "EndNestedStyle" => NestedDelimiter::EndNestedStyle,
        "Dropcap" => NestedDelimiter::Dropcap,
        "EmSpace" => NestedDelimiter::EmSpace,
        "EnSpace" => NestedDelimiter::EnSpace,
        "NonbreakingSpace" => NestedDelimiter::NonbreakingSpace,
        "IndentHereTab" => NestedDelimiter::IndentHereTab,
        "AutoPageNumber" => NestedDelimiter::AutoPageNumber,
        "SectionMarker" => NestedDelimiter::SectionMarker,
        "InlineGraphic" => NestedDelimiter::InlineGraphic,
        "Repeat" => NestedDelimiter::Repeat,
        _ => NestedDelimiter::Unknown,
    }
}

/// A `<Delimiter type="string">` value: the literal characters.
pub fn delimiter_from_string(v: &str) -> NestedDelimiter {
    let mut chars = v.chars();
    match (chars.next(), chars.next()) {
        (None, _) => NestedDelimiter::Unknown,
        (Some(c), None) => NestedDelimiter::Char(c),
        _ => NestedDelimiter::AnyOf(v.to_string()),
    }
}

/// How InDesign spells `d`: `(true, enumeration)` or `(false, string)`.
/// `None` for the ones it has no spelling of (`Unknown`, and the
/// quote classes only the engine's older `<NestedStyle>` named).
pub fn delimiter_to_idml(d: &NestedDelimiter) -> Option<(bool, String)> {
    let e = |s: &str| Some((true, s.to_string()));
    match d {
        NestedDelimiter::Characters => e("AnyCharacter"),
        NestedDelimiter::Words => e("AnyWord"),
        NestedDelimiter::Sentences => e("Sentence"),
        NestedDelimiter::AnyLetter => e("Letters"),
        NestedDelimiter::AnyDigit => e("Digits"),
        NestedDelimiter::Tab => e("Tabs"),
        NestedDelimiter::ForcedLineBreak => e("ForcedLineBreak"),
        NestedDelimiter::EndNestedStyle => e("EndNestedStyle"),
        NestedDelimiter::Dropcap => e("Dropcap"),
        NestedDelimiter::EmSpace => e("EmSpace"),
        NestedDelimiter::EnSpace => e("EnSpace"),
        NestedDelimiter::NonbreakingSpace => e("NonbreakingSpace"),
        NestedDelimiter::IndentHereTab => e("IndentHereTab"),
        NestedDelimiter::AutoPageNumber => e("AutoPageNumber"),
        NestedDelimiter::SectionMarker => e("SectionMarker"),
        NestedDelimiter::InlineGraphic => e("InlineGraphic"),
        NestedDelimiter::Repeat => e("Repeat"),
        NestedDelimiter::Char(c) => Some((false, c.to_string())),
        NestedDelimiter::AnyOf(s) => Some((false, s.clone())),
        NestedDelimiter::AnyDoubleQuotes => Some((false, "\"\u{201C}\u{201D}".to_string())),
        NestedDelimiter::AnySingleQuotes => Some((false, "'\u{2018}\u{2019}".to_string())),
        NestedDelimiter::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(xml: &str, name: &[u8]) -> StyleList {
        let mut reader = Reader::from_reader(xml.as_bytes());
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf).unwrap() {
                Event::Start(e) if e.name().as_ref() == name => break,
                Event::Eof => panic!("no {name:?}"),
                _ => {}
            }
            buf.clear();
        }
        let list = read_style_list(&mut reader, name).unwrap();
        // The reader's own setting is back.
        assert!(reader.config().trim_text_start);
        list
    }

    /// The probe document's `Nested` style, as InDesign 20.0.1 exported it.
    #[test]
    fn reads_nested_styles_as_indesign_writes_them() {
        let xml = r#"<AllNestedStyles type="list">
            <ListItem type="record">
                <AppliedCharacterStyle type="object">CharacterStyle/DropMagenta</AppliedCharacterStyle>
                <Delimiter type="enumeration">Dropcap</Delimiter>
                <Repetition type="long">1</Repetition>
                <Inclusive type="boolean">true</Inclusive>
            </ListItem>
            <ListItem type="record">
                <AppliedCharacterStyle type="object">CharacterStyle/NestBlue</AppliedCharacterStyle>
                <Delimiter type="enumeration">AnyWord</Delimiter>
                <Repetition type="long">2</Repetition>
                <Inclusive type="boolean">false</Inclusive>
            </ListItem>
            <ListItem type="record">
                <AppliedCharacterStyle type="object">CharacterStyle/NestRed</AppliedCharacterStyle>
                <Delimiter type="string">:</Delimiter>
                <Repetition type="long">1</Repetition>
                <Inclusive type="boolean">true</Inclusive>
            </ListItem>
            <ListItem type="record">
                <AppliedCharacterStyle type="object">CharacterStyle/NestRed</AppliedCharacterStyle>
                <Delimiter type="string"> </Delimiter>
                <Repetition type="long">3</Repetition>
                <Inclusive type="boolean">true</Inclusive>
            </ListItem>
        </AllNestedStyles>"#;
        let StyleList::Nested(list) = read(xml, b"AllNestedStyles") else {
            panic!("not nested")
        };
        assert_eq!(list.len(), 4);
        assert_eq!(
            list[0].applied_character_style,
            "CharacterStyle/DropMagenta"
        );
        assert_eq!(list[0].delimiter, NestedDelimiter::Dropcap);
        assert_eq!(list[1].delimiter, NestedDelimiter::Words);
        assert_eq!(list[1].repetition, 2);
        assert!(!list[1].inclusive);
        assert_eq!(list[2].delimiter, NestedDelimiter::Char(':'));
        // A space delimiter survives a trimming reader.
        assert_eq!(list[3].delimiter, NestedDelimiter::Char(' '));
        assert_eq!(list[3].repetition, 3);
    }

    #[test]
    fn reads_line_and_grep_styles() {
        let xml = r#"<P><AllLineStyles type="list"><ListItem type="record"><AppliedCharacterStyle type="object">CharacterStyle/U</AppliedCharacterStyle><LineCount type="long">2</LineCount><RepeatLast type="long">-1</RepeatLast></ListItem></AllLineStyles><AllGREPStyles type="list"><ListItem type="record"><AppliedCharacterStyle type="object">CharacterStyle/G</AppliedCharacterStyle><GrepExpression type="string">\d+&lt;x&gt;</GrepExpression></ListItem></AllGREPStyles></P>"#;
        assert_eq!(
            read(xml, b"AllLineStyles"),
            StyleList::Line(vec![LineStyle {
                applied_character_style: "CharacterStyle/U".into(),
                line_count: 2,
                repeat_last: Some(-1),
            }])
        );
        assert_eq!(
            read(xml, b"AllGREPStyles"),
            StyleList::Grep(vec![GrepStyle {
                applied_character_style: "CharacterStyle/G".into(),
                grep_expression: r"\d+<x>".into(),
            }])
        );
    }

    /// Every enumeration InDesign 20.0.1 wrote for the 17 delimiters its
    /// DOM offers reads to a delimiter, and spells back the same.
    #[test]
    fn every_indesign_delimiter_round_trips() {
        for v in [
            "AnyCharacter",
            "AnyWord",
            "AutoPageNumber",
            "Digits",
            "Dropcap",
            "EmSpace",
            "EndNestedStyle",
            "EnSpace",
            "ForcedLineBreak",
            "IndentHereTab",
            "InlineGraphic",
            "Letters",
            "NonbreakingSpace",
            "Repeat",
            "SectionMarker",
            "Sentence",
            "Tabs",
        ] {
            let d = delimiter_from_enumeration(v);
            assert_ne!(d, NestedDelimiter::Unknown, "{v}");
            assert_eq!(delimiter_to_idml(&d), Some((true, v.to_string())), "{v}");
        }
        assert_eq!(
            delimiter_from_string(".,"),
            NestedDelimiter::AnyOf(".,".into())
        );
        assert_eq!(
            delimiter_to_idml(&NestedDelimiter::AnyOf(".,".into())),
            Some((false, ".,".to_string()))
        );
    }
}
