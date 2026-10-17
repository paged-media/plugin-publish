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

//! `designmap.xml` — the root manifest that lists referenced spreads,
//! stories, masters, preferences, and so on.
//!
//! Only a tiny subset of attributes is extracted here — enough to drive
//! seed-corpus round-trips. Full schema coverage lands during Phase 0.

use quick_xml::events::Event;

use crate::util::attr;
use crate::ParseError;

pub use paged_model::DesignMap;

pub use paged_model::{
    Article, Bookmark, ColorSettings, CrossReference, CrossReferenceBlock, CrossReferenceFormat,
    DocumentPreference, FootnoteOptions, GridPreference, Hyperlink, HyperlinkDestination,
    HyperlinkDestinationKind, IndexCrossReference, IndexOptions, IndexTopic, Layer, NumberingStyle,
    Section, SpreadRef, StoryRef, TextVariable,
};

/// Parse a `designmap.xml` byte slice.
pub fn parse_designmap(xml: &[u8]) -> Result<DesignMap, ParseError> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(false);

    let mut out = DesignMap::default();
    let mut buf = Vec::new();
    // Stack of currently-open `<Layer Self=...>` ids, so a nested
    // `<Layer>` (layer group / folder) records its parent. Only
    // `Event::Start` opens a scope; a self-closing `<Layer/>` (the
    // flat common case) records `parent_id` from the stack top but
    // doesn't push, keeping flat documents byte-identical.
    let mut layer_stack: Vec<String> = Vec::new();
    // Open `<Topic>` elements: InDesign nests a subtopic inside its parent
    // (core ADR 168), and a topic's `<CrossReference>`s inside it.
    let mut topic_stack: Vec<String> = Vec::new();
    // Open `<Bookmark>` elements: a nested bookmark's parent (core ADR 171).
    let mut bookmark_stack: Vec<String> = Vec::new();
    // The open `<CrossReferenceFormat>` (index into
    // `out.cross_reference_formats`), whose `<BuildingBlock>`s fold in.
    let mut current_xref_format: Option<usize> = None;
    // Inside `<EndnoteOption>`: its typed children fold in (core ADR 170).
    let mut in_endnote_option = false;
    // W1.4 — the `<TextVariable>` currently being parsed (the
    // wrapping form parks here so its `<TextVariablePreference>`
    // child can fold in before `</TextVariable>` pushes it).
    let mut current_text_variable: Option<TextVariable> = None;
    // A custom text variable's `<Contents>` while it is open: its text
    // arrives in pieces (an entity is its own event) and is verbatim —
    // leading and trailing spaces are part of the value.
    let mut custom_contents: Option<String> = None;
    // The `<Section>` / `<Hyperlink>` currently open in ELEMENT form
    // (index into `out.sections` / `out.hyperlinks`), so a typed
    // `<Properties>` child can fold into it: InDesign writes a section's
    // `PageNumberStyle` and a hyperlink's `Destination` as typed
    // Properties children, never as attributes (measured on InDesign
    // 20.0.1 — the attribute form of `Destination` is ignored, and
    // together with `DestinationUniqueKey` makes the file unopenable).
    // The attribute forms stay readable for the engine's own older
    // fixtures.
    let mut current_section: Option<usize> = None;
    let mut current_hyperlink: Option<usize> = None;
    // The typed Properties child whose text is about to arrive.
    let mut pending: Option<PendingProperty> = None;
    // `DestinationUniqueKey` → destination `Self`, for hyperlinks that
    // name their destination ONLY by key (no attribute, no child).
    let mut dest_keys: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // Index-aligned with `out.hyperlinks`: the key each hyperlink
    // carried, resolved after the whole file is read.
    let mut hyperlink_keys: Vec<Option<String>> = Vec::new();

    // Open element names, root first — only to recognise the
    // DOCUMENT's own `<Properties><Label>` (an element nested anywhere
    // else in the designmap labels that element, not the document).
    let mut open_path: Vec<Vec<u8>> = Vec::new();

    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match &ev {
            Event::Start(e) => open_path.push(e.name().as_ref().to_vec()),
            Event::End(_) => {
                open_path.pop();
            }
            Event::Empty(e)
                if e.name().as_ref() == b"KeyValuePair"
                    && open_path.len() == 3
                    && open_path[0] == b"Document"
                    && open_path[1] == b"Properties"
                    && open_path[2] == b"Label" =>
            {
                let key = crate::util::attr_unescaped(e, b"Key");
                let value = crate::util::label_value(e);
                if let (Some(key), Some(value)) = (key, value) {
                    // One entry per key, last write wins — the
                    // page-item label rule.
                    match out.labels.iter_mut().find(|(k, _)| *k == key) {
                        Some(slot) => slot.1 = value,
                        None => out.labels.push((key, value)),
                    }
                }
            }
            _ => {}
        }
        if let Event::End(ref e) = ev {
            match e.name().as_ref() {
                b"Layer" => {
                    layer_stack.pop();
                }
                b"TextVariable" => {
                    if let Some(var) = current_text_variable.take() {
                        out.text_variables.push(var);
                    }
                }
                b"Contents" => {
                    if let (Some(text), Some(var)) =
                        (custom_contents.take(), current_text_variable.as_mut())
                    {
                        var.contents = Some(text);
                    }
                }
                b"Section" => current_section = None,
                b"Hyperlink" => current_hyperlink = None,
                b"Topic" => {
                    topic_stack.pop();
                }
                b"Bookmark" => {
                    bookmark_stack.pop();
                }
                b"CrossReferenceFormat" => current_xref_format = None,
                b"PageNumberStyle" | b"Destination" => pending = None,
                b"EndnoteOption" => in_endnote_option = false,
                b"EndnoteNumberingStyle"
                | b"RestartEndnoteNumbering"
                | b"EndnoteMarkerPositioning"
                | b"ScopeValue"
                | b"FrameCreateOption"
                | b"ShowEndnotePrefixSuffix" => pending = None,
                _ => {}
            }
        }
        let is_start = matches!(ev, Event::Start(_));
        match ev {
            Event::Start(e) | Event::Empty(e) => {
                if e.name().as_ref() == b"Document" {
                    out.dom_version = attr(&e, b"DOMVersion");
                    out.document_name = attr(&e, b"Name");
                    out.color_settings = ColorSettings {
                        cmyk_profile: attr(&e, b"CMYKProfile"),
                        rgb_profile: attr(&e, b"RGBProfile"),
                        solid_color_intent: attr(&e, b"SolidColorIntent"),
                        after_blending_intent: attr(&e, b"AfterBlendingIntent"),
                        default_image_intent: attr(&e, b"DefaultImageIntent"),
                    };
                }
                read_preference(&e, &mut out);
                if e.name().as_ref() == b"Layer" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        out.layers.push(Layer {
                            self_id: self_id.clone(),
                            name: attr(&e, b"Name"),
                            visible: attr(&e, b"Visible")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(true),
                            locked: attr(&e, b"Locked")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(false),
                            printable: attr(&e, b"Printable")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(true),
                            parent_id: layer_stack.last().cloned(),
                        });
                        // A non-self-closing <Layer> opens a group
                        // scope; its descendant layers inherit it.
                        if is_start {
                            layer_stack.push(self_id);
                        }
                    }
                }
                if e.name().as_ref() == b"TextVariable" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        let var = TextVariable {
                            self_id,
                            name: attr(&e, b"Name"),
                            variable_type: attr(&e, b"VariableType"),
                            ..TextVariable::default()
                        };
                        // A self-closing `<TextVariable/>` carries no
                        // preference child; push it straight away.
                        // The wrapping form parks it until `</TextVariable>`
                        // so the `<TextVariablePreference>` child folds in.
                        if is_start {
                            current_text_variable = Some(var);
                        } else {
                            out.text_variables.push(var);
                        }
                    }
                }
                // The type-specific payload of the enclosing
                // `<TextVariable>`. InDesign writes a TYPED child per
                // variable type (measured, InDesign 20.0.1 — thoughts ADR
                // 033):
                //
                //   MatchParagraphStylePreference  AppliedParagraphStyle
                //   MatchCharacterStylePreference  AppliedCharacterStyle
                //       + SearchStrategy, ChangeCase, DeleteEndPunctuation
                //   PageNumberVariablePreference   Format, Scope
                //   ChapterNumberVariablePreference Format
                //   FileNameVariablePreference     IncludePath, IncludeExtension
                //   DateVariablePreference         Format
                //   CustomTextVariablePreference   <Properties><Contents>
                //
                // all with TextBefore / TextAfter. The engine's older
                // fixtures wrote a generic `<TextVariablePreference>`
                // (`Contents`, `Format`, `AppliedParagraphStyle`, `Use`),
                // a spelling InDesign ignores; it stays readable.
                if let Some(var) = current_text_variable.as_mut() {
                    match e.name().as_ref() {
                        b"TextVariablePreference" => {
                            var.contents = attr(&e, b"Contents").or(var.contents.take());
                            var.date_format = attr(&e, b"Format").or(var.date_format.take());
                            var.running_header_style = attr(&e, b"AppliedParagraphStyle")
                                .or(var.running_header_style.take());
                            var.running_header_character_style = attr(&e, b"AppliedCharacterStyle")
                                .or(var.running_header_character_style.take());
                            var.running_header_use =
                                attr(&e, b"Use").or(var.running_header_use.take());
                            read_decoration(var, &e);
                        }
                        b"MatchParagraphStylePreference" | b"MatchCharacterStylePreference" => {
                            if e.name().as_ref() == b"MatchParagraphStylePreference" {
                                var.running_header_style = attr(&e, b"AppliedParagraphStyle");
                            } else {
                                var.running_header_character_style =
                                    attr(&e, b"AppliedCharacterStyle");
                            }
                            var.running_header_use = attr(&e, b"SearchStrategy");
                            var.change_case = attr(&e, b"ChangeCase");
                            var.delete_end_punctuation =
                                attr(&e, b"DeleteEndPunctuation").as_deref() == Some("true");
                            read_decoration(var, &e);
                        }
                        b"PageNumberVariablePreference" => {
                            var.number_format = attr(&e, b"Format");
                            var.page_number_scope = attr(&e, b"Scope");
                            read_decoration(var, &e);
                        }
                        b"ChapterNumberVariablePreference" => {
                            var.number_format = attr(&e, b"Format");
                            read_decoration(var, &e);
                        }
                        b"FileNameVariablePreference" => {
                            var.include_path = attr(&e, b"IncludePath").as_deref() == Some("true");
                            var.include_extension =
                                attr(&e, b"IncludeExtension").as_deref() == Some("true");
                            read_decoration(var, &e);
                        }
                        b"DateVariablePreference" => {
                            var.date_format = attr(&e, b"Format");
                            read_decoration(var, &e);
                        }
                        b"CustomTextVariablePreference" => read_decoration(var, &e),
                        b"Contents" if is_start => custom_contents = Some(String::new()),
                        _ => {}
                    }
                }
                // W1.4 — hyperlink destination resources.
                if matches!(
                    e.name().as_ref(),
                    b"HyperlinkURLDestination"
                        | b"HyperlinkPageDestination"
                        | b"HyperlinkTextDestination"
                ) {
                    if let (Some(self_id), Some(key)) =
                        (attr(&e, b"Self"), attr(&e, b"DestinationUniqueKey"))
                    {
                        dest_keys.insert(key, self_id);
                    }
                }
                if e.name().as_ref() == b"HyperlinkURLDestination" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        let url = attr(&e, b"DestinationURL").unwrap_or_default();
                        out.hyperlink_destinations.push(HyperlinkDestination {
                            self_id,
                            kind: HyperlinkDestinationKind::Url(url),
                        });
                    }
                }
                if e.name().as_ref() == b"HyperlinkPageDestination" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        if let Some(page) = attr(&e, b"DestinationPage") {
                            out.hyperlink_destinations.push(HyperlinkDestination {
                                self_id,
                                kind: HyperlinkDestinationKind::Page(page),
                            });
                        }
                    }
                }
                if e.name().as_ref() == b"HyperlinkTextDestination" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        if let Some(text) = attr(&e, b"DestinationText") {
                            out.hyperlink_destinations.push(HyperlinkDestination {
                                self_id,
                                kind: HyperlinkDestinationKind::TextAnchor(text),
                            });
                        }
                    }
                }
                if e.name().as_ref() == b"Section" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        out.sections.push(Section {
                            self_id,
                            page_start: attr(&e, b"PageStart"),
                            continue_numbering: attr(&e, b"ContinueNumbering")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(false),
                            start_at: attr(&e, b"PageNumberStart").and_then(|s| s.parse().ok()),
                            numbering_style: attr(&e, b"PageNumberStyle")
                                .map(|s| NumberingStyle::from_idml(&s))
                                .unwrap_or(NumberingStyle::Arabic),
                            section_prefix: attr(&e, b"SectionPrefix"),
                            marker: attr(&e, b"Marker"),
                            include_prefix: attr(&e, b"IncludeSectionPrefix")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(false),
                        });
                        if is_start {
                            current_section = Some(out.sections.len() - 1);
                        }
                    }
                }
                // InDesign's spelling of a section's numbering style:
                // `<Properties><PageNumberStyle type="enumeration">LowerRoman
                // </PageNumberStyle></Properties>` (a union type — it may
                // also name a custom numbering list, hence a child rather
                // than an attribute). The attribute form above stays
                // readable for the engine's older fixtures.
                if e.name().as_ref() == b"PageNumberStyle" && is_start && current_section.is_some()
                {
                    pending = Some(PendingProperty::SectionPageNumberStyle);
                }
                // `<EndnoteOption>` (core ADR 170): attributes here, the
                // enumerations as typed `<Properties>` children below.
                if e.name().as_ref() == b"EndnoteOption" {
                    let raw = |name: &[u8]| crate::util::attr_unescaped(&e, name);
                    out.endnote_options = Some(paged_model::EndnoteOptions {
                        title: raw(b"EndnoteTitle"),
                        title_style: attr(&e, b"EndnoteTitleStyle"),
                        start_at: attr(&e, b"StartEndnoteNumberAt").and_then(|s| s.parse().ok()),
                        marker_style: attr(&e, b"EndnoteMarkerStyle"),
                        text_style: attr(&e, b"EndnoteTextStyle"),
                        separator_text: raw(b"EndnoteSeparatorText"),
                        prefix: raw(b"EndnotePrefix"),
                        suffix: raw(b"EndnoteSuffix"),
                        ..Default::default()
                    });
                    in_endnote_option = is_start;
                }
                if in_endnote_option && is_start {
                    let field = match e.name().as_ref() {
                        b"EndnoteNumberingStyle" => Some("numbering_style"),
                        b"RestartEndnoteNumbering" => Some("restart"),
                        b"EndnoteMarkerPositioning" => Some("marker_positioning"),
                        b"ScopeValue" => Some("scope"),
                        b"FrameCreateOption" => Some("frame_create"),
                        b"ShowEndnotePrefixSuffix" => Some("show_prefix_suffix"),
                        _ => None,
                    };
                    if let Some(field) = field {
                        pending = Some(PendingProperty::Endnote(field));
                    }
                }
                if e.name().as_ref() == b"Destination" && is_start && current_hyperlink.is_some() {
                    pending = Some(PendingProperty::HyperlinkDestination);
                }
                if e.name().as_ref() == b"Article" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        // `MemberItemRefs` is the typical attribute on
                        // a self-closing Article; nested
                        // <ArticleMember> children are flattened to
                        // their `ItemRef` attribute by a future polish.
                        let members = attr(&e, b"MemberItemRefs")
                            .map(|s| {
                                s.split_whitespace()
                                    .map(|t| t.to_string())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        out.articles.push(Article {
                            self_id,
                            name: attr(&e, b"Name"),
                            members,
                        });
                    }
                }
                if e.name().as_ref() == b"Hyperlink" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        // The destination REF, in precedence order: the
                        // `Destination` attribute (engine fixtures), the
                        // `<Properties><Destination type="object">` child
                        // (InDesign; folded in below), and last the
                        // `DestinationUniqueKey` resolved through the
                        // destinations' own keys once the file is read.
                        // Storing the bare key here — as this once did —
                        // left every InDesign-authored link dangling: no
                        // destination is `Self`-named by its key.
                        out.hyperlinks.push(Hyperlink {
                            self_id,
                            name: attr(&e, b"Name"),
                            source: attr(&e, b"Source"),
                            destination: attr(&e, b"Destination"),
                        });
                        hyperlink_keys.push(attr(&e, b"DestinationUniqueKey"));
                        if is_start {
                            current_hyperlink = Some(out.hyperlinks.len() - 1);
                        }
                    }
                }
                if e.name().as_ref() == b"Bookmark" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        // InDesign nests a bookmark inside its parent's
                        // element (core ADR 171).
                        if let Some(parent) = bookmark_stack.last() {
                            out.bookmark_parents.insert(self_id.clone(), parent.clone());
                        }
                        if is_start {
                            bookmark_stack.push(self_id.clone());
                        }
                        out.bookmarks.push(Bookmark {
                            self_id,
                            name: attr(&e, b"Name"),
                            destination: attr(&e, b"Destination"),
                        });
                    }
                }
                if e.name().as_ref() == b"CrossReferenceSource" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        out.cross_references.push(CrossReference {
                            self_id,
                            name: attr(&e, b"Name"),
                            format: attr(&e, b"AppliedFormat"),
                            destination: attr(&e, b"Destination"),
                        });
                    }
                }
                if e.name().as_ref() == b"Topic" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        if let Some(parent) = topic_stack.last() {
                            out.index_topic_parents
                                .insert(self_id.clone(), parent.clone());
                        }
                        if is_start {
                            topic_stack.push(self_id.clone());
                        }
                        out.index_topics.push(IndexTopic {
                            self_id,
                            name: attr(&e, b"Name"),
                            sort_order: attr(&e, b"SortOrder"),
                        });
                    }
                }
                // What a cross-reference says, as building blocks (core
                // ADR 169). The custom text is verbatim: its spaces are
                // part of it ("“ on page ").
                if e.name().as_ref() == b"CrossReferenceFormat" {
                    if let Some(self_id) = attr(&e, b"Self") {
                        out.cross_reference_formats.push(CrossReferenceFormat {
                            self_id,
                            name: crate::util::attr_unescaped(&e, b"Name"),
                            applied_character_style: attr(&e, b"AppliedCharacterStyle"),
                            blocks: Vec::new(),
                        });
                        if is_start {
                            current_xref_format = Some(out.cross_reference_formats.len() - 1);
                        }
                    }
                }
                if e.name().as_ref() == b"BuildingBlock" {
                    if let Some(format) =
                        current_xref_format.and_then(|i| out.cross_reference_formats.get_mut(i))
                    {
                        format.blocks.push(CrossReferenceBlock {
                            block_type: attr(&e, b"BlockType").unwrap_or_default(),
                            custom_text: crate::util::attr_unescaped(&e, b"CustomText"),
                            applied_character_style: attr(&e, b"AppliedCharacterStyle"),
                            applied_delimiter: crate::util::attr_unescaped(&e, b"AppliedDelimiter"),
                            include_delimiter: attr(&e, b"IncludeDelimiter")
                                .and_then(|s| s.parse().ok()),
                        });
                    }
                }
                // A topic's See / See also reference (not a text
                // `<CrossReferenceSource>`, nor a `<CrossReferenceFormat>`).
                if e.name().as_ref() == b"CrossReference" {
                    if let (Some(self_id), Some(topic)) = (attr(&e, b"Self"), topic_stack.last()) {
                        out.index_cross_references.push(IndexCrossReference {
                            self_id,
                            topic: topic.clone(),
                            referenced_topic: attr(&e, b"ReferencedTopic"),
                            kind: attr(&e, b"CrossReferenceType"),
                            custom_type_string: attr(&e, b"CustomTypeString")
                                .filter(|s| !s.is_empty()),
                        });
                    }
                }
                let src = attr(&e, b"src");
                match e.name().as_ref() {
                    b"idPkg:Spread" => {
                        if let Some(src) = src {
                            out.spreads.push(SpreadRef { src });
                        }
                    }
                    b"idPkg:Story" => {
                        if let Some(src) = src {
                            out.stories.push(StoryRef { src });
                        }
                    }
                    b"idPkg:MasterSpread" => {
                        if let Some(src) = src {
                            out.master_spreads.push(src);
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref t) if custom_contents.is_some() => {
                if let Some(text) = custom_contents.as_mut() {
                    text.push_str(
                        &t.xml_content(quick_xml::XmlVersion::Implicit1_0)
                            .unwrap_or_default(),
                    );
                }
            }
            Event::GeneralRef(ref r) if custom_contents.is_some() => {
                let name = String::from_utf8_lossy(r.as_ref());
                let resolved = quick_xml::escape::unescape(&format!("&{name};"))
                    .map(|c| c.into_owned())
                    .unwrap_or_default();
                if let Some(text) = custom_contents.as_mut() {
                    text.push_str(&resolved);
                }
            }
            Event::Text(t) => {
                if let Some(which) = pending.take() {
                    let text = t
                        .xml_content(quick_xml::XmlVersion::Implicit1_0)
                        .map(|c| c.into_owned())
                        .unwrap_or_default();
                    let text = text.trim();
                    if text.is_empty() {
                        // Indentation before the real text — keep waiting.
                        pending = Some(which);
                    } else {
                        match which {
                            PendingProperty::SectionPageNumberStyle => {
                                if let Some(sec) =
                                    current_section.and_then(|i| out.sections.get_mut(i))
                                {
                                    sec.numbering_style = NumberingStyle::from_idml(text);
                                }
                            }
                            PendingProperty::Endnote(field) => {
                                if let Some(o) = out.endnote_options.as_mut() {
                                    let v = Some(text.to_string());
                                    match field {
                                        "numbering_style" => o.numbering_style = v,
                                        "restart" => o.restart = v,
                                        "marker_positioning" => o.marker_positioning = v,
                                        "scope" => o.scope = v,
                                        "frame_create" => o.frame_create = v,
                                        _ => o.show_prefix_suffix = v,
                                    }
                                }
                            }
                            PendingProperty::HyperlinkDestination => {
                                if let Some(h) =
                                    current_hyperlink.and_then(|i| out.hyperlinks.get_mut(i))
                                {
                                    if h.destination.is_none() {
                                        h.destination = Some(text.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    // Hyperlinks that named their destination only by
    // `DestinationUniqueKey` resolve through the destinations' keys.
    for (h, key) in out.hyperlinks.iter_mut().zip(hyperlink_keys) {
        if h.destination.is_none() {
            if let Some(id) = key.and_then(|k| dest_keys.get(&k)) {
                h.destination = Some(id.clone());
            }
        }
    }
    read_footnote_option_properties(xml, &mut out.footnote_options)?;
    Ok(out)
}

/// The document-level preferences InDesign keeps in
/// `Resources/Preferences.xml` (measured in 271 of the 305 census
/// documents) and the engine's own older fixtures wrote into
/// `designmap.xml`. One reader serves both parts.
fn read_preference(e: &quick_xml::events::BytesStart, out: &mut DesignMap) {
    // `<IndexOptions>`: how Generate Index writes the index (core ADR
    // 166). Its separators are verbatim: leading and trailing spaces are
    // the value.
    if e.name().as_ref() == b"IndexOptions" {
        let b = |name: &[u8]| attr(e, name).and_then(|s| s.parse::<bool>().ok());
        let raw = |name: &[u8]| crate::util::attr_unescaped(e, name);
        out.index_options = Some(IndexOptions {
            title: raw(b"Title"),
            title_style: attr(e, b"TitleStyle"),
            replace_existing_index: b(b"ReplaceExistingIndex"),
            include_book_documents: b(b"IncludeBookDocuments"),
            include_hidden_entries: b(b"IncludeHiddenEntries"),
            index_format: attr(e, b"IndexFormat"),
            include_section_headings: b(b"IncludeSectionHeadings"),
            include_empty_index_sections: b(b"IncludeEmptyIndexSections"),
            level_styles: [
                attr(e, b"Level1Style"),
                attr(e, b"Level2Style"),
                attr(e, b"Level3Style"),
                attr(e, b"Level4Style"),
            ],
            section_heading_style: attr(e, b"SectionHeadingStyle"),
            page_number_style: attr(e, b"PageNumberStyle"),
            cross_reference_style: attr(e, b"CrossReferenceStyle"),
            cross_reference_topic_style: attr(e, b"CrossReferenceTopicStyle"),
            following_topic_separator: raw(b"FollowingTopicSeparator"),
            between_entries_separator: raw(b"BetweenEntriesSeparator"),
            page_range_separator: raw(b"PageRangeSeparator"),
            between_page_numbers_separator: raw(b"BetweenPageNumbersSeparator"),
            before_cross_reference_separator: raw(b"BeforeCrossReferenceSeparator"),
            entry_end_separator: raw(b"EntryEndSeparator"),
        });
    }
    if e.name().as_ref() == b"DocumentPreference" {
        let f = |name: &[u8]| -> f32 { attr(e, name).and_then(|s| s.parse().ok()).unwrap_or(0.0) };
        out.document_preference = DocumentPreference {
            bleed_top: f(b"DocumentBleedTopOffset"),
            bleed_bottom: f(b"DocumentBleedBottomOffset"),
            bleed_inside_or_left: f(b"DocumentBleedInsideOrLeftOffset"),
            bleed_outside_or_right: f(b"DocumentBleedOutsideOrRightOffset"),
            slug_top: f(b"SlugTopOffset"),
            slug_bottom: f(b"SlugBottomOffset"),
            slug_inside_or_left: f(b"SlugInsideOrLeftOffset"),
            slug_right_or_outside: f(b"SlugRightOrOutsideOffset"),
            page_width: attr(e, b"PageWidth").and_then(|s| s.parse::<f32>().ok()),
            page_height: attr(e, b"PageHeight").and_then(|s| s.parse::<f32>().ok()),
            facing_pages: attr(e, b"FacingPages").and_then(|s| s.parse::<bool>().ok()),
            page_binding: attr(e, b"PageBinding"),
        };
    }
    // W1.8 — `<FootnoteOption>` document-level footnote
    // separator + spacing settings. InDesign serialises
    // this once per document (inside `<RootFootnoteStory>`
    // or directly under `<Document>`); we match on the
    // element name wherever it appears. Attribute names
    // mirror the DOM `FootnoteOption` object.
    if e.name().as_ref() == b"FootnoteOption" {
        let f = |name: &[u8]| -> Option<f32> { attr(e, name).and_then(|s| s.parse().ok()) };
        out.footnote_options = FootnoteOptions {
            present: true,
            rule_on: attr(e, b"RuleOn").and_then(|s| s.parse().ok()),
            rule_color: attr(e, b"RuleColor"),
            rule_tint: f(b"RuleTint"),
            rule_line_weight: f(b"RuleLineWeight"),
            rule_width: f(b"RuleWidth"),
            rule_left_indent: f(b"RuleLeftIndent"),
            rule_offset: f(b"RuleOffset"),
            separator_text: attr(e, b"SeparatorText"),
            spacer: f(b"Spacer"),
            space_between: f(b"SpaceBetween"),
            start_at: attr(e, b"StartAt").and_then(|s| s.trim().parse().ok()),
            // InDesign writes the style as a typed `<Properties>` child
            // (see `read_footnote_option_properties`); an attribute is
            // read too.
            numbering_style: attr(e, b"FootnoteNumberingStyle"),
            // Around the number, where `ShowPrefixSuffix` (a child) says
            // (core L-01c).
            prefix: attr(e, b"Prefix").filter(|s| !s.is_empty()),
            suffix: attr(e, b"Suffix").filter(|s| !s.is_empty()),
            show_prefix_suffix: attr(e, b"ShowPrefixSuffix"),
            restart_numbering: attr(e, b"RestartNumbering"),
            // A frame with columns sets its notes across them (core
            // ADR 160); a frame may override it.
            enable_straddling: attr(e, b"EnableStraddling").and_then(|s| s.trim().parse().ok()),
            // A note too tall for its frame: continued under the
            // continuing rule, or its reference line moved on (core
            // ADR 161). The rule's colour is a typed child too.
            no_splitting: attr(e, b"NoSplitting").and_then(|s| s.trim().parse().ok()),
            continuing_rule: paged_model::ContinuingRule {
                on: attr(e, b"ContinuingRuleOn").and_then(|s| s.trim().parse().ok()),
                line_weight: f(b"ContinuingRuleLineWeight"),
                width: f(b"ContinuingRuleWidth"),
                offset: f(b"ContinuingRuleOffset"),
                left_indent: f(b"ContinuingRuleLeftIndent"),
                color: attr(e, b"ContinuingRuleColor"),
                tint: f(b"ContinuingRuleTint"),
            },
        };
    }
    // R-11 — `<TransparencyPreference>`: the document's global light,
    // which every effect with `UseGlobalLight="true"` takes its angle
    // (and a bevel its altitude) from. InDesign writes it in
    // Preferences.xml; absent attributes are its 120° / 30°.
    if e.name().as_ref() == b"TransparencyPreference" {
        let f = |name: &[u8]| -> Option<f32> { attr(e, name).and_then(|s| s.parse().ok()) };
        out.transparency_preference = paged_model::TransparencyPreference {
            global_light_angle: f(b"GlobalLightAngle"),
            global_light_altitude: f(b"GlobalLightAltitude"),
        };
    }
    // W2.5 — `<GridPreference>` baseline-grid + document-
    // grid settings (serialised once under `<Document>`).
    // Surfaced read-only for the editor's baseline panel +
    // overlay; the renderer never draws it.
    if e.name().as_ref() == b"GridPreference" {
        let f = |name: &[u8]| -> Option<f32> { attr(e, name).and_then(|s| s.parse().ok()) };
        out.grid_preference = GridPreference {
            present: true,
            baseline_start: f(b"BaselineStart"),
            baseline_division: f(b"BaselineDivision"),
            baseline_grid_shown: attr(e, b"BaselineGridShown").and_then(|s| s.parse().ok()),
            baseline_grid_relative_option: attr(e, b"BaselineGridRelativeOption"),
            baseline_color: attr(e, b"BaselineColor"),
            horizontal_gridline_division: f(b"HorizontalGridlineDivision"),
            vertical_gridline_division: f(b"VerticalGridlineDivision"),
        };
    }
}

/// `Resources/Preferences.xml`: InDesign's place for `DocumentPreference`
/// (bleed and slug), `FootnoteOption` and `GridPreference`. Read after the
/// design map, so InDesign's spelling wins over the engine's older one.
pub fn parse_preferences_part(xml: &[u8], out: &mut DesignMap) -> Result<(), ParseError> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) => read_preference(&e, out),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    read_footnote_option_properties(xml, &mut out.footnote_options)?;
    Ok(())
}

/// The typed `<Properties>` children of a `<FootnoteOption>`: InDesign
/// writes its enumerations there, not as attributes —
///
/// ```xml
/// <FootnoteOption StartAt="1" …>
///   <Properties>
///     <FootnoteNumberingStyle type="enumeration">Arabic</FootnoteNumberingStyle>
/// ```
///
/// (InDesign 20.0.1's own export). Read after the element's attributes;
/// a child wins over an attribute of the same name. `RestartNumbering`,
/// `ShowPrefixSuffix` and `RuleColor` are children too: a rule colour
/// written only as an attribute reads back Black in InDesign.
fn read_footnote_option_properties(
    xml: &[u8],
    options: &mut FootnoteOptions,
) -> Result<(), ParseError> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut in_option = false;
    let mut field: Option<Vec<u8>> = None;
    let mut text = String::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if e.name().as_ref() == b"FootnoteOption" => in_option = true,
            Event::End(e) if e.name().as_ref() == b"FootnoteOption" => in_option = false,
            Event::Start(e)
                if in_option
                    && matches!(
                        e.name().as_ref(),
                        b"FootnoteNumberingStyle"
                            | b"RestartNumbering"
                            | b"ShowPrefixSuffix"
                            | b"RuleColor"
                            | b"ContinuingRuleColor"
                    ) =>
            {
                field = Some(e.name().as_ref().to_vec());
                text.clear();
            }
            Event::Text(t) if field.is_some() => {
                text.push_str(
                    &t.xml_content(quick_xml::XmlVersion::Implicit1_0)
                        .unwrap_or_default(),
                );
            }
            Event::End(_) if field.is_some() => {
                let value = text.trim();
                if !value.is_empty() {
                    let slot = match field.as_deref() {
                        Some(b"RestartNumbering") => &mut options.restart_numbering,
                        Some(b"ShowPrefixSuffix") => &mut options.show_prefix_suffix,
                        Some(b"RuleColor") => &mut options.rule_color,
                        Some(b"ContinuingRuleColor") => &mut options.continuing_rule.color,
                        _ => &mut options.numbering_style,
                    };
                    *slot = Some(value.to_string());
                }
                field = None;
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// The document's `<ChapterNumberPreference>` from `Resources/Preferences.xml`
/// — the chapter number `ChapterNumberType` variables print (a document
/// setting, not a section one; measured, InDesign 20.0.1). InDesign writes
/// the number as an attribute and the format as a typed Properties child:
///
/// ```xml
/// <ChapterNumberPreference ChapterNumber="1" ChapterNumberSource="…">
///   <Properties><ChapterNumberFormat type="string">1, 2, 3, 4...</ChapterNumberFormat></Properties>
/// </ChapterNumberPreference>
/// ```
pub fn parse_chapter_number_preference(
    xml: &[u8],
) -> Result<Option<paged_model::ChapterNumberPreference>, ParseError> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut out: Option<paged_model::ChapterNumberPreference> = None;
    let mut in_pref = false;
    let mut in_format = false;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e)
                if e.name().as_ref() == b"ChapterNumberPreference" =>
            {
                out = Some(paged_model::ChapterNumberPreference {
                    number: attr(&e, b"ChapterNumber").and_then(|n| n.trim().parse().ok()),
                    format: None,
                });
                in_pref = true;
            }
            Event::Start(e) if in_pref && e.name().as_ref() == b"ChapterNumberFormat" => {
                in_format = true;
            }
            Event::Text(t) if in_format => {
                let text = t
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .unwrap_or_default();
                if let Some(pref) = out.as_mut() {
                    pref.format.get_or_insert_with(String::new).push_str(&text);
                }
            }
            Event::End(e) if e.name().as_ref() == b"ChapterNumberFormat" => in_format = false,
            Event::End(e) if e.name().as_ref() == b"ChapterNumberPreference" => break,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// `TextBefore` / `TextAfter`, which every variable preference carries.
fn read_decoration(var: &mut TextVariable, e: &quick_xml::events::BytesStart) {
    var.text_before = attr(e, b"TextBefore").or(var.text_before.take());
    var.text_after = attr(e, b"TextAfter").or(var.text_after.take());
}

/// A typed `<Properties>` child of the open `<Section>` / `<Hyperlink>`
/// whose text content is the value.
#[derive(Clone, Copy)]
enum PendingProperty {
    SectionPageNumberStyle,
    HyperlinkDestination,
    /// A typed `<Properties>` child of `<EndnoteOption>` (core ADR 170).
    Endnote(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <idPkg:MasterSpread src="MasterSpreads/MasterSpread_ua.xml"/>
  <idPkg:Spread src="Spreads/Spread_u1.xml"/>
  <idPkg:Spread src="Spreads/Spread_u2.xml"/>
  <idPkg:Story src="Stories/Story_u10.xml"/>
</Document>"#;

    #[test]
    fn parses_spread_and_story_manifest() {
        let dm = parse_designmap(SAMPLE).unwrap();
        assert_eq!(dm.spreads.len(), 2);
        assert_eq!(dm.stories.len(), 1);
        assert_eq!(dm.master_spreads.len(), 1);
        assert_eq!(dm.spreads[0].src, "Spreads/Spread_u1.xml");
        assert_eq!(dm.stories[0].src, "Stories/Story_u10.xml");
    }

    /// InDesign 20.0.1's index (`index-probe`, core ADR 168): a subtopic
    /// nests inside its parent, a See reference inside its topic, and the
    /// options live in `Preferences.xml`.
    #[test]
    fn reads_the_index_as_indesign_writes_it() {
        let dm = parse_designmap(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Document>
	<Index Self="u135">
		<Topic Self="u135TopicnApples" SortOrder="" Name="Apples" />
		<Topic Self="u135TopicnCherries" SortOrder="" Name="Cherries">
			<Topic Self="u135TopicnCherriesTopicnSour" SortOrder="" Name="Sour" />
		</Topic>
		<Topic Self="u135TopicnFruit" SortOrder="" Name="Fruit">
			<CrossReference Self="u159" ReferencedTopic="u135TopicnApples" CrossReferenceType="See" CustomTypeString="" />
		</Topic>
	</Index>
</Document>"#,
        )
        .unwrap();
        assert_eq!(dm.index_topics.len(), 4);
        assert_eq!(
            dm.index_topic_parents
                .get("u135TopicnCherriesTopicnSour")
                .map(String::as_str),
            Some("u135TopicnCherries")
        );
        assert_eq!(dm.index_topic_parents.len(), 1);
        let x = &dm.index_cross_references;
        assert_eq!(x.len(), 1);
        assert_eq!(x[0].topic, "u135TopicnFruit");
        assert_eq!(x[0].referenced_topic.as_deref(), Some("u135TopicnApples"));
        assert_eq!(x[0].kind.as_deref(), Some("See"));
        assert_eq!(x[0].custom_type_string, None);

        let mut dm = DesignMap::default();
        parse_preferences_part(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<IndexOptions Title="Index" TitleStyle="ParagraphStyle/$ID/Index Title" ReplaceExistingIndex="true" IncludeBookDocuments="false" IncludeHiddenEntries="false" IndexFormat="NestedFormat" IncludeSectionHeadings="true" IncludeEmptyIndexSections="false" Level1Style="ParagraphStyle/$ID/Index Level 1" Level2Style="ParagraphStyle/$ID/Index Level 2" Level3Style="ParagraphStyle/$ID/[No paragraph style]" Level4Style="ParagraphStyle/$ID/[No paragraph style]" SectionHeadingStyle="ParagraphStyle/$ID/Index Section Head" PageNumberStyle="CharacterStyle/$ID/[No character style]" CrossReferenceStyle="CharacterStyle/$ID/Index Cross-reference" CrossReferenceTopicStyle="CharacterStyle/$ID/[No character style]" FollowingTopicSeparator="  " BetweenEntriesSeparator="; " PageRangeSeparator="^=" BetweenPageNumbersSeparator=", " BeforeCrossReferenceSeparator=". " EntryEndSeparator="" />
</idPkg:Preferences>"#,
            &mut dm,
        )
        .unwrap();
        let o = dm.index_options.expect("IndexOptions");
        assert_eq!(o.title.as_deref(), Some("Index"));
        assert_eq!(o.include_section_headings, Some(true));
        assert_eq!(o.include_empty_index_sections, Some(false));
        assert_eq!(o.index_format.as_deref(), Some("NestedFormat"));
        assert_eq!(
            o.level_styles[1].as_deref(),
            Some("ParagraphStyle/$ID/Index Level 2")
        );
        assert_eq!(
            o.cross_reference_style.as_deref(),
            Some("CharacterStyle/$ID/Index Cross-reference")
        );
        assert_eq!(o.following_topic_separator.as_deref(), Some("  "));
        assert_eq!(o.between_page_numbers_separator.as_deref(), Some(", "));
        assert_eq!(o.before_cross_reference_separator.as_deref(), Some(". "));
        assert_eq!(o.page_range_separator.as_deref(), Some("^="));
        assert_eq!(o.entry_end_separator.as_deref(), Some(""));
    }

    /// Bookmarks nest as InDesign writes them (core ADR 171): a child's
    /// `<Bookmark>` inside its parent's.
    #[test]
    fn nested_bookmarks_record_their_parent() {
        let dm = parse_designmap(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Document>
	<Bookmark Self="u29532" Name="INTRODUCTION" Destination="HyperlinkPageDestination/.169265">
		<Bookmark Self="u29538" Name="Message from The Chairman" Destination="HyperlinkPageDestination/.169271" />
	</Bookmark>
	<Bookmark Self="u2953c" Name="OVERVIEW" Destination="HyperlinkPageDestination/.169275" />
</Document>"#,
        )
        .unwrap();
        let names: Vec<_> = dm.bookmarks.iter().map(|b| b.name.as_deref()).collect();
        assert_eq!(
            names,
            [
                Some("INTRODUCTION"),
                Some("Message from The Chairman"),
                Some("OVERVIEW")
            ]
        );
        assert_eq!(
            dm.bookmark_parents.get("u29538").map(String::as_str),
            Some("u29532")
        );
        assert_eq!(dm.bookmark_parents.len(), 1);
    }

    /// `<EndnoteOption>` as InDesign 20.0.1 writes it (core ADR 170).
    #[test]
    fn reads_the_endnote_options() {
        let dm = parse_designmap(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Document>
	<EndnoteOption EndnoteTitle="Endnotes" EndnoteTitleStyle="ParagraphStyle/$ID/NormalParagraphStyle" StartEndnoteNumberAt="3" EndnoteMarkerStyle="CharacterStyle/$ID/[No character style]" EndnoteTextStyle="ParagraphStyle/$ID/NormalParagraphStyle" EndnoteSeparatorText="&#x9;" EndnotePrefix="" EndnoteSuffix="">
		<Properties>
			<EndnoteNumberingStyle type="enumeration">LowerRoman</EndnoteNumberingStyle>
			<RestartEndnoteNumbering type="enumeration">Continuous</RestartEndnoteNumbering>
			<EndnoteMarkerPositioning type="enumeration">SuperscriptMarker</EndnoteMarkerPositioning>
			<ScopeValue type="enumeration">EndnoteDocumentScope</ScopeValue>
			<FrameCreateOption type="enumeration">NewPage</FrameCreateOption>
			<ShowEndnotePrefixSuffix type="enumeration">NoPrefixSuffix</ShowEndnotePrefixSuffix>
		</Properties>
	</EndnoteOption>
</Document>"#,
        )
        .unwrap();
        let o = dm.endnote_options.expect("EndnoteOption");
        assert_eq!(o.title.as_deref(), Some("Endnotes"));
        assert_eq!(o.start_at, Some(3));
        assert_eq!(o.separator_text.as_deref(), Some("\t"));
        assert_eq!(o.numbering_style.as_deref(), Some("LowerRoman"));
        assert_eq!(o.restart.as_deref(), Some("Continuous"));
        assert_eq!(o.scope.as_deref(), Some("EndnoteDocumentScope"));
        assert_eq!(o.frame_create.as_deref(), Some("NewPage"));
        assert_eq!(o.show_prefix_suffix.as_deref(), Some("NoPrefixSuffix"));
        assert_eq!(o.label(1), "iii");
    }

    /// InDesign 20.0.1's cross-reference formats (`xref-probe`, core ADR
    /// 169): building blocks, the custom text verbatim, in the document's
    /// own words.
    #[test]
    fn reads_cross_reference_formats_as_indesign_writes_them() {
        let dm = parse_designmap(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Document>
	<CrossReferenceFormat Self="u9f" Name="Full Paragraph &amp; Page Number" AppliedCharacterStyle="n">
		<BuildingBlock Self="u9fBuildingBlock0" BlockType="CustomStringBuildingBlock" AppliedCharacterStyle="n" CustomText="“" AppliedDelimiter="$ID/" IncludeDelimiter="false" />
		<BuildingBlock Self="u9fBuildingBlock1" BlockType="FullParagraphBuildingBlock" AppliedCharacterStyle="n" CustomText="$ID/" AppliedDelimiter="$ID/" IncludeDelimiter="false" />
		<BuildingBlock Self="u9fBuildingBlock2" BlockType="CustomStringBuildingBlock" AppliedCharacterStyle="n" CustomText="” on page " AppliedDelimiter="$ID/" IncludeDelimiter="false" />
		<BuildingBlock Self="u9fBuildingBlock3" BlockType="PageNumberBuildingBlock" AppliedCharacterStyle="n" CustomText="$ID/" AppliedDelimiter="$ID/" IncludeDelimiter="false" />
	</CrossReferenceFormat>
	<CrossReferenceFormat Self="ua5" Name="Page Number" AppliedCharacterStyle="n">
		<BuildingBlock Self="ua5BuildingBlock0" BlockType="PageNumberBuildingBlock" AppliedCharacterStyle="n" CustomText="$ID/" AppliedDelimiter="$ID/" IncludeDelimiter="false" />
	</CrossReferenceFormat>
</Document>"#
                .as_bytes(),
        )
        .unwrap();
        let f = &dm.cross_reference_formats;
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].name.as_deref(), Some("Full Paragraph & Page Number"));
        let kinds: Vec<&str> = f[0].blocks.iter().map(|b| b.block_type.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "CustomStringBuildingBlock",
                "FullParagraphBuildingBlock",
                "CustomStringBuildingBlock",
                "PageNumberBuildingBlock"
            ]
        );
        assert_eq!(f[0].blocks[2].custom_text.as_deref(), Some("” on page "));
        assert_eq!(f[0].blocks[0].include_delimiter, Some(false));
        assert_eq!(f[1].blocks.len(), 1);
    }

    const LAYERS_SAMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Layer Self="ua" Name="Layer 1" Visible="true" Locked="false" Printable="true"/>
  <Layer Self="ub" Name="Guides" Visible="true" Locked="true" Printable="false"/>
  <Layer Self="uc" Name="Hidden" Visible="false" Printable="true"/>
  <Layer Self="ud" Name="Defaults"/>
</Document>"#;

    #[test]
    fn q17_layer_printable_attribute_round_trips() {
        let dm = parse_designmap(LAYERS_SAMPLE).unwrap();
        assert_eq!(dm.layers.len(), 4);
        let printable: Vec<bool> = dm.layers.iter().map(|l| l.printable).collect();
        assert_eq!(printable, vec![true, false, true, true]);
        let visible: Vec<bool> = dm.layers.iter().map(|l| l.visible).collect();
        assert_eq!(visible, vec![true, true, false, true]);
    }

    #[test]
    fn flat_layers_have_no_parent() {
        let dm = parse_designmap(LAYERS_SAMPLE).unwrap();
        assert!(dm.layers.iter().all(|l| l.parent_id.is_none()));
    }

    #[test]
    fn nested_layers_capture_parent() {
        // A layer group (folder): the non-self-closing <Layer> opens a
        // scope; its child <Layer> records the parent's Self. A sibling
        // top-level layer after the group closes is parentless again.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Layer Self="grp" Name="Group">
    <Layer Self="child1" Name="Child 1"/>
    <Layer Self="child2" Name="Child 2"/>
  </Layer>
  <Layer Self="peer" Name="Peer"/>
</Document>"#;
        let dm = parse_designmap(xml).unwrap();
        assert_eq!(dm.layers.len(), 4);
        let by_id = |id: &str| dm.layers.iter().find(|l| l.self_id == id).unwrap();
        assert_eq!(by_id("grp").parent_id, None);
        assert_eq!(by_id("child1").parent_id.as_deref(), Some("grp"));
        assert_eq!(by_id("child2").parent_id.as_deref(), Some("grp"));
        assert_eq!(by_id("peer").parent_id, None);
    }

    #[test]
    fn numbering_style_formats() {
        assert_eq!(NumberingStyle::Arabic.format(3), "3");
        assert_eq!(NumberingStyle::UpperRoman.format(4), "IV");
        assert_eq!(NumberingStyle::LowerRoman.format(3), "iii");
        assert_eq!(NumberingStyle::LowerRoman.format(9), "ix");
        assert_eq!(NumberingStyle::UpperAlpha.format(1), "A");
        assert_eq!(NumberingStyle::UpperAlpha.format(27), "AA");
        assert_eq!(NumberingStyle::LowerAlpha.format(2), "b");
        // 0 / out-of-range fall back to Arabic digits, never empty.
        assert_eq!(NumberingStyle::UpperRoman.format(0), "0");
    }

    #[test]
    fn parses_section_definitions() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Section Self="sec1" PageStart="page1" PageNumberStyle="LowerRoman"
           PageNumberStart="1" ContinueNumbering="false"/>
  <Section Self="sec2" PageStart="page3" PageNumberStyle="Arabic"
           SectionPrefix="A-" IncludeSectionPrefix="true" PageNumberStart="1"/>
</Document>"#;
        let dm = parse_designmap(xml).unwrap();
        assert_eq!(dm.sections.len(), 2);
        assert_eq!(dm.sections[0].page_start.as_deref(), Some("page1"));
        assert_eq!(dm.sections[0].numbering_style, NumberingStyle::LowerRoman);
        assert_eq!(dm.sections[0].start_at, Some(1));
        assert_eq!(dm.sections[1].numbering_style, NumberingStyle::Arabic);
        assert_eq!(dm.sections[1].section_prefix.as_deref(), Some("A-"));
        assert!(dm.sections[1].include_prefix);
    }

    #[test]
    fn reads_every_page_number_style_indesign_offers() {
        // InDesign 20.0.1 writes the style as a typed child of the
        // section (core ADR 157); the newer styles read as themselves.
        for (idml, style) in [
            ("Kanji", NumberingStyle::Kanji),
            ("HebrewBiblical", NumberingStyle::HebrewBiblical),
            ("ArabicAbjad", NumberingStyle::ArabicAbjad),
            ("DoubleLeadingZeros", NumberingStyle::DoubleLeadingZeros),
        ] {
            let xml = format!(
                r#"<Document><Section Self="s" PageStart="p" PageNumberStart="12"><Properties><PageNumberStyle type="enumeration">{idml}</PageNumberStyle></Properties></Section></Document>"#
            );
            let dm = parse_designmap(xml.as_bytes()).unwrap();
            assert_eq!(dm.sections[0].numbering_style, style, "{idml}");
            assert_eq!(style.as_idml(), idml);
        }
        assert_eq!(NumberingStyle::HebrewBiblical.format(12), "יב");
        assert_eq!(NumberingStyle::DoubleLeadingZeros.format(12), "012");
    }

    #[test]
    fn reads_dom_version_when_present() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="18.5">
  <idPkg:Spread src="Spreads/Spread_u1.xml"/>
</Document>"#;
        let dm = parse_designmap(xml).unwrap();
        assert_eq!(dm.dom_version.as_deref(), Some("18.5"));
    }

    #[test]
    fn dom_version_absent_is_none() {
        // SAMPLE's <Document> carries no DOMVersion attribute.
        let dm = parse_designmap(SAMPLE).unwrap();
        assert_eq!(dm.dom_version, None);
    }

    #[test]
    fn parses_hyperlink_resources_and_destinations() {
        // W1.4 — hyperlink definitions + their destination resources.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" Name="brochure.indd">
  <HyperlinkURLDestination Self="d1" DestinationURL="https://paged.media"/>
  <HyperlinkPageDestination Self="d2" DestinationPage="Page/p3"/>
  <HyperlinkTextDestination Self="d3" DestinationText="Story/s9"/>
  <Hyperlink Self="h1" Name="web" Source="HyperlinkTextSource/src1" Destination="d1"/>
  <Hyperlink Self="h2" Name="jump" Source="HyperlinkTextSource/src2" Destination="d2"/>
</Document>"#;
        let dm = parse_designmap(xml).unwrap();
        assert_eq!(dm.document_name.as_deref(), Some("brochure.indd"));
        assert_eq!(dm.hyperlinks.len(), 2);
        assert_eq!(
            dm.hyperlinks[0].source.as_deref(),
            Some("HyperlinkTextSource/src1")
        );
        assert_eq!(dm.hyperlinks[0].destination.as_deref(), Some("d1"));
        assert_eq!(dm.hyperlink_destinations.len(), 3);
        assert!(matches!(
            &dm.hyperlink_destinations[0].kind,
            HyperlinkDestinationKind::Url(u) if u == "https://paged.media"
        ));
        assert!(matches!(
            &dm.hyperlink_destinations[1].kind,
            HyperlinkDestinationKind::Page(p) if p == "Page/p3"
        ));
        assert!(matches!(
            &dm.hyperlink_destinations[2].kind,
            HyperlinkDestinationKind::TextAnchor(t) if t == "Story/s9"
        ));
    }

    #[test]
    fn parses_text_variable_with_preference_contents() {
        // W1.4 — a custom text variable folds in its
        // <TextVariablePreference Contents="..."> child.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <TextVariable Self="TextVariable/v1" Name="Season" VariableType="CustomTextType">
    <TextVariablePreference Contents="Spring 2026"/>
  </TextVariable>
  <TextVariable Self="TextVariable/v2" Name="Pages" VariableType="PageCountType"/>
</Document>"#;
        let dm = parse_designmap(xml).unwrap();
        assert_eq!(dm.text_variables.len(), 2);
        let custom = &dm.text_variables[0];
        assert_eq!(custom.variable_type.as_deref(), Some("CustomTextType"));
        assert_eq!(custom.contents.as_deref(), Some("Spring 2026"));
        let pc = &dm.text_variables[1];
        assert_eq!(pc.variable_type.as_deref(), Some("PageCountType"));
        assert_eq!(pc.contents, None);
    }

    /// The typed preference children exactly as InDesign 20.0.1 exported
    /// them (DOM-built document, 2026-10-01; thoughts ADR 033).
    #[test]
    fn parses_indesigns_typed_variable_preferences() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
	<TextVariable Self="dTextVariablenRH" Name="RH" VariableType="MatchParagraphStyleType">
		<MatchParagraphStylePreference TextBefore="" TextAfter="" AppliedParagraphStyle="ParagraphStyle/Heading" SearchStrategy="LastOnPage" ChangeCase="Uppercase" DeleteEndPunctuation="true" />
	</TextVariable>
	<TextVariable Self="dTextVariablenCH" Name="CH" VariableType="MatchCharacterStyleType">
		<MatchCharacterStylePreference TextBefore="[" TextAfter="]" AppliedCharacterStyle="CharacterStyle/Keyword" SearchStrategy="FirstOnPage" ChangeCase="Titlecase" DeleteEndPunctuation="false" />
	</TextVariable>
	<TextVariable Self="dTextVariablenLP" Name="LP" VariableType="LastPageNumberType">
		<PageNumberVariablePreference TextBefore="" Format="UpperRoman" TextAfter="" Scope="SectionScope" />
	</TextVariable>
	<TextVariable Self="dTextVariablenChap" Name="Chap" VariableType="ChapterNumberType">
		<ChapterNumberVariablePreference TextBefore="" Format="Current" TextAfter="" />
	</TextVariable>
	<TextVariable Self="dTextVariablenFile" Name="File" VariableType="FileNameType">
		<FileNameVariablePreference TextBefore="" IncludePath="false" IncludeExtension="true" TextAfter="" />
	</TextVariable>
	<TextVariable Self="dTextVariablenDate" Name="Date" VariableType="CreationDateType">
		<DateVariablePreference TextBefore="" Format="yyyy-MM-dd" TextAfter="" />
	</TextVariable>
	<TextVariable Self="dTextVariablenCust" Name="Cust" VariableType="CustomTextType">
		<CustomTextVariablePreference>
			<Properties>
				<Contents type="string"> R&amp;D notes </Contents>
			</Properties>
		</CustomTextVariablePreference>
	</TextVariable>
</Document>"#;
        let dm = parse_designmap(xml).unwrap();
        let v = |id: &str| {
            dm.text_variables
                .iter()
                .find(|v| v.self_id == id)
                .unwrap_or_else(|| panic!("{id}"))
        };
        let rh = v("dTextVariablenRH");
        assert_eq!(
            rh.running_header_style.as_deref(),
            Some("ParagraphStyle/Heading")
        );
        assert_eq!(rh.running_header_character_style, None);
        assert_eq!(rh.running_header_use.as_deref(), Some("LastOnPage"));
        assert_eq!(rh.change_case.as_deref(), Some("Uppercase"));
        assert!(rh.delete_end_punctuation);
        let ch = v("dTextVariablenCH");
        assert_eq!(
            ch.running_header_style, None,
            "a character style has its own slot"
        );
        assert_eq!(
            ch.running_header_character_style.as_deref(),
            Some("CharacterStyle/Keyword")
        );
        assert_eq!(ch.running_header_use.as_deref(), Some("FirstOnPage"));
        assert_eq!(ch.change_case.as_deref(), Some("Titlecase"));
        assert!(!ch.delete_end_punctuation);
        assert_eq!(ch.text_before.as_deref(), Some("["));
        assert_eq!(ch.text_after.as_deref(), Some("]"));
        let lp = v("dTextVariablenLP");
        assert_eq!(lp.page_number_scope.as_deref(), Some("SectionScope"));
        assert_eq!(lp.number_format.as_deref(), Some("UpperRoman"));
        assert_eq!(
            v("dTextVariablenChap").number_format.as_deref(),
            Some("Current")
        );
        let file = v("dTextVariablenFile");
        assert!(file.include_extension && !file.include_path);
        assert_eq!(
            v("dTextVariablenDate").date_format.as_deref(),
            Some("yyyy-MM-dd")
        );
        assert_eq!(
            v("dTextVariablenCust").contents.as_deref(),
            Some(" R&D notes "),
            "custom contents are verbatim, entities resolved"
        );
    }

    #[test]
    fn parses_the_documents_chapter_number_preference() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
	<ChapterNumberPreference ChapterNumber="4" ChapterNumberSource="ContinueFromPreviousDocument">
		<Properties>
			<ChapterNumberFormat type="string">I, II, III, IV...</ChapterNumberFormat>
		</Properties>
	</ChapterNumberPreference>
</idPkg:Preferences>"#;
        let pref = parse_chapter_number_preference(xml).unwrap().unwrap();
        assert_eq!(pref.number, Some(4));
        assert_eq!(pref.format.as_deref(), Some("I, II, III, IV..."));
        assert_eq!(
            parse_chapter_number_preference(b"<idPkg:Preferences/>").unwrap(),
            None
        );
    }
}

#[cfg(test)]
mod document_preference_tests {
    use super::*;

    #[test]
    fn parses_bleed_and_slug_offsets() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document DOMVersion="18.5">
  <DocumentPreference PageWidth="595.2755905511812" PageHeight="841.8897637795276"
    DocumentBleedTopOffset="8.503937007874017"
    DocumentBleedBottomOffset="8.503937007874017"
    DocumentBleedInsideOrLeftOffset="8.503937007874017"
    DocumentBleedOutsideOrRightOffset="8.503937007874017"
    SlugTopOffset="14.173228346456694"
    SlugBottomOffset="0"
    SlugInsideOrLeftOffset="0"
    SlugRightOrOutsideOffset="0"/>
</Document>"#;
        let dm = parse_designmap(xml).expect("parse");
        let p = dm.document_preference;
        assert!((p.bleed_top - 8.5039).abs() < 1e-3);
        assert!((p.bleed_outside_or_right - 8.5039).abs() < 1e-3);
        assert!((p.slug_top - 14.1732).abs() < 1e-3);
        assert_eq!(p.slug_bottom, 0.0);
    }

    #[test]
    fn absent_element_defaults_to_zero() {
        let xml = br#"<?xml version="1.0"?><Document DOMVersion="18.5"/>"#;
        let dm = parse_designmap(xml).expect("parse");
        assert_eq!(dm.document_preference, DocumentPreference::default());
    }

    /// InDesign keeps bleed, footnote options and the grid in
    /// `Resources/Preferences.xml`; read after the design map, its values
    /// win, and an element the part does not carry leaves the design map's
    /// reading alone.
    #[test]
    fn the_preferences_part_wins_over_the_designmap() {
        let mut dm = parse_designmap(
            br#"<Document DOMVersion="20.0" Self="d">
<DocumentPreference DocumentBleedTopOffset="1"/>
<GridPreference BaselineDivision="12"/>
</Document>"#,
        )
        .unwrap();
        parse_preferences_part(
            br#"<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<DocumentPreference PageHeight="841.8897637795275" PageWidth="595.275590551181" FacingPages="false" DocumentBleedTopOffset="8.503937007874017" DocumentBleedBottomOffset="8.503937007874017" DocumentBleedInsideOrLeftOffset="8.503937007874017" DocumentBleedOutsideOrRightOffset="8.503937007874017"/>
<FootnoteOption RuleOn="true" RuleLineWeight="0.5" SpaceBetween="3" SeparatorText="&#x9;"/>
</idPkg:Preferences>"#,
            &mut dm,
        )
        .unwrap();
        let b = &dm.document_preference;
        assert!((b.bleed_top - 8.503_937).abs() < 1e-3, "{b:?}");
        assert!((b.bleed_outside_or_right - 8.503_937).abs() < 1e-3);
        assert!(dm.footnote_options.present);
        assert_eq!(dm.footnote_options.rule_on, Some(true));
        assert_eq!(dm.footnote_options.space_between, Some(3.0));
        assert_eq!(
            dm.grid_preference.baseline_division,
            Some(12.0),
            "kept from the design map"
        );
    }

    /// The page setup InDesign writes on the same element: page size,
    /// facing pages and binding (sample.idml's spelling).
    #[test]
    fn reads_page_size_facing_pages_and_binding() {
        let mut dm = DesignMap::default();
        parse_preferences_part(
            br#"<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<DocumentPreference PageHeight="841.889763778" PageWidth="595.275590551" PagesPerDocument="1" FacingPages="false" DocumentBleedTopOffset="0" PageBinding="RightToLeft" ColumnDirection="Horizontal">
<Properties><ColumnGuideColor type="enumeration">Violet</ColumnGuideColor></Properties>
</DocumentPreference>
</idPkg:Preferences>"#,
            &mut dm,
        )
        .unwrap();
        let p = &dm.document_preference;
        assert!((p.page_width.unwrap() - 595.275_6).abs() < 1e-3, "{p:?}");
        assert!((p.page_height.unwrap() - 841.889_8).abs() < 1e-3);
        assert_eq!(p.facing_pages, Some(false));
        assert_eq!(p.page_binding.as_deref(), Some("RightToLeft"));
        // Absent attributes stay absent.
        let dm = parse_designmap(
            br#"<Document><DocumentPreference DocumentBleedTopOffset="3"/></Document>"#,
        )
        .unwrap();
        let p = &dm.document_preference;
        assert_eq!(
            (
                p.page_width,
                p.page_height,
                p.facing_pages,
                p.page_binding.as_deref()
            ),
            (None, None, None, None)
        );
    }

    #[test]
    fn parses_footnote_option_rule_and_spacing() {
        // W1.8 — a document-level <FootnoteOption> as InDesign
        // serialises it (PascalCase DOM-mirroring attributes). The
        // parser must lift the separator-rule and spacing settings.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document DOMVersion="20.0">
  <RootFootnoteStory>
    <FootnoteOption RuleOn="true" RuleColor="Color/FootRule" RuleTint="100"
      RuleLineWeight="1.5" RuleWidth="120" RuleLeftIndent="6" RuleOffset="4"
      SeparatorText="^t" Spacer="9" SpaceBetween="3"/>
  </RootFootnoteStory>
</Document>"#;
        let dm = parse_designmap(xml).expect("parse");
        let fo = &dm.footnote_options;
        assert!(fo.present);
        assert!(!fo.is_default());
        assert_eq!(fo.rule_on, Some(true));
        assert!(fo.rule_on_effective());
        assert_eq!(fo.rule_color.as_deref(), Some("Color/FootRule"));
        assert_eq!(fo.rule_tint, Some(100.0));
        assert_eq!(fo.rule_line_weight, Some(1.5));
        assert_eq!(fo.rule_width, Some(120.0));
        assert_eq!(fo.rule_left_indent, Some(6.0));
        assert_eq!(fo.rule_offset, Some(4.0));
        assert_eq!(fo.separator_text.as_deref(), Some("^t"));
        assert_eq!(fo.spacer, Some(9.0));
        assert_eq!(fo.space_between, Some(3.0));
    }

    #[test]
    fn no_splitting_and_the_continuing_rule_are_read() {
        // Core ADR 161: InDesign writes `NoSplitting` and the continuing
        // rule's attributes on `<FootnoteOption>`, its colour as a typed
        // child.
        let xml = br#"<?xml version="1.0"?><idPkg:Preferences xmlns:idPkg="x"><FootnoteOption NoSplitting="true" ContinuingRuleOn="true" ContinuingRuleLineWeight="1" ContinuingRuleWidth="288" ContinuingRuleOffset="2" ContinuingRuleTint="50"><Properties><ContinuingRuleColor type="object">Color/Black</ContinuingRuleColor></Properties></FootnoteOption></idPkg:Preferences>"#;
        let mut dm = DesignMap::default();
        parse_preferences_part(xml, &mut dm).expect("parse");
        let fo = &dm.footnote_options;
        assert_eq!(fo.no_splitting, Some(true));
        let r = &fo.continuing_rule;
        assert_eq!(r.on, Some(true));
        assert_eq!(
            (r.line_weight, r.width, r.offset),
            (Some(1.0), Some(288.0), Some(2.0))
        );
        assert_eq!(r.tint, Some(50.0));
        assert_eq!(r.color.as_deref(), Some("Color/Black"));
    }

    #[test]
    fn footnote_straddling_is_read() {
        // Core ADR 160: `EnableStraddling` sets a frame's notes across its
        // columns, as InDesign 20.0.1 writes it on `<FootnoteOption>`.
        let xml = br#"<?xml version="1.0"?><idPkg:Preferences xmlns:idPkg="x"><FootnoteOption EnableStraddling="true" StartAt="1"/></idPkg:Preferences>"#;
        let mut dm = DesignMap::default();
        parse_preferences_part(xml, &mut dm).expect("parse");
        assert_eq!(dm.footnote_options.enable_straddling, Some(true));
        let frame = paged_model::FrameFootnotes::default();
        assert!(frame.spans(&dm.footnote_options));
    }

    #[test]
    fn footnote_numbering_options_are_read_in_indesigns_spelling() {
        // InDesign 20.0.1's own Preferences.xml (a corpus pack's, trimmed):
        // Prefix / Suffix as attributes, the enumerations and the rule
        // colour as typed `<Properties>` children (core L-01c).
        let xml = br#"<?xml version="1.0"?><idPkg:Preferences xmlns:idPkg="x">
<FootnoteOption StartAt="1" Prefix="(" Suffix=")" SeparatorText="&#x9;" RuleOn="true">
  <Properties>
    <FootnoteNumberingStyle type="enumeration">LowerRoman</FootnoteNumberingStyle>
    <RestartNumbering type="enumeration">PageRestart</RestartNumbering>
    <ShowPrefixSuffix type="enumeration">PrefixSuffixReference</ShowPrefixSuffix>
    <MarkerPositioning type="enumeration">SuperscriptMarker</MarkerPositioning>
    <RuleColor type="object">Color/Red</RuleColor>
  </Properties>
</FootnoteOption></idPkg:Preferences>"#;
        let mut dm = DesignMap::default();
        parse_preferences_part(xml, &mut dm).expect("parse");
        let fo = &dm.footnote_options;
        assert_eq!(fo.prefix.as_deref(), Some("("));
        assert_eq!(fo.suffix.as_deref(), Some(")"));
        assert_eq!(fo.numbering_style.as_deref(), Some("LowerRoman"));
        assert_eq!(fo.restart_numbering.as_deref(), Some("PageRestart"));
        assert_eq!(
            fo.show_prefix_suffix.as_deref(),
            Some("PrefixSuffixReference")
        );
        assert_eq!(fo.rule_color.as_deref(), Some("Color/Red"));
        assert_eq!(fo.reference_label(2), "(ii)");
        assert_eq!(fo.text_label(2), "ii");
    }

    #[test]
    fn footnote_option_rule_off_is_distinct_from_absent() {
        // RuleOn="false" must round-trip as Some(false) — the renderer
        // distinguishes "rule explicitly off" from "no element at all"
        // (which defaults to rule ON, InDesign's behaviour).
        let off = br#"<?xml version="1.0"?><Document><FootnoteOption RuleOn="false"/></Document>"#;
        let dm = parse_designmap(off).expect("parse");
        assert!(dm.footnote_options.present);
        assert_eq!(dm.footnote_options.rule_on, Some(false));
        assert!(!dm.footnote_options.rule_on_effective());

        let absent = br#"<?xml version="1.0"?><Document/>"#;
        let dm = parse_designmap(absent).expect("parse");
        assert!(dm.footnote_options.is_default());
        assert_eq!(dm.footnote_options.rule_on, None);
        // Absent ⇒ default to rule ON.
        assert!(dm.footnote_options.rule_on_effective());
    }

    #[test]
    fn parses_grid_preference_baseline_grid() {
        // W2.5 — `<GridPreference>` as InDesign serialises it. The
        // parser lifts the baseline-grid subset (start / division /
        // shown / relative-to / colour) for the editor's baseline panel.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<Document DOMVersion="20.0">
  <GridPreference BaselineStart="48" BaselineDivision="12"
    BaselineGridShown="true" BaselineGridRelativeOption="TopMargin"
    BaselineColor="Color/Grid"
    HorizontalGridlineDivision="72" VerticalGridlineDivision="72"/>
</Document>"#;
        let dm = parse_designmap(xml).expect("parse");
        let gp = &dm.grid_preference;
        assert!(gp.present);
        assert_eq!(gp.baseline_start, Some(48.0));
        assert_eq!(gp.baseline_division, Some(12.0));
        assert_eq!(gp.baseline_grid_shown, Some(true));
        assert_eq!(
            gp.baseline_grid_relative_option.as_deref(),
            Some("TopMargin")
        );
        assert_eq!(gp.baseline_color.as_deref(), Some("Color/Grid"));
        assert_eq!(gp.horizontal_gridline_division, Some(72.0));
        assert_eq!(gp.vertical_gridline_division, Some(72.0));
    }

    #[test]
    fn grid_preference_absent_is_default() {
        let absent = br#"<?xml version="1.0"?><Document/>"#;
        let dm = parse_designmap(absent).expect("parse");
        assert!(!dm.grid_preference.present);
        assert_eq!(dm.grid_preference, GridPreference::default());
        assert_eq!(dm.grid_preference.baseline_division, None);
    }
}
