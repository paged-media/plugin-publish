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

//! `Resources/Preferences.xml` save-back: the document-level text
//! preference that states how the engine composed the document's
//! variable faces.
//!
//! # What InDesign 20.0.1 actually does (measured 2026-09-05)
//!
//! InDesign 2025 ships `TextPreference UseOpticalSize="true"`: with it,
//! every run set in a variable font that has an optical-size (`opsz`)
//! axis is re-instanced at its point size in a NATIVE document
//! (`Optical size-9.500`, `12pt`; `designAxes` `400,9.5`), and for
//! Source Serif 4 and Fraunces that instance comes back `SUBSTITUTED`.
//! With `UseOpticalSize="false"` the same runs resolve to the font's
//! default instance (`400,20`; `9,400,0,1`), every face `INSTALLED`.
//! A package whose part spells the attribute opens with it (measured:
//! `doc.textPreferences.useOpticalSize == false` on open).
//!
//! The engine composes every variable face at its default instance (it
//! applies no axis from the point size), so the default instance IS
//! what the document was composed with, and the export states it: text
//! InDesign sets or re-flows later is instanced the way the engine
//! composed it, not by size. It is NOT what resolves the faces of an
//! engine-authored file — the styles are built before this part is
//! read, and a carrier naming an `AppliedFont` without a `FontStyle` is
//! instanced by size regardless (measured: variant `opsz-off`, 7
//! unresolved faces either way); that is [`crate::face`]'s rule.
//!
//! A part that already spells the preference — either way — passes
//! through byte-identical (an InDesign-authored document keeps its
//! author's choice); a `<TextPreference>` without the attribute gains
//! it; a part without the element gains
//! `<TextPreference UseOpticalSize="false"/>`.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};

use crate::fonts::FontFace;
use quick_xml::{Reader, Writer};

use crate::rewrite::emit_empty_with_attrs;

const TEXT_PREFERENCE: &[u8] = b"TextPreference";
const USE_OPTICAL_SIZE: &[u8] = b"UseOpticalSize";
const TEXT_DEFAULT: &[u8] = b"TextDefault";
const ROOT: &[u8] = b"idPkg:Preferences";

/// Whether `<TextPreference>` carries the `UseOpticalSize` attribute.
fn has_use_optical_size(e: &BytesStart) -> bool {
    e.attributes()
        .flatten()
        .any(|a| a.key.as_ref() == USE_OPTICAL_SIZE)
}

/// `<TextDefault FontStyle="…" PointSize="12"><Properties><AppliedFont
/// type="string">FAMILY</AppliedFont><Leading type="enumeration">Auto
/// </Leading></Properties></TextDefault>` — InDesign's spelling of the
/// document's text defaults (the face as a typed child, like a style's),
/// carrying the engine's: its fallback face at 12 pt with auto leading
/// (`paged-renderer` sets an unsized run at 12 pt and leads it at
/// 120 %). Text no style reaches — a run with no paragraph style, or one
/// naming a style the document does not define — composes in this in
/// InDesign as it does in the engine (measured 2026-09-05: 4,449
/// Minion Pro characters → 6).
fn write_text_default(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    face: &FontFace,
) -> Result<(), quick_xml::Error> {
    let mut start = BytesStart::new("TextDefault");
    start.push_attribute(("FontStyle", face.style.as_str()));
    start.push_attribute(("PointSize", "12"));
    writer.write_event(Event::Start(start))?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    let mut af = BytesStart::new("AppliedFont");
    af.push_attribute(("type", "string"));
    writer.write_event(Event::Start(af))?;
    writer.write_event(Event::Text(BytesText::new(&face.family)))?;
    writer.write_event(Event::End(BytesEnd::new("AppliedFont")))?;
    let mut leading = BytesStart::new("Leading");
    leading.push_attribute(("type", "enumeration"));
    writer.write_event(Event::Start(leading))?;
    writer.write_event(Event::Text(BytesText::new("Auto")))?;
    writer.write_event(Event::End(BytesEnd::new("Leading")))?;
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    writer.write_event(Event::End(BytesEnd::new("TextDefault")))?;
    Ok(())
}

fn write_text_preference(writer: &mut Writer<Cursor<Vec<u8>>>) -> Result<(), quick_xml::Error> {
    emit_empty_with_attrs(
        writer,
        "TextPreference",
        &[("UseOpticalSize", "false".to_string())],
    )
}

/// Bring `Resources/Preferences.xml` in line with the engine's
/// composition: `<TextPreference UseOpticalSize="false"/>` when the part
/// spells no optical-size preference (see the module doc). Byte-identical
/// when the part already spells one.
pub fn patch_preferences(
    original: &[u8],
    default_face: Option<&FontFace>,
) -> Result<Vec<u8>, quick_xml::Error> {
    // First pass: what the part already says.
    let (has_text_pref, has_attr, has_text_default) = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut found = (false, false, false);
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Empty(e) | Event::Start(e) if e.name().as_ref() == TEXT_PREFERENCE => {
                    found.0 = true;
                    found.1 = has_use_optical_size(&e);
                }
                Event::Empty(e) | Event::Start(e) if e.name().as_ref() == TEXT_DEFAULT => {
                    found.2 = true;
                }
                _ => {}
            }
            buf.clear();
        }
        found
    };
    // The default to state, when the part has none and the host named
    // the face it composes unstyled text with.
    let text_default = default_face.filter(|_| !has_text_default);
    if has_attr && text_default.is_none() {
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
            // The element exists without the attribute: add it, keeping
            // the element's own shape (self-closing or not).
            Event::Empty(ref e)
                if has_text_pref && !has_attr && e.name().as_ref() == TEXT_PREFERENCE =>
            {
                let mut owned = e.to_owned();
                owned.push_attribute((USE_OPTICAL_SIZE, b"false".as_slice()));
                writer.write_event(Event::Empty(owned))?;
            }
            Event::Start(ref e)
                if has_text_pref && !has_attr && e.name().as_ref() == TEXT_PREFERENCE =>
            {
                let mut owned = e.to_owned();
                owned.push_attribute((USE_OPTICAL_SIZE, b"false".as_slice()));
                writer.write_event(Event::Start(owned))?;
            }
            // No element: a self-closing root opens up around it (and
            // around the text default) …
            Event::Empty(ref e) if e.name().as_ref() == ROOT => {
                writer.write_event(Event::Start(e.to_owned()))?;
                if !has_text_pref {
                    write_text_preference(&mut writer)?;
                }
                if let Some(face) = text_default {
                    write_text_default(&mut writer, face)?;
                }
                writer.write_event(Event::End(BytesEnd::new("idPkg:Preferences")))?;
            }
            // … and an open root takes them as its last children.
            Event::End(ref e) if e.name().as_ref() == ROOT => {
                if !has_text_pref {
                    write_text_preference(&mut writer)?;
                }
                if let Some(face) = text_default {
                    write_text_default(&mut writer, face)?;
                }
                writer.write_event(Event::End(e.to_owned()))?;
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"/>"#;

    #[test]
    fn an_empty_part_gains_the_preference() {
        let out = String::from_utf8(patch_preferences(EMPTY.as_bytes(), None).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><TextPreference UseOpticalSize="false"/></idPkg:Preferences>"#
        );
    }

    #[test]
    fn an_open_root_without_the_element_gains_it_last() {
        let src = r#"<idPkg:Preferences xmlns:idPkg="x" DOMVersion="20.0"><ViewPreference HorizontalMeasurementUnits="Points"/></idPkg:Preferences>"#;
        let out = String::from_utf8(patch_preferences(src.as_bytes(), None).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<idPkg:Preferences xmlns:idPkg="x" DOMVersion="20.0"><ViewPreference HorizontalMeasurementUnits="Points"/><TextPreference UseOpticalSize="false"/></idPkg:Preferences>"#
        );
    }

    #[test]
    fn an_element_without_the_attribute_gains_it() {
        let src = r#"<idPkg:Preferences xmlns:idPkg="x" DOMVersion="20.0"><TextPreference TypographersQuotes="true"/></idPkg:Preferences>"#;
        let out = String::from_utf8(patch_preferences(src.as_bytes(), None).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<idPkg:Preferences xmlns:idPkg="x" DOMVersion="20.0"><TextPreference TypographersQuotes="true" UseOpticalSize="false"/></idPkg:Preferences>"#
        );
        let open = r#"<idPkg:Preferences xmlns:idPkg="x"><TextPreference TypographersQuotes="true"><Properties/></TextPreference></idPkg:Preferences>"#;
        let out = String::from_utf8(patch_preferences(open.as_bytes(), None).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<idPkg:Preferences xmlns:idPkg="x"><TextPreference TypographersQuotes="true" UseOpticalSize="false"><Properties/></TextPreference></idPkg:Preferences>"#
        );
    }

    #[test]
    fn a_spelled_preference_passes_through_byte_identical_either_way() {
        for value in ["true", "false"] {
            let src = format!(
                r#"<idPkg:Preferences xmlns:idPkg="x" DOMVersion="20.0"><TextPreference TypographersQuotes="true" UseOpticalSize="{value}" SmallCap="70"/></idPkg:Preferences>"#
            );
            assert_eq!(
                patch_preferences(src.as_bytes(), None).unwrap(),
                src.as_bytes()
            );
        }
    }

    #[test]
    fn the_hosts_default_face_becomes_the_text_default_once() {
        let face = FontFace::synthesized("Inter", "Regular");
        let out =
            String::from_utf8(patch_preferences(EMPTY.as_bytes(), Some(&face)).unwrap()).unwrap();
        assert!(out.contains(r#"<TextPreference UseOpticalSize="false"/><TextDefault FontStyle="Regular" PointSize="12"><Properties><AppliedFont type="string">Inter</AppliedFont><Leading type="enumeration">Auto</Leading></Properties></TextDefault></idPkg:Preferences>"#), "{out}");
        // A part that already states its defaults keeps them.
        let src = r#"<idPkg:Preferences xmlns:idPkg="x"><TextPreference UseOpticalSize="true"/><TextDefault PointSize="10"><Properties><AppliedFont type="string">Minion Pro</AppliedFont></Properties></TextDefault></idPkg:Preferences>"#;
        assert_eq!(
            patch_preferences(src.as_bytes(), Some(&face)).unwrap(),
            src.as_bytes()
        );
        // And one stating the preference but not the defaults gains only the defaults.
        let src = r#"<idPkg:Preferences xmlns:idPkg="x"><TextPreference UseOpticalSize="true"/></idPkg:Preferences>"#;
        let out =
            String::from_utf8(patch_preferences(src.as_bytes(), Some(&face)).unwrap()).unwrap();
        assert_eq!(
            out,
            r#"<idPkg:Preferences xmlns:idPkg="x"><TextPreference UseOpticalSize="true"/><TextDefault FontStyle="Regular" PointSize="12"><Properties><AppliedFont type="string">Inter</AppliedFont><Leading type="enumeration">Auto</Leading></Properties></TextDefault></idPkg:Preferences>"#
        );
    }
}
