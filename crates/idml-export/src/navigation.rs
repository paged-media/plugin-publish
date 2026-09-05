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

//! `designmap.xml` save-back for the document-level resources InDesign
//! was measured to drop from engine-authored files: sections, the
//! hyperlink block (destinations, hyperlinks, bookmarks) and conditional
//! text. One streaming pass, patching in place what the source already
//! carries in InDesign's spelling, NORMALISING what it carries in the
//! engine's older spelling, and appending what the model has and the
//! source lacks.
//!
//! # What InDesign 20.0.1 actually reads (measured 2026-09-05)
//!
//! The experiment (`thoughts`: indesign/experiments, variants V0–VF) is
//! the source of every rule here; each is a measured fact, not a guess:
//!
//! * **Hyperlinks bind only when the whole block sits AFTER the last
//!   `<idPkg:Story>` include.** Our block sat before the stories and
//!   InDesign reported 0 hyperlinks (V0/V4); moving it alone made the
//!   destinations + bookmarks appear (V3); `DestinationUniqueKey` alone
//!   changes nothing (V4) and is optional.
//! * **`<Hyperlink>` names its destination as a typed Properties child**
//!   — `<Properties><BorderColor type="enumeration">Black</BorderColor>
//!   <Destination type="object">HyperlinkURLDestination/…</Destination>
//!   </Properties>` (V6a: 2 of 2 bound). The `Destination="…"`
//!   ATTRIBUTE is ignored, and attribute + `DestinationUniqueKey` on the
//!   same element makes the file UNOPENABLE (V5). `Source` stays an
//!   attribute; our prefixed ids (`HyperlinkTextSource/u…`) are fine.
//! * **A text destination is an inline story marker**, never a designmap
//!   element (V6c) — see `rewrite::inject_story_navigation`; this pass
//!   DROPS any designmap `<HyperlinkTextDestination>`.
//! * **Conditions live in `designmap.xml`**, direct children of
//!   `<Document>` (after `<NamedGrid>` / before `<idPkg:Preferences>`;
//!   after the `<Layer>`s also works — V2/VF). The invented
//!   `<RootConditionalTextGroup>` wrapper hides everything inside it
//!   (V0: 0 conditions). The indicator colour is a typed child
//!   (`<Properties><IndicatorColor type="enumeration">Green</…>`); the
//!   attribute form is ignored. A set's members are
//!   `<Properties><SetConditions><VisibilityPair Condition="…"
//!   Visibility="true"/>…`; the `Conditions="a b"` attribute is ignored
//!   and every such set then silently captures ALL conditions. A
//!   `<ConditionalTextPreference ShowConditionIndicators="ShowIndicators"
//!   ActiveConditionSet="n"/>` follows them.
//! * A `<Section>` spells its numbering style as
//!   `<Properties><PageNumberStyle type="enumeration">LowerRoman</…>`
//!   (a union type — it may also name a custom list). Sections sit after
//!   the `<idPkg:Spread>` includes.
//!
//! # Byte identity
//!
//! A source already in InDesign's spelling round-trips byte-for-byte
//! when the model agrees with it: every patch is a `Keep` and every
//! element streams through verbatim. A source in the engine's older
//! spelling is REWRITTEN on every export — that is the point: an
//! existing book exports correctly without re-authoring.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use idml_import::{
    Bookmark, ConditionDef, ConditionSetDef, Hyperlink, HyperlinkDestination,
    HyperlinkDestinationKind, NumberingStyle,
};

use crate::rewrite::{
    attr_value, emit_empty_with_attrs, emit_start_with_attrs, patch_start, Patch,
};

/// A model section prepared for emission (`Length` is derived from the
/// page order, which only the caller with the whole document knows).
pub(crate) struct SectionSpec {
    pub self_id: String,
    pub page_start: Option<String>,
    pub length: usize,
    pub continue_numbering: bool,
    pub include_prefix: bool,
    pub start_at: Option<u32>,
    pub section_prefix: Option<String>,
    pub marker: Option<String>,
    pub numbering_style: NumberingStyle,
}

/// Everything the pass needs from the model, prepared by the caller.
pub(crate) struct NavigationPlan<'a> {
    pub sections: Vec<SectionSpec>,
    /// URL + page destinations only — text anchors are story markers.
    pub destinations: Vec<&'a HyperlinkDestination>,
    pub hyperlinks: &'a [Hyperlink],
    pub bookmarks: &'a [Bookmark],
    pub conditions: &'a BTreeMap<String, ConditionDef>,
    pub condition_sets: &'a BTreeMap<String, ConditionSetDef>,
    /// Indicator colours the model does not carry, scraped from wherever
    /// the source spelled them (`Resources/Styles.xml` or the designmap),
    /// keyed by condition `Self`.
    pub indicator_colors: HashMap<String, String>,
    /// The `<CrossReferenceFormat>` a format-less `<CrossReferenceSource>`
    /// is pointed at (`rewrite::inject_story_navigation`); emitted before
    /// the story includes when the designmap has no format of that id.
    /// `None` when no story needs one.
    pub xref_format: Option<String>,
}

/// The one cross-reference format the exporter owns: InDesign's "Full
/// Paragraph & Page Number" building blocks (copied from a 20.0.1 export,
/// English strings).
fn write_xref_format(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    id: &str,
) -> Result<(), quick_xml::Error> {
    emit_start_with_attrs(
        writer,
        "CrossReferenceFormat",
        &[
            ("Self", id.to_string()),
            ("Name", "Full Paragraph & Page Number".to_string()),
            ("AppliedCharacterStyle", "n".to_string()),
        ],
    )?;
    let blocks: [(&str, &str); 4] = [
        ("CustomStringBuildingBlock", "\u{201c}"),
        ("FullParagraphBuildingBlock", "$ID/"),
        ("CustomStringBuildingBlock", "\u{201d} on page "),
        ("PageNumberBuildingBlock", "$ID/"),
    ];
    for (i, (kind, text)) in blocks.iter().enumerate() {
        emit_empty_with_attrs(
            writer,
            "BuildingBlock",
            &[
                ("Self", format!("{id}BuildingBlock{i}")),
                ("BlockType", kind.to_string()),
                ("AppliedCharacterStyle", "n".to_string()),
                ("CustomText", text.to_string()),
                ("AppliedDelimiter", "$ID/".to_string()),
                ("IncludeDelimiter", "false".to_string()),
            ],
        )?;
    }
    writer.write_event(Event::End(BytesEnd::new("CrossReferenceFormat")))?;
    Ok(())
}

pub(crate) fn numbering_style_idml(s: NumberingStyle) -> &'static str {
    match s {
        NumberingStyle::Arabic => "Arabic",
        NumberingStyle::UpperRoman => "UpperRoman",
        NumberingStyle::LowerRoman => "LowerRoman",
        NumberingStyle::UpperAlpha => "UpperLetters",
        NumberingStyle::LowerAlpha => "LowerLetters",
    }
}

fn is_nav_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"HyperlinkURLDestination"
            | b"HyperlinkPageDestination"
            | b"HyperlinkTextDestination"
            | b"Hyperlink"
            | b"Bookmark"
    )
}

fn is_condition_name(name: &[u8]) -> bool {
    matches!(name, b"Condition" | b"ConditionSet")
}

/// What the pre-pass learned about the source designmap.
#[derive(Default)]
struct Layout {
    story_refs: usize,
    spread_refs: usize,
    sections: usize,
    /// Navigation elements (hyperlink block) in source order, and how
    /// many story refs preceded each — `true` when it sits after the
    /// last `<idPkg:Story>`.
    nav_after_stories: Vec<bool>,
    /// Document-level conditions / sets present.
    conditions: usize,
    condition_ids: HashSet<String>,
    section_ids: HashSet<String>,
    destination_ids: HashSet<String>,
    hyperlink_ids: HashSet<String>,
    bookmark_ids: HashSet<String>,
    /// Largest `DestinationUniqueKey` in use.
    max_key: u64,
    /// Existing key per destination `Self`.
    dest_keys: HashMap<String, u64>,
    has_named_grid: bool,
    /// Top-level `<Layer>` elements (depth 2).
    top_layers: usize,
    has_preferences_ref: bool,
    has_conditional_text_preference: bool,
    has_master_or_spread_ref: bool,
    xref_format_ids: HashSet<String>,
}

fn scan(original: &[u8]) -> Result<Layout, quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut out = Layout::default();
    let mut depth = 0usize;
    let mut stories_seen = 0usize;
    // Second pass semantics in one: count stories first so "after the
    // last" is decidable — cheap enough to just read twice.
    let story_total = {
        let mut r = Reader::from_reader(original);
        r.config_mut().trim_text(false);
        let mut b = Vec::new();
        let mut n = 0usize;
        loop {
            match r.read_event_into(&mut b)? {
                Event::Eof => break,
                Event::Empty(ref e) | Event::Start(ref e)
                    if e.name().as_ref() == b"idPkg:Story" =>
                {
                    n += 1
                }
                _ => {}
            }
            b.clear();
        }
        n
    };
    out.story_refs = story_total;
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_start = matches!(ev, Event::Start(_));
                if is_start {
                    depth += 1;
                }
                let at_depth = if is_start { depth } else { depth + 1 };
                let name = e.name().as_ref().to_vec();
                match name.as_slice() {
                    b"idPkg:Story" => stories_seen += 1,
                    b"idPkg:Spread" => out.spread_refs += 1,
                    b"idPkg:MasterSpread" => out.has_master_or_spread_ref = true,
                    b"idPkg:Preferences" => out.has_preferences_ref = true,
                    b"NamedGrid" => out.has_named_grid = true,
                    b"ConditionalTextPreference" => out.has_conditional_text_preference = true,
                    b"CrossReferenceFormat" => {
                        if let Some(id) = attr_value(e, b"Self") {
                            out.xref_format_ids.insert(id);
                        }
                    }
                    b"Layer" if at_depth == 2 => out.top_layers += 1,
                    b"Section" => {
                        out.sections += 1;
                        if let Some(id) = attr_value(e, b"Self") {
                            out.section_ids.insert(id);
                        }
                    }
                    n if is_nav_name(n) => {
                        out.nav_after_stories.push(stories_seen == story_total);
                        let id = attr_value(e, b"Self");
                        let key = attr_value(e, b"DestinationUniqueKey")
                            .and_then(|k| k.parse::<u64>().ok());
                        if let Some(k) = key {
                            out.max_key = out.max_key.max(k);
                        }
                        match (n, id) {
                            (b"Hyperlink", Some(id)) => {
                                out.hyperlink_ids.insert(id);
                            }
                            (b"Bookmark", Some(id)) => {
                                out.bookmark_ids.insert(id);
                            }
                            (_, Some(id)) => {
                                if let Some(k) = key {
                                    out.dest_keys.insert(id.clone(), k);
                                }
                                out.destination_ids.insert(id);
                            }
                            _ => {}
                        }
                    }
                    n if is_condition_name(n) && at_depth == 2 => {
                        out.conditions += 1;
                        if let Some(id) = attr_value(e, b"Self") {
                            out.condition_ids.insert(id);
                        }
                    }
                    _ => {}
                }
                if name == b"idPkg:Spread" {
                    out.has_master_or_spread_ref = true;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

// ---- emitters (InDesign's spelling) ---------------------------------------

fn write_properties_text_child(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    name: &str,
    ty: &str,
    text: &str,
) -> Result<(), quick_xml::Error> {
    let mut e = BytesStart::new(name);
    e.push_attribute(("type", ty));
    writer.write_event(Event::Start(e))?;
    writer.write_event(Event::Text(BytesText::new(text)))?;
    writer.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

fn write_section(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    s: &SectionSpec,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![
        ("Self", s.self_id.clone()),
        ("Length", s.length.to_string()),
        ("Name", String::new()),
        ("ContinueNumbering", s.continue_numbering.to_string()),
        ("IncludeSectionPrefix", s.include_prefix.to_string()),
    ];
    if let Some(n) = s.start_at {
        attrs.push(("PageNumberStart", n.to_string()));
    }
    attrs.push(("Marker", s.marker.clone().unwrap_or_default()));
    attrs.push(("PageStart", s.page_start.clone().unwrap_or_default()));
    attrs.push((
        "SectionPrefix",
        s.section_prefix.clone().unwrap_or_default(),
    ));
    emit_start_with_attrs(writer, "Section", &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_properties_text_child(
        writer,
        "PageNumberStyle",
        "enumeration",
        numbering_style_idml(s.numbering_style),
    )?;
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    writer.write_event(Event::End(BytesEnd::new("Section")))?;
    Ok(())
}

fn write_destination(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    d: &HyperlinkDestination,
    key: u64,
) -> Result<(), quick_xml::Error> {
    let short = d
        .self_id
        .rsplit('/')
        .next()
        .unwrap_or(&d.self_id)
        .to_string();
    match &d.kind {
        HyperlinkDestinationKind::Url(url) => emit_empty_with_attrs(
            writer,
            "HyperlinkURLDestination",
            &[
                ("Self", d.self_id.clone()),
                ("Name", url.clone()),
                ("DestinationURL", url.clone()),
                ("Hidden", "false".to_string()),
                ("DestinationUniqueKey", key.to_string()),
            ],
        ),
        HyperlinkDestinationKind::Page(page) => emit_empty_with_attrs(
            writer,
            "HyperlinkPageDestination",
            &[
                ("Self", d.self_id.clone()),
                ("Name", short),
                ("NameManually", "true".to_string()),
                ("DestinationPage", page.clone()),
                ("ViewSetting", "Fixed".to_string()),
                ("ViewPercentage", "100".to_string()),
                ("Hidden", "false".to_string()),
                ("DestinationUniqueKey", key.to_string()),
            ],
        ),
        // Never a designmap element — an inline story marker instead.
        HyperlinkDestinationKind::TextAnchor(_) => Ok(()),
    }
}

fn write_hyperlink(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    h: &Hyperlink,
    key: Option<u64>,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![
        ("Self", h.self_id.clone()),
        (
            "Name",
            h.name
                .clone()
                .unwrap_or_else(|| h.self_id.rsplit('/').next().unwrap_or("").to_string()),
        ),
    ];
    if let Some(src) = &h.source {
        attrs.push(("Source", src.clone()));
    }
    attrs.push(("Visible", "false".to_string()));
    attrs.push(("Highlight", "None".to_string()));
    attrs.push(("Width", "Thin".to_string()));
    attrs.push(("BorderStyle", "Solid".to_string()));
    attrs.push(("Hidden", "false".to_string()));
    if let Some(k) = key {
        attrs.push(("DestinationUniqueKey", k.to_string()));
    }
    emit_start_with_attrs(writer, "Hyperlink", &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_properties_text_child(writer, "BorderColor", "enumeration", "Black")?;
    if let Some(dest) = &h.destination {
        write_properties_text_child(writer, "Destination", "object", dest)?;
    }
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    writer.write_event(Event::End(BytesEnd::new("Hyperlink")))?;
    Ok(())
}

fn write_bookmark(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    b: &Bookmark,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![("Self", b.self_id.clone())];
    if let Some(n) = &b.name {
        attrs.push(("Name", n.clone()));
    }
    if let Some(d) = &b.destination {
        attrs.push(("Destination", d.clone()));
    }
    emit_empty_with_attrs(writer, "Bookmark", &attrs)
}

fn write_condition(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    c: &ConditionDef,
    color: Option<&str>,
) -> Result<(), quick_xml::Error> {
    let attrs: Vec<(&str, String)> = vec![
        ("Self", c.self_id.clone()),
        ("Name", c.name.clone().unwrap_or_default()),
        (
            "IndicatorMethod",
            c.indicator_method
                .clone()
                .unwrap_or_else(|| "UseHighlight".to_string()),
        ),
        ("Visible", c.visible.unwrap_or(true).to_string()),
    ];
    emit_start_with_attrs(writer, "Condition", &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_properties_text_child(
        writer,
        "IndicatorColor",
        "enumeration",
        color.unwrap_or("Red"),
    )?;
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    writer.write_event(Event::End(BytesEnd::new("Condition")))?;
    Ok(())
}

fn write_condition_set(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    s: &ConditionSetDef,
) -> Result<(), quick_xml::Error> {
    let attrs: Vec<(&str, String)> = vec![
        ("Self", s.self_id.clone()),
        ("Name", s.name.clone().unwrap_or_default()),
    ];
    emit_start_with_attrs(writer, "ConditionSet", &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    writer.write_event(Event::Start(BytesStart::new("SetConditions")))?;
    for member in &s.conditions {
        emit_empty_with_attrs(
            writer,
            "VisibilityPair",
            &[
                ("Condition", member.clone()),
                ("Visibility", "true".to_string()),
            ],
        )?;
    }
    writer.write_event(Event::End(BytesEnd::new("SetConditions")))?;
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    writer.write_event(Event::End(BytesEnd::new("ConditionSet")))?;
    Ok(())
}

fn write_conditional_text_preference(
    writer: &mut Writer<Cursor<Vec<u8>>>,
) -> Result<(), quick_xml::Error> {
    emit_empty_with_attrs(
        writer,
        "ConditionalTextPreference",
        &[
            ("ShowConditionIndicators", "ShowIndicators".to_string()),
            ("ActiveConditionSet", "n".to_string()),
        ],
    )
}

// ---- the pass ---------------------------------------------------------------

/// A buffered source element subtree (start + children + end), kept
/// verbatim until the pass knows what to do with it.
struct Subtree {
    name: Vec<u8>,
    start: BytesStart<'static>,
    /// `true` for a self-closing element (no children, no End).
    empty: bool,
    children: Vec<Event<'static>>,
}

impl Subtree {
    /// The text of the first `<NAME>` child (a typed Properties child).
    fn child_text(&self, name: &[u8]) -> Option<String> {
        let mut inside = false;
        for ev in &self.children {
            match ev {
                Event::Start(e) if e.name().as_ref() == name => inside = true,
                Event::Text(t) if inside => {
                    return t
                        .xml_content(quick_xml::XmlVersion::Implicit1_0)
                        .ok()
                        .map(|c| c.trim().to_string());
                }
                Event::End(e) if e.name().as_ref() == name => inside = false,
                _ => {}
            }
        }
        None
    }
    fn has_child(&self, name: &[u8]) -> bool {
        self.children
            .iter()
            .any(|ev| matches!(ev, Event::Start(e) | Event::Empty(e) if e.name().as_ref() == name))
    }
    fn replay(self, writer: &mut Writer<Cursor<Vec<u8>>>) -> Result<(), quick_xml::Error> {
        if self.empty {
            writer.write_event(Event::Empty(self.start))?;
        } else {
            writer.write_event(Event::Start(self.start))?;
            for ev in self.children {
                writer.write_event(ev)?;
            }
            writer.write_event(Event::End(BytesEnd::new(
                String::from_utf8_lossy(&self.name).into_owned(),
            )))?;
        }
        Ok(())
    }
}

/// The start tag to write after a patch: the ORIGINAL when the rebuilt
/// tag differs from it only by the spelling `patch_start` cannot keep (a
/// trailing space before `/>`, which InDesign writes), else the rebuilt
/// one. This is what keeps an unchanged InDesign element byte-identical.
fn unchanged_or(
    original: &BytesStart<'static>,
    rebuilt: BytesStart<'static>,
) -> BytesStart<'static> {
    if original.as_ref().trim_ascii_end() == rebuilt.as_ref().trim_ascii_end() {
        original.clone()
    } else {
        rebuilt
    }
}

fn opt_str_patch(raw: Option<&str>, v: Option<&str>) -> Patch {
    match v {
        Some(v) if raw == Some(v) => Patch::Keep,
        Some(v) => Patch::Set(v.to_string()),
        None => Patch::Keep,
    }
}

fn bool_patch(raw: Option<&str>, v: bool) -> Patch {
    if raw == Some(if v { "true" } else { "false" }) {
        Patch::Keep
    } else {
        Patch::Set(v.to_string())
    }
}

/// Render a source `<Section>` subtree against its model spec: patched
/// attributes, the `PageNumberStyle` typed child brought to the model's
/// value (added when absent and non-default).
fn emit_section(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sub: Subtree,
    s: &SectionSpec,
) -> Result<(), quick_xml::Error> {
    let style = numbering_style_idml(s.numbering_style);
    let start = patch_start(
        &sub.start,
        |k, raw| {
            let raw = std::str::from_utf8(raw).ok();
            match k {
                b"PageStart" => Some(opt_str_patch(raw, s.page_start.as_deref())),
                b"ContinueNumbering" => Some(bool_patch(raw, s.continue_numbering)),
                b"IncludeSectionPrefix" => Some(bool_patch(raw, s.include_prefix)),
                b"PageNumberStart" => Some(match s.start_at {
                    Some(n) if raw.and_then(|r| r.parse::<u32>().ok()) == Some(n) => Patch::Keep,
                    Some(n) => Patch::Set(n.to_string()),
                    None => Patch::Remove,
                }),
                b"SectionPrefix" => Some(opt_str_patch(
                    raw,
                    Some(s.section_prefix.as_deref().unwrap_or("")),
                )),
                b"Marker" => Some(opt_str_patch(raw, Some(s.marker.as_deref().unwrap_or("")))),
                b"Length" => Some(if raw == Some(s.length.to_string().as_str()) {
                    Patch::Keep
                } else {
                    Patch::Set(s.length.to_string())
                }),
                // The attribute spelling of the style (engine fixtures):
                // patched in place; InDesign's child spelling is handled
                // below.
                b"PageNumberStyle" => Some(if raw == Some(style) {
                    Patch::Keep
                } else {
                    Patch::Set(style.to_string())
                }),
                _ => None,
            }
        },
        &[],
    )?;
    let start = unchanged_or(&sub.start, start);
    let attr_style = attr_value(&sub.start, b"PageNumberStyle").is_some();
    let child_style = sub.child_text(b"PageNumberStyle");
    let needs_child =
        !attr_style && child_style.is_none() && s.numbering_style != NumberingStyle::Arabic;
    if sub.empty && !needs_child {
        writer.write_event(Event::Empty(start))?;
        return Ok(());
    }
    writer.write_event(Event::Start(start))?;
    // Children: replay, rewriting the PageNumberStyle child's text when
    // it disagrees with the model.
    let mut in_pns = false;
    let mut had_properties = false;
    for ev in sub.children {
        match ev {
            Event::Start(e) if e.name().as_ref() == b"PageNumberStyle" => {
                in_pns = true;
                writer.write_event(Event::Start(e))?;
            }
            Event::Text(t) if in_pns => {
                let raw = t
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .map(|c| c.into_owned())
                    .unwrap_or_default();
                if raw.trim() == style {
                    writer.write_event(Event::Text(t))?;
                } else {
                    writer.write_event(Event::Text(BytesText::new(style)))?;
                }
            }
            Event::End(e) if e.name().as_ref() == b"PageNumberStyle" => {
                in_pns = false;
                writer.write_event(Event::End(e))?;
            }
            Event::End(e) if e.name().as_ref() == b"Properties" => {
                had_properties = true;
                if needs_child {
                    write_properties_text_child(writer, "PageNumberStyle", "enumeration", style)?;
                }
                writer.write_event(Event::End(e))?;
            }
            other => writer.write_event(other)?,
        }
    }
    if needs_child && !had_properties {
        writer.write_event(Event::Start(BytesStart::new("Properties")))?;
        write_properties_text_child(writer, "PageNumberStyle", "enumeration", style)?;
        writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("Section")))?;
    Ok(())
}

/// Render a source `<Hyperlink>` against its model: verbatim when it is
/// already in InDesign's spelling and agrees with the model, else
/// re-serialised canonically (the `Destination` attribute — ignored by
/// InDesign, fatal next to a key — becomes the typed child).
fn emit_hyperlink(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sub: Subtree,
    h: &Hyperlink,
    key: Option<u64>,
) -> Result<(), quick_xml::Error> {
    let attr_dest = attr_value(&sub.start, b"Destination");
    let child_dest = sub.child_text(b"Destination");
    let source_ok = attr_value(&sub.start, b"Source") == h.source;
    let key_ok = match key {
        Some(k) => attr_value(&sub.start, b"DestinationUniqueKey") == Some(k.to_string()),
        None => true,
    };
    let canonical = attr_dest.is_none()
        && child_dest.as_deref() == h.destination.as_deref()
        && source_ok
        && key_ok
        && !sub.empty;
    if canonical {
        return sub.replay(writer);
    }
    write_hyperlink(writer, h, key)
}

fn emit_destination(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sub: Subtree,
    d: &HyperlinkDestination,
    key: u64,
) -> Result<(), quick_xml::Error> {
    // In place, with the key ensured; everything else (Name, ViewBounds,
    // Hidden…) is the source's business.
    let start = patch_start(
        &sub.start,
        |k, raw| match k {
            b"DestinationUniqueKey" => Some(if raw == key.to_string().as_bytes() {
                Patch::Keep
            } else {
                Patch::Set(key.to_string())
            }),
            b"DestinationURL" => match &d.kind {
                HyperlinkDestinationKind::Url(u) => Some(opt_str_patch(
                    std::str::from_utf8(raw).ok(),
                    Some(u.as_str()),
                )),
                _ => None,
            },
            b"DestinationPage" => match &d.kind {
                HyperlinkDestinationKind::Page(p) => Some(opt_str_patch(
                    std::str::from_utf8(raw).ok(),
                    Some(p.as_str()),
                )),
                _ => None,
            },
            _ => None,
        },
        &[("DestinationUniqueKey", key.to_string())],
    )?;
    let start = unchanged_or(&sub.start, start);
    Subtree { start, ..sub }.replay(writer)
}

fn emit_condition(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sub: Subtree,
    c: &ConditionDef,
    fallback_color: Option<&str>,
) -> Result<(), quick_xml::Error> {
    let attr_color = attr_value(&sub.start, b"IndicatorColor");
    let child_color = sub.child_text(b"IndicatorColor");
    if attr_color.is_some() || sub.empty || child_color.is_none() {
        // Engine spelling (attribute colour, or no colour at all) →
        // canonical, carrying the colour the source had.
        let color = attr_color
            .or(child_color)
            .or_else(|| fallback_color.map(String::from));
        return write_condition(writer, c, color.as_deref());
    }
    let visible = c.visible.unwrap_or(true);
    let start = patch_start(
        &sub.start,
        |k, raw| {
            let raw = std::str::from_utf8(raw).ok();
            match k {
                b"Visible" => Some(bool_patch(raw, visible)),
                b"IndicatorMethod" => Some(opt_str_patch(raw, c.indicator_method.as_deref())),
                _ => None,
            }
        },
        &[("Visible", visible.to_string())],
    )?;
    let start = unchanged_or(&sub.start, start);
    Subtree { start, ..sub }.replay(writer)
}

fn emit_condition_set(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sub: Subtree,
    s: &ConditionSetDef,
) -> Result<(), quick_xml::Error> {
    if attr_value(&sub.start, b"Conditions").is_some()
        || sub.empty
        || !sub.has_child(b"SetConditions")
    {
        return write_condition_set(writer, s);
    }
    // Canonical form: the members are the model's truth; re-serialise
    // only when they differ.
    let mut source_members: Vec<String> = Vec::new();
    for ev in &sub.children {
        if let Event::Empty(e) | Event::Start(e) = ev {
            if e.name().as_ref() == b"VisibilityPair" {
                if let Some(m) = attr_value(e, b"Condition") {
                    source_members.push(m);
                }
            }
        }
    }
    if source_members == s.conditions {
        return sub.replay(writer);
    }
    write_condition_set(writer, s)
}

/// Patch `designmap.xml` for sections, the hyperlink block and the
/// conditions (see the module doc). Byte-identical when nothing differs
/// and the source is already in InDesign's spelling.
pub(crate) fn patch_designmap_navigation(
    original: &[u8],
    plan: &NavigationPlan<'_>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let layout = scan(original)?;

    // Destination keys: existing ones are kept, missing ones allocated
    // above the largest in use (stable across saves: same source, same
    // model, same allocation order).
    let mut next_key = layout.max_key + 1;
    let mut key_of: HashMap<String, u64> = layout.dest_keys.clone();
    for d in &plan.destinations {
        if !key_of.contains_key(&d.self_id) {
            key_of.insert(d.self_id.clone(), next_key);
            next_key += 1;
        }
    }
    let hyperlink_key = |h: &Hyperlink| -> Option<u64> {
        h.destination
            .as_deref()
            .and_then(|d| key_of.get(d).copied())
    };
    let hyperlinks_by_id: HashMap<&str, &Hyperlink> = plan
        .hyperlinks
        .iter()
        .map(|h| (h.self_id.as_str(), h))
        .collect();
    let destinations_by_id: HashMap<&str, &HyperlinkDestination> = plan
        .destinations
        .iter()
        .map(|d| (d.self_id.as_str(), *d))
        .collect();
    let bookmarks_by_id: HashMap<&str, &Bookmark> = plan
        .bookmarks
        .iter()
        .map(|b| (b.self_id.as_str(), b))
        .collect();
    let sections_by_id: HashMap<&str, &SectionSpec> = plan
        .sections
        .iter()
        .map(|s| (s.self_id.as_str(), s))
        .collect();

    // The hyperlink block MOVES when any of it sits before the last
    // story include; a block already after the stories stays put.
    let move_block = layout.nav_after_stories.iter().any(|after| !after);
    let nav_total = layout.nav_after_stories.len();

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    let mut depth = 0usize;
    let mut stories_seen = 0usize;
    let mut spreads_seen = 0usize;
    let mut sections_seen = 0usize;
    let mut nav_seen = 0usize;
    let mut conditions_seen = 0usize;
    let mut top_layers_seen = 0usize;
    let mut xref_format_done = false;
    let mut new_sections_done = layout.sections == 0 && layout.spread_refs == 0;
    let mut new_nav_done = false;
    let mut new_conditions_done = false;
    // The buffered element being captured (a nav element, a section, a
    // condition), plus the depth it opened at.
    let mut capture: Option<(Subtree, usize)> = None;
    // Elements moved out of place, rendered, awaiting the nav slot.
    let mut moved: Writer<Cursor<Vec<u8>>> = Writer::new(Cursor::new(Vec::new()));
    let mut moved_any = false;

    // Render one captured subtree to `w` (in place or into the moved buffer).
    let render = |w: &mut Writer<Cursor<Vec<u8>>>, sub: Subtree| -> Result<(), quick_xml::Error> {
        let id = attr_value(&sub.start, b"Self");
        match sub.name.as_slice() {
            b"Section" => match id.as_deref().and_then(|i| sections_by_id.get(i)) {
                Some(s) => emit_section(w, sub, s),
                None if id.is_some() => Ok(()), // deleted
                None => sub.replay(w),
            },
            b"Hyperlink" => match id.as_deref().and_then(|i| hyperlinks_by_id.get(i)) {
                Some(h) => emit_hyperlink(w, sub, h, hyperlink_key(h)),
                None if id.is_some() => Ok(()),
                None => sub.replay(w),
            },
            b"HyperlinkURLDestination" | b"HyperlinkPageDestination" => {
                match id.as_deref().and_then(|i| destinations_by_id.get(i)) {
                    Some(d) => {
                        let key = key_of.get(&d.self_id).copied().unwrap_or(0);
                        emit_destination(w, sub, d, key)
                    }
                    None if id.is_some() => Ok(()),
                    None => sub.replay(w),
                }
            }
            // Never a designmap element: it lives inline in its story.
            b"HyperlinkTextDestination" => Ok(()),
            b"Bookmark" => match id.as_deref().and_then(|i| bookmarks_by_id.get(i)) {
                Some(_) => sub.replay(w),
                None if id.is_some() => Ok(()),
                None => sub.replay(w),
            },
            b"Condition" => match id.as_deref().and_then(|i| plan.conditions.get(i)) {
                Some(c) => {
                    let fallback = id.as_deref().and_then(|i| plan.indicator_colors.get(i));
                    emit_condition(w, sub, c, fallback.map(|s| s.as_str()))
                }
                None if id.is_some() => Ok(()),
                None => sub.replay(w),
            },
            b"ConditionSet" => match id.as_deref().and_then(|i| plan.condition_sets.get(i)) {
                Some(s) => emit_condition_set(w, sub, s),
                None if id.is_some() => Ok(()),
                None => sub.replay(w),
            },
            _ => sub.replay(w),
        }
    };

    let write_new_sections = |w: &mut Writer<Cursor<Vec<u8>>>| -> Result<(), quick_xml::Error> {
        for s in &plan.sections {
            if !layout.section_ids.contains(&s.self_id) {
                write_section(w, s)?;
            }
        }
        Ok(())
    };
    let write_new_nav = |w: &mut Writer<Cursor<Vec<u8>>>| -> Result<(), quick_xml::Error> {
        // Each `Self` once. The annual's model carried `Hyperlink/ueef094`
        // TWICE (a paged.doc lowering minted one id for two links) and the
        // designmap then carried it twice; InDesign dropped both.
        let mut written: HashSet<&str> = HashSet::new();
        for d in &plan.destinations {
            if !layout.destination_ids.contains(&d.self_id) && written.insert(&d.self_id) {
                write_destination(w, d, key_of.get(&d.self_id).copied().unwrap_or(0))?;
            }
        }
        for h in plan.hyperlinks {
            if !layout.hyperlink_ids.contains(&h.self_id) && written.insert(&h.self_id) {
                write_hyperlink(w, h, hyperlink_key(h))?;
            }
        }
        for b in plan.bookmarks {
            if !layout.bookmark_ids.contains(&b.self_id) && written.insert(&b.self_id) {
                write_bookmark(w, b)?;
            }
        }
        Ok(())
    };
    let write_new_conditions = |w: &mut Writer<Cursor<Vec<u8>>>| -> Result<(), quick_xml::Error> {
        for (id, c) in plan.conditions {
            if !layout.condition_ids.contains(id) {
                write_condition(w, c, plan.indicator_colors.get(id).map(|s| s.as_str()))?;
            }
        }
        for (id, s) in plan.condition_sets {
            if !layout.condition_ids.contains(id) {
                write_condition_set(w, s)?;
            }
        }
        let any_conditions = !plan.conditions.is_empty() || !plan.condition_sets.is_empty();
        if !layout.has_conditional_text_preference && any_conditions {
            write_conditional_text_preference(w)?;
        }
        Ok(())
    };
    let any_new_conditions = plan
        .conditions
        .keys()
        .chain(plan.condition_sets.keys())
        .any(|id| !layout.condition_ids.contains(id));
    // Where new conditions go when the designmap carries none yet.
    let condition_slot_after_named_grid = layout.conditions == 0 && layout.has_named_grid;
    let condition_slot_after_layers =
        layout.conditions == 0 && !layout.has_named_grid && layout.top_layers > 0;
    let condition_slot_before_preferences = layout.conditions == 0
        && !layout.has_named_grid
        && layout.top_layers == 0
        && layout.has_preferences_ref;
    let condition_slot_before_spreads = layout.conditions == 0
        && !layout.has_named_grid
        && layout.top_layers == 0
        && !layout.has_preferences_ref
        && layout.has_master_or_spread_ref;

    // After the LAST source element of a kind has been rendered, the
    // model's NEW elements of that kind follow it.
    macro_rules! after_element {
        ($name:expr) => {{
            let name: &[u8] = $name;
            if name == b"Section" && sections_seen == layout.sections && !new_sections_done {
                write_new_sections(&mut writer)?;
                new_sections_done = true;
            }
            if is_nav_name(name) && !move_block && nav_seen == nav_total && !new_nav_done {
                write_new_nav(&mut writer)?;
                new_nav_done = true;
            }
            if is_condition_name(name)
                && conditions_seen == layout.conditions
                && !new_conditions_done
            {
                write_new_conditions(&mut writer)?;
                new_conditions_done = true;
            }
        }};
    }
    macro_rules! render_captured {
        ($sub:expr) => {{
            let sub: Subtree = $sub;
            let name = sub.name.clone();
            if move_block && is_nav_name(&name) {
                moved_any = true;
                render(&mut moved, sub)?;
            } else {
                render(&mut writer, sub)?;
            }
            after_element!(&name);
        }};
    }
    fn count_kind(
        name: &[u8],
        sections_seen: &mut usize,
        nav_seen: &mut usize,
        conditions_seen: &mut usize,
    ) {
        if name == b"Section" {
            *sections_seen += 1;
        } else if is_nav_name(name) {
            *nav_seen += 1;
        } else {
            *conditions_seen += 1;
        }
    }
    fn is_captured_name(name: &[u8]) -> bool {
        name == b"Section" || is_nav_name(name) || is_condition_name(name)
    }

    loop {
        let ev = reader.read_event_into(&mut buf)?;
        // ---- capturing a subtree: everything until its End buffers ----
        if let Some((sub, open_depth)) = capture.as_mut() {
            match ev {
                Event::Start(e) => {
                    depth += 1;
                    sub.children.push(Event::Start(e.into_owned()));
                }
                Event::End(e) => {
                    if depth == *open_depth {
                        depth -= 1;
                        let (sub, _) = capture.take().expect("capturing");
                        render_captured!(sub);
                    } else {
                        depth -= 1;
                        sub.children.push(Event::End(e.into_owned()));
                    }
                }
                Event::Eof => break,
                other => sub.children.push(other.into_owned()),
            }
            buf.clear();
            continue;
        }
        match ev {
            Event::Eof => break,
            Event::Start(e) if depth == 1 && is_captured_name(e.name().as_ref()) => {
                let name = e.name().as_ref().to_vec();
                count_kind(
                    &name,
                    &mut sections_seen,
                    &mut nav_seen,
                    &mut conditions_seen,
                );
                depth += 1;
                capture = Some((
                    Subtree {
                        name,
                        start: e.into_owned(),
                        empty: false,
                        children: Vec::new(),
                    },
                    depth,
                ));
            }
            Event::Empty(e) if depth == 1 && is_captured_name(e.name().as_ref()) => {
                let name = e.name().as_ref().to_vec();
                count_kind(
                    &name,
                    &mut sections_seen,
                    &mut nav_seen,
                    &mut conditions_seen,
                );
                let sub = Subtree {
                    name,
                    start: e.into_owned(),
                    empty: true,
                    children: Vec::new(),
                };
                render_captured!(sub);
            }
            Event::Start(e) => {
                depth += 1;
                let name = e.name().as_ref().to_vec();
                if depth == 2 && name == b"Layer" {
                    top_layers_seen += 1;
                }
                writer.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                let at_depth = depth + 1;
                if (name == b"idPkg:BackingStory" || name == b"idPkg:Story") && !xref_format_done {
                    if let Some(id) = plan.xref_format.as_deref() {
                        if !layout.xref_format_ids.contains(id) {
                            write_xref_format(&mut writer, id)?;
                        }
                    }
                    xref_format_done = true;
                }
                if name == b"idPkg:Preferences"
                    && condition_slot_before_preferences
                    && !new_conditions_done
                    && any_new_conditions
                {
                    write_new_conditions(&mut writer)?;
                    new_conditions_done = true;
                }
                if (name == b"idPkg:MasterSpread" || name == b"idPkg:Spread")
                    && condition_slot_before_spreads
                    && !new_conditions_done
                    && any_new_conditions
                {
                    write_new_conditions(&mut writer)?;
                    new_conditions_done = true;
                }
                writer.write_event(Event::Empty(e.into_owned()))?;
                match name.as_slice() {
                    b"idPkg:Spread" => {
                        spreads_seen += 1;
                        if layout.sections == 0
                            && spreads_seen == layout.spread_refs
                            && !new_sections_done
                        {
                            write_new_sections(&mut writer)?;
                            new_sections_done = true;
                        }
                    }
                    b"idPkg:Story" => {
                        stories_seen += 1;
                        if stories_seen == layout.story_refs {
                            // THE nav slot: the moved block, then the new
                            // elements (when nothing existed to follow).
                            if moved_any {
                                let bytes = std::mem::replace(
                                    &mut moved,
                                    Writer::new(Cursor::new(Vec::new())),
                                )
                                .into_inner()
                                .into_inner();
                                std::io::Write::write_all(writer.get_mut(), &bytes)?;
                                moved_any = false;
                            }
                            if (move_block || nav_total == 0) && !new_nav_done {
                                write_new_nav(&mut writer)?;
                                new_nav_done = true;
                            }
                        }
                    }
                    b"NamedGrid" if at_depth == 2 => {
                        if condition_slot_after_named_grid
                            && !new_conditions_done
                            && any_new_conditions
                        {
                            write_new_conditions(&mut writer)?;
                            new_conditions_done = true;
                        }
                    }
                    b"Layer" if at_depth == 2 => {
                        top_layers_seen += 1;
                        if condition_slot_after_layers
                            && top_layers_seen == layout.top_layers
                            && !new_conditions_done
                            && any_new_conditions
                        {
                            write_new_conditions(&mut writer)?;
                            new_conditions_done = true;
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = e.name().as_ref().to_vec();
                if name == b"Document" {
                    // Flush everything still unplaced before the root
                    // closes — a resource is never silently dropped.
                    if !xref_format_done {
                        if let Some(id) = plan.xref_format.as_deref() {
                            if !layout.xref_format_ids.contains(id) {
                                write_xref_format(&mut writer, id)?;
                            }
                        }
                        xref_format_done = true;
                    }
                    if !new_sections_done {
                        write_new_sections(&mut writer)?;
                        new_sections_done = true;
                    }
                    if moved_any {
                        let bytes =
                            std::mem::replace(&mut moved, Writer::new(Cursor::new(Vec::new())))
                                .into_inner()
                                .into_inner();
                        std::io::Write::write_all(writer.get_mut(), &bytes)?;
                        moved_any = false;
                    }
                    if !new_nav_done {
                        write_new_nav(&mut writer)?;
                        new_nav_done = true;
                    }
                    if !new_conditions_done && any_new_conditions {
                        write_new_conditions(&mut writer)?;
                        new_conditions_done = true;
                    }
                }
                writer.write_event(Event::End(e))?;
                if depth == 2 {
                    if name == b"NamedGrid"
                        && condition_slot_after_named_grid
                        && !new_conditions_done
                        && any_new_conditions
                    {
                        write_new_conditions(&mut writer)?;
                        new_conditions_done = true;
                    }
                    if name == b"Layer"
                        && condition_slot_after_layers
                        && top_layers_seen == layout.top_layers
                        && !new_conditions_done
                        && any_new_conditions
                    {
                        write_new_conditions(&mut writer)?;
                        new_conditions_done = true;
                    }
                }
                depth = depth.saturating_sub(1);
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}
