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

//! New-resource injection for save-back (W1.15 lane 2).
//!
//! Swatches / gradients created by ops since load live in the model's
//! `palette` but have no `<Color>` / `<Gradient>` element in
//! `Resources/Graphic.xml`; paragraph / character / OBJECT styles
//! created by ops live in `styles` with no `<ParagraphStyle>` /
//! `<CharacterStyle>` / `<ObjectStyle>` in `Resources/Styles.xml`. A
//! round-trip that leaves them unserialised re-opens with a
//! *referenced-but-undefined* resource — a frame whose
//! `FillColor="Color/u3"` resolves to nothing. This module closes that
//! gap by **injecting** the missing entries into the existing resource
//! XML, just before the matching close tag, in the canonical `paged_gen`
//! shape so a re-parse reproduces the resolved appearance.
//!
//! The object-style lane is the same defect one rung up: page items
//! carry `AppliedObjectStyle`, `Operation::CreateObjectStyle` mints a
//! definition into `styles.object_styles`, and until that definition is
//! written the exported package has an applied style nothing defines.
//! (The pointer itself is written by the page-item patch lane — see
//! `rewrite::applied_object_style_patch`.)
//!
//! Both patchers are pure pass-throughs when the model carries nothing
//! the source XML lacks — they re-emit the original token stream and
//! splice nothing, so an unmutated document's resource entries stay
//! byte-identical (the writer then takes the verbatim copy path).

use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::graphic::{ColorEntry, GradientEntry, GradientKind, Graphic};
use idml_import::styles::{CharacterStyleDef, ObjectStyleDef, ParagraphStyleDef, StyleSheet};

use crate::rewrite::{escape_attr, format_f32};

/// Read an attribute's decoded value off a start tag (local copy of the
/// rewrite helper — kept private so the two modules stay independent).
fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| std::str::from_utf8(&a.value).ok().map(|s| s.to_string()))
}

/// Emit a self-closing element from `(key, value)` pairs (values
/// escaped). Element name taken verbatim.
fn emit_empty(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    name: &str,
    attrs: &[(&str, String)],
) -> Result<(), quick_xml::Error> {
    let mut e = BytesStart::new(name.to_string());
    for (k, v) in attrs {
        e.push_attribute((k.as_bytes(), escape_attr(v).as_bytes()));
    }
    writer.write_event(Event::Empty(e))?;
    Ok(())
}

/// Whitespace-separated channel values the IDML way.
fn format_color_value(value: &[f32]) -> String {
    value
        .iter()
        .map(|v| format_f32(*v))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------
// Graphic.xml — colours + gradients
// ---------------------------------------------------------------------

/// Serialise one model `<Color>` swatch in the canonical attribute order
/// (`Self Model Space ColorValue Name [AlternateSpace AlternateColorValue
/// TintValue]`). The parser keys on these attributes, so a re-parse
/// reproduces the swatch and any frame referencing it resolves.
fn write_color(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    c: &ColorEntry,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![
        ("Self", c.self_id.clone()),
        ("Model", c.model.as_attr().to_string()),
        ("Space", c.space.as_attr().to_string()),
        ("ColorValue", format_color_value(&c.value)),
    ];
    if let Some(name) = &c.name {
        attrs.push(("Name", name.clone()));
    }
    if let Some(alt) = c.alternate_space {
        attrs.push(("AlternateSpace", alt.as_attr().to_string()));
        attrs.push((
            "AlternateColorValue",
            format_color_value(&c.alternate_value),
        ));
    }
    if let Some(t) = c.tint {
        attrs.push(("TintValue", format_f32(t)));
    }
    emit_empty(writer, "Color", &attrs)
}

/// Serialise one model `<Gradient>` swatch + its stops.
fn write_gradient(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    g: &GradientEntry,
) -> Result<(), quick_xml::Error> {
    let kind = match g.kind {
        GradientKind::Radial => "Radial",
        // Linear is the IDML default for an unknown / linear type.
        GradientKind::Linear | GradientKind::Unknown => "Linear",
    };
    let mut attrs: Vec<(&str, String)> = vec![("Self", g.self_id.clone())];
    if let Some(name) = &g.name {
        attrs.push(("Name", name.clone()));
    }
    attrs.push(("Type", kind.to_string()));
    let mut start = BytesStart::new("Gradient");
    for (k, v) in &attrs {
        start.push_attribute((k.as_bytes(), escape_attr(v).as_bytes()));
    }
    writer.write_event(Event::Start(start))?;
    for s in &g.stops {
        let mut sattrs: Vec<(&str, String)> = vec![
            ("StopColor", s.stop_color.clone()),
            ("Location", format_f32(s.location_pct)),
        ];
        if let Some(m) = s.midpoint_pct {
            sattrs.push(("Midpoint", format_f32(m)));
        }
        emit_empty(writer, "GradientStop", &sattrs)?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Gradient")))?;
    Ok(())
}

/// Rewrite `Resources/Graphic.xml` so every model `<Color>` / `<Gradient>`
/// is present. Existing entries pass through verbatim; entries the source
/// lacks are appended just before `</idPkg:Graphic>`. Byte-identical to
/// `original` when nothing new.
pub fn patch_graphic(original: &[u8], palette: &Graphic) -> Result<Vec<u8>, quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    let mut seen_colors: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_gradients: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                match e.name().as_ref() {
                    b"Color" => {
                        if let Some(id) = attr_value(e, b"Self") {
                            seen_colors.insert(id);
                        }
                    }
                    b"Gradient" => {
                        if let Some(id) = attr_value(e, b"Self") {
                            seen_gradients.insert(id);
                        }
                    }
                    _ => {}
                }
                writer.write_event(ev.borrow())?;
            }
            Event::End(ref e) if e.name().as_ref() == b"idPkg:Graphic" => {
                // Inject the model entries the source never carried, in
                // the palette's stable BTreeMap order.
                for c in palette.colors.values() {
                    if !seen_colors.contains(&c.self_id) {
                        write_color(&mut writer, c)?;
                    }
                }
                for g in palette.gradients.values() {
                    if !seen_gradients.contains(&g.self_id) {
                        write_gradient(&mut writer, g)?;
                    }
                }
                writer.write_event(ev.borrow())?;
            }
            _ => writer.write_event(ev.borrow())?,
        }
        buf.clear();
    }

    Ok(writer.into_inner().into_inner())
}

// ---------------------------------------------------------------------
// Styles.xml — paragraph + character styles
// ---------------------------------------------------------------------

/// Common authoring fields shared by paragraph + character styles. Only
/// the high-frequency knobs are serialised; the rest cascade from
/// `BasedOn` / the document default (a freshly-created style carries only
/// name + based_on until a `SetStyleProperty` writes a field).
fn push_style_common(
    attrs: &mut Vec<(&'static str, String)>,
    name: &Option<String>,
    based_on: &Option<String>,
    font: &Option<String>,
    font_style: &Option<String>,
    point_size: Option<f32>,
    fill_color: &Option<String>,
) {
    if let Some(n) = name {
        attrs.push(("Name", n.clone()));
    }
    if let Some(b) = based_on {
        attrs.push(("BasedOn", b.clone()));
    }
    if let Some(f) = font {
        attrs.push(("AppliedFont", f.clone()));
    }
    if let Some(fs) = font_style {
        attrs.push(("FontStyle", fs.clone()));
    }
    if let Some(sz) = point_size {
        attrs.push(("PointSize", format_f32(sz)));
    }
    if let Some(fc) = fill_color {
        attrs.push(("FillColor", fc.clone()));
    }
}

fn write_paragraph_style(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    s: &ParagraphStyleDef,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![("Self", s.self_id.clone())];
    push_style_common(
        &mut attrs,
        &s.name,
        &s.based_on,
        &s.font,
        &s.font_style,
        s.point_size,
        &s.fill_color,
    );
    emit_empty(writer, "ParagraphStyle", &attrs)
}

fn write_character_style(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    s: &CharacterStyleDef,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![("Self", s.self_id.clone())];
    push_style_common(
        &mut attrs,
        &s.name,
        &s.based_on,
        &s.font,
        &s.font_style,
        s.point_size,
        &s.fill_color,
    );
    emit_empty(writer, "CharacterStyle", &attrs)
}

/// True for an id in IDML's reserved `$ID/[…]` namespace
/// (`ObjectStyle/$ID/[None]`). These are InDesign's own defaults: it
/// materialises them itself, so the writer never SYNTHESISES one the
/// source package didn't carry — emitting a user-shaped `<ObjectStyle>`
/// for `[None]` would present an application default as an authored
/// style (and re-open as a duplicate next to the real one).
fn is_reserved_style_id(id: &str) -> bool {
    id.contains("/$ID/")
}

/// Serialise one model `<ObjectStyle>`. Covers the whole field
/// vocabulary [`ObjectStyleDef`] models — the fill / stroke / corner
/// defaults a page item inherits when it carries no override — so a
/// re-parse reproduces the resolved appearance.
///
/// `BasedOn` is written as an ATTRIBUTE, matching what
/// [`push_style_common`] already does for the paragraph / character
/// lanes. The parser accepts either that or InDesign's
/// `<Properties><BasedOn type="object">…</BasedOn></Properties>` child
/// form, so the cascade survives the round trip; the attribute form
/// keeps this a single self-closing element like its siblings.
fn write_object_style(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    s: &ObjectStyleDef,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![("Self", s.self_id.clone())];
    if let Some(n) = &s.name {
        attrs.push(("Name", n.clone()));
    }
    if let Some(b) = &s.based_on {
        attrs.push(("BasedOn", b.clone()));
    }
    if let Some(c) = &s.fill_color {
        attrs.push(("FillColor", c.clone()));
    }
    if let Some(t) = s.fill_tint {
        attrs.push(("FillTint", format_f32(t)));
    }
    if let Some(c) = &s.stroke_color {
        attrs.push(("StrokeColor", c.clone()));
    }
    if let Some(t) = s.stroke_tint {
        attrs.push(("StrokeTint", format_f32(t)));
    }
    if let Some(w) = s.stroke_weight {
        attrs.push(("StrokeWeight", format_f32(w)));
    }
    if let Some(o) = &s.corner_option {
        attrs.push(("CornerOption", o.clone()));
    }
    if let Some(r) = s.corner_radius {
        attrs.push(("CornerRadius", format_f32(r)));
    }
    emit_empty(writer, "ObjectStyle", &attrs)
}

/// The object styles `styles` carries that `seen` (the source part)
/// doesn't define, minus the reserved `$ID/[…]` entries. Empty for an
/// unmutated document, which is what keeps the part byte-identical.
fn missing_object_styles<'a>(
    styles: &'a StyleSheet,
    seen: &'a std::collections::HashSet<String>,
) -> impl Iterator<Item = &'a ObjectStyleDef> {
    styles
        .object_styles
        .values()
        .filter(move |s| !seen.contains(&s.self_id) && !is_reserved_style_id(&s.self_id))
}

/// What the WHOLE `Resources/Styles.xml` holds — the knowledge a single
/// forward pass does not have when it reaches the first group.
///
/// # The defect this closes
///
/// `patch_styles` used to accumulate its "already defined" set as it
/// read, and inject the model styles it had not seen yet at the FIRST
/// `</RootParagraphStyleGroup>`. But a part can carry more than one root
/// group of a kind — the corpus's generated packages open with a group
/// holding only InDesign's reserved `$ID/[No paragraph style]` and put
/// the document's real styles in a SECOND group further down. At the
/// first close every real style was still "unseen", so all of them were
/// injected there, and then the second group defined them again: two
/// elements with the same `Self` in one part, the injected copy missing
/// everything the writer does not model (`NextStyle`, `Justification`,
/// `Hyphenation`, `AppliedNumberingList`, `StrokeColor`).
///
/// This is the same shape as the story rewrite's range misalignment (see
/// [`crate::rewrite::rewrite_story`]) and as the nested-group
/// duplication fixed before it: a streaming pass deciding "is this new?"
/// from what it happens to have read so far. The fix is the same in
/// kind — establish the whole-document fact FIRST — even though the fact
/// itself is different, so the two are not shared code.
#[derive(Default)]
struct StylesLayout {
    para: std::collections::HashSet<String>,
    character: std::collections::HashSet<String>,
    object: std::collections::HashSet<String>,
    /// How many `<Root*StyleGroup>` elements of each kind the part
    /// carries. New definitions go into the LAST one, which is where a
    /// document that has both keeps its own styles.
    para_groups: usize,
    char_groups: usize,
    object_groups: usize,
}

/// Read `original` once for [`StylesLayout`]. Deliberately a separate
/// pass over the same bytes rather than a lookahead: the emitting pass
/// then has one rule and no state that means different things at
/// different points in the stream.
fn scan_styles(original: &[u8]) -> Result<StylesLayout, quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut out = StylesLayout::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => match e.name().as_ref() {
                b"ParagraphStyle" => {
                    if let Some(id) = attr_value(e, b"Self") {
                        out.para.insert(id);
                    }
                }
                b"CharacterStyle" => {
                    if let Some(id) = attr_value(e, b"Self") {
                        out.character.insert(id);
                    }
                }
                b"ObjectStyle" => {
                    if let Some(id) = attr_value(e, b"Self") {
                        out.object.insert(id);
                    }
                }
                b"RootParagraphStyleGroup" => out.para_groups += 1,
                b"RootCharacterStyleGroup" => out.char_groups += 1,
                b"RootObjectStyleGroup" => out.object_groups += 1,
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// Rewrite `Resources/Styles.xml` so every model paragraph / character
/// / object style is present. New paragraph styles are injected before
/// the LAST `</RootParagraphStyleGroup>`, character styles before the
/// last `</RootCharacterStyleGroup>`, object styles before the last
/// `</RootObjectStyleGroup>`. Byte-identical to `original` when
/// nothing new (and when the source has no group, the new styles flush
/// at `</idPkg:Styles>` so they aren't silently dropped).
///
/// "New" means absent from the WHOLE part, not merely unseen so far —
/// see [`StylesLayout`].
pub fn patch_styles(original: &[u8], styles: &StyleSheet) -> Result<Vec<u8>, quick_xml::Error> {
    let layout = scan_styles(original)?;

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    // Which occurrence of each root group the stream is on, so the
    // injection lands in the last one.
    let mut para_group_seen = 0usize;
    let mut char_group_seen = 0usize;
    let mut object_group_seen = 0usize;
    let mut in_last_para_group = false;
    let mut in_last_char_group = false;
    let mut in_last_object_group = false;
    let mut para_group_closed = false;
    let mut char_group_closed = false;
    let mut object_group_closed = false;

    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Eof => break,
            // An EMPTY `<RootObjectStyleGroup/>` (a document with no
            // object styles at all) has no close tag to inject before,
            // so expand it into a real element around the new defs.
            // Only fires on the LAST such element, and only when there
            // is something to write — otherwise it falls through and
            // passes the empty tag through verbatim.
            Event::Empty(ref e)
                if e.name().as_ref() == b"RootObjectStyleGroup"
                    && object_group_seen + 1 == layout.object_groups
                    && missing_object_styles(styles, &layout.object)
                        .next()
                        .is_some() =>
            {
                object_group_seen += 1;
                writer.write_event(Event::Start(e.borrow()))?;
                for s in missing_object_styles(styles, &layout.object) {
                    write_object_style(&mut writer, s)?;
                }
                writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                    "RootObjectStyleGroup",
                )))?;
                object_group_closed = true;
            }
            Event::Start(ref e) | Event::Empty(ref e) => {
                // Only the group counters need tracking here now; which
                // styles exist was settled by the pre-pass.
                match e.name().as_ref() {
                    b"RootParagraphStyleGroup" => {
                        para_group_seen += 1;
                        in_last_para_group = para_group_seen == layout.para_groups;
                    }
                    b"RootCharacterStyleGroup" => {
                        char_group_seen += 1;
                        in_last_char_group = char_group_seen == layout.char_groups;
                    }
                    b"RootObjectStyleGroup" => {
                        object_group_seen += 1;
                        in_last_object_group = object_group_seen == layout.object_groups;
                    }
                    _ => {}
                }
                writer.write_event(ev.borrow())?;
            }
            Event::End(ref e)
                if e.name().as_ref() == b"RootParagraphStyleGroup" && in_last_para_group =>
            {
                for s in styles.paragraph_styles.values() {
                    if !layout.para.contains(&s.self_id) {
                        write_paragraph_style(&mut writer, s)?;
                    }
                }
                para_group_closed = true;
                writer.write_event(ev.borrow())?;
            }
            Event::End(ref e)
                if e.name().as_ref() == b"RootCharacterStyleGroup" && in_last_char_group =>
            {
                for s in styles.character_styles.values() {
                    if !layout.character.contains(&s.self_id) {
                        write_character_style(&mut writer, s)?;
                    }
                }
                char_group_closed = true;
                writer.write_event(ev.borrow())?;
            }
            Event::End(ref e)
                if e.name().as_ref() == b"RootObjectStyleGroup" && in_last_object_group =>
            {
                for s in missing_object_styles(styles, &layout.object) {
                    write_object_style(&mut writer, s)?;
                }
                object_group_closed = true;
                writer.write_event(ev.borrow())?;
            }
            Event::End(ref e) if e.name().as_ref() == b"idPkg:Styles" => {
                // Fallback: a source with no Root*StyleGroup (rare) still
                // gets the new defs so a reference never dangles.
                if !para_group_closed {
                    for s in styles.paragraph_styles.values() {
                        if !layout.para.contains(&s.self_id) {
                            write_paragraph_style(&mut writer, s)?;
                        }
                    }
                }
                if !char_group_closed {
                    for s in styles.character_styles.values() {
                        if !layout.character.contains(&s.self_id) {
                            write_character_style(&mut writer, s)?;
                        }
                    }
                }
                // Object styles need their group: a bare `<ObjectStyle>`
                // loose in the part isn't the shape InDesign reads them
                // from, so synthesise the wrapper rather than flush the
                // defs on their own.
                if !object_group_closed
                    && missing_object_styles(styles, &layout.object)
                        .next()
                        .is_some()
                {
                    writer.write_event(Event::Start(BytesStart::new("RootObjectStyleGroup")))?;
                    for s in missing_object_styles(styles, &layout.object) {
                        write_object_style(&mut writer, s)?;
                    }
                    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                        "RootObjectStyleGroup",
                    )))?;
                }
                writer.write_event(ev.borrow())?;
            }
            _ => writer.write_event(ev.borrow())?,
        }
        buf.clear();
    }

    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use idml_import::ObjectStyleDef;

    /// A minimal but REAL-SHAPED `Resources/Styles.xml`: the three root
    /// groups InDesign always writes, each carrying its reserved
    /// `$ID/[…]` entry, plus one user-authored object style with the
    /// full field vocabulary `ObjectStyleDef` models.
    const STYLES_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]" Imported="false"/></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]" Imported="false"/></RootParagraphStyleGroup>
<RootObjectStyleGroup Self="u9f"><ObjectStyle Self="ObjectStyle/$ID/[None]" Name="$ID/[None]" Imported="false"/><ObjectStyle Self="ObjectStyle/Callout" Name="Callout" FillColor="Color/Black" FillTint="20" StrokeColor="Color/Paper" StrokeTint="55" StrokeWeight="1.5" CornerOption="RoundedCorner" CornerRadius="6"/></RootObjectStyleGroup>
</idPkg:Styles>"#;

    fn sheet() -> StyleSheet {
        idml_import::parse_stylesheet(STYLES_XML).expect("parse styles")
    }

    /// Build the def `Operation::CreateObjectStyle` produces: a
    /// `default()` body carrying only `self_id` / `name` / `based_on`
    /// (every other field cascades). `paged-mutate` can't be depended
    /// on from here — core's `paged-scene` / `paged-mutate` dep on
    /// `idml-import` would make the git dep circular (see the commit
    /// that re-homed the integration tests) — so this mirrors the
    /// `style_crud!` create arm exactly.
    fn created_object_style(self_id: &str, name: &str, based_on: Option<&str>) -> ObjectStyleDef {
        ObjectStyleDef {
            self_id: self_id.to_string(),
            name: Some(name.to_string()),
            based_on: based_on.map(str::to_string),
            ..Default::default()
        }
    }

    /// THE PRIME INVARIANT (the lazy-verbatim posture): a document
    /// nobody mutated re-emits its `Resources/Styles.xml` byte-for-byte,
    /// so `write_idml` takes the raw-copy path for that entry.
    #[test]
    fn unmutated_styles_resource_round_trips_byte_identically() {
        let out = patch_styles(STYLES_XML, &sheet()).expect("patch");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(STYLES_XML),
            "an unmutated style sheet must reproduce its on-disk bytes"
        );
    }

    /// An object style created since load is written into
    /// `<RootObjectStyleGroup>` — the resource part InDesign reads
    /// object styles from. Without this the exported package carries an
    /// `AppliedObjectStyle` pointing at a definition that doesn't
    /// exist.
    #[test]
    fn scene_created_object_style_is_written_into_the_root_group() {
        let mut styles = sheet();
        let def = created_object_style("ObjectStyle/u0", "Sidebar", Some("ObjectStyle/Callout"));
        styles.object_styles.insert(def.self_id.clone(), def);

        let out = patch_styles(STYLES_XML, &styles).expect("patch");
        let s = String::from_utf8(out).expect("utf8");
        assert!(
            s.contains(r#"<ObjectStyle Self="ObjectStyle/u0" Name="Sidebar" BasedOn="ObjectStyle/Callout"/>"#),
            "the new object style must be defined, not just referenced: {s}"
        );
        // ...and INSIDE the root group, not loose in the part.
        let group_close = s.find("</RootObjectStyleGroup>").expect("group close");
        let def_at = s.find(r#"Self="ObjectStyle/u0""#).expect("emitted");
        assert!(def_at < group_close, "must land inside the group: {s}");
    }
}
