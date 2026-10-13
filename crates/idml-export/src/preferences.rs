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

/// `DocumentBleedUniformSize` / `DocumentSlugUniformSize` for these
/// values: InDesign applies the top value to every side while the flag is
/// true (its default when absent), so four different bleeds need it false
/// (asked of InDesign 20.0.1 through core's InDesign lane, scenario
/// `bleed`: 9 / 12 / 3 / 6 pt read back as 9 on every side).
fn uniform(values: [f32; 4]) -> &'static str {
    if values.iter().all(|v| (v - values[0]).abs() <= 1e-4) {
        "true"
    } else {
        "false"
    }
}

fn uniform_flags(pref: &paged_model::DocumentPreference) -> [(&'static str, &'static str); 2] {
    [
        (
            "DocumentBleedUniformSize",
            uniform([
                pref.bleed_top,
                pref.bleed_bottom,
                pref.bleed_inside_or_left,
                pref.bleed_outside_or_right,
            ]),
        ),
        (
            "DocumentSlugUniformSize",
            uniform([
                pref.slug_top,
                pref.slug_bottom,
                pref.slug_inside_or_left,
                pref.slug_right_or_outside,
            ]),
        ),
    ]
}

/// A `<DocumentPreference>` for a package that has none — in neither
/// `Resources/Preferences.xml` nor `designmap.xml` — but whose model sets a
/// bleed or a slug: without the element InDesign opens the document at its
/// own defaults and the edit is lost (asked of InDesign 20.0.1 through
/// core's InDesign lane, scenario `bleed`: a 9 / 12 / 3 / 6 pt bleed set on
/// a generated document opened with none).
///
/// The element carries the first page's size and `FacingPages="true"` —
/// what InDesign assumes when the element is absent — because an element
/// without them makes InDesign re-shuffle every page into its own spread
/// (see [`patch_document_preference`]), and the bleed and slug values that
/// are not 0. It goes last in the part's root. Byte-identical when the
/// package already has the element or the model sets neither.
pub fn insert_document_preference(
    original: &[u8],
    designmap: Option<&[u8]>,
    pref: &paged_model::DocumentPreference,
    page_size: Option<(f32, f32)>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let named = |part: &[u8]| part.windows(19).any(|w| w == b"<DocumentPreference");
    let values: [(&str, f32); 8] = [
        ("DocumentBleedTopOffset", pref.bleed_top),
        ("DocumentBleedBottomOffset", pref.bleed_bottom),
        ("DocumentBleedInsideOrLeftOffset", pref.bleed_inside_or_left),
        (
            "DocumentBleedOutsideOrRightOffset",
            pref.bleed_outside_or_right,
        ),
        ("SlugTopOffset", pref.slug_top),
        ("SlugBottomOffset", pref.slug_bottom),
        ("SlugInsideOrLeftOffset", pref.slug_inside_or_left),
        ("SlugRightOrOutsideOffset", pref.slug_right_or_outside),
    ];
    let set: Vec<(&str, f32)> = values.into_iter().filter(|(_, v)| v.abs() > 1e-4).collect();
    let Some((width, height)) = page_size else {
        return Ok(original.to_vec());
    };
    if set.is_empty() || named(original) || designmap.is_some_and(named) {
        return Ok(original.to_vec());
    }
    let mut attrs = vec![
        ("PageHeight", crate::rewrite::format_f32(height)),
        ("PageWidth", crate::rewrite::format_f32(width)),
        ("FacingPages", "true".to_string()),
    ];
    attrs.extend(
        set.iter()
            .map(|(n, v)| (*n, crate::rewrite::format_f32(*v))),
    );
    attrs.extend(uniform_flags(pref).map(|(n, v)| (n, v.to_string())));

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Empty(ref e) if e.name().as_ref() == ROOT => {
                writer.write_event(Event::Start(e.to_owned()))?;
                emit_empty_with_attrs(&mut writer, "DocumentPreference", &attrs)?;
                writer.write_event(Event::End(BytesEnd::new("idPkg:Preferences")))?;
            }
            Event::End(ref e) if e.name().as_ref() == ROOT => {
                emit_empty_with_attrs(&mut writer, "DocumentPreference", &attrs)?;
                writer.write_event(Event::End(e.to_owned()))?;
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

/// The document's bleed and slug written onto the `<DocumentPreference>`
/// the part already carries — in `Resources/Preferences.xml`, where
/// InDesign keeps it, or in `designmap.xml`, where the engine's older
/// fixtures put it.
///
/// Only an existing element is patched, attribute by attribute: InDesign
/// re-shuffles every page into its own spread when a package that never
/// had a `DocumentPreference` gains one without its page-size and
/// facing-page attributes (measured 2026-09-05), so a part without the
/// element is left alone. Byte-identical when every value already agrees.
pub fn patch_document_preference(
    original: &[u8],
    pref: &paged_model::DocumentPreference,
) -> Result<Vec<u8>, quick_xml::Error> {
    let wanted: [(&[u8], f32); 8] = [
        (b"DocumentBleedTopOffset", pref.bleed_top),
        (b"DocumentBleedBottomOffset", pref.bleed_bottom),
        (
            b"DocumentBleedInsideOrLeftOffset",
            pref.bleed_inside_or_left,
        ),
        (
            b"DocumentBleedOutsideOrRightOffset",
            pref.bleed_outside_or_right,
        ),
        (b"SlugTopOffset", pref.slug_top),
        (b"SlugBottomOffset", pref.slug_bottom),
        (b"SlugInsideOrLeftOffset", pref.slug_inside_or_left),
        (b"SlugRightOrOutsideOffset", pref.slug_right_or_outside),
    ];
    // An absent attribute reads as 0 on import, so 0 needs no attribute.
    let differs = |e: &BytesStart| {
        wanted.iter().any(|(name, v)| {
            let have = e
                .attributes()
                .flatten()
                .find(|a| a.key.as_ref() == *name)
                .and_then(|a| std::str::from_utf8(&a.value).ok()?.parse::<f32>().ok())
                .unwrap_or(0.0);
            (have - v).abs() > 1e-4
        })
    };
    let needs = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut found = false;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Empty(e) | Event::Start(e)
                    if e.name().as_ref() == b"DocumentPreference" && differs(&e) =>
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
    if !needs {
        return Ok(original.to_vec());
    }
    // The uniform-size flags follow the values written (see `uniform`).
    let flags = uniform_flags(pref);
    let rewrite = |e: &BytesStart| -> BytesStart<'static> {
        let mut out = BytesStart::new(String::from_utf8_lossy(e.name().as_ref()).into_owned());
        let mut written: Vec<&[u8]> = Vec::new();
        for a in e.attributes().flatten() {
            if let Some((name, v)) = wanted.iter().find(|(name, _)| a.key.as_ref() == *name) {
                out.push_attribute((*name, crate::rewrite::format_f32(*v).as_bytes()));
                written.push(name);
            } else if let Some((name, v)) =
                flags.iter().find(|(n, _)| a.key.as_ref() == n.as_bytes())
            {
                out.push_attribute((*name, *v));
                written.push(name.as_bytes());
            } else {
                out.push_attribute((a.key.as_ref(), a.value.as_ref()));
            }
        }
        for (name, v) in &wanted {
            if !written.contains(name) && v.abs() > 1e-4 {
                out.push_attribute((*name, crate::rewrite::format_f32(*v).as_bytes()));
            }
        }
        for (name, v) in &flags {
            if !written.contains(&name.as_bytes()) && *v == "false" {
                out.push_attribute((*name, *v));
            }
        }
        out
    };
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Empty(ref e) if e.name().as_ref() == b"DocumentPreference" => {
                writer.write_event(Event::Empty(rewrite(e)))?
            }
            Event::Start(ref e) if e.name().as_ref() == b"DocumentPreference" => {
                writer.write_event(Event::Start(rewrite(e)))?
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

    fn bleed(top: f32) -> paged_model::DocumentPreference {
        paged_model::DocumentPreference {
            bleed_top: top,
            ..Default::default()
        }
    }

    #[test]
    fn a_package_without_the_preference_gains_one_for_its_bleed() {
        let src = r#"<idPkg:Preferences xmlns:idPkg="x" DOMVersion="20.0"><ViewPreference/></idPkg:Preferences>"#;
        let out = String::from_utf8(
            insert_document_preference(src.as_bytes(), None, &bleed(9.0), Some((595.276, 841.89)))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            out,
            r#"<idPkg:Preferences xmlns:idPkg="x" DOMVersion="20.0"><ViewPreference/><DocumentPreference PageHeight="841.89" PageWidth="595.276" FacingPages="true" DocumentBleedTopOffset="9" DocumentBleedUniformSize="false" DocumentSlugUniformSize="true"/></idPkg:Preferences>"#
        );
    }

    #[test]
    fn the_preference_is_not_inserted_twice_nor_without_a_reason() {
        let src = br#"<idPkg:Preferences xmlns:idPkg="x"><DocumentPreference PageWidth="612"/></idPkg:Preferences>"#;
        let size = Some((612.0, 792.0));
        assert_eq!(
            insert_document_preference(src, None, &bleed(9.0), size).unwrap(),
            src.to_vec()
        );
        let bare = br#"<idPkg:Preferences xmlns:idPkg="x"/>"#;
        let dm = br#"<Document><DocumentPreference PageWidth="612"/></Document>"#;
        assert_eq!(
            insert_document_preference(bare, Some(dm), &bleed(9.0), size).unwrap(),
            bare.to_vec()
        );
        assert_eq!(
            insert_document_preference(bare, None, &bleed(0.0), size).unwrap(),
            bare.to_vec()
        );
    }

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
    fn a_changed_bleed_is_written_onto_the_existing_preference() {
        let src = br#"<idPkg:Preferences><DocumentPreference PageWidth="595" FacingPages="false" DocumentBleedTopOffset="0" DocumentBleedBottomOffset="0"/></idPkg:Preferences>"#;
        let mut p = paged_model::DocumentPreference::default();
        assert_eq!(
            patch_document_preference(src, &p).unwrap(),
            src.to_vec(),
            "unchanged"
        );
        p.bleed_top = 8.5;
        p.bleed_outside_or_right = 3.0;
        let out = String::from_utf8(patch_document_preference(src, &p).unwrap()).unwrap();
        assert!(out.contains(r#"PageWidth="595" FacingPages="false" DocumentBleedTopOffset="8.5" DocumentBleedBottomOffset="0" DocumentBleedOutsideOrRightOffset="3""#), "{out}");
        let none = br#"<idPkg:Preferences><TextPreference/></idPkg:Preferences>"#;
        assert_eq!(
            patch_document_preference(none, &p).unwrap(),
            none.to_vec(),
            "no element is minted"
        );
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
