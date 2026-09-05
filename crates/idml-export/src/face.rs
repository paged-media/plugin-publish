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

//! Explicit-face save-back: every paragraph style and every run that
//! names an `AppliedFont` without a `FontStyle` gets the style the
//! engine composed it with, spelled out.
//!
//! # What InDesign 20.0.1 actually does (measured 2026-09-05)
//!
//! A `<ParagraphStyle>` or `<CharacterStyleRange>` that carries
//! `<Properties><AppliedFont>` and no `FontStyle` attribute is NOT read
//! as "the family's regular face": InDesign picks an instance from the
//! run's point size — `Optical size-9.500`, `12pt`, `8pt`,
//! `9pt-30.000` — and for a variable font with an optical-size axis
//! (Source Serif 4, Fraunces) that instance comes back `SUBSTITUTED`
//! (PostScript name `Source Serif 4`,
//! `Fraunces_30.000opsz_400.000wght_1ital`): the body face of the whole
//! book falls to Myriad Pro. A static face under the same rule is
//! `NOT_AVAILABLE` (Noto Sans Hebrew, installed). Neither the
//! `TextPreference UseOpticalSize` preference (honoured on open, but
//! after the styles are built) nor a `FontStyle` on the root
//! `[No paragraph style]` changes this (variants V1, opsz-off). Spelling
//! `FontStyle` on the carriers themselves does: 20 font entries, 7
//! unresolved → 13 entries, every one `INSTALLED`, 143 → 140 overset
//! stories (variants V2–V4). Character styles need nothing — a run
//! inherits its face through them correctly (V4).
//!
//! The value written is the engine's own cascade — the run's character
//! style, then its paragraph style, then the style's `BasedOn` chain
//! (`paged_scene::Document::resolved_run_attrs`,
//! `StyleSheet::resolve_paragraph`) — and `Regular` when nothing in the
//! cascade names a style, which is the face the engine composes with.
//! A carrier that already spells its `FontStyle`, or names no
//! `AppliedFont`, passes through byte-identical.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::StyleSheet;

const PARAGRAPH_STYLE: &[u8] = b"ParagraphStyle";
const PARAGRAPH_RANGE: &[u8] = b"ParagraphStyleRange";
const CHARACTER_RANGE: &[u8] = b"CharacterStyleRange";
const APPLIED_FONT: &[u8] = b"AppliedFont";
const FONT_STYLE: &[u8] = b"FontStyle";

fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| std::str::from_utf8(&a.value).ok().map(|s| s.to_string()))
}

fn has_attr(e: &BytesStart, key: &[u8]) -> bool {
    e.attributes().flatten().any(|a| a.key.as_ref() == key)
}

/// A cascaded style name, `Regular` when the cascade names none.
fn style_or_regular(style: Option<&str>) -> String {
    style
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Regular")
        .to_string()
}

fn with_font_style(e: &BytesStart, style: &str) -> BytesStart<'static> {
    let mut owned = e.to_owned();
    owned.push_attribute((FONT_STYLE, style.as_bytes()));
    owned
}

/// `Resources/Styles.xml`: every `<ParagraphStyle>` that names an
/// `AppliedFont` without a `FontStyle` gains the style its `BasedOn`
/// cascade names (`Regular` when none). Byte-identical otherwise.
pub fn patch_styles_face(
    original: &[u8],
    styles: &StyleSheet,
) -> Result<Vec<u8>, quick_xml::Error> {
    // Pass 1 — which styles need it (keyed by `Self`).
    let mut needs: HashMap<String, String> = HashMap::new();
    {
        let mut reader = Reader::from_reader(original);
        let config = reader.config_mut();
        config.expand_empty_elements = false;
        config.trim_text(false);
        let mut buf = Vec::new();
        // The open `<ParagraphStyle>` whose children are streaming:
        // its id and whether it already spells a `FontStyle`.
        let mut open: Option<(String, bool)> = None;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(ref e) if e.name().as_ref() == PARAGRAPH_STYLE => {
                    open = attr_value(e, b"Self").map(|id| (id, has_attr(e, FONT_STYLE)));
                }
                Event::End(ref e) if e.name().as_ref() == PARAGRAPH_STYLE => open = None,
                Event::Start(ref e) | Event::Empty(ref e) if e.name().as_ref() == APPLIED_FONT => {
                    if let Some((id, false)) = &open {
                        let resolved = styles.resolve_paragraph(id);
                        needs.insert(id.clone(), style_or_regular(resolved.font_style.as_deref()));
                    }
                }
                _ => {}
            }
            buf.clear();
        }
    }
    if needs.is_empty() {
        return Ok(original.to_vec());
    }
    // Pass 2 — spell it on the start tag.
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
            Event::Start(ref e) if e.name().as_ref() == PARAGRAPH_STYLE => {
                match attr_value(e, b"Self").and_then(|id| needs.get(&id)) {
                    Some(style) => writer.write_event(Event::Start(with_font_style(e, style)))?,
                    None => writer.write_event(ev.borrow())?,
                }
            }
            other => writer.write_event(other.borrow())?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

/// A story part: every `<CharacterStyleRange>` that names an
/// `AppliedFont` without a `FontStyle` gains the style the engine's run
/// cascade names — its character style, then the enclosing paragraph's
/// style (each through `BasedOn`), `Regular` when none. Runs nested in
/// tables and footnotes resolve against THEIR enclosing paragraph.
/// Byte-identical otherwise.
pub fn patch_story_face(original: &[u8], styles: &StyleSheet) -> Result<Vec<u8>, quick_xml::Error> {
    let cascade = |character_style: Option<&str>, paragraph_style: Option<&str>| -> String {
        let from_character = character_style
            .and_then(|id| styles.resolve_character(id).font_style)
            .filter(|s| !s.trim().is_empty());
        let from_paragraph = || {
            paragraph_style
                .and_then(|id| styles.resolve_paragraph(id).font_style)
                .filter(|s| !s.trim().is_empty())
        };
        style_or_regular(from_character.or_else(from_paragraph).as_deref())
    };
    // Pass 1 — which runs need it (keyed by run ordinal in document
    // order, counting every `<CharacterStyleRange>` start, nested ones
    // included).
    let mut needs: HashMap<usize, String> = HashMap::new();
    {
        let mut reader = Reader::from_reader(original);
        let config = reader.config_mut();
        config.expand_empty_elements = false;
        config.trim_text(false);
        let mut buf = Vec::new();
        let mut ordinal = 0usize;
        let mut paragraphs: Vec<Option<String>> = Vec::new();
        // (ordinal, character style, already spells FontStyle)
        let mut runs: Vec<(usize, Option<String>, bool)> = Vec::new();
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(ref e) if e.name().as_ref() == PARAGRAPH_RANGE => {
                    paragraphs.push(attr_value(e, b"AppliedParagraphStyle"));
                }
                Event::End(ref e) if e.name().as_ref() == PARAGRAPH_RANGE => {
                    paragraphs.pop();
                }
                Event::Start(ref e) if e.name().as_ref() == CHARACTER_RANGE => {
                    ordinal += 1;
                    runs.push((
                        ordinal,
                        attr_value(e, b"AppliedCharacterStyle"),
                        has_attr(e, FONT_STYLE),
                    ));
                }
                Event::Empty(ref e) if e.name().as_ref() == CHARACTER_RANGE => ordinal += 1,
                Event::End(ref e) if e.name().as_ref() == CHARACTER_RANGE => {
                    runs.pop();
                }
                Event::Start(ref e) | Event::Empty(ref e) if e.name().as_ref() == APPLIED_FONT => {
                    if let Some((ord, character_style, false)) = runs.last() {
                        let paragraph_style = paragraphs.last().and_then(|p| p.as_deref());
                        needs.insert(*ord, cascade(character_style.as_deref(), paragraph_style));
                    }
                }
                _ => {}
            }
            buf.clear();
        }
    }
    if needs.is_empty() {
        return Ok(original.to_vec());
    }
    // Pass 2 — spell it on the start tag.
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut ordinal = 0usize;
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Eof => break,
            Event::Start(ref e) if e.name().as_ref() == CHARACTER_RANGE => {
                ordinal += 1;
                match needs.get(&ordinal) {
                    Some(style) => writer.write_event(Event::Start(with_font_style(e, style)))?,
                    None => writer.write_event(ev.borrow())?,
                }
            }
            Event::Empty(ref e) if e.name().as_ref() == CHARACTER_RANGE => {
                ordinal += 1;
                writer.write_event(ev.borrow())?;
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
    use idml_import::styles::{CharacterStyleDef, ParagraphStyleDef};

    fn sheet() -> StyleSheet {
        let mut s = StyleSheet::default();
        s.paragraph_styles.insert(
            "ParagraphStyle/Body".to_string(),
            ParagraphStyleDef {
                font: Some("Source Serif 4".to_string()),
                ..Default::default()
            },
        );
        s.paragraph_styles.insert(
            "ParagraphStyle/Deck".to_string(),
            ParagraphStyleDef {
                font_style: Some("Italic".to_string()),
                ..Default::default()
            },
        );
        s.paragraph_styles.insert(
            "ParagraphStyle/Deck Small".to_string(),
            ParagraphStyleDef {
                based_on: Some("ParagraphStyle/Deck".to_string()),
                font: Some("Fraunces".to_string()),
                ..Default::default()
            },
        );
        s.character_styles.insert(
            "CharacterStyle/Strong".to_string(),
            CharacterStyleDef {
                font_style: Some("Bold".to_string()),
                ..Default::default()
            },
        );
        s
    }

    #[test]
    fn a_paragraph_style_naming_a_font_gains_its_cascaded_style() {
        let src = r#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/Body" Name="Body" PointSize="9.5"><Properties><AppliedFont type="string">Source Serif 4</AppliedFont></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Deck Small" Name="Deck Small"><Properties><BasedOn type="object">ParagraphStyle/Deck</BasedOn><AppliedFont type="string">Fraunces</AppliedFont></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Deck" Name="Deck" FontStyle="Italic"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Plain" Name="Plain"><Properties><BasedOn type="object">ParagraphStyle/Body</BasedOn></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Empty" Name="Empty"/></RootParagraphStyleGroup></idPkg:Styles>"#;
        let out = String::from_utf8(patch_styles_face(src.as_bytes(), &sheet()).unwrap()).unwrap();
        assert!(out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Body" Name="Body" PointSize="9.5" FontStyle="Regular">"#), "{out}");
        assert!(out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Deck Small" Name="Deck Small" FontStyle="Italic">"#), "{out}");
        // Already spelled, no font, and empty: untouched.
        assert!(
            out.contains(
                r#"<ParagraphStyle Self="ParagraphStyle/Deck" Name="Deck" FontStyle="Italic">"#
            ),
            "{out}"
        );
        assert!(
            out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Plain" Name="Plain">"#),
            "{out}"
        );
        assert!(
            out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Empty" Name="Empty"/>"#),
            "{out}"
        );
    }

    #[test]
    fn a_run_naming_a_font_gains_the_style_its_cascade_names() {
        let src = r#"<Story Self="u1"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Deck"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content>a</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Strong"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content>b</Content></CharacterStyleRange><CharacterStyleRange FontStyle="Light"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content>c</Content></CharacterStyleRange><CharacterStyleRange><Content>d</Content></CharacterStyleRange><CharacterStyleRange/></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange><Properties><AppliedFont type="string">JetBrains Mono</AppliedFont></Properties><Content>e</Content><Table Self="t"><Cell Self="c"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Deck"><CharacterStyleRange><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content>f</Content></CharacterStyleRange></ParagraphStyleRange></Cell></Table></CharacterStyleRange></ParagraphStyleRange></Story>"#;
        let out = String::from_utf8(patch_story_face(src.as_bytes(), &sheet()).unwrap()).unwrap();
        // a: paragraph cascade (Deck → Italic); b: character style wins
        // (Strong → Bold); c: spelled already; d: no font; e: nothing in
        // the cascade → Regular; f: nested in a table cell → ITS
        // paragraph (Deck → Italic).
        assert!(out.contains(r#"<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" FontStyle="Italic"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content>a</Content>"#), "{out}");
        assert!(out.contains(r#"<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Strong" FontStyle="Bold"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content>b</Content>"#), "{out}");
        assert!(out.contains(r#"<CharacterStyleRange FontStyle="Light"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content>c</Content>"#), "{out}");
        assert!(out.contains(r#"<CharacterStyleRange><Content>d</Content></CharacterStyleRange><CharacterStyleRange/>"#), "{out}");
        assert!(out.contains(r#"<CharacterStyleRange FontStyle="Regular"><Properties><AppliedFont type="string">JetBrains Mono</AppliedFont></Properties><Content>e</Content>"#), "{out}");
        assert!(out.contains(r#"<CharacterStyleRange FontStyle="Italic"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content>f</Content>"#), "{out}");
    }

    #[test]
    fn a_story_with_nothing_to_spell_is_byte_identical() {
        let src = r#"<Story Self="u1"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange FontStyle="Regular"><Properties><AppliedFont type="string">Source Serif 4</AppliedFont></Properties><Content>x</Content></CharacterStyleRange><CharacterStyleRange PointSize="9"><Content>y</Content></CharacterStyleRange></ParagraphStyleRange></Story>"#;
        assert_eq!(
            patch_story_face(src.as_bytes(), &sheet()).unwrap(),
            src.as_bytes()
        );
        let styles = r#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/Deck" Name="Deck" FontStyle="Italic"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties></ParagraphStyle></RootParagraphStyleGroup></idPkg:Styles>"#;
        assert_eq!(
            patch_styles_face(styles.as_bytes(), &sheet()).unwrap(),
            styles.as_bytes()
        );
    }
}
