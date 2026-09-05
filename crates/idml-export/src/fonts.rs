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

//! `Resources/Fonts.xml` — every face the document applies, declared as
//! the INSTANCE InDesign binds.
//!
//! InDesign reads the `<FontFamily>` / `<Font>` declarations to bind
//! each `AppliedFont` + `FontStyle` pair in the stories and styles to
//! an installed face. Measured on InDesign 20.0.1 with the faces
//! installed: a family-only declaration (`Name="Source Serif 4"
//! PostScriptName="SourceSerif4"`, the fixture generator's form) comes
//! back SUBSTITUTED even though the family is there — no installed
//! instance carries that PostScript name (Source Serif 4 Regular is
//! `SourceSerif4Roman-Regular`, JetBrains Mono Italic is
//! `JetBrainsMonoItalic-Regular`: infixes only the font file's `fvar`
//! records carry). What binds is InDesign's own spelling, one `<Font>`
//! per instance, copied from its export of a variable-font document:
//!
//! ```xml
//! <FontFamily Self="dif5" Name="Fraunces">
//!   <Font Self="dif5FontnFraunces Regular" FontFamily="Fraunces"
//!         Name="Fraunces Regular" PostScriptName="Fraunces-Regular"
//!         Status="Installed" FontStyleName="Regular" FontType="OpenTypeTT"
//!         WritingScript="0" FullName="Fraunces Regular"
//!         FullNameNative="Fraunces Regular" FontStyleNameNative="Regular"
//!         PlatformName="$ID/" Version="Version 1.000;[b76b70a41]"
//!         TypekitID="$ID/" NumDesignAxes="4"
//!         DesignAxesName="Optical%20Size Weight Softness Wonky"
//!         DesignAxesValues="9 400 0 1" />
//! ```
//!
//! So this pass collects the `(family, style)` pairs the model applies
//! (paragraph and character styles through their `BasedOn` cascade,
//! every run through the run > character style > paragraph style
//! cascade; `Regular` when no `FontStyle` is set) and declares each as
//! an instance:
//!
//! * the host's byte-derived [`FontFace`]s (its registry, expanded per
//!   `fvar` named instance — PostScript name, full name, version,
//!   outline type, design axes) name the face when they know it;
//! * a face known only by name is SYNTHESISED (`Name` = "family
//!   style", `PostScriptName` = the two joined by `-` with spaces
//!   removed, no axes) — the convention most families follow.
//!
//! An existing `<Font>` is kept byte-identical when it is already an
//! instance (`Name` == `FontFamily` + " " + `FontStyleName`, exactly —
//! every InDesign-authored entry in the corpus is), so an
//! InDesign-authored package round-trips untouched. A family-only entry
//! is REWRITTEN in place as the instance it stands for (its `Self`
//! kept), from bytes when the host has them, synthesised otherwise —
//! never kept. A missing face joins its family's element; a missing
//! family is appended.

use std::collections::{BTreeMap, HashSet};
use std::io::Cursor;

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{Paragraph, StyleSheet};
use paged_scene::Document;

use crate::images::percent_encode;
use crate::rewrite::{attr_value, emit_empty_with_attrs, emit_start_with_attrs, format_f32};

const PKG_NS: &str = "http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging";

/// One face, named the way `Resources/Fonts.xml` declares an instance.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFace {
    /// `FontFamily` — the `AppliedFont` value the document uses.
    pub family: String,
    /// `FontStyleName` — the `FontStyle` value (`Regular` when unset);
    /// for a variable font the named instance's subfamily.
    pub style: String,
    /// `PostScriptName`: the `fvar` instance's `postScriptNameID` name,
    /// else the `name` table's (static font), else synthesised.
    pub postscript_name: String,
    /// `FullName` — InDesign writes "<Family> <Style>" for a variable
    /// instance (the regular one included); a static font's own full
    /// name otherwise.
    pub full_name: String,
    /// `FontType`: `OpenTypeCFF`, `OpenTypeTT`, `TrueType` or
    /// `OpenTypeCID` — InDesign's vocabulary.
    pub font_type: String,
    /// `Version` (`name` id 5), written only when known.
    pub version: Option<String>,
    /// A variable instance's `(axis name, coordinate)`s in axis order —
    /// `NumDesignAxes` / `DesignAxesName` / `DesignAxesValues`. Empty
    /// for a static face.
    pub axes: Vec<(String, f32)>,
}

impl FontFace {
    /// A face known only by the names a style carries.
    pub fn synthesized(family: &str, style: &str) -> Self {
        let style = if style.trim().is_empty() {
            "Regular"
        } else {
            style.trim()
        };
        let compact = |s: &str| s.split_whitespace().collect::<String>();
        Self {
            family: family.to_string(),
            style: style.to_string(),
            postscript_name: format!("{}-{}", compact(family), compact(style)),
            full_name: format!("{family} {style}"),
            font_type: "TrueType".to_string(),
            version: None,
            axes: Vec::new(),
        }
    }

    /// `Name` — InDesign's "family style" spelling (the regular face
    /// too: `Fraunces Regular`).
    pub fn name(&self) -> String {
        format!("{} {}", self.family, self.style)
    }
}

/// The face for `(family, style)`: the host's byte-derived one when it
/// knows it (exact style, then case-insensitive), else synthesised.
pub fn resolve_face(family: &str, style: &str, known: &[FontFace]) -> FontFace {
    let style = if style.trim().is_empty() {
        "Regular"
    } else {
        style.trim()
    };
    known
        .iter()
        .find(|k| k.family == family && k.style == style)
        .or_else(|| {
            known
                .iter()
                .find(|k| k.family == family && k.style.eq_ignore_ascii_case(style))
        })
        .cloned()
        .unwrap_or_else(|| FontFace::synthesized(family, style))
}

/// The `(family, style)` pairs the document applies, in first-use
/// order, deduplicated. Styles first (their cascade resolved), then
/// every run through the run > character style > paragraph style
/// cascade — a run with no font anywhere in its cascade contributes
/// nothing (InDesign's own default face is not modelled).
fn applied_pairs(doc: &Document) -> Vec<(String, String)> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |family: Option<&str>, style: Option<&str>| {
        let Some(family) = family.map(str::trim).filter(|f| !f.is_empty()) else {
            return;
        };
        let style = style
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Regular");
        let key = (family.to_string(), style.to_string());
        if seen.insert(key.clone()) {
            out.push(key);
        }
    };
    let styles: &StyleSheet = &doc.styles;
    for id in styles.paragraph_styles.keys() {
        let r = styles.resolve_paragraph(id);
        push(r.font.as_deref(), r.font_style.as_deref());
    }
    for id in styles.character_styles.keys() {
        let r = styles.resolve_character(id);
        push(r.font.as_deref(), r.font_style.as_deref());
    }
    fn walk(
        doc: &Document,
        paras: &[Paragraph],
        push: &mut impl FnMut(Option<&str>, Option<&str>),
    ) {
        for p in paras {
            for run in &p.runs {
                let r = doc.resolved_run_attrs(p, run);
                push(r.font.as_deref(), r.font_style.as_deref());
            }
            if let Some(t) = &p.table {
                for c in &t.cells {
                    walk(doc, &c.paragraphs, push);
                }
            }
            for f in &p.footnotes {
                walk(doc, &f.paragraphs, push);
            }
        }
    }
    for s in &doc.stories {
        walk(doc, &s.story.paragraphs, &mut push);
    }
    out
}

/// Every face the export must declare: the instance of each `(family,
/// style)` the document applies, resolved against `known` (the host's
/// byte-derived faces) or synthesised — sorted by family then style so
/// the output is deterministic.
pub fn used_faces(doc: &Document, known: &[FontFace]) -> Vec<FontFace> {
    let mut faces: BTreeMap<(String, String), FontFace> = BTreeMap::new();
    for (family, style) in applied_pairs(doc) {
        faces
            .entry((family.clone(), style.clone()))
            .or_insert_with(|| resolve_face(&family, &style, known));
    }
    faces.into_values().collect()
}

/// What `Resources/Fonts.xml` already declares.
#[derive(Default)]
struct FontsLayout {
    /// `(FontFamily, FontStyleName)` of every `<Font>`.
    pairs: HashSet<(String, String)>,
}

fn scan(original: &[u8]) -> Result<FontsLayout, quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut out = FontsLayout::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) if e.name().as_ref() == b"Font" => {
                if let Some(family) = attr_value(e, b"FontFamily") {
                    out.pairs.insert((family, style_of(e)));
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// `FontStyleName`, `Regular` when absent or blank.
fn style_of(e: &BytesStart) -> String {
    attr_value(e, b"FontStyleName")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Regular".to_string())
}

/// Whether an existing `<Font>` is already an instance declaration:
/// `Name` == `FontFamily` + " " + `FontStyleName`, byte for byte (every
/// InDesign-authored entry in the corpus is, trailing spaces included).
fn is_instance_form(e: &BytesStart) -> bool {
    match (
        attr_value(e, b"FontFamily"),
        attr_value(e, b"FontStyleName"),
        attr_value(e, b"Name"),
    ) {
        (Some(family), Some(style), Some(name)) => name == format!("{family} {style}"),
        _ => false,
    }
}

/// `Self` for a family the part does not yet declare.
fn family_self(family: &str) -> String {
    format!(
        "FontFamily/{}",
        family
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>()
    )
}

/// The attributes of one `<Font …/>` in InDesign's order. `Self`
/// follows InDesign's `<family Self>Fontn<Name>` form unless the caller
/// keeps an existing one.
fn font_attrs(self_id: String, face: &FontFace) -> Vec<(&'static str, String)> {
    let name = face.name();
    let mut attrs: Vec<(&str, String)> = vec![
        ("Self", self_id),
        ("FontFamily", face.family.clone()),
        ("Name", name),
        ("PostScriptName", face.postscript_name.clone()),
        ("Status", "Installed".to_string()),
        ("FontStyleName", face.style.clone()),
        ("FontType", face.font_type.clone()),
        ("WritingScript", "0".to_string()),
        ("FullName", face.full_name.clone()),
        ("FullNameNative", face.full_name.clone()),
        ("FontStyleNameNative", face.style.clone()),
        ("PlatformName", "$ID/".to_string()),
    ];
    if let Some(v) = &face.version {
        attrs.push(("Version", v.clone()));
    }
    attrs.push(("TypekitID", "$ID/".to_string()));
    if !face.axes.is_empty() {
        attrs.push(("NumDesignAxes", face.axes.len().to_string()));
        attrs.push((
            "DesignAxesName",
            face.axes
                .iter()
                .map(|(n, _)| percent_encode(n))
                .collect::<Vec<_>>()
                .join(" "),
        ));
        attrs.push((
            "DesignAxesValues",
            face.axes
                .iter()
                .map(|(_, v)| format_f32(*v))
                .collect::<Vec<_>>()
                .join(" "),
        ));
    }
    attrs
}

fn write_font(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    self_id: String,
    face: &FontFace,
) -> Result<(), quick_xml::Error> {
    emit_empty_with_attrs(writer, "Font", &font_attrs(self_id, face))
}

fn write_family(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    family: &str,
    faces: &[&FontFace],
) -> Result<(), quick_xml::Error> {
    let id = family_self(family);
    emit_start_with_attrs(
        writer,
        "FontFamily",
        &[("Self", id.clone()), ("Name", family.to_string())],
    )?;
    for f in faces {
        write_font(writer, format!("{id}Fontn{}", f.name()), f)?;
    }
    writer.write_event(Event::End(BytesEnd::new("FontFamily")))?;
    Ok(())
}

/// Rewrite `Resources/Fonts.xml` so every face in `applied` is declared
/// as an instance and no family-only `<Font>` survives. `known` names
/// the faces the host has bytes for (a family-only entry for a face the
/// document no longer applies is rewritten from these, or synthesised).
/// Byte-identical to `original` when it already declares every applied
/// face and every entry is an instance.
pub fn patch_fonts(
    original: &[u8],
    applied: &[FontFace],
    known: &[FontFace],
) -> Result<Vec<u8>, quick_xml::Error> {
    let layout = scan(original)?;
    // family → the faces to add, in `applied` order.
    let mut missing: BTreeMap<String, Vec<&FontFace>> = BTreeMap::new();
    for f in applied {
        if !layout.pairs.contains(&(f.family.clone(), f.style.clone())) {
            missing.entry(f.family.clone()).or_default().push(f);
        }
    }

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    // The `<FontFamily>` currently open: (Name, Self).
    let mut open_family: Option<(String, String)> = None;
    // Inside a family-only `<Font>` being replaced: skip its subtree.
    let mut skipping: usize = 0;
    // The instance a family-only entry becomes: `applied` (which
    // already resolved against `known`) first, then `known`, else
    // synthesised.
    let rewrite_of = |e: &BytesStart| -> Option<(String, FontFace)> {
        if is_instance_form(e) {
            return None;
        }
        let family = attr_value(e, b"FontFamily")?;
        let style = style_of(e);
        let self_id = attr_value(e, b"Self")
            .unwrap_or_else(|| format!("{}Fontn{family} {style}", family_self(&family)));
        let face = applied
            .iter()
            .find(|f| f.family == family && f.style == style)
            .cloned()
            .unwrap_or_else(|| resolve_face(&family, &style, known));
        Some((self_id, face))
    };
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        if skipping > 0 {
            match ev {
                Event::Eof => break,
                Event::Start(_) => skipping += 1,
                Event::End(_) => skipping -= 1,
                _ => {}
            }
            buf.clear();
            continue;
        }
        match ev {
            Event::Eof => break,
            Event::Start(ref e) if e.name().as_ref() == b"Font" => match rewrite_of(e) {
                Some((self_id, face)) => {
                    write_font(&mut writer, self_id, &face)?;
                    skipping = 1;
                }
                None => writer.write_event(ev.borrow())?,
            },
            Event::Empty(ref e) if e.name().as_ref() == b"Font" => match rewrite_of(e) {
                Some((self_id, face)) => write_font(&mut writer, self_id, &face)?,
                None => writer.write_event(ev.borrow())?,
            },
            Event::Start(ref e) if e.name().as_ref() == b"FontFamily" => {
                open_family = attr_value(e, b"Name").zip(attr_value(e, b"Self"));
                writer.write_event(ev.borrow())?;
            }
            // A self-closing family (declared, no faces): expand it
            // around the faces it is missing.
            Event::Empty(ref e) if e.name().as_ref() == b"FontFamily" => {
                let hit = attr_value(e, b"Name")
                    .zip(attr_value(e, b"Self"))
                    .and_then(|(name, id)| missing.remove(&name).map(|faces| (id, faces)));
                match hit {
                    Some((id, faces)) => {
                        writer.write_event(Event::Start(e.borrow()))?;
                        for f in faces {
                            write_font(&mut writer, format!("{id}Fontn{}", f.name()), f)?;
                        }
                        writer.write_event(Event::End(BytesEnd::new("FontFamily")))?;
                    }
                    None => writer.write_event(ev.borrow())?,
                }
            }
            Event::End(ref e) if e.name().as_ref() == b"FontFamily" => {
                if let Some((name, id)) = open_family.take() {
                    if let Some(faces) = missing.remove(&name) {
                        for f in faces {
                            write_font(&mut writer, format!("{id}Fontn{}", f.name()), f)?;
                        }
                    }
                }
                writer.write_event(ev.borrow())?;
            }
            Event::End(ref e) if e.name().as_ref() == b"idPkg:Fonts" => {
                // Families the part never declared.
                for (family, faces) in std::mem::take(&mut missing) {
                    write_family(&mut writer, &family, &faces)?;
                }
                writer.write_event(ev.borrow())?;
            }
            _ => writer.write_event(ev.borrow())?,
        }
        buf.clear();
    }
    let out = writer.into_inner().into_inner();
    Ok(if out == original {
        original.to_vec()
    } else {
        out
    })
}

/// A whole `Resources/Fonts.xml` for a package that has none.
pub fn fonts_part(faces: &[FontFace], dom_version: &str) -> Result<Vec<u8>, quick_xml::Error> {
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    writer.write_event(Event::Decl(BytesDecl::new(
        "1.0",
        Some("UTF-8"),
        Some("yes"),
    )))?;
    let mut root = BytesStart::new("idPkg:Fonts");
    root.push_attribute(("xmlns:idPkg", PKG_NS));
    root.push_attribute(("DOMVersion", dom_version));
    writer.write_event(Event::Start(root))?;
    let mut by_family: BTreeMap<&str, Vec<&FontFace>> = BTreeMap::new();
    for f in faces {
        by_family.entry(f.family.as_str()).or_default().push(f);
    }
    for (family, faces) in by_family {
        write_family(&mut writer, family, &faces)?;
    }
    writer.write_event(Event::End(BytesEnd::new("idPkg:Fonts")))?;
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fraunces_regular() -> FontFace {
        FontFace {
            family: "Fraunces".into(),
            style: "Regular".into(),
            postscript_name: "Fraunces-Regular".into(),
            full_name: "Fraunces Regular".into(),
            font_type: "OpenTypeTT".into(),
            version: Some("Version 1.000;[b76b70a41]".into()),
            axes: vec![
                ("Optical Size".into(), 9.0),
                ("Weight".into(), 400.0),
                ("Softness".into(), 0.0),
                ("Wonky".into(), 1.0),
            ],
        }
    }

    /// InDesign 20.0.1's own entry for the Fraunces Regular instance
    /// (`vf-canonical.idml`), attribute for attribute.
    const INDESIGN_FRAUNCES: &str = r#"<Font Self="dif5FontnFraunces Regular" FontFamily="Fraunces" Name="Fraunces Regular" PostScriptName="Fraunces-Regular" Status="Installed" FontStyleName="Regular" FontType="OpenTypeTT" WritingScript="0" FullName="Fraunces Regular" FullNameNative="Fraunces Regular" FontStyleNameNative="Regular" PlatformName="$ID/" Version="Version 1.000;[b76b70a41]" TypekitID="$ID/" NumDesignAxes="4" DesignAxesName="Optical%20Size Weight Softness Wonky" DesignAxesValues="9 400 0 1"/>"#;

    #[test]
    fn a_variable_instance_is_written_in_indesigns_spelling() {
        let out = fonts_part(&[fraunces_regular()], "20.0").unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains(&INDESIGN_FRAUNCES.replace("dif5Fontn", "FontFamily/FrauncesFontn")),
            "{out}"
        );
    }

    #[test]
    fn a_synthesised_face_follows_the_common_naming_convention() {
        let f = FontFace::synthesized("Noto Sans Hebrew", "Regular");
        assert_eq!(f.name(), "Noto Sans Hebrew Regular");
        assert_eq!(f.postscript_name, "NotoSansHebrew-Regular");
        assert_eq!(f.full_name, "Noto Sans Hebrew Regular");
        assert!(f.axes.is_empty());
        let b = FontFace::synthesized("Source Serif 4", "Semibold Italic");
        assert_eq!(b.postscript_name, "SourceSerif4-SemiboldItalic");
        assert_eq!(FontFace::synthesized("X", "").style, "Regular");
    }

    /// The fixture generator's family-only form (measured SUBSTITUTED).
    const FAMILY_ONLY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Fonts xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><FontFamily Self="FontFamily/Fraunces" Name="Fraunces"><Font Self="Font/Fraunces" FontFamily="Fraunces" Name="Fraunces" PostScriptName="Fraunces" Status="Installed" FontStyleName="Regular" FontType="TrueType"/></FontFamily></idPkg:Fonts>"#;

    #[test]
    fn a_family_only_entry_is_rewritten_as_the_instance_keeping_its_self() {
        // From bytes (the registry knows the face)…
        let out = patch_fonts(FAMILY_ONLY.as_bytes(), &[], &[fraunces_regular()]).unwrap();
        let out = String::from_utf8(out).unwrap();
        let want = INDESIGN_FRAUNCES.replace("dif5FontnFraunces Regular", "Font/Fraunces");
        assert!(out.contains(&want), "{out}");
        assert_eq!(out.matches("<Font ").count(), 1);
        // …and the rewritten part is stable.
        assert_eq!(
            patch_fonts(out.as_bytes(), &[], &[fraunces_regular()]).unwrap(),
            out.as_bytes()
        );
        // Without bytes: synthesised, still an instance, never family-only.
        let out =
            String::from_utf8(patch_fonts(FAMILY_ONLY.as_bytes(), &[], &[]).unwrap()).unwrap();
        assert!(
            out.contains(r#"<Font Self="Font/Fraunces" FontFamily="Fraunces" Name="Fraunces Regular" PostScriptName="Fraunces-Regular" Status="Installed" FontStyleName="Regular" FontType="TrueType" WritingScript="0" FullName="Fraunces Regular" FullNameNative="Fraunces Regular" FontStyleNameNative="Regular" PlatformName="$ID/" TypekitID="$ID/"/>"#),
            "{out}"
        );
        assert!(!out.contains(r#"Name="Fraunces" "#), "{out}");
    }

    const INSTANCE_FORM: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Fonts xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><FontFamily Self="di39" Name="Minion Pro"><Font Self="di39FontnMinion Pro Regular" FontFamily="Minion Pro" Name="Minion Pro Regular" PostScriptName="MinionPro-Regular" Status="Installed" FontStyleName="Regular" FontType="OpenTypeCFF" WritingScript="0" FullName="Minion Pro" FullNameNative="Minion Pro" FontStyleNameNative="Regular" PlatformName="$ID/" Version="Version 2.112" TypekitID="$ID/" /><Font Self="di39FontnMinion Pro Bold Cond" FontFamily="Minion Pro" Name="Minion Pro Bold Cond" PostScriptName="MinionPro-BoldCn" Status="Substituted" FontStyleName="Bold Cond" FontType="OpenTypeCFF" WritingScript="0" FullName="Minion Pro Bold Cond" FullNameNative="Minion Pro Bold Cond" FontStyleNameNative="Bold Cond" PlatformName="$ID/" Version="Version 2.112" TypekitID="$ID/" /></FontFamily></idPkg:Fonts>"#;

    #[test]
    fn indesigns_own_instance_entries_pass_through_byte_identically() {
        let applied = [
            FontFace::synthesized("Minion Pro", "Regular"),
            FontFace::synthesized("Minion Pro", "Bold Cond"),
        ];
        // Even when the host has bytes for the face: InDesign's entry
        // stands (a registered file may not be the installed one).
        let known = [FontFace {
            postscript_name: "MinionPro-Regular-VF".into(),
            ..FontFace::synthesized("Minion Pro", "Regular")
        }];
        assert_eq!(
            patch_fonts(INSTANCE_FORM.as_bytes(), &applied, &known).unwrap(),
            INSTANCE_FORM.as_bytes()
        );
        assert_eq!(
            patch_fonts(INSTANCE_FORM.as_bytes(), &[], &[]).unwrap(),
            INSTANCE_FORM.as_bytes()
        );
    }

    #[test]
    fn a_missing_style_joins_its_family_and_a_missing_family_is_appended() {
        let applied = [
            FontFace::synthesized("Minion Pro", "Italic"),
            fraunces_regular(),
        ];
        let out = String::from_utf8(patch_fonts(INSTANCE_FORM.as_bytes(), &applied, &[]).unwrap())
            .unwrap();
        assert!(out.contains(r#"<Font Self="di39FontnMinion Pro Italic" FontFamily="Minion Pro" Name="Minion Pro Italic" PostScriptName="MinionPro-Italic" Status="Installed" FontStyleName="Italic" FontType="TrueType" WritingScript="0" FullName="Minion Pro Italic" FullNameNative="Minion Pro Italic" FontStyleNameNative="Italic" PlatformName="$ID/" TypekitID="$ID/"/></FontFamily>"#), "{out}");
        assert!(
            out.contains(&format!(
                r#"<FontFamily Self="FontFamily/Fraunces" Name="Fraunces">{}</FontFamily></idPkg:Fonts>"#,
                INDESIGN_FRAUNCES.replace("dif5Fontn", "FontFamily/FrauncesFontn")
            )),
            "{out}"
        );
        assert_eq!(
            patch_fonts(out.as_bytes(), &applied, &[]).unwrap(),
            out.as_bytes(),
            "idempotent"
        );
    }

    #[test]
    fn resolve_prefers_the_hosts_face_then_synthesises() {
        let known = [fraunces_regular()];
        assert_eq!(resolve_face("Fraunces", "Regular", &known), known[0]);
        assert_eq!(resolve_face("Fraunces", "regular", &known), known[0]);
        assert_eq!(resolve_face("Fraunces", "", &known), known[0]);
        assert_eq!(
            resolve_face("Fraunces", "Bold", &known),
            FontFace::synthesized("Fraunces", "Bold")
        );
    }
}
