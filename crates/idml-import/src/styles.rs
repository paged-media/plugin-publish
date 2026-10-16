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

//! `Resources/Styles.xml` — paragraph and character style sheet.
//!
//! IDML's typical layout:
//!
//! ```xml
//! <idPkg:Styles>
//!   <RootCharacterStyleGroup>
//!     <CharacterStyle Self="CharacterStyle/$ID/[No character style]" .../>
//!     <CharacterStyle Self="CharacterStyle/Bold" FontStyle="Bold" .../>
//!   </RootCharacterStyleGroup>
//!   <RootParagraphStyleGroup>
//!     <ParagraphStyle Self="ParagraphStyle/Body"
//!                     AppliedFont="Body Font"
//!                     PointSize="11" .../>
//!   </RootParagraphStyleGroup>
//! </idPkg:Styles>
//! ```
//!
//! Only the cascadable attributes the renderer currently consumes
//! land here (font / style / size / fill / tracking + paragraph
//! geometry knobs). `BasedOn` chains are followed at resolve time;
//! cycles are bounded by `MAX_BASED_ON_DEPTH`.

use quick_xml::events::Event;

use crate::spread::{CornerOption, CornerSpec};
use crate::story::{Justification, TabStop};

use crate::util::{attr, parse_tint_attr};
use crate::ParseError;
pub use paged_model::{
    CellStyleDef, CharacterStyleDef, ConditionDef, ConditionSetDef, GrepStyle, LineStyle,
    NestedDelimiter, NestedStyle, NumberingListDef, ObjectStyleDef, ParagraphBorder, ParagraphRule,
    ParagraphShading, ParagraphStyleDef, ResolvedCell, ResolvedCharacter, ResolvedObject,
    ResolvedParagraph, ResolvedTable, StripeDef, StrokeStyleDef, StrokeStyleKind, TOCStyleDef,
    TOCStyleEntryDef, TableStyleDef,
};

pub use paged_model::StyleSheet;
use paged_model::{
    AutoSizingType, FirstBaselineOffset, ObjectStyleCategories, TextFrameOptions,
    VerticalJustification,
};

/// Parse a [`ParagraphRule`] from the `<prefix>*` attrs. `prefix` is either
/// `"RuleAbove"` or `"RuleBelow"` to match IDML's two attribute families.
/// (De-inherented from `ParagraphRule::from_attrs` so the type can move to
/// `paged-model`; the XML parsing stays in the parser — N6.)
pub(crate) fn parse_paragraph_rule(
    e: &quick_xml::events::BytesStart,
    prefix: &str,
) -> ParagraphRule {
    // Construct attr names on the fly. quick-xml accepts &[u8] keys
    // for `attr()`; building owned Vec<u8> per attr is fine — this
    // runs once per style at parse time.
    let key = |suffix: &str| -> Vec<u8> {
        let mut v = Vec::with_capacity(prefix.len() + suffix.len());
        v.extend_from_slice(prefix.as_bytes());
        v.extend_from_slice(suffix.as_bytes());
        v
    };
    ParagraphRule {
        on: attr(e, &key("")).and_then(|s| s.parse().ok()),
        color: attr(e, &key("Color")),
        tint: attr(e, &key("Tint")).and_then(|s| s.parse().ok()),
        weight: attr(e, &key("LineWeight"))
            .and_then(|s| s.parse().ok())
            .or_else(|| attr(e, &key("Weight")).and_then(|s| s.parse().ok())),
        offset: attr(e, &key("Offset")).and_then(|s| s.parse().ok()),
        left_indent: attr(e, &key("LeftIndent")).and_then(|s| s.parse().ok()),
        right_indent: attr(e, &key("RightIndent")).and_then(|s| s.parse().ok()),
        width: attr(e, &key("Width")),
    }
}

/// Parse a [`ParagraphBorder`] from the `ParagraphBorder*` attrs off a
/// `<ParagraphStyle>` (or `<ParagraphStyleRange>`) element. Returns a
/// fully-default instance when no attrs are present; callers can check `on`
/// to know whether to emit. (De-inherented from `ParagraphBorder::from_attrs`
/// — N6.)
pub(crate) fn parse_paragraph_border(e: &quick_xml::events::BytesStart) -> ParagraphBorder {
    // Order matches Rectangle::corners — clockwise from top-left.
    let per = [
        (
            "ParagraphBorderTopLeftCornerOption",
            "ParagraphBorderTopLeftCornerRadius",
        ),
        (
            "ParagraphBorderTopRightCornerOption",
            "ParagraphBorderTopRightCornerRadius",
        ),
        (
            "ParagraphBorderBottomRightCornerOption",
            "ParagraphBorderBottomRightCornerRadius",
        ),
        (
            "ParagraphBorderBottomLeftCornerOption",
            "ParagraphBorderBottomLeftCornerRadius",
        ),
    ];
    let mut corners = [CornerSpec::default(); 4];
    for (i, (oname, rname)) in per.iter().enumerate() {
        corners[i].option = attr(e, oname.as_bytes())
            .as_deref()
            .and_then(CornerOption::from_idml);
        corners[i].radius = attr(e, rname.as_bytes()).and_then(|s| s.parse().ok());
    }
    ParagraphBorder {
        on: attr(e, b"ParagraphBorderOn").and_then(|s| s.parse().ok()),
        color: attr(e, b"ParagraphBorderColor"),
        tint: attr(e, b"ParagraphBorderTint").and_then(|s| s.parse().ok()),
        weight: attr(e, b"ParagraphBorderWeight").and_then(|s| s.parse().ok()),
        offset_top: attr(e, b"ParagraphBorderTopOffset").and_then(|s| s.parse().ok()),
        offset_left: attr(e, b"ParagraphBorderLeftOffset").and_then(|s| s.parse().ok()),
        offset_bottom: attr(e, b"ParagraphBorderBottomOffset").and_then(|s| s.parse().ok()),
        offset_right: attr(e, b"ParagraphBorderRightOffset").and_then(|s| s.parse().ok()),
        width: attr(e, b"ParagraphBorderWidth"),
        corners,
    }
}

/// Parse a [`ParagraphShading`] from the `ParagraphShading*` attrs off a
/// `<ParagraphStyle>` (or `<ParagraphStyleRange>`) element. Returns a
/// fully-default instance when no attrs are present; callers can check `on`
/// to know whether to emit. (De-inherented from `ParagraphShading::from_attrs`
/// — N6.)
pub(crate) fn parse_paragraph_shading(e: &quick_xml::events::BytesStart) -> ParagraphShading {
    ParagraphShading {
        on: attr(e, b"ParagraphShadingOn").and_then(|s| s.parse().ok()),
        color: attr(e, b"ParagraphShadingColor"),
        tint: attr(e, b"ParagraphShadingTint").and_then(|s| s.parse().ok()),
        width: attr(e, b"ParagraphShadingWidth"),
        offset_top: attr(e, b"ParagraphShadingTopOffset").and_then(|s| s.parse().ok()),
        offset_left: attr(e, b"ParagraphShadingLeftOffset").and_then(|s| s.parse().ok()),
        offset_bottom: attr(e, b"ParagraphShadingBottomOffset").and_then(|s| s.parse().ok()),
        offset_right: attr(e, b"ParagraphShadingRightOffset").and_then(|s| s.parse().ok()),
        top_origin: attr(e, b"ParagraphShadingTopOrigin"),
        bottom_origin: attr(e, b"ParagraphShadingBottomOrigin"),
        clip_to_frame: attr(e, b"ParagraphShadingClipToFrame").and_then(|s| s.parse().ok()),
        overprint: attr(e, b"ParagraphShadingOverprint").and_then(|s| s.parse().ok()),
        suppress_printing: attr(e, b"ParagraphShadingSuppressPrinting")
            .and_then(|s| s.parse().ok()),
    }
}

/// Identifies which kind of style is open while we walk
/// `<Properties>` children that carry attributes-as-elements
/// (e.g. `<AppliedFont type="string">…</AppliedFont>`,
/// `<BasedOn type="string">…</BasedOn>`).
#[derive(Debug, Clone, Copy)]
enum CurrentStyle {
    Character,
    Paragraph,
    Object,
    Cell,
    Table,
}

/// Element-form attributes inside `<Properties>` we want to push back
/// into the current style block. Keys are the element name; the
/// next text event lands here.
/// A style's `<Leading>` child: points, or `Auto`.
///
/// `Auto` is a VALUE, not "nothing set": a style that says it over a
/// parent with a fixed leading is auto-leaded in InDesign. The model has
/// no enum for it; a leading of 0 is auto leading to the engine (every
/// consumer takes `> 0` as explicit), and the exporter writes 0 back as
/// `Auto`.
fn style_leading(text: &str) -> Option<f32> {
    let text = text.trim();
    if text == "Auto" {
        return Some(0.0);
    }
    text.parse().ok()
}

/// Which ruby or kenten face a character style's `<Properties>` child names.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MarkFace {
    RubyFont,
    RubyFontStyle,
    KentenFont,
    KentenFontStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CurrentProperty {
    AppliedFont,
    /// `<RubyFont>`, `<RubyFontStyle>`, `<KentenFont>` or
    /// `<KentenFontStyle>` inside a `CharacterStyle`'s `<Properties>`:
    /// the faces ruby and a custom kenten mark print in.
    MarkFace(MarkFace),
    BasedOn,
    /// `<Leading type="unit">13</Leading>` — InDesign's spelling of a
    /// style's leading; the `enumeration` child `Auto` is auto leading,
    /// see [`style_leading`].
    Leading,
    /// `<NumberingExpression type="string">^#.^t</NumberingExpression>`
    /// inside a `ParagraphStyle`'s `<Properties>` block. Paragraph-only.
    NumberingExpression,
    /// `<NumberingFormat type="string">1, 2, 3, 4...</NumberingFormat>` —
    /// the only spelling InDesign reads (the attribute is ignored;
    /// measured 2026-09-06). Paragraph-only.
    NumberingFormat,
    /// `<AppliedNumberingList type="object">NumberingList/X</AppliedNumberingList>`,
    /// likewise. Paragraph-only.
    AppliedNumberingList,
    /// `<SpanSplitColumnCount type="short">2</…>` (or `enumeration` All)
    /// — InDesign's only spelling. Paragraph-only.
    SpanSplitColumnCount,
    /// `<BalanceRaggedLines type="enumeration">FullyBalanced</…>` —
    /// InDesign's only spelling (ADR 153). Paragraph-only.
    BalanceRaggedLines,
    /// `<KinsokuSet type="enumeration">HardKinsoku</…>` (or `Nothing`, no
    /// kinsoku) — the spelling InDesign writes. Paragraph-only.
    KinsokuSet,
    /// `<BulletsCharacterStyle type="object">CharacterStyle/X</…>` — the
    /// only spelling InDesign reads (`list-overrides` o14/o15, 2026-10-01).
    /// Paragraph-only.
    BulletsCharacterStyle,
    /// `<NumberingCharacterStyle type="object">…</…>`, likewise (o16/o17).
    NumberingCharacterStyle,
    /// `<BulletsTextAfter type="string">…</…>` — read as well as the
    /// attribute InDesign itself writes. Paragraph-only.
    BulletsTextAfter,
}

/// One field of a `<TabList>` record (`<ListItem type="record">`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum TabField {
    Alignment,
    AlignmentCharacter,
    Leader,
    Position,
}

impl TabField {
    fn of(name: &[u8]) -> Option<Self> {
        match name {
            b"Alignment" => Some(Self::Alignment),
            b"AlignmentCharacter" => Some(Self::AlignmentCharacter),
            b"Leader" => Some(Self::Leader),
            b"Position" => Some(Self::Position),
            _ => None,
        }
    }
}

pub fn parse_stylesheet(xml: &[u8]) -> Result<StyleSheet, ParseError> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut out = StyleSheet::default();
    let mut buf = Vec::new();
    // Track the open ParagraphStyle's id so nested <TabStop>
    // children attach to the right entry.
    let mut current_paragraph_style: Option<String> = None;
    // Same idea for <CharacterStyle>, used when we read
    // <AppliedFont> as an element inside <Properties>.
    let mut current_character_style: Option<String> = None;
    let mut current_object_style: Option<String> = None;
    let mut current_cell_style: Option<String> = None;
    let mut current_table_style: Option<String> = None;
    // Track an open `<TOCStyle>` so nested `<TOCStyleEntry>` /
    // `<Properties>` text events attach to the right entry. TOC
    // styles aren't part of the cascade-tracking `CurrentStyle`
    // because they don't share the AppliedFont / BasedOn /
    // NumberingExpression property elements the others do.
    let mut current_toc_style: Option<String> = None;
    // Inside an element-form `<TOCStyleEntry>`, and which of its typed
    // `<Properties>` children is open: InDesign's spelling of the entry's
    // format, page-number and separator styles.
    let mut in_toc_entry = false;
    let mut toc_field: Option<&'static str> = None;
    // Track an open element-form `<StripedStrokeStyle>` so its
    // `<Stripe>` children attach to the right definition (W1.2).
    let mut current_stroke_style: Option<String> = None;
    let mut current_style: Option<CurrentStyle> = None;
    let mut pending_property: Option<CurrentProperty> = None;
    // A paragraph style's `<TabList type="list">`, as InDesign spells it:
    // one `<ListItem type="record">` per stop, its fields as typed child
    // elements (the story reader's record form). Open while inside the
    // list; the record being read, whether it named a position, and the
    // field whose text comes next.
    let mut in_tab_list = false;
    let mut pending_tab: Option<(TabStop, bool)> = None;
    let mut tab_field: Option<TabField> = None;
    // The engine's older `<NestedStyle>` children, per paragraph style.
    // InDesign never reads them; they apply only to a style that spells
    // no `<AllNestedStyles>` list (resolved after the walk).
    let mut legacy_nested: std::collections::BTreeMap<String, Vec<NestedStyle>> =
        std::collections::BTreeMap::new();
    // Paragraph styles that spelled `<AllNestedStyles>` (even empty).
    let mut spelled_nested: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => match e.name().as_ref() {
                b"CharacterStyle" => {
                    if let Some(s) = parse_character_style(&e) {
                        current_character_style = Some(s.self_id.clone());
                        current_style = Some(CurrentStyle::Character);
                        out.character_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"ParagraphStyle" => {
                    if let Some(s) = parse_paragraph_style(&e) {
                        current_paragraph_style = Some(s.self_id.clone());
                        current_style = Some(CurrentStyle::Paragraph);
                        out.paragraph_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"ObjectStyle" => {
                    if let Some(s) = parse_object_style(&e) {
                        current_object_style = Some(s.self_id.clone());
                        current_style = Some(CurrentStyle::Object);
                        out.object_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"CellStyle" => {
                    if let Some(s) = parse_cell_style(&e) {
                        current_cell_style = Some(s.self_id.clone());
                        current_style = Some(CurrentStyle::Cell);
                        out.cell_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"TableStyle" => {
                    if let Some(s) = parse_table_style(&e) {
                        current_table_style = Some(s.self_id.clone());
                        current_style = Some(CurrentStyle::Table);
                        out.table_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"TOCStyle" => {
                    if let Some(s) = parse_toc_style(&e) {
                        current_toc_style = Some(s.self_id.clone());
                        out.toc_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"DashedStrokeStyle"
                | b"DottedStrokeStyle"
                | b"StripedStrokeStyle"
                | b"WavyStrokeStyle" => {
                    // Real-world IDMLs emit these as self-closing
                    // (handled in the Empty branch) but the schema
                    // permits child `<Properties>` and `<Stripe>`
                    // children; accept either. Remember the open id
                    // so `<Stripe>` children attach to it (W1.2).
                    if let Some(def) = parse_stroke_style(&e) {
                        current_stroke_style = Some(def.self_id.clone());
                        out.stroke_styles.insert(def.self_id.clone(), def);
                    }
                }
                b"NumberingList" => {
                    if let Some(def) = parse_numbering_list(&e) {
                        out.numbering_lists.insert(def.self_id.clone(), def);
                    }
                }
                b"TOCStyleEntry" => {
                    // Element-form `<TOCStyleEntry>...</TOCStyleEntry>`:
                    // InDesign's own, its format style a `<Properties>`
                    // child read below.
                    if let (Some(id), Some(entry)) =
                        (current_toc_style.as_deref(), parse_toc_style_entry(&e))
                    {
                        if let Some(t) = out.toc_styles.get_mut(id) {
                            t.entries.push(entry);
                            in_toc_entry = true;
                        }
                    }
                }
                b"FormatStyle" if in_toc_entry => toc_field = Some("FormatStyle"),
                b"PageNumberStyle" if in_toc_entry => toc_field = Some("PageNumberStyle"),
                b"SeparatorStyle" if in_toc_entry => toc_field = Some("SeparatorStyle"),
                // Nested, line and GREP styles: typed lists in the
                // style's `<Properties>` (see [`crate::style_lists`]).
                b"AllNestedStyles" | b"AllLineStyles" | b"AllGREPStyles"
                    if matches!(current_style, Some(CurrentStyle::Paragraph)) =>
                {
                    let name = e.name().as_ref().to_vec();
                    let list = crate::style_lists::read_style_list(&mut reader, &name)?;
                    if let Some(p) = current_paragraph_style
                        .as_deref()
                        .and_then(|id| out.paragraph_styles.get_mut(id))
                    {
                        match list {
                            crate::style_lists::StyleList::Nested(l) => {
                                spelled_nested.insert(p.self_id.clone());
                                p.nested_styles = l;
                            }
                            crate::style_lists::StyleList::Line(l) => p.line_styles = l,
                            crate::style_lists::StyleList::Grep(l) => p.grep_styles = l,
                        }
                    }
                }
                b"AppliedFont" if current_style.is_some() => {
                    pending_property = Some(CurrentProperty::AppliedFont);
                }
                b"RubyFont" | b"RubyFontStyle" | b"KentenFont" | b"KentenFontStyle"
                    if matches!(current_style, Some(CurrentStyle::Character)) =>
                {
                    pending_property = Some(CurrentProperty::MarkFace(match e.name().as_ref() {
                        b"RubyFont" => MarkFace::RubyFont,
                        b"RubyFontStyle" => MarkFace::RubyFontStyle,
                        b"KentenFont" => MarkFace::KentenFont,
                        _ => MarkFace::KentenFontStyle,
                    }));
                }
                b"BasedOn" if current_style.is_some() => {
                    pending_property = Some(CurrentProperty::BasedOn);
                }
                b"Leading"
                    if matches!(
                        current_style,
                        Some(CurrentStyle::Paragraph) | Some(CurrentStyle::Character)
                    ) =>
                {
                    pending_property = Some(CurrentProperty::Leading);
                }
                b"NumberingExpression"
                    if matches!(current_style, Some(CurrentStyle::Paragraph)) =>
                {
                    pending_property = Some(CurrentProperty::NumberingExpression);
                }
                b"NumberingFormat" if matches!(current_style, Some(CurrentStyle::Paragraph)) => {
                    pending_property = Some(CurrentProperty::NumberingFormat);
                }
                b"AppliedNumberingList"
                    if matches!(current_style, Some(CurrentStyle::Paragraph)) =>
                {
                    pending_property = Some(CurrentProperty::AppliedNumberingList);
                }
                b"SpanSplitColumnCount"
                    if matches!(current_style, Some(CurrentStyle::Paragraph)) =>
                {
                    pending_property = Some(CurrentProperty::SpanSplitColumnCount);
                }
                b"BalanceRaggedLines" if matches!(current_style, Some(CurrentStyle::Paragraph)) => {
                    pending_property = Some(CurrentProperty::BalanceRaggedLines);
                }
                b"KinsokuSet" if matches!(current_style, Some(CurrentStyle::Paragraph)) => {
                    pending_property = Some(CurrentProperty::KinsokuSet);
                }
                b"BulletsCharacterStyle"
                    if matches!(current_style, Some(CurrentStyle::Paragraph)) =>
                {
                    pending_property = Some(CurrentProperty::BulletsCharacterStyle);
                }
                b"NumberingCharacterStyle"
                    if matches!(current_style, Some(CurrentStyle::Paragraph)) =>
                {
                    pending_property = Some(CurrentProperty::NumberingCharacterStyle);
                }
                b"BulletsTextAfter" if matches!(current_style, Some(CurrentStyle::Paragraph)) => {
                    pending_property = Some(CurrentProperty::BulletsTextAfter);
                }
                b"TabList" if matches!(current_style, Some(CurrentStyle::Paragraph)) => {
                    in_tab_list = true;
                }
                b"ListItem" if in_tab_list => {
                    pending_tab = Some((
                        TabStop {
                            position: 0.0,
                            alignment: None,
                            alignment_character: None,
                            leader: None,
                        },
                        false,
                    ));
                    tab_field = None;
                }
                name if pending_tab.is_some() => {
                    tab_field = TabField::of(name);
                }
                _ => {}
            },
            Event::Text(t) if tab_field.is_some() => {
                let text = t
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .map(|c| c.trim().to_string())
                    .unwrap_or_default();
                if let (Some(field), Some((stop, has_position))) =
                    (tab_field.take(), pending_tab.as_mut())
                {
                    match field {
                        TabField::Position => {
                            if let Ok(v) = text.parse::<f32>() {
                                stop.position = v;
                                *has_position = true;
                            }
                        }
                        // An empty field (`<Leader type="string"></Leader>`)
                        // has no text event at all, so it stays `None` —
                        // the story reader's reading too.
                        _ if text.is_empty() => {}
                        TabField::Alignment => stop.alignment = Some(text),
                        TabField::AlignmentCharacter => stop.alignment_character = Some(text),
                        TabField::Leader => stop.leader = Some(text),
                    }
                }
            }
            Event::Text(t) if toc_field.is_some() => {
                let field = toc_field.take();
                let text = t
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .map(|c| c.trim().to_string())
                    .unwrap_or_default();
                let entry = current_toc_style
                    .as_deref()
                    .and_then(|id| out.toc_styles.get_mut(id))
                    .and_then(|t| t.entries.last_mut());
                if let (Some(entry), false) = (entry, text.is_empty()) {
                    let slot = match field {
                        Some("FormatStyle") => &mut entry.format_style,
                        Some("PageNumberStyle") => &mut entry.page_number_style,
                        _ => &mut entry.separator_style,
                    };
                    // An attribute of the older spelling wins.
                    if slot.is_none() {
                        *slot = Some(text);
                    }
                }
            }
            Event::Text(t) if pending_property.is_some() => {
                let text = t
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .map(|c| c.into_owned())
                    .unwrap_or_default();
                if text.is_empty() {
                    pending_property = None;
                } else {
                    match (current_style, pending_property) {
                        (Some(CurrentStyle::Paragraph), Some(prop)) => {
                            if let Some(id) = current_paragraph_style.as_deref() {
                                if let Some(p) = out.paragraph_styles.get_mut(id) {
                                    match prop {
                                        CurrentProperty::AppliedFont => {
                                            if p.font.is_none() {
                                                p.font = Some(text);
                                            }
                                        }
                                        // Read on character styles only.
                                        CurrentProperty::MarkFace(_) => {}
                                        CurrentProperty::BasedOn => {
                                            if p.based_on.is_none() {
                                                p.based_on =
                                                    Some(qualify_parent("ParagraphStyle", text));
                                            }
                                        }
                                        CurrentProperty::Leading => {
                                            if p.leading.is_none() {
                                                p.leading = style_leading(&text);
                                            }
                                        }
                                        CurrentProperty::NumberingExpression => {
                                            if p.numbering_expression.is_none() {
                                                p.numbering_expression = Some(text);
                                            }
                                        }
                                        CurrentProperty::NumberingFormat => {
                                            p.numbering_format = Some(text);
                                        }
                                        CurrentProperty::SpanSplitColumnCount => {
                                            p.span_columns.count =
                                                paged_model::SpanSplitColumnCount::from_idml(&text);
                                        }
                                        CurrentProperty::BalanceRaggedLines => {
                                            p.line_setting.balance =
                                                paged_model::BalanceRaggedLines::from_idml(&text);
                                        }
                                        CurrentProperty::KinsokuSet => {
                                            p.kinsoku_set = Some(text);
                                        }
                                        // The child beats an attribute of
                                        // ours: InDesign ignores the latter.
                                        CurrentProperty::BulletsCharacterStyle => {
                                            p.bullets_character_style = Some(text);
                                        }
                                        CurrentProperty::NumberingCharacterStyle => {
                                            p.bullets_and_numbering_digits_character_style =
                                                Some(text);
                                        }
                                        CurrentProperty::BulletsTextAfter => {
                                            if p.bullets_text_after.is_none() {
                                                p.bullets_text_after = Some(text);
                                            }
                                        }
                                        CurrentProperty::AppliedNumberingList => {
                                            p.applied_numbering_list = match text.as_str() {
                                                "n" | "NumberingList/n" | "" => None,
                                                v if v.ends_with("[No numbering list]") => None,
                                                _ => Some(text),
                                            };
                                        }
                                    }
                                }
                            }
                        }
                        (Some(CurrentStyle::Character), Some(prop)) => {
                            if let Some(id) = current_character_style.as_deref() {
                                if let Some(c) = out.character_styles.get_mut(id) {
                                    match prop {
                                        CurrentProperty::AppliedFont => {
                                            if c.font.is_none() {
                                                c.font = Some(text);
                                            }
                                        }
                                        CurrentProperty::MarkFace(face) => {
                                            let m = &mut c.marks;
                                            let slot = match face {
                                                MarkFace::RubyFont => &mut m.ruby_font,
                                                MarkFace::RubyFontStyle => &mut m.ruby_font_style,
                                                MarkFace::KentenFont => &mut m.kenten_font,
                                                MarkFace::KentenFontStyle => {
                                                    &mut m.kenten_font_style
                                                }
                                            };
                                            if slot.is_none() {
                                                *slot = Some(text);
                                            }
                                        }
                                        CurrentProperty::BasedOn => {
                                            if c.based_on.is_none() {
                                                c.based_on =
                                                    Some(qualify_parent("CharacterStyle", text));
                                            }
                                        }
                                        CurrentProperty::Leading => {
                                            if c.leading.is_none() {
                                                c.leading = style_leading(&text);
                                            }
                                        }
                                        // NumberingExpression is paragraph-only.
                                        CurrentProperty::NumberingExpression
                                        | CurrentProperty::NumberingFormat
                                        | CurrentProperty::AppliedNumberingList
                                        | CurrentProperty::SpanSplitColumnCount
                                        | CurrentProperty::BalanceRaggedLines
                                        | CurrentProperty::KinsokuSet
                                        | CurrentProperty::BulletsCharacterStyle
                                        | CurrentProperty::NumberingCharacterStyle
                                        | CurrentProperty::BulletsTextAfter => {}
                                    }
                                }
                            }
                        }
                        (Some(CurrentStyle::Object), Some(CurrentProperty::BasedOn)) => {
                            if let Some(id) = current_object_style.as_deref() {
                                if let Some(o) = out.object_styles.get_mut(id) {
                                    if o.based_on.is_none() {
                                        o.based_on = Some(qualify_parent("ObjectStyle", text));
                                    }
                                }
                            }
                        }
                        (Some(CurrentStyle::Cell), Some(CurrentProperty::BasedOn)) => {
                            if let Some(id) = current_cell_style.as_deref() {
                                if let Some(c) = out.cell_styles.get_mut(id) {
                                    if c.based_on.is_none() {
                                        c.based_on = Some(qualify_parent("CellStyle", text));
                                    }
                                }
                            }
                        }
                        (Some(CurrentStyle::Table), Some(CurrentProperty::BasedOn)) => {
                            if let Some(id) = current_table_style.as_deref() {
                                if let Some(t) = out.table_styles.get_mut(id) {
                                    if t.based_on.is_none() {
                                        t.based_on = Some(qualify_parent("TableStyle", text));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    pending_property = None;
                }
            }
            Event::Empty(e) => match e.name().as_ref() {
                // A paragraph style's restart policy: an empty
                // `<Properties>` child carrying attributes (core ADR 158).
                b"NumberingRestartPolicies"
                    if matches!(current_style, Some(CurrentStyle::Paragraph)) =>
                {
                    if let Some(p) = current_paragraph_style
                        .as_deref()
                        .and_then(|id| out.paragraph_styles.get_mut(id))
                    {
                        p.list_level.restart_policy = crate::util::restart_policy(&e);
                    }
                }
                b"CharacterStyle" => {
                    if let Some(s) = parse_character_style(&e) {
                        out.character_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"ParagraphStyle" => {
                    if let Some(s) = parse_paragraph_style(&e) {
                        out.paragraph_styles.insert(s.self_id.clone(), s);
                    }
                }
                // Self-closing forms of the page-item style kinds.
                // IDML's default `[None]` entries ship as
                // `<ObjectStyle Self="..." Name="..." .../>` with
                // no body — without these arms the BTreeMap never
                // populates and `documentCollection:objectStyles`
                // returns empty even though the entries exist.
                b"ObjectStyle" => {
                    if let Some(s) = parse_object_style(&e) {
                        out.object_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"CellStyle" => {
                    if let Some(s) = parse_cell_style(&e) {
                        out.cell_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"TableStyle" => {
                    if let Some(s) = parse_table_style(&e) {
                        out.table_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"TOCStyle" => {
                    // Self-closing `<TOCStyle ... />` — common for
                    // the document's default empty TOCStyle that
                    // carries no entries (real-world IDMLs ship this
                    // even when the document has no TOC).
                    if let Some(s) = parse_toc_style(&e) {
                        out.toc_styles.insert(s.self_id.clone(), s);
                    }
                }
                b"TOCStyleEntry" => {
                    if let (Some(id), Some(entry)) =
                        (current_toc_style.as_deref(), parse_toc_style_entry(&e))
                    {
                        if let Some(t) = out.toc_styles.get_mut(id) {
                            t.entries.push(entry);
                        }
                    }
                }
                b"TabStop" => {
                    if let (Some(id), Some(stop)) = (
                        current_paragraph_style.as_deref(),
                        parse_tab_stop_styles(&e),
                    ) {
                        if let Some(p) = out.paragraph_styles.get_mut(id) {
                            p.tab_list.push(stop);
                        }
                    }
                }
                b"NestedStyle" => {
                    if let (Some(id), Some(ns)) = (
                        current_paragraph_style.as_deref(),
                        parse_legacy_nested_style(&e),
                    ) {
                        legacy_nested.entry(id.to_string()).or_default().push(ns);
                    }
                }
                b"DashedStrokeStyle"
                | b"DottedStrokeStyle"
                | b"StripedStrokeStyle"
                | b"WavyStrokeStyle" => {
                    if let Some(def) = parse_stroke_style(&e) {
                        out.stroke_styles.insert(def.self_id.clone(), def);
                    }
                }
                b"Stripe" => {
                    // A `<Stripe Left=… Width=…/>` child of an open
                    // `<StripedStrokeStyle>` (W1.2). Append in source
                    // order so the renderer's perpendicular offsets
                    // march top→bottom across the stroke width.
                    if let (Some(id), Some(stripe)) =
                        (current_stroke_style.as_deref(), parse_stripe(&e))
                    {
                        if let Some(def) = out.stroke_styles.get_mut(id) {
                            def.stripes.push(stripe);
                        }
                    }
                }
                b"NumberingList" => {
                    if let Some(def) = parse_numbering_list(&e) {
                        out.numbering_lists.insert(def.self_id.clone(), def);
                    }
                }
                b"BulletChar" => {
                    if let (Some(id), Some(cp)) = (
                        current_paragraph_style.as_deref(),
                        attr(&e, b"BulletCharacterValue").and_then(|s| s.parse::<u32>().ok()),
                    ) {
                        if let Some(p) = out.paragraph_styles.get_mut(id) {
                            p.bullet_character = Some(cp);
                        }
                    }
                }
                _ => {}
            },
            Event::End(e) => match e.name().as_ref() {
                b"DashedStrokeStyle"
                | b"DottedStrokeStyle"
                | b"StripedStrokeStyle"
                | b"WavyStrokeStyle" => {
                    current_stroke_style = None;
                }
                b"ParagraphStyle" => {
                    current_paragraph_style = None;
                    in_tab_list = false;
                    pending_tab = None;
                    tab_field = None;
                    if matches!(current_style, Some(CurrentStyle::Paragraph)) {
                        current_style = None;
                    }
                }
                b"ListItem" if in_tab_list => {
                    // A record without a position is no stop (the story
                    // reader's rule).
                    if let (Some((stop, true)), Some(id)) =
                        (pending_tab.take(), current_paragraph_style.as_deref())
                    {
                        if let Some(p) = out.paragraph_styles.get_mut(id) {
                            p.tab_list.push(stop);
                        }
                    }
                    tab_field = None;
                }
                b"TabList" if in_tab_list => {
                    in_tab_list = false;
                    pending_tab = None;
                    tab_field = None;
                }
                _ if pending_tab.is_some() => {
                    // A field closed; an empty one never saw its text.
                    tab_field = None;
                }
                b"CharacterStyle" => {
                    current_character_style = None;
                    if matches!(current_style, Some(CurrentStyle::Character)) {
                        current_style = None;
                    }
                }
                b"ObjectStyle" => {
                    current_object_style = None;
                    if matches!(current_style, Some(CurrentStyle::Object)) {
                        current_style = None;
                    }
                }
                b"CellStyle" => {
                    current_cell_style = None;
                    if matches!(current_style, Some(CurrentStyle::Cell)) {
                        current_style = None;
                    }
                }
                b"TableStyle" => {
                    current_table_style = None;
                    if matches!(current_style, Some(CurrentStyle::Table)) {
                        current_style = None;
                    }
                }
                b"TOCStyle" => {
                    current_toc_style = None;
                }
                b"TOCStyleEntry" => {
                    in_toc_entry = false;
                    toc_field = None;
                }
                b"FormatStyle" | b"PageNumberStyle" | b"SeparatorStyle" => toc_field = None,
                b"AppliedFont"
                | b"BasedOn"
                | b"Leading"
                | b"NumberingExpression"
                | b"BulletsCharacterStyle"
                | b"NumberingCharacterStyle"
                | b"BulletsTextAfter" => {
                    // Pending property is consumed by the next
                    // Text event; clearing here prevents
                    // mismatched-tag leaks if the element was
                    // empty (no text content).
                    pending_property = None;
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    // The legacy spelling stands in only where InDesign's is absent.
    for (id, list) in legacy_nested {
        if spelled_nested.contains(&id) {
            continue;
        }
        if let Some(p) = out.paragraph_styles.get_mut(&id) {
            p.nested_styles = list;
        }
    }
    // Conditions + sets: one reader for every spelling (see
    // [`parse_conditions`]), so Styles.xml and designmap.xml read the
    // same way.
    let (conditions, condition_sets) = parse_conditions(xml)?;
    out.conditions = conditions;
    out.condition_sets = condition_sets;
    Ok(out)
}

/// The conditions and condition sets read from one part, keyed by `Self`.
pub type ConditionTables = (
    std::collections::BTreeMap<String, ConditionDef>,
    std::collections::BTreeMap<String, ConditionSetDef>,
);

/// Parse every `<Condition>` / `<ConditionSet>` in `xml`, wherever it
/// sits — the reader is deliberately position-blind because three
/// spellings exist in the wild and all three must keep opening:
///
/// * InDesign's own (measured on InDesign 20.0.1, 2026-09-05): direct
///   children of `<Document>` in `designmap.xml`, the set's members as
///   `<Properties><SetConditions><VisibilityPair Condition="…"
///   Visibility="true"/>…</SetConditions></Properties>` children;
/// * the engine's pre-2026-09 fixtures: inside `Resources/Styles.xml`,
///   wrapped in an invented `<RootConditionalTextGroup>` (InDesign
///   drops everything inside it), the set's members as a
///   space-separated `Conditions="a b"` attribute (InDesign ignores it);
/// * the same without the wrapper (what InDesign tolerates in
///   Styles.xml).
///
/// The importer accepts all of them; the exporter writes only the
/// first. A `Conditions` attribute and `VisibilityPair` children on the
/// same set merge (deduplicated, attribute order first).
pub fn parse_conditions(xml: &[u8]) -> Result<ConditionTables, ParseError> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut conditions = std::collections::BTreeMap::new();
    let mut sets = std::collections::BTreeMap::new();
    let mut buf = Vec::new();
    // The `<ConditionSet>` currently open (element form), so its
    // `<VisibilityPair>` children attach to it.
    let mut current_set: Option<String> = None;
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_start = matches!(ev, Event::Start(_));
                match e.name().as_ref() {
                    b"Condition" => {
                        if let Some(def) = parse_condition(e) {
                            conditions.insert(def.self_id.clone(), def);
                        }
                    }
                    b"ConditionSet" => {
                        if let Some(def) = parse_condition_set(e) {
                            if is_start {
                                current_set = Some(def.self_id.clone());
                            }
                            sets.insert(def.self_id.clone(), def);
                        }
                    }
                    b"VisibilityPair" => {
                        if let (Some(id), Some(member)) =
                            (current_set.as_deref(), attr(e, b"Condition"))
                        {
                            if let Some(set) = sets.get_mut(id) {
                                // The visibility the set recorded for the
                                // condition: applying the set restores it.
                                if attr(e, b"Visibility").as_deref() == Some("false")
                                    && !set.hidden.contains(&member)
                                {
                                    set.hidden.push(member.clone());
                                }
                                if !set.conditions.contains(&member) {
                                    set.conditions.push(member);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref e) if e.name().as_ref() == b"ConditionSet" => current_set = None,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok((conditions, sets))
}

/// A style's parent as the id the style table is keyed by.
///
/// InDesign names its reserved roots by `$ID` string, without the kind:
/// `<BasedOn type="string">$ID/[No paragraph style]</BasedOn>`, while the
/// root itself is keyed `ParagraphStyle/$ID/[No paragraph style]`. Stored
/// raw, the cascade stopped one step short of the root in every
/// InDesign-authored document (272 of the 305 in the census), so styles
/// fell back to engine defaults instead of the document's own root.
fn qualify_parent(kind: &str, parent: String) -> String {
    if parent.starts_with("$ID/") {
        format!("{kind}/{parent}")
    } else {
        parent
    }
}

/// The ruby and kenten settings a `<CharacterStyleRange>` or
/// `<CharacterStyle>` carries beyond their flag, kind, text and kenten
/// size, as InDesign 20.0.1 spells them (core
/// `tools/indesign-export/ruby-kenten-probe`): `RubyFontSize`,
/// `RubyXScale` / `RubyYScale`, `RubyXOffset` / `RubyYOffset`,
/// `RubyPosition`, `RubyAlignment`, `KentenXScale` / `KentenYScale`,
/// `KentenPlacement`, `KentenPosition`, `KentenAlignment`,
/// `RubyParentSpacing`, `RubyParentOverhangAmount`, `RubyAutoScaling`,
/// `RubyParentScalingPercent`. The faces
/// (`RubyFont`, `KentenFont` and their styles) are `<Properties>`
/// children in InDesign's IDML, as `AppliedFont` is, and are read there.
pub(crate) fn mark_settings(e: &quick_xml::events::BytesStart) -> paged_model::MarkSettings {
    let num = |key: &[u8]| attr(e, key).and_then(|s| s.parse().ok());
    paged_model::MarkSettings {
        ruby_font_size: num(b"RubyFontSize"),
        ruby_x_scale: num(b"RubyXScale"),
        ruby_y_scale: num(b"RubyYScale"),
        ruby_x_offset: num(b"RubyXOffset"),
        ruby_y_offset: num(b"RubyYOffset"),
        ruby_position: attr(e, b"RubyPosition"),
        ruby_alignment: attr(e, b"RubyAlignment"),
        kenten_x_scale: num(b"KentenXScale"),
        kenten_y_scale: num(b"KentenYScale"),
        kenten_placement: num(b"KentenPlacement"),
        kenten_position: attr(e, b"KentenPosition"),
        kenten_alignment: attr(e, b"KentenAlignment"),
        ruby_parent_spacing: attr(e, b"RubyParentSpacing"),
        ruby_overhang_amount: attr(e, b"RubyParentOverhangAmount"),
        ruby_auto_scaling: attr(e, b"RubyAutoScaling").and_then(|s| s.parse().ok()),
        ruby_parent_scaling_percent: num(b"RubyParentScalingPercent"),
        // The faces are read from <Properties>; anything core adds later
        // starts unset here rather than failing to build.
        ..Default::default()
    }
}

fn parse_character_style(e: &quick_xml::events::BytesStart) -> Option<CharacterStyleDef> {
    // `Swatch/None` is IDML's literal for "no stroke" — normalise to
    // None so a `BasedOn` cascade can fall through to a real colour.
    let normalize = |c: Option<String>| match c.as_deref() {
        Some("Swatch/None") | Some("n") | Some("") => None,
        _ => c,
    };
    Some(CharacterStyleDef {
        self_id: attr(e, b"Self")?,
        name: attr(e, b"Name"),
        based_on: attr(e, b"BasedOn").map(|v| qualify_parent("CharacterStyle", v)),
        font: attr(e, b"AppliedFont"),
        font_style: attr(e, b"FontStyle"),
        point_size: attr(e, b"PointSize").and_then(|s| s.parse().ok()),
        leading: attr(e, b"Leading").and_then(|s| s.parse().ok()),
        fill_color: attr(e, b"FillColor"),
        fill_tint: parse_tint_attr(e, b"FillTint"),
        stroke_color: normalize(attr(e, b"StrokeColor")),
        stroke_weight: attr(e, b"StrokeWeight").and_then(|s| s.parse().ok()),
        capitalization: attr(e, b"Capitalization"),
        baseline_shift: attr(e, b"BaselineShift").and_then(|s| s.parse().ok()),
        horizontal_scale: attr(e, b"HorizontalScale").and_then(|s| s.parse().ok()),
        vertical_scale: attr(e, b"VerticalScale").and_then(|s| s.parse().ok()),
        skew: attr(e, b"Skew").and_then(|s| s.parse().ok()),
        position: attr(e, b"Position"),
        tracking: attr(e, b"Tracking").and_then(|s| s.parse().ok()),
        underline: attr(e, b"Underline").and_then(|s| s.parse().ok()),
        strikethru: attr(e, b"StrikeThru").and_then(|s| s.parse().ok()),
        no_break: attr(e, b"NoBreak").and_then(|s| s.parse().ok()),
        overprint_fill: attr(e, b"OverprintFill").and_then(|s| s.parse().ok()),
        overprint_stroke: attr(e, b"OverprintStroke").and_then(|s| s.parse().ok()),
        ruby_flag: attr(e, b"RubyFlag").and_then(|s| crate::util::parse_flag(&s)),
        ruby_type: attr(e, b"RubyType"),
        ruby_string: attr(e, b"RubyString"),
        kenten_kind: attr(e, b"KentenKind"),
        // InDesign 20.0.1 writes a `Custom` kind's character as
        // `KentenCustomCharacter` (core ruby-kenten-probe).
        kenten_character: attr(e, b"KentenCustomCharacter"),
        kenten_font_size: attr(e, b"KentenFontSize").and_then(|s| s.parse().ok()),
        marks: mark_settings(e),
        ligatures_on: attr(e, b"Ligatures").and_then(|s| s.parse().ok()),
        kerning_method: attr(e, b"KerningMethod"),
        otf: crate::story::parse_otf_features(e),
    })
}

fn parse_tab_stop_styles(e: &quick_xml::events::BytesStart) -> Option<TabStop> {
    let position = attr(e, b"Position").and_then(|s| s.parse::<f32>().ok())?;
    Some(TabStop {
        position,
        alignment: attr(e, b"Alignment"),
        alignment_character: attr(e, b"AlignmentCharacter"),
        leader: attr(e, b"Leader"),
    })
}

/// The engine's older spelling of one nested style: a `<NestedStyle
/// AppliedCharacterStyle=… Delimiter=… Repetition=… Inclusive=…/>` child
/// of a `<ParagraphStyle>`. InDesign does not read it (its spelling is
/// the `<AllNestedStyles>` list, [`crate::style_lists`]); it is kept as a
/// legacy read for the packages written with it. Returns None when
/// `AppliedCharacterStyle` is missing (the entry becomes a no-op without
/// an override style).
pub fn parse_legacy_nested_style(e: &quick_xml::events::BytesStart) -> Option<NestedStyle> {
    let applied = attr(e, b"AppliedCharacterStyle")?;
    let delim_str = attr(e, b"Delimiter").unwrap_or_default();
    let delimiter = parse_nested_delimiter(&delim_str);
    let repetition = attr(e, b"Repetition")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(1);
    let inclusive = attr(e, b"Inclusive")
        .and_then(|s| s.parse::<bool>().ok())
        .unwrap_or(true);
    Some(NestedStyle {
        applied_character_style: applied,
        delimiter,
        repetition,
        inclusive,
    })
}

fn parse_nested_delimiter(s: &str) -> NestedDelimiter {
    // IDML serialises a small enum of named delimiters and falls back
    // to literal Unicode codepoints for "single char" cases. Names
    // come from the `Delimiter` attribute in the ParagraphStyle XML.
    match s {
        "Words" => NestedDelimiter::Words,
        "Sentences" => NestedDelimiter::Sentences,
        "Characters" => NestedDelimiter::Characters,
        "ANY_DIGIT" | "AnyDigit" => NestedDelimiter::AnyDigit,
        "ANY_LETTER" | "AnyLetter" => NestedDelimiter::AnyLetter,
        "ANY_DOUBLE_QUOTES" | "AnyDoubleQuotes" => NestedDelimiter::AnyDoubleQuotes,
        "ANY_SINGLE_QUOTES" | "AnySingleQuotes" => NestedDelimiter::AnySingleQuotes,
        "Tab" | "tab" => NestedDelimiter::Tab,
        "ForcedLineBreak" => NestedDelimiter::ForcedLineBreak,
        "EndNestedStyle" => NestedDelimiter::EndNestedStyle,
        // Bare Unicode codepoint string, e.g. ":" or "".
        _ => match s.chars().next() {
            Some(c) if s.chars().count() == 1 => NestedDelimiter::Char(c),
            _ => NestedDelimiter::Unknown,
        },
    }
}

/// Phase 5 — parse one `<Condition>` element from Resources/Styles.xml.
/// Returns `None` when `Self` is missing (the element is unaddressable).
fn parse_condition(e: &quick_xml::events::BytesStart) -> Option<ConditionDef> {
    Some(ConditionDef {
        self_id: attr(e, b"Self")?,
        name: attr(e, b"Name"),
        visible: attr(e, b"Visible").and_then(|s| s.parse().ok()),
        indicator_method: attr(e, b"IndicatorMethod"),
    })
}

fn parse_condition_set(e: &quick_xml::events::BytesStart) -> Option<ConditionSetDef> {
    let self_id = attr(e, b"Self")?;
    let conditions = attr(e, b"Conditions")
        .map(|s| {
            s.split_whitespace()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(ConditionSetDef {
        self_id,
        name: attr(e, b"Name"),
        conditions,
        hidden: Vec::new(),
    })
}

/// Every `<NumberingList>` in a part. InDesign writes them in
/// `designmap.xml` (271 of the 305 census documents, its own `[Default]`
/// included); the engine's fixtures wrote them into Styles.xml.
pub fn parse_numbering_lists_in(xml: &[u8]) -> Result<Vec<NumberingListDef>, ParseError> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"NumberingList" => {
                if let Some(def) = parse_numbering_list(&e) {
                    out.push(def);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// W1.22 — parse one `<NumberingList>` resource. Returns `None` when
/// `Self` is missing (unaddressable). Mirrors `parse_condition`.
fn parse_numbering_list(e: &quick_xml::events::BytesStart) -> Option<NumberingListDef> {
    Some(NumberingListDef {
        self_id: attr(e, b"Self")?,
        name: attr(e, b"Name"),
        continue_across_stories: attr(e, b"ContinueNumbersAcrossStories")
            .and_then(|s| s.parse().ok()),
        continue_across_documents: attr(e, b"ContinueNumbersAcrossDocuments")
            .and_then(|s| s.parse().ok()),
    })
}

fn parse_table_style(e: &quick_xml::events::BytesStart) -> Option<TableStyleDef> {
    let self_id = attr(e, b"Self")?;
    let normalize = |c: Option<String>| match c.as_deref() {
        Some("Swatch/None") | Some("n") | Some("") => None,
        _ => c,
    };
    Some(TableStyleDef {
        self_id,
        name: attr(e, b"Name"),
        based_on: attr(e, b"BasedOn").map(|v| qualify_parent("TableStyle", v)),
        header_region_cell_style: normalize(attr(e, b"HeaderRegionCellStyle")),
        body_region_cell_style: normalize(attr(e, b"BodyRegionCellStyle")),
        footer_region_cell_style: normalize(attr(e, b"FooterRegionCellStyle")),
        left_column_region_cell_style: normalize(attr(e, b"LeftColumnRegionCellStyle")),
        right_column_region_cell_style: normalize(attr(e, b"RightColumnRegionCellStyle")),
        top_border_stroke_color: normalize(attr(e, b"TopBorderStrokeColor")),
        top_border_stroke_weight: attr(e, b"TopBorderStrokeWeight").and_then(|s| s.parse().ok()),
        bottom_border_stroke_color: normalize(attr(e, b"BottomBorderStrokeColor")),
        bottom_border_stroke_weight: attr(e, b"BottomBorderStrokeWeight")
            .and_then(|s| s.parse().ok()),
        left_border_stroke_color: normalize(attr(e, b"LeftBorderStrokeColor")),
        left_border_stroke_weight: attr(e, b"LeftBorderStrokeWeight").and_then(|s| s.parse().ok()),
        right_border_stroke_color: normalize(attr(e, b"RightBorderStrokeColor")),
        right_border_stroke_weight: attr(e, b"RightBorderStrokeWeight")
            .and_then(|s| s.parse().ok()),
        alternating_fills: attr(e, b"AlternatingFills"),
        start_row_fill_color: normalize(attr(e, b"StartRowFillColor")),
        start_row_fill_count: attr(e, b"StartRowFillCount").and_then(|s| s.parse().ok()),
        start_row_fill_tint: parse_tint_attr(e, b"StartRowFillTint"),
        end_row_fill_color: normalize(attr(e, b"EndRowFillColor")),
        end_row_fill_count: attr(e, b"EndRowFillCount").and_then(|s| s.parse().ok()),
        end_row_fill_tint: parse_tint_attr(e, b"EndRowFillTint"),
        skip_first_alternating_fill_rows: attr(e, b"SkipFirstAlternatingFillRows")
            .and_then(|s| s.parse().ok()),
        skip_last_alternating_fill_rows: attr(e, b"SkipLastAlternatingFillRows")
            .and_then(|s| s.parse().ok()),
        start_column_fill_color: normalize(attr(e, b"StartColumnFillColor")),
        start_column_fill_count: attr(e, b"StartColumnFillCount").and_then(|s| s.parse().ok()),
        start_column_fill_tint: parse_tint_attr(e, b"StartColumnFillTint"),
        end_column_fill_color: normalize(attr(e, b"EndColumnFillColor")),
        end_column_fill_count: attr(e, b"EndColumnFillCount").and_then(|s| s.parse().ok()),
        end_column_fill_tint: parse_tint_attr(e, b"EndColumnFillTint"),
        skip_first_alternating_fill_columns: attr(e, b"SkipFirstAlternatingFillColumns")
            .and_then(|s| s.parse().ok()),
        skip_last_alternating_fill_columns: attr(e, b"SkipLastAlternatingFillColumns")
            .and_then(|s| s.parse().ok()),
    })
}

fn parse_cell_style(e: &quick_xml::events::BytesStart) -> Option<CellStyleDef> {
    let self_id = attr(e, b"Self")?;
    let normalize = |c: Option<String>| match c.as_deref() {
        Some("Swatch/None") | Some("n") | Some("") => None,
        _ => c,
    };
    Some(CellStyleDef {
        self_id,
        name: attr(e, b"Name"),
        based_on: attr(e, b"BasedOn").map(|v| qualify_parent("CellStyle", v)),
        fill_color: normalize(attr(e, b"FillColor")),
        vertical_justification: attr(e, b"VerticalJustification"),
        rotation_angle: attr(e, b"RotationAngle").and_then(|s| s.parse().ok()),
        top_edge_stroke_color: normalize(attr(e, b"TopEdgeStrokeColor")),
        top_edge_stroke_weight: attr(e, b"TopEdgeStrokeWeight").and_then(|s| s.parse().ok()),
        bottom_edge_stroke_color: normalize(attr(e, b"BottomEdgeStrokeColor")),
        bottom_edge_stroke_weight: attr(e, b"BottomEdgeStrokeWeight").and_then(|s| s.parse().ok()),
        left_edge_stroke_color: normalize(attr(e, b"LeftEdgeStrokeColor")),
        left_edge_stroke_weight: attr(e, b"LeftEdgeStrokeWeight").and_then(|s| s.parse().ok()),
        right_edge_stroke_color: normalize(attr(e, b"RightEdgeStrokeColor")),
        right_edge_stroke_weight: attr(e, b"RightEdgeStrokeWeight").and_then(|s| s.parse().ok()),
        top_edge_stroke_type: attr(e, b"TopEdgeStrokeType"),
        bottom_edge_stroke_type: attr(e, b"BottomEdgeStrokeType"),
        left_edge_stroke_type: attr(e, b"LeftEdgeStrokeType"),
        right_edge_stroke_type: attr(e, b"RightEdgeStrokeType"),
        // InDesign keeps a cell's first-baseline policy on its style rather
        // than on each <Cell> (core `table-cell-baseline` page 2).
        first_baseline_offset: attr(e, b"FirstBaselineOffset"),
        minimum_first_baseline_offset: attr(e, b"MinimumFirstBaselineOffset")
            .and_then(|s| s.parse().ok()),
        fill_tint: parse_tint_attr(e, b"FillTint"),
        top_edge_stroke_tint: parse_tint_attr(e, b"TopEdgeStrokeTint"),
        bottom_edge_stroke_tint: parse_tint_attr(e, b"BottomEdgeStrokeTint"),
        left_edge_stroke_tint: parse_tint_attr(e, b"LeftEdgeStrokeTint"),
        right_edge_stroke_tint: parse_tint_attr(e, b"RightEdgeStrokeTint"),
    })
}

fn parse_object_style(e: &quick_xml::events::BytesStart) -> Option<ObjectStyleDef> {
    let self_id = attr(e, b"Self")?;
    let stroke_weight = attr(e, b"StrokeWeight").and_then(|s| s.parse().ok());
    // IDML stores "no stroke" as the literal "Swatch/None"; treat
    // that as missing so the cascade can fall through to a real
    // colour from BasedOn rather than handing the renderer a paint
    // it can't resolve.
    let normalize = |c: Option<String>| match c.as_deref() {
        Some("Swatch/None") | Some("n") | Some("") => None,
        _ => c,
    };
    Some(ObjectStyleDef {
        self_id,
        name: attr(e, b"Name"),
        based_on: attr(e, b"BasedOn").map(|v| qualify_parent("ObjectStyle", v)),
        fill_color: normalize(attr(e, b"FillColor")),
        fill_tint: parse_tint_attr(e, b"FillTint"),
        stroke_color: normalize(attr(e, b"StrokeColor")),
        stroke_tint: parse_tint_attr(e, b"StrokeTint"),
        stroke_weight,
        corner_radius: attr(e, b"CornerRadius").and_then(|s| s.parse().ok()),
        corner_option: attr(e, b"CornerOption"),
        corners: crate::spread::read_corners(e),
        enable: ObjectStyleCategories {
            fill: flag(e, b"EnableFill"),
            stroke: flag(e, b"EnableStroke"),
            stroke_and_corner_options: flag(e, b"EnableStrokeAndCornerOptions"),
            paragraph_style: flag(e, b"EnableParagraphStyle"),
            text_frame_general_options: flag(e, b"EnableTextFrameGeneralOptions"),
            text_frame_baseline_options: flag(e, b"EnableTextFrameBaselineOptions"),
            text_frame_auto_sizing_options: flag(e, b"EnableTextFrameAutoSizingOptions"),
            text_wrap_and_others: flag(e, b"EnableTextWrapAndOthers"),
            anchored_object_options: flag(e, b"EnableAnchoredObjectOptions"),
            frame_fitting_options: flag(e, b"EnableFrameFittingOptions"),
        },
        // Read from its `<TextFramePreference>`, `<TextWrapPreference>`
        // and `<AnchoredObjectSetting>` children by
        // `read_object_style_text_frames`.
        text_frame: None,
        text_wrap: None,
        anchored: None,
    })
}

fn flag(e: &quick_xml::events::BytesStart, name: &[u8]) -> Option<bool> {
    attr(e, name).and_then(|v| v.parse().ok())
}

/// The `<TextFramePreference>` of each `<ObjectStyle>` in a styles part:
/// vertical justification, insets (InDesign's typed `<InsetSpacing>`
/// list, or the attribute), columns, first baseline and auto-sizing.
/// InDesign leaves the options a frame takes from its object style off
/// the frame (asked of InDesign 20.0.1: a frame of a style with
/// `VerticalJustification="CenterAlign"` is written without it), so the
/// style is where they are.
pub fn read_object_style_text_frames(
    xml: &[u8],
    styles: &mut std::collections::BTreeMap<String, ObjectStyleDef>,
) -> Result<(), ParseError> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    // The `<ObjectStyle>` being read, whether its `<TextFramePreference>`
    // is open, and the `<InsetSpacing>` list items read so far.
    let mut style: Option<String> = None;
    let mut in_tfp = false;
    let mut in_twp = false;
    let mut insets: Option<Vec<f32>> = None;
    let mut in_item = false;
    loop {
        let event = reader.read_event_into(&mut buf)?;
        let started = matches!(event, Event::Start(_));
        match event {
            Event::Eof => break,
            Event::Start(e) if e.name().as_ref() == b"ObjectStyle" => {
                style = attr(&e, b"Self");
            }
            Event::End(e) if e.name().as_ref() == b"ObjectStyle" => style = None,
            Event::Start(e) | Event::Empty(e)
                if style.is_some() && e.name().as_ref() == b"TextFramePreference" =>
            {
                let o = TextFrameOptions {
                    vertical_justification: attr(&e, b"VerticalJustification")
                        .as_deref()
                        .and_then(VerticalJustification::from_idml),
                    inset_spacing: attr(&e, b"InsetSpacing").and_then(|s| four(&s)),
                    column_count: attr(&e, b"TextColumnCount").and_then(|s| s.parse().ok()),
                    column_gutter: attr(&e, b"TextColumnGutter").and_then(|s| s.parse().ok()),
                    first_baseline_offset: attr(&e, b"FirstBaselineOffset")
                        .as_deref()
                        .and_then(FirstBaselineOffset::from_idml),
                    minimum_first_baseline_offset: attr(&e, b"MinimumFirstBaselineOffset")
                        .and_then(|s| s.parse().ok()),
                    auto_sizing: attr(&e, b"AutoSizingType")
                        .as_deref()
                        .and_then(AutoSizingType::from_idml),
                };
                if let Some(s) = style.as_deref().and_then(|id| styles.get_mut(id)) {
                    s.text_frame = Some(o);
                }
                in_tfp = true;
            }
            Event::End(e) if e.name().as_ref() == b"TextFramePreference" => in_tfp = false,
            // The style's own text wrap and anchored-object options: an item
            // of the style that states none takes them, with the category
            // on (asked of InDesign 20.0.1, ADR 143).
            Event::Start(e) | Event::Empty(e)
                if style.is_some() && e.name().as_ref() == b"TextWrapPreference" =>
            {
                let wrap = paged_model::TextWrap {
                    mode: attr(&e, b"TextWrapMode")
                        .as_deref()
                        .map(paged_model::TextWrapMode::from_idml)
                        .unwrap_or(paged_model::TextWrapMode::None),
                    offsets: [0.0; 4],
                    invert: attr(&e, b"Inverse")
                        .or_else(|| attr(&e, b"Inverted"))
                        .and_then(|s| s.parse::<bool>().ok()),
                    contour_type: None,
                    include_inside_edges: None,
                    side: attr(&e, b"TextWrapSide")
                        .as_deref()
                        .and_then(paged_model::TextWrapSide::from_idml),
                    apply_to_master_page_only: attr(&e, b"ApplyToMasterPageOnly")
                        .and_then(|s| s.parse::<bool>().ok()),
                };
                if let Some(s) = style.as_deref().and_then(|id| styles.get_mut(id)) {
                    s.text_wrap = Some(wrap);
                }
                in_twp = started;
            }
            Event::End(e) if e.name().as_ref() == b"TextWrapPreference" => in_twp = false,
            Event::Start(e) | Event::Empty(e)
                if in_twp && e.name().as_ref() == b"TextWrapOffset" =>
            {
                let side = |k: &[u8]| attr(&e, k).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let offsets = [side(b"Top"), side(b"Left"), side(b"Bottom"), side(b"Right")];
                if let Some(w) = style
                    .as_deref()
                    .and_then(|id| styles.get_mut(id))
                    .and_then(|s| s.text_wrap.as_mut())
                {
                    w.offsets = offsets;
                }
            }
            Event::Start(e) | Event::Empty(e)
                if in_twp && e.name().as_ref() == b"ContourOption" =>
            {
                if let Some(w) = style
                    .as_deref()
                    .and_then(|id| styles.get_mut(id))
                    .and_then(|s| s.text_wrap.as_mut())
                {
                    w.contour_type = attr(&e, b"ContourType")
                        .as_deref()
                        .map(paged_model::ContourOptionType::from_idml);
                    w.include_inside_edges =
                        attr(&e, b"IncludeInsideEdges").and_then(|s| s.parse::<bool>().ok());
                }
            }
            Event::Start(e) | Event::Empty(e)
                if style.is_some() && e.name().as_ref() == b"AnchoredObjectSetting" =>
            {
                let setting = crate::story::parse_anchored_object_setting(&e);
                if let Some(s) = style.as_deref().and_then(|id| styles.get_mut(id)) {
                    s.anchored = Some(setting);
                }
            }
            Event::Start(e) if in_tfp && e.name().as_ref() == b"InsetSpacing" => {
                insets = Some(Vec::new());
            }
            Event::Start(e) if insets.is_some() && e.name().as_ref() == b"ListItem" => {
                in_item = true;
            }
            Event::End(e) if e.name().as_ref() == b"ListItem" => in_item = false,
            Event::Text(t) if in_item => {
                let v = t
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .unwrap_or_default();
                if let (Some(list), Ok(n)) = (insets.as_mut(), v.trim().parse::<f32>()) {
                    list.push(n);
                }
            }
            Event::End(e) if e.name().as_ref() == b"InsetSpacing" => {
                let list = insets.take().unwrap_or_default();
                if let ([a, b, c, d], Some(s)) = (
                    list.as_slice(),
                    style.as_deref().and_then(|id| styles.get_mut(id)),
                ) {
                    if let Some(tf) = s.text_frame.as_mut() {
                        tf.inset_spacing = Some([*a, *b, *c, *d]);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn four(s: &str) -> Option<[f32; 4]> {
    let v: Vec<f32> = s
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect();
    (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
}

// See `parse_toc_style_entry`: the struct update keeps this literal
// compiling as the model grows.
#[allow(clippy::needless_update)]
fn parse_toc_style(e: &quick_xml::events::BytesStart) -> Option<TOCStyleDef> {
    Some(TOCStyleDef {
        self_id: attr(e, b"Self")?,
        name: attr(e, b"Name"),
        title: attr(e, b"Title"),
        title_style: attr(e, b"TitleStyle"),
        include_book_documents: attr(e, b"IncludeBookDocuments").and_then(|s| s.parse().ok()),
        include_hidden: attr(e, b"IncludeHidden").and_then(|s| s.parse().ok()),
        run_in: attr(e, b"RunIn").and_then(|s| s.parse().ok()),
        entries: Vec::new(),
        create_bookmarks: attr(e, b"CreateBookmarks").and_then(|s| s.parse().ok()),
        ..TOCStyleDef::default()
    })
}

/// A `<TOCStyleEntry>` start tag. InDesign 20.0.1 writes the style an
/// entry collects as the entry's `Name` (the paragraph style's name, no
/// `IncludeStyle`), its format style as a `<Properties>` child (read by the
/// walker), where the page number stands as `PageNumberPosition`
/// (`AfterEntry`, `BeforeEntry`, `None`) and the separator behind `$ID/`
/// (`$ID/^t`). The engine's older attributes (`IncludeStyle`,
/// `FormatStyle`, `PageNumber`) still read, and win where present.
// The model grows TOC entry fields core-first; the struct update keeps this
// literal compiling against the next model as well as this one.
#[allow(clippy::needless_update)]
fn parse_toc_style_entry(e: &quick_xml::events::BytesStart) -> Option<TOCStyleEntryDef> {
    let name = attr(e, b"Name");
    let include_style = attr(e, b"IncludeStyle").or_else(|| {
        name.as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| format!("ParagraphStyle/{}", n.replace(':', "%3a")))
    });
    let page_number = attr(e, b"PageNumber").or_else(|| {
        (attr(e, b"PageNumberPosition").as_deref() == Some("None"))
            .then(|| "NoPageNumber".to_string())
    });
    let separator = attr(e, b"Separator").map(|s| match s.strip_prefix("$ID/") {
        Some(rest) => rest.to_string(),
        None => s,
    });
    Some(TOCStyleEntryDef {
        name,
        include_style,
        format_style: attr(e, b"FormatStyle"),
        level: attr(e, b"Level").and_then(|s| s.parse().ok()),
        page_number,
        separator,
        page_number_position: attr(e, b"PageNumberPosition"),
        sort_alphabet: attr(e, b"SortAlphabet").and_then(|s| s.parse().ok()),
        ..TOCStyleEntryDef::default()
    })
}

/// Track 4a / W1.2: parse a `<DashedStrokeStyle>` / `<DottedStrokeStyle>`
/// / `<StripedStrokeStyle>` / `<WavyStrokeStyle>` element. Pulls the
/// `Self` id, the `Pattern` attribute (dashed/dotted) as a list of
/// on/off lengths in pt, and the `<WavyStrokeStyle>` `Width` /
/// `Wavelength` ratios. `<Stripe>` children are merged in afterward by
/// the element walker. Returns `None` only when `Self` is missing —
/// unrecognised element shapes are still useful to remember.
fn parse_stroke_style(e: &quick_xml::events::BytesStart) -> Option<StrokeStyleDef> {
    let self_id = attr(e, b"Self")?;
    let kind = match e.name().as_ref() {
        b"DashedStrokeStyle" => StrokeStyleKind::Dashed,
        b"DottedStrokeStyle" => StrokeStyleKind::Dotted,
        b"StripedStrokeStyle" => StrokeStyleKind::Striped,
        b"WavyStrokeStyle" => StrokeStyleKind::Wavy,
        _ => return None,
    };
    // InDesign writes the dashes as `DashArray`, in points (asked of
    // InDesign 20.0.1: a style made with dashArray [12, 3, 3, 3] mm exports
    // `DashArray="34.0157… 8.5039… 8.5039… 8.5039…"`); `Pattern` is the
    // engine's older spelling.
    let pattern = attr(e, b"Pattern")
        .or_else(|| attr(e, b"DashArray"))
        .map(|s| {
            s.split_ascii_whitespace()
                .filter_map(|tok| tok.parse::<f32>().ok())
                .collect()
        })
        .unwrap_or_default();
    Some(StrokeStyleDef {
        self_id,
        name: attr(e, b"Name"),
        kind,
        pattern,
        stripes: Vec::new(),
        wave_width: attr(e, b"Width").and_then(|s| s.parse().ok()),
        wave_length: attr(e, b"Wavelength").and_then(|s| s.parse().ok()),
        gap_color: match attr(e, b"GapColor").as_deref() {
            Some("Swatch/None") | Some("n") | Some("") | None => None,
            _ => attr(e, b"GapColor"),
        },
        gap_tint: attr(e, b"GapTint").and_then(|s| s.parse().ok()),
    })
}

/// W1.2: parse one `<Stripe Left="…" Width="…"/>` child of a
/// `<StripedStrokeStyle>`. Both attributes are InDesign 0..1 ratios of
/// the total stroke weight. Returns `None` when `Width` is absent (a
/// zero-width stripe paints nothing).
fn parse_stripe(e: &quick_xml::events::BytesStart) -> Option<StripeDef> {
    let width = attr(e, b"Width").and_then(|s| s.parse::<f32>().ok())?;
    let left = attr(e, b"Left")
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.0);
    Some(StripeDef { left, width })
}

// `needless_update`: the `..Default::default()` tail is deliberate (see the
// literal); it is redundant only while every model field is listed.
#[allow(clippy::needless_update)]
fn parse_paragraph_style(e: &quick_xml::events::BytesStart) -> Option<ParagraphStyleDef> {
    // `Swatch/None` is IDML's literal for "no stroke" — normalise to
    // None so a `BasedOn` cascade can fall through to a real colour.
    let normalize = |c: Option<String>| match c.as_deref() {
        Some("Swatch/None") | Some("n") | Some("") => None,
        _ => c,
    };
    Some(ParagraphStyleDef {
        self_id: attr(e, b"Self")?,
        name: attr(e, b"Name"),
        based_on: attr(e, b"BasedOn").map(|v| qualify_parent("ParagraphStyle", v)),
        font: attr(e, b"AppliedFont"),
        font_style: attr(e, b"FontStyle"),
        point_size: attr(e, b"PointSize").and_then(|s| s.parse().ok()),
        leading: attr(e, b"Leading").and_then(|s| s.parse().ok()),
        fill_color: attr(e, b"FillColor"),
        fill_tint: parse_tint_attr(e, b"FillTint"),
        stroke_color: normalize(attr(e, b"StrokeColor")),
        stroke_weight: attr(e, b"StrokeWeight").and_then(|s| s.parse().ok()),
        capitalization: attr(e, b"Capitalization"),
        baseline_shift: attr(e, b"BaselineShift").and_then(|s| s.parse().ok()),
        horizontal_scale: attr(e, b"HorizontalScale").and_then(|s| s.parse().ok()),
        vertical_scale: attr(e, b"VerticalScale").and_then(|s| s.parse().ok()),
        skew: attr(e, b"Skew").and_then(|s| s.parse().ok()),
        position: attr(e, b"Position"),
        tracking: attr(e, b"Tracking").and_then(|s| s.parse().ok()),
        justification: attr(e, b"Justification")
            .as_deref()
            .and_then(Justification::from_idml),
        first_line_indent: attr(e, b"FirstLineIndent").and_then(|s| s.parse().ok()),
        left_indent: attr(e, b"LeftIndent").and_then(|s| s.parse().ok()),
        right_indent: attr(e, b"RightIndent").and_then(|s| s.parse().ok()),
        space_before: attr(e, b"SpaceBefore").and_then(|s| s.parse().ok()),
        space_after: attr(e, b"SpaceAfter").and_then(|s| s.parse().ok()),
        underline: attr(e, b"Underline").and_then(|s| s.parse().ok()),
        strikethru: attr(e, b"StrikeThru").and_then(|s| s.parse().ok()),
        tab_list: Vec::new(),
        bullets_list_type: attr(e, b"BulletsAndNumberingListType"),
        bullet_character: None,
        bullets_text_after: attr(e, b"BulletsTextAfter"),
        numbering_format: attr(e, b"NumberingFormat"),
        bullets_character_style: attr(e, b"BulletsCharacterStyle"),
        bullets_and_numbering_digits_character_style: attr(
            e,
            b"BulletsAndNumberingDigitsCharacterStyle",
        ),
        numbering_expression: attr(e, b"NumberingExpression"),
        numbering_start_at: attr(e, b"NumberingStartAt").and_then(|s| s.parse().ok()),
        numbering_continue: attr(e, b"NumberingContinue").and_then(|s| s.parse().ok()),
        // W1.22 — `n` and the `[No numbering list]` default both mean
        // "no named list"; normalise so a `BasedOn` cascade doesn't
        // pin a paragraph to the sentinel.
        applied_numbering_list: match attr(e, b"AppliedNumberingList").as_deref() {
            Some("n") | Some("NumberingList/n") | Some("") => None,
            Some(s) if s.ends_with("[No numbering list]") => None,
            _ => attr(e, b"AppliedNumberingList"),
        },
        next_style: attr(e, b"NextStyle"),
        hyphenation: attr(e, b"Hyphenation").and_then(|s| s.parse().ok()),
        hyphenation_zone: attr(e, b"HyphenationZone").and_then(|s| s.parse().ok()),
        hyphenate_after_first: attr(e, b"HyphenateAfterFirst").and_then(|s| s.parse().ok()),
        hyphenate_before_last: attr(e, b"HyphenateBeforeLast").and_then(|s| s.parse().ok()),
        hyphenate_words_longer_than: attr(e, b"HyphenateWordsLongerThan")
            .and_then(|s| s.parse().ok()),
        hyphenate_capitalized_words: attr(e, b"HyphenateCapitalizedWords")
            .and_then(|s| s.parse().ok()),
        hyphenate_last_word: attr(e, b"HyphenateLastWord").and_then(|s| s.parse().ok()),
        hyphenate_across_columns: attr(e, b"HyphenateAcrossColumns").and_then(|s| s.parse().ok()),
        hyphenate_ladder_limit: attr(e, b"HyphenateLadderLimit").and_then(|s| s.parse().ok()),
        hyphen_weight: attr(e, b"HyphenWeight").and_then(|s| s.parse().ok()),
        composer: attr(e, b"Composer").map(|s| paged_model::Composer::from_idml(&s)),
        // ADR 028 — keep options live on paragraph STYLES in real documents
        // (a heading style's KeepWithNext), so the style must carry them.
        keep_lines_together: attr(e, b"KeepLinesTogether").and_then(|s| s.parse().ok()),
        keep_all_lines_together: attr(e, b"KeepAllLinesTogether").and_then(|s| s.parse().ok()),
        keep_first_lines: attr(e, b"KeepFirstLines").and_then(|s| s.parse().ok()),
        keep_last_lines: attr(e, b"KeepLastLines").and_then(|s| s.parse().ok()),
        keep_with_next: attr(e, b"KeepWithNext").and_then(|s| s.parse().ok()),
        start_paragraph: attr(e, b"StartParagraph")
            .and_then(|s| paged_model::StartParagraph::from_idml(&s)),
        span_columns: paged_model::SpanColumns {
            column_type: attr(e, b"SpanColumnType")
                .and_then(|s| paged_model::SpanColumnType::from_idml(&s)),
            count: None,
            min_space_before: attr(e, b"SpanColumnMinSpaceBefore").and_then(|s| s.parse().ok()),
            min_space_after: attr(e, b"SpanColumnMinSpaceAfter").and_then(|s| s.parse().ok()),
            inside_gutter: attr(e, b"SplitColumnInsideGutter").and_then(|s| s.parse().ok()),
            outside_gutter: attr(e, b"SplitColumnOutsideGutter").and_then(|s| s.parse().ok()),
        },
        line_setting: crate::util::line_setting_attrs(e),
        list_level: crate::util::list_level_attrs(e),
        // Align to baseline grid: real documents set it on the body
        // style, not on each range.
        grid_alignment: attr(e, b"GridAlignment")
            .and_then(|s| paged_model::GridAlignment::from_idml(&s)),
        grid_align_first_line_only: attr(e, b"GridAlignFirstLineOnly").and_then(|s| s.parse().ok()),
        applied_language: attr(e, b"AppliedLanguage"),
        minimum_word_spacing: attr(e, b"MinimumWordSpacing").and_then(|s| s.parse().ok()),
        desired_word_spacing: attr(e, b"DesiredWordSpacing").and_then(|s| s.parse().ok()),
        maximum_word_spacing: attr(e, b"MaximumWordSpacing").and_then(|s| s.parse().ok()),
        minimum_letter_spacing: attr(e, b"MinimumLetterSpacing").and_then(|s| s.parse().ok()),
        desired_letter_spacing: attr(e, b"DesiredLetterSpacing").and_then(|s| s.parse().ok()),
        maximum_letter_spacing: attr(e, b"MaximumLetterSpacing").and_then(|s| s.parse().ok()),
        minimum_glyph_scaling: attr(e, b"MinimumGlyphScaling").and_then(|s| s.parse().ok()),
        desired_glyph_scaling: attr(e, b"DesiredGlyphScaling").and_then(|s| s.parse().ok()),
        maximum_glyph_scaling: attr(e, b"MaximumGlyphScaling").and_then(|s| s.parse().ok()),
        drop_cap_characters: attr(e, b"DropCapCharacters").and_then(|s| s.parse().ok()),
        drop_cap_lines: attr(e, b"DropCapLines").and_then(|s| s.parse().ok()),
        // `DropcapDetail` is InDesign's spelling; `DropCapDetail` what Paged once wrote.
        drop_cap_detail: attr(e, b"DropcapDetail")
            .or_else(|| attr(e, b"DropCapDetail"))
            .and_then(|s| s.parse().ok()),
        overprint_fill: attr(e, b"OverprintFill").and_then(|s| s.parse().ok()),
        overprint_stroke: attr(e, b"OverprintStroke").and_then(|s| s.parse().ok()),
        kinsoku_set: attr(e, b"KinsokuSet"),
        kinsoku_type: attr(e, b"KinsokuType"),
        mojikumi_table: attr(e, b"MojikumiTable"),
        mojikumi_set: attr(e, b"MojikumiSet"),
        shading: parse_paragraph_shading(e),
        rule_above: parse_paragraph_rule(e, "RuleAbove"),
        rule_below: parse_paragraph_rule(e, "RuleBelow"),
        border: parse_paragraph_border(e),
        // `Ligatures`, `KerningMethod` and the `OTF*` toggles: InDesign
        // writes run formatting on paragraph styles as on character
        // styles (asked 2026-10-12: `Ligatures="false"
        // OTFHistorical="true" OTFStylisticAlternate="true"
        // KerningMethod="$ID/Optical"` on a `<ParagraphStyle>`). The
        // kerning value keeps its `$ID/` prefix, as the character
        // style's does.
        ligatures_on: attr(e, b"Ligatures").and_then(|s| s.parse().ok()),
        kerning_method: attr(e, b"KerningMethod"),
        otf: crate::story::parse_otf_features(e),
        // Filled by the `<AllNestedStyles>` / `<AllLineStyles>` /
        // `<AllGREPStyles>` children (or the legacy `<NestedStyle>`).
        nested_styles: Vec::new(),
        // `true` on a style whose list is empty — one that cleared its
        // parent's — and `false` on one that inherits it.
        empty_nested_styles: attr(e, b"EmptyNestedStyles").and_then(|s| s.parse().ok()),
        empty_line_styles: attr(e, b"EmptyLineStyles").and_then(|s| s.parse().ok()),
        empty_grep_styles: attr(e, b"EmptyGrepStyles").and_then(|s| s.parse().ok()),
        // Fields this parser does not read yet fall back to their default;
        // lets the core model grow (e.g. keep options, ADR 028) without
        // breaking this crate at a pinned rev.
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <RootCharacterStyleGroup>
    <CharacterStyle Self="CharacterStyle/Base"
                    Name="Base"
                    AppliedFont="Body Font"
                    PointSize="11"
                    FillColor="Color/Black"/>
    <CharacterStyle Self="CharacterStyle/Bold"
                    Name="Bold"
                    BasedOn="CharacterStyle/Base"
                    FontStyle="Bold"/>
  </RootCharacterStyleGroup>
  <RootParagraphStyleGroup>
    <ParagraphStyle Self="ParagraphStyle/Body"
                    Name="Body"
                    AppliedFont="Body Font"
                    PointSize="11"
                    Justification="LeftAlign"
                    SpaceAfter="6"/>
    <ParagraphStyle Self="ParagraphStyle/Heading"
                    Name="Heading"
                    BasedOn="ParagraphStyle/Body"
                    PointSize="22"
                    FontStyle="Bold"/>
  </RootParagraphStyleGroup>
</idPkg:Styles>"#;

    /// InDesign keeps a cell's first-baseline policy on its cell style
    /// (core `table-cell-baseline` page 2); the reader takes it there, and
    /// the BasedOn cascade carries it to a child style.
    #[test]
    fn a_cell_styles_first_baseline_policy_is_read_and_cascades() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x">
          <RootCellStyleGroup>
            <CellStyle Self="CellStyle/Cap" Name="Cap"
                       FirstBaselineOffset="CapHeight" MinimumFirstBaselineOffset="30"/>
            <CellStyle Self="CellStyle/Child" Name="Child" BasedOn="CellStyle/Cap"/>
          </RootCellStyleGroup>
        </idPkg:Styles>"#;
        let sheet = parse_stylesheet(xml).unwrap();
        let cap = &sheet.cell_styles["CellStyle/Cap"];
        assert_eq!(cap.first_baseline_offset.as_deref(), Some("CapHeight"));
        assert_eq!(cap.minimum_first_baseline_offset, Some(30.0));
        let child = sheet.resolve_cell("CellStyle/Child");
        assert_eq!(child.first_baseline_offset.as_deref(), Some("CapHeight"));
        assert_eq!(child.minimum_first_baseline_offset, Some(30.0));
    }

    #[test]
    fn span_and_split_columns_parse_and_cascade_on_styles() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Head" SpanColumnType="SpanColumns" SpanColumnMinSpaceAfter="9">
              <Properties><SpanSplitColumnCount type="short">2</SpanSplitColumnCount></Properties>
            </ParagraphStyle>
            <ParagraphStyle Self="ParagraphStyle/Sub" SpanColumnMinSpaceBefore="4">
              <Properties><BasedOn type="object">ParagraphStyle/Head</BasedOn></Properties>
            </ParagraphStyle>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let sub = s.resolve_paragraph("ParagraphStyle/Sub");
        assert_eq!(
            sub.span_columns.column_type,
            Some(paged_model::SpanColumnType::SpanColumns)
        );
        assert_eq!(
            sub.span_columns.count,
            Some(paged_model::SpanSplitColumnCount::Count(2))
        );
        assert_eq!(sub.span_columns.min_space_after, Some(9.0));
        assert_eq!(sub.span_columns.min_space_before, Some(4.0));
    }

    #[test]
    fn parses_styles_table() {
        let s = parse_stylesheet(SAMPLE).unwrap();
        assert_eq!(s.character_styles.len(), 2);
        assert_eq!(s.paragraph_styles.len(), 2);
        let bold = s.character_styles.get("CharacterStyle/Bold").unwrap();
        assert_eq!(bold.based_on.as_deref(), Some("CharacterStyle/Base"));
        assert_eq!(bold.font_style.as_deref(), Some("Bold"));
    }

    #[test]
    fn resolve_character_walks_based_on_chain() {
        let s = parse_stylesheet(SAMPLE).unwrap();
        let r = s.resolve_character("CharacterStyle/Bold");
        // FontStyle from Bold itself; AppliedFont + PointSize +
        // FillColor inherited from Base.
        assert_eq!(r.font_style.as_deref(), Some("Bold"));
        assert_eq!(r.font.as_deref(), Some("Body Font"));
        assert_eq!(r.point_size, Some(11.0));
        assert_eq!(r.fill_color.as_deref(), Some("Color/Black"));
    }

    #[test]
    fn resolve_paragraph_walks_based_on_chain() {
        let s = parse_stylesheet(SAMPLE).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Heading");
        assert_eq!(r.point_size, Some(22.0)); // override
        assert_eq!(r.font.as_deref(), Some("Body Font")); // inherited
        assert_eq!(r.justification, Some(Justification::LeftAlign));
        assert_eq!(r.space_after, Some(6.0));
    }

    #[test]
    fn parses_and_cascades_the_composer() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Body" Composer="HL Single"/>
            <ParagraphStyle Self="ParagraphStyle/Sub" BasedOn="ParagraphStyle/Body"/>
            <ParagraphStyle Self="ParagraphStyle/Plugin" Composer="Some Plugin Composer"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        assert_eq!(
            s.resolve_paragraph("ParagraphStyle/Body").composer,
            Some(paged_model::Composer::SingleLine)
        );
        assert_eq!(
            s.resolve_paragraph("ParagraphStyle/Sub").composer,
            Some(paged_model::Composer::SingleLine)
        );
        // A composer InDesign did not write is kept verbatim.
        assert_eq!(
            s.resolve_paragraph("ParagraphStyle/Plugin")
                .composer
                .map(|c| c.as_idml().to_string()),
            Some("Some Plugin Composer".to_string())
        );
    }

    #[test]
    fn parses_and_cascades_hyphenation_zone() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Body"
                            Hyphenation="true"
                            HyphenationZone="36"/>
            <ParagraphStyle Self="ParagraphStyle/Sub"
                            BasedOn="ParagraphStyle/Body"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        // Direct value parses.
        let body = s.resolve_paragraph("ParagraphStyle/Body");
        assert_eq!(body.hyphenation_zone, Some(36.0));
        // BasedOn child with no own zone inherits it.
        let sub = s.resolve_paragraph("ParagraphStyle/Sub");
        assert_eq!(sub.hyphenation_zone, Some(36.0));
    }

    #[test]
    fn reads_drop_cap_detail_in_indesigns_spelling() {
        // InDesign writes `DropcapDetail`; older Paged builds wrote
        // `DropCapDetail`. Both read, and the value cascades.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Body" DropcapDetail="1"/>
            <ParagraphStyle Self="ParagraphStyle/Sub" BasedOn="ParagraphStyle/Body"/>
            <ParagraphStyle Self="ParagraphStyle/Old" DropCapDetail="2"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        assert_eq!(
            s.resolve_paragraph("ParagraphStyle/Body").drop_cap_detail,
            Some(1)
        );
        assert_eq!(
            s.resolve_paragraph("ParagraphStyle/Sub").drop_cap_detail,
            Some(1)
        );
        assert_eq!(
            s.resolve_paragraph("ParagraphStyle/Old").drop_cap_detail,
            Some(2)
        );
    }

    /// ADR 028 — keep options live on paragraph STYLES in real documents
    /// (a heading's KeepWithNext); they must parse and cascade.
    #[test]
    fn parses_and_cascades_keep_options() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Heading"
                            KeepWithNext="2"
                            StartParagraph="NextOddPage"
                            KeepLinesTogether="true"
                            KeepAllLinesTogether="true"
                            KeepFirstLines="3"
                            KeepLastLines="4"/>
            <ParagraphStyle Self="ParagraphStyle/Sub"
                            BasedOn="ParagraphStyle/Heading"
                            KeepWithNext="1"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let h = s.resolve_paragraph("ParagraphStyle/Heading");
        assert_eq!(h.keep_with_next, Some(2));
        assert_eq!(
            h.start_paragraph,
            Some(paged_model::StartParagraph::NextOddPage)
        );
        assert_eq!(h.keep_lines_together, Some(true));
        assert_eq!(h.keep_all_lines_together, Some(true));
        assert_eq!(h.keep_first_lines, Some(3));
        assert_eq!(h.keep_last_lines, Some(4));
        // The child overrides one and inherits the rest.
        let sub = s.resolve_paragraph("ParagraphStyle/Sub");
        assert_eq!(sub.keep_with_next, Some(1));
        assert_eq!(sub.keep_all_lines_together, Some(true));
        assert_eq!(sub.keep_last_lines, Some(4));
    }

    /// Align to baseline grid lives on the body style in real documents
    /// (`GridAlignment="AlignBaseline"`, 29 styles of one magazine
    /// template); both attributes parse and cascade through `BasedOn`.
    #[test]
    fn parses_and_cascades_grid_alignment() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Body" GridAlignment="AlignBaseline"
                            GridAlignFirstLineOnly="false"/>
            <ParagraphStyle Self="ParagraphStyle/Lead" BasedOn="ParagraphStyle/Body"
                            GridAlignFirstLineOnly="true"/>
            <ParagraphStyle Self="ParagraphStyle/Free" BasedOn="ParagraphStyle/Body"
                            GridAlignment="None"/>
            <ParagraphStyle Self="ParagraphStyle/Odd" GridAlignment="SomethingElse"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let body = s.resolve_paragraph("ParagraphStyle/Body");
        assert_eq!(
            body.grid_alignment,
            Some(paged_model::GridAlignment::AlignBaseline)
        );
        assert_eq!(body.grid_align_first_line_only, Some(false));
        let lead = s.resolve_paragraph("ParagraphStyle/Lead");
        assert_eq!(
            lead.grid_alignment,
            Some(paged_model::GridAlignment::AlignBaseline)
        );
        assert_eq!(lead.grid_align_first_line_only, Some(true));
        let free = s.resolve_paragraph("ParagraphStyle/Free");
        assert_eq!(free.grid_alignment, Some(paged_model::GridAlignment::None));
        // A spelling InDesign does not write reads as unset.
        let odd = s.resolve_paragraph("ParagraphStyle/Odd");
        assert_eq!(odd.grid_alignment, None);
    }

    #[test]
    fn q09_parses_paragraph_shading_attrs() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Banner"
                            ParagraphShadingOn="true"
                            ParagraphShadingColor="Color/Brand"
                            ParagraphShadingTint="20"
                            ParagraphShadingWidth="ColumnWidth"
                            ParagraphShadingTopOffset="2"
                            ParagraphShadingBottomOffset="2"
                            ParagraphShadingLeftOffset="6"
                            ParagraphShadingRightOffset="6"
                            ParagraphShadingTopOrigin="AscentTopOrigin"
                            ParagraphShadingClipToFrame="false"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Banner").unwrap();
        let sh = &p.shading;
        assert_eq!(sh.on, Some(true));
        assert_eq!(sh.color.as_deref(), Some("Color/Brand"));
        assert_eq!(sh.tint, Some(20.0));
        assert_eq!(sh.width.as_deref(), Some("ColumnWidth"));
        assert_eq!(sh.offset_top, Some(2.0));
        assert_eq!(sh.offset_bottom, Some(2.0));
        assert_eq!(sh.offset_left, Some(6.0));
        assert_eq!(sh.offset_right, Some(6.0));
        assert_eq!(sh.top_origin.as_deref(), Some("AscentTopOrigin"));
        assert_eq!(sh.clip_to_frame, Some(false));
    }

    #[test]
    fn q09_paragraph_shading_inherits_from_based_on() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Base"
                            ParagraphShadingOn="true"
                            ParagraphShadingColor="Color/Brand"
                            ParagraphShadingTint="50"/>
            <ParagraphStyle Self="ParagraphStyle/Child"
                            BasedOn="ParagraphStyle/Base"
                            ParagraphShadingTint="20"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Child");
        // tint overridden, color + on inherited.
        assert_eq!(r.shading.on, Some(true));
        assert_eq!(r.shading.color.as_deref(), Some("Color/Brand"));
        assert_eq!(r.shading.tint, Some(20.0));
    }

    #[test]
    fn q09_parses_paragraph_border_attrs() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Boxed"
                            ParagraphBorderOn="true"
                            ParagraphBorderColor="Color/Brand"
                            ParagraphBorderTint="40"
                            ParagraphBorderWeight="1"
                            ParagraphBorderTopOffset="2"
                            ParagraphBorderBottomOffset="3"
                            ParagraphBorderLeftOffset="4"
                            ParagraphBorderRightOffset="5"
                            ParagraphBorderWidth="ColumnWidth"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Boxed").unwrap();
        let b = &p.border;
        assert_eq!(b.on, Some(true));
        assert_eq!(b.color.as_deref(), Some("Color/Brand"));
        assert_eq!(b.tint, Some(40.0));
        assert_eq!(b.weight, Some(1.0));
        assert_eq!(b.offset_top, Some(2.0));
        assert_eq!(b.offset_bottom, Some(3.0));
        assert_eq!(b.offset_left, Some(4.0));
        assert_eq!(b.offset_right, Some(5.0));
        assert_eq!(b.width.as_deref(), Some("ColumnWidth"));
    }

    #[test]
    fn q09_paragraph_border_inherits_from_based_on() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Base"
                            ParagraphBorderOn="true"
                            ParagraphBorderColor="Color/Brand"
                            ParagraphBorderWeight="2"/>
            <ParagraphStyle Self="ParagraphStyle/Child"
                            BasedOn="ParagraphStyle/Base"
                            ParagraphBorderWeight="1"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Child");
        // weight overridden, color + on inherited.
        assert_eq!(r.border.on, Some(true));
        assert_eq!(r.border.color.as_deref(), Some("Color/Brand"));
        assert_eq!(r.border.weight, Some(1.0));
    }

    #[test]
    fn q09_paragraph_border_per_corner_attrs_round_trip() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Boxed"
                            ParagraphBorderOn="true"
                            ParagraphBorderTopLeftCornerOption="RoundedCorner"
                            ParagraphBorderTopLeftCornerRadius="6"
                            ParagraphBorderTopRightCornerOption="RoundedCorner"
                            ParagraphBorderTopRightCornerRadius="7"
                            ParagraphBorderBottomRightCornerOption="None"
                            ParagraphBorderBottomRightCornerRadius="0"
                            ParagraphBorderBottomLeftCornerOption="RoundedCorner"
                            ParagraphBorderBottomLeftCornerRadius="9"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Boxed").unwrap();
        let c = &p.border.corners;
        assert_eq!(c[0].radius, Some(6.0));
        assert_eq!(c[0].option, Some(crate::CornerOption::Rounded));
        assert_eq!(c[1].radius, Some(7.0));
        assert_eq!(c[2].radius, Some(0.0));
        assert_eq!(c[2].option, Some(crate::CornerOption::None));
        assert_eq!(c[3].radius, Some(9.0));
        assert_eq!(c[3].option, Some(crate::CornerOption::Rounded));
    }

    #[test]
    fn q09_paragraph_border_corner_inherits_from_based_on() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Base"
                            ParagraphBorderOn="true"
                            ParagraphBorderTopLeftCornerOption="RoundedCorner"
                            ParagraphBorderTopLeftCornerRadius="5"
                            ParagraphBorderTopRightCornerOption="RoundedCorner"
                            ParagraphBorderTopRightCornerRadius="5"/>
            <ParagraphStyle Self="ParagraphStyle/Child"
                            BasedOn="ParagraphStyle/Base"
                            ParagraphBorderTopRightCornerRadius="8"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Child");
        // top-left inherited fully; top-right radius overridden but
        // option still inherited from parent.
        assert_eq!(r.border.corners[0].radius, Some(5.0));
        assert_eq!(
            r.border.corners[0].option,
            Some(crate::CornerOption::Rounded)
        );
        assert_eq!(r.border.corners[1].radius, Some(8.0));
        assert_eq!(
            r.border.corners[1].option,
            Some(crate::CornerOption::Rounded)
        );
    }

    #[test]
    fn q20_parses_letter_glyph_spacing_attrs() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Tight"
                            MinimumLetterSpacing="-5"
                            DesiredLetterSpacing="0"
                            MaximumLetterSpacing="10"
                            MinimumGlyphScaling="95"
                            DesiredGlyphScaling="100"
                            MaximumGlyphScaling="105"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Tight").unwrap();
        assert_eq!(p.minimum_letter_spacing, Some(-5.0));
        assert_eq!(p.desired_letter_spacing, Some(0.0));
        assert_eq!(p.maximum_letter_spacing, Some(10.0));
        assert_eq!(p.minimum_glyph_scaling, Some(95.0));
        assert_eq!(p.desired_glyph_scaling, Some(100.0));
        assert_eq!(p.maximum_glyph_scaling, Some(105.0));
    }

    #[test]
    fn q20_letter_glyph_spacing_inherits_from_based_on() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Base"
                            MinimumLetterSpacing="-3"
                            MaximumLetterSpacing="8"
                            MinimumGlyphScaling="97"
                            MaximumGlyphScaling="103"/>
            <ParagraphStyle Self="ParagraphStyle/Child"
                            BasedOn="ParagraphStyle/Base"
                            MaximumLetterSpacing="15"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Child");
        assert_eq!(r.minimum_letter_spacing, Some(-3.0)); // inherited
        assert_eq!(r.maximum_letter_spacing, Some(15.0)); // overridden
        assert_eq!(r.minimum_glyph_scaling, Some(97.0)); // inherited
        assert_eq!(r.maximum_glyph_scaling, Some(103.0)); // inherited
    }

    #[test]
    fn parses_bullets_on_paragraph_style() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Bulleted"
                            BulletsAndNumberingListType="BulletList"
                            BulletsTextAfter=" ">
              <Properties>
                <BulletChar BulletCharacterValue="8226"/>
              </Properties>
            </ParagraphStyle>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Bulleted").unwrap();
        assert_eq!(p.bullets_list_type.as_deref(), Some("BulletList"));
        assert_eq!(p.bullet_character, Some(8226)); // U+2022 BULLET
        assert_eq!(p.bullets_text_after.as_deref(), Some(" "));
    }

    /// InDesign's own spelling of the marker character styles: typed
    /// `<Properties>` children (267 of 267 corpus packages; the attributes
    /// are ignored, `list-overrides` o14-o17). A child beats an attribute
    /// of ours.
    #[test]
    fn parses_marker_character_styles_as_indesign_spells_them() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup><ParagraphStyle Self="ParagraphStyle/N" BulletsAndNumberingListType="NumberedList" BulletsCharacterStyle="CharacterStyle/Old"><Properties><BulletChar BulletCharacterType="UnicodeOnly" BulletCharacterValue="8226" /><NumberingFormat type="string">1, 2, 3, 4...</NumberingFormat><BulletsCharacterStyle type="object">CharacterStyle/Dot</BulletsCharacterStyle><NumberingCharacterStyle type="object">CharacterStyle/Digits</NumberingCharacterStyle></Properties></ParagraphStyle></RootParagraphStyleGroup></idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/N").unwrap();
        assert_eq!(
            p.bullets_character_style.as_deref(),
            Some("CharacterStyle/Dot")
        );
        assert_eq!(
            p.bullets_and_numbering_digits_character_style.as_deref(),
            Some("CharacterStyle/Digits")
        );
        assert_eq!(p.numbering_format.as_deref(), Some("1, 2, 3, 4..."));
    }

    /// InDesign spells a paragraph STYLE's tab stops the way it spells a
    /// range's: `<Properties><TabList type="list">` of `<ListItem
    /// type="record">` children. The style reader only knew the
    /// generator's `<TabStop Position=…/>`, so every InDesign-authored
    /// style lost its stops — the renderer never set them, and a save
    /// dropped the `<TabList>` from an untouched `Resources/Styles.xml`
    /// (the exporter settles the child against a model that had none).
    /// The style is the one that lost it: `the-picture.idml`'s
    /// "docx direct format 9".
    #[test]
    fn parses_a_style_tab_list_as_indesign_spells_it() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup>
		<ParagraphStyle Self="ParagraphStyle/docx direct format 9" Name="docx direct format 9" NextStyle="ParagraphStyle/docx direct format 9">
			<Properties>
				<BasedOn type="object">ParagraphStyle/docx defaults</BasedOn>
				<PreviewColor type="enumeration">Nothing</PreviewColor>
				<TabList type="list">
					<ListItem type="record">
						<Alignment type="enumeration">RightAlign</Alignment>
						<AlignmentCharacter type="string">.</AlignmentCharacter>
						<Leader type="string">.</Leader>
						<Position type="unit">216</Position>
					</ListItem>
					<ListItem type="record">
						<Alignment type="enumeration">LeftAlign</Alignment>
						<AlignmentCharacter type="string">.</AlignmentCharacter>
						<Leader type="string"></Leader>
						<Position type="unit">300.5</Position>
					</ListItem>
				</TabList>
			</Properties>
		</ParagraphStyle>
		<ParagraphStyle Self="ParagraphStyle/Plain" Name="Plain"><Properties><TabList type="list"/></Properties></ParagraphStyle>
		</RootParagraphStyleGroup></idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = &s.paragraph_styles["ParagraphStyle/docx direct format 9"];
        assert_eq!(
            p.tab_list,
            vec![
                TabStop {
                    position: 216.0,
                    alignment: Some("RightAlign".into()),
                    alignment_character: Some(".".into()),
                    leader: Some(".".into()),
                },
                TabStop {
                    position: 300.5,
                    alignment: Some("LeftAlign".into()),
                    alignment_character: Some(".".into()),
                    leader: None,
                },
            ]
        );
        // The rest of the block still reads.
        assert_eq!(p.based_on.as_deref(), Some("ParagraphStyle/docx defaults"));
        assert!(s.paragraph_styles["ParagraphStyle/Plain"]
            .tab_list
            .is_empty());
    }

    #[test]
    fn parses_bullets_character_style_attrs() {
        // Both `BulletsCharacterStyle` (bullets) and
        // `BulletsAndNumberingDigitsCharacterStyle` (numbered-list
        // digits) survive the parser as plain string refs.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Bulleted"
                            BulletsAndNumberingListType="BulletList"
                            BulletsCharacterStyle="CharacterStyle/RedDot"/>
            <ParagraphStyle Self="ParagraphStyle/Numbered"
                            BulletsAndNumberingListType="NumberedList"
                            BulletsAndNumberingDigitsCharacterStyle="CharacterStyle/BlueDigit"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let b = s.paragraph_styles.get("ParagraphStyle/Bulleted").unwrap();
        assert_eq!(
            b.bullets_character_style.as_deref(),
            Some("CharacterStyle/RedDot")
        );
        assert!(b.bullets_and_numbering_digits_character_style.is_none());
        let n = s.paragraph_styles.get("ParagraphStyle/Numbered").unwrap();
        assert_eq!(
            n.bullets_and_numbering_digits_character_style.as_deref(),
            Some("CharacterStyle/BlueDigit")
        );
        assert!(n.bullets_character_style.is_none());
    }

    #[test]
    fn resolve_paragraph_propagates_bullets_character_style_through_based_on() {
        // A child style without its own BulletsCharacterStyle should
        // inherit it via BasedOn so cascade-only IDMLs continue
        // working.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Base"
                            BulletsAndNumberingListType="BulletList"
                            BulletsCharacterStyle="CharacterStyle/RedDot"
                            BulletsAndNumberingDigitsCharacterStyle="CharacterStyle/BlueDigit"/>
            <ParagraphStyle Self="ParagraphStyle/Child"
                            BasedOn="ParagraphStyle/Base"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Child");
        assert_eq!(
            r.bullets_character_style.as_deref(),
            Some("CharacterStyle/RedDot")
        );
        assert_eq!(
            r.bullets_and_numbering_digits_character_style.as_deref(),
            Some("CharacterStyle/BlueDigit")
        );
    }

    #[test]
    fn resolve_unknown_id_returns_default() {
        let s = parse_stylesheet(SAMPLE).unwrap();
        let r = s.resolve_character("CharacterStyle/Missing");
        assert!(r.font.is_none());
        assert!(r.point_size.is_none());
    }

    #[test]
    fn resolve_terminates_on_cyclic_based_on() {
        // Two styles BasedOn each other — resolution must not hang.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootCharacterStyleGroup>
            <CharacterStyle Self="CharacterStyle/A" BasedOn="CharacterStyle/B" PointSize="10"/>
            <CharacterStyle Self="CharacterStyle/B" BasedOn="CharacterStyle/A" FontStyle="Bold"/>
          </RootCharacterStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_character("CharacterStyle/A");
        // Both were folded in once; the depth limiter prevents looping.
        assert_eq!(r.point_size, Some(10.0));
        assert_eq!(r.font_style.as_deref(), Some("Bold"));
    }

    #[test]
    fn parses_a_paragraph_styles_list_level() {
        // Core ADR 158: the level and whether its restart policy applies
        // as attributes, the policy as an empty `<Properties>` child, as
        // `numbering-multilevel` spells them for InDesign.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/L2" NumberingLevel="2" NumberingApplyRestartPolicy="true"><Properties><NumberingFormat type="string">a, b, c, d...</NumberingFormat><NumberingRestartPolicies RestartPolicy="RangeOfPreviousLevels" LowerLevel="1" UpperLevel="1"/></Properties></ParagraphStyle>
            <ParagraphStyle Self="ParagraphStyle/Keep" NumberingLevel="3" NumberingApplyRestartPolicy="false"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let l2 = s.paragraph_styles.get("ParagraphStyle/L2").unwrap();
        assert_eq!(
            l2.list_level,
            paged_model::ListLevel {
                level: Some(2),
                apply_restart: Some(true),
                restart_policy: Some(paged_model::RestartPolicy {
                    kind: paged_model::RestartPolicyKind::RangeOfPreviousLevels,
                    lower: 1,
                    upper: 1,
                }),
            }
        );
        assert_eq!(l2.numbering_format.as_deref(), Some("a, b, c, d..."));
        let keep = s.paragraph_styles.get("ParagraphStyle/Keep").unwrap();
        assert_eq!(keep.list_level.level, Some(3));
        assert_eq!(keep.list_level.apply_restart, Some(false));
        assert_eq!(keep.list_level.restart_policy, None);
    }

    #[test]
    fn parses_numbering_expression_start_at_and_continue_attrs() {
        // Real-world IDML carries these as attributes on the
        // ParagraphStyle start tag for the simple cases.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Numbered"
                            BulletsAndNumberingListType="NumberedList"
                            NumberingExpression="Step ^# of 5^t"
                            NumberingStartAt="5"
                            NumberingContinue="false"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Numbered").unwrap();
        assert_eq!(p.numbering_expression.as_deref(), Some("Step ^# of 5^t"));
        assert_eq!(p.numbering_start_at, Some(5));
        assert_eq!(p.numbering_continue, Some(false));
    }

    #[test]
    fn parses_numbering_expression_as_property_element() {
        // InDesign often emits NumberingExpression as an element-form
        // child of <Properties> (typed string), not as an attribute.
        // The parser must pick that up so the cascade carries the
        // template forward.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Numbered"
                            BulletsAndNumberingListType="NumberedList">
              <Properties>
                <NumberingExpression type="string">^#)^t</NumberingExpression>
              </Properties>
            </ParagraphStyle>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Numbered").unwrap();
        assert_eq!(p.numbering_expression.as_deref(), Some("^#)^t"));
    }

    #[test]
    fn resolve_paragraph_propagates_numbering_overrides_through_based_on() {
        // Numbered base style sets the expression + start; a child
        // style only flips Continue. Cascade should pull the
        // expression and StartAt from the parent while overriding
        // Continue locally.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Base"
                            NumberingExpression="^#.^t"
                            NumberingStartAt="3"/>
            <ParagraphStyle Self="ParagraphStyle/Child"
                            BasedOn="ParagraphStyle/Base"
                            NumberingContinue="true"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Child");
        assert_eq!(r.numbering_expression.as_deref(), Some("^#.^t"));
        assert_eq!(r.numbering_start_at, Some(3));
        assert_eq!(r.numbering_continue, Some(true));
    }

    /// InDesign's own spelling of a reserved root parent is the `$ID`
    /// string without the kind; the cascade must still reach the root.
    #[test]
    fn resolve_reaches_no_paragraph_style_root() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]"
                            Name="$ID/[No paragraph style]" PointSize="11" Hyphenation="false">
              <Properties><AppliedFont type="string">Minion Pro</AppliedFont></Properties>
            </ParagraphStyle>
            <ParagraphStyle Self="ParagraphStyle/Body" Name="Body" PointSize="9">
              <Properties><BasedOn type="string">$ID/[No paragraph style]</BasedOn></Properties>
            </ParagraphStyle>
          </RootParagraphStyleGroup>
          <RootCharacterStyleGroup>
            <CharacterStyle Self="CharacterStyle/$ID/[No character style]"
                            Name="$ID/[No character style]"/>
            <CharacterStyle Self="CharacterStyle/Emph" Name="Emph" FontStyle="Italic"
                            BasedOn="$ID/[No character style]"/>
          </RootCharacterStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Body");
        assert_eq!(r.font.as_deref(), Some("Minion Pro"), "the root's face");
        assert_eq!(r.point_size, Some(9.0), "the style's own size wins");
        assert_eq!(
            s.character_styles["CharacterStyle/Emph"]
                .based_on
                .as_deref(),
            Some("CharacterStyle/$ID/[No character style]")
        );
    }

    /// InDesign exports `AppliedFont` and `BasedOn` as element-form
    /// children of `<Properties>` rather than attributes on the
    /// style element. Without this path the cascade reads
    /// `font: None` for every paragraph style and runs that only
    /// inherit a font through their applied paragraph style end up
    /// fontless — and therefore unshaped — in real-world IDMLs.
    #[test]
    fn parses_applied_font_and_based_on_as_property_elements() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Body" Name="Body"
                            FontStyle="Italic" PointSize="8"
                            FillColor="Color/Red">
              <Properties>
                <BasedOn type="string">$ID/[No paragraph style]</BasedOn>
                <Leading type="unit">12</Leading>
                <AppliedFont type="string">Open Sans</AppliedFont>
              </Properties>
            </ParagraphStyle>
          </RootParagraphStyleGroup>
          <RootCharacterStyleGroup>
            <CharacterStyle Self="CharacterStyle/Emph" Name="Emph">
              <Properties>
                <BasedOn type="string">CharacterStyle/Base</BasedOn>
                <AppliedFont type="string">Minion Pro</AppliedFont>
              </Properties>
            </CharacterStyle>
          </RootCharacterStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Body").unwrap();
        assert_eq!(p.font.as_deref(), Some("Open Sans"));
        assert_eq!(
            p.based_on.as_deref(),
            Some("ParagraphStyle/$ID/[No paragraph style]")
        );
        let c = s.character_styles.get("CharacterStyle/Emph").unwrap();
        assert_eq!(c.font.as_deref(), Some("Minion Pro"));
        assert_eq!(c.based_on.as_deref(), Some("CharacterStyle/Base"));

        // Cascade pulls font through to the resolved paragraph attrs.
        let r = s.resolve_paragraph("ParagraphStyle/Body");
        assert_eq!(r.font.as_deref(), Some("Open Sans"));
    }

    /// InDesign 20.0.1's own spelling (`toc-probe`, core ADR 165): the
    /// collected style is the entry's `Name`, the format a `<Properties>`
    /// child, the number position `PageNumberPosition`, the separator
    /// behind `$ID/`.
    #[test]
    fn parses_indesigns_toc_style_spelling() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
	<TOCStyle Self="TOCStyle/Contents" TitleStyle="ParagraphStyle/TOC Title" Title="Contents" Name="Contents" RunIn="false" IncludeHidden="false" IncludeBookDocuments="false" CreateBookmarks="true" SetStoryDirection="Horizontal" NumberedParagraphs="ExcludeNumbers" MakeAnchor="false" RemoveForcedLineBreak="false">
		<TOCStyleEntry Self="u145TOCStyleEntry0" Name="H1" Level="1" PageNumberPosition="AfterEntry" Separator="$ID/^t" SortAlphabet="false">
			<Properties>
				<FormatStyle type="object">ParagraphStyle/TOC 1</FormatStyle>
				<PageNumberStyle type="object">CharacterStyle/TOC Page</PageNumberStyle>
				<SeparatorStyle type="string">$ID/[Same Style]</SeparatorStyle>
			</Properties>
		</TOCStyleEntry>
		<TOCStyleEntry Self="u145TOCStyleEntry1" Name="Group:H2" Level="2" PageNumberPosition="BeforeEntry" Separator="$ID/ " SortAlphabet="false">
			<Properties>
				<FormatStyle type="object">ParagraphStyle/TOC 2</FormatStyle>
			</Properties>
		</TOCStyleEntry>
		<TOCStyleEntry Self="u145TOCStyleEntry2" Name="H3" Level="3" PageNumberPosition="None" Separator="$ID/" SortAlphabet="false">
			<Properties>
				<FormatStyle type="object">ParagraphStyle/TOC 3</FormatStyle>
			</Properties>
		</TOCStyleEntry>
	</TOCStyle>
</idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let toc = s.toc_styles.get("TOCStyle/Contents").unwrap();
        assert_eq!(toc.title.as_deref(), Some("Contents"));
        let e = &toc.entries;
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].include_style.as_deref(), Some("ParagraphStyle/H1"));
        assert_eq!(e[0].format_style.as_deref(), Some("ParagraphStyle/TOC 1"));
        assert_eq!(e[0].separator.as_deref(), Some("^t"));
        assert_eq!(e[0].page_number, None);
        assert_eq!(
            e[1].include_style.as_deref(),
            Some("ParagraphStyle/Group%3aH2")
        );
        assert_eq!(e[1].format_style.as_deref(), Some("ParagraphStyle/TOC 2"));
        assert_eq!(e[1].separator.as_deref(), Some(" "));
        assert_eq!(e[2].page_number.as_deref(), Some("NoPageNumber"));
        assert_eq!(e[2].separator.as_deref(), Some(""));
        // Where the number stands, and the styles it and the separator
        // are set in.
        assert_eq!(e[0].page_number_position.as_deref(), Some("AfterEntry"));
        assert_eq!(e[1].page_number_position.as_deref(), Some("BeforeEntry"));
        assert_eq!(e[2].page_number_position.as_deref(), Some("None"));
        assert_eq!(
            e[0].page_number_style.as_deref(),
            Some("CharacterStyle/TOC Page")
        );
        assert_eq!(e[0].separator_style.as_deref(), Some("$ID/[Same Style]"));
        assert_eq!(e[1].page_number_style, None);
        assert_eq!(e[0].sort_alphabet, Some(false));
        assert_eq!(toc.create_bookmarks, Some(true));
    }

    #[test]
    fn legacy_parses_toc_style_with_entries() {
        // The engine's older spelling: `IncludeStyle`, `FormatStyle` and
        // `PageNumber` as attributes. It still reads.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/TocTitle" Name="TocTitle"/>
            <ParagraphStyle Self="ParagraphStyle/Heading1" Name="Heading1"/>
            <ParagraphStyle Self="ParagraphStyle/Heading2" Name="Heading2"/>
            <ParagraphStyle Self="ParagraphStyle/TocFormat1" Name="TocFormat1"/>
            <ParagraphStyle Self="ParagraphStyle/TocFormat2" Name="TocFormat2"/>
          </RootParagraphStyleGroup>
          <TOCStyle Self="TOCStyle/Main" Name="Main" Title="Contents"
                    TitleStyle="ParagraphStyle/TocTitle"
                    IncludeBookDocuments="false"
                    IncludeHidden="false"
                    RunIn="false">
            <TOCStyleEntry Name="Heading1"
                           IncludeStyle="ParagraphStyle/Heading1"
                           FormatStyle="ParagraphStyle/TocFormat1"
                           Level="1"
                           PageNumber="On"
                           Separator="^t"/>
            <TOCStyleEntry Name="Heading2"
                           IncludeStyle="ParagraphStyle/Heading2"
                           FormatStyle="ParagraphStyle/TocFormat2"
                           Level="2"
                           PageNumber="On"
                           Separator=" -- "/>
          </TOCStyle>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let toc = s.toc_styles.get("TOCStyle/Main").unwrap();
        assert_eq!(toc.title.as_deref(), Some("Contents"));
        assert_eq!(toc.title_style.as_deref(), Some("ParagraphStyle/TocTitle"));
        assert_eq!(toc.include_book_documents, Some(false));
        assert_eq!(toc.include_hidden, Some(false));
        assert_eq!(toc.run_in, Some(false));
        assert_eq!(toc.entries.len(), 2);
        let e1 = &toc.entries[0];
        assert_eq!(e1.include_style.as_deref(), Some("ParagraphStyle/Heading1"));
        assert_eq!(
            e1.format_style.as_deref(),
            Some("ParagraphStyle/TocFormat1")
        );
        assert_eq!(e1.level, Some(1));
        assert_eq!(e1.page_number.as_deref(), Some("On"));
        assert_eq!(e1.separator.as_deref(), Some("^t"));
        let e2 = &toc.entries[1];
        assert_eq!(e2.level, Some(2));
        assert_eq!(e2.separator.as_deref(), Some(" -- "));
    }

    #[test]
    fn parses_self_closing_empty_toc_style() {
        // InDesign always emits a default `<TOCStyle .../>` even when
        // the document has no TOC. The parser must accept the self-
        // closing form and produce an entry with no children.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <TOCStyle Self="TOCStyle/$ID/DefaultTOCStyleName"
                    Name="$ID/DefaultTOCStyleName"
                    Title="Contents"
                    TitleStyle="ParagraphStyle/$ID/[No paragraph style]"
                    RunIn="false"
                    IncludeHidden="false"
                    IncludeBookDocuments="false"/>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let toc = s
            .toc_styles
            .get("TOCStyle/$ID/DefaultTOCStyleName")
            .unwrap();
        assert_eq!(toc.title.as_deref(), Some("Contents"));
        assert!(toc.entries.is_empty());
    }

    // ---- CJK Stage 1 (parser surface) ----

    #[test]
    fn paragraph_style_captures_kinsoku_and_mojikumi_attributes() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/Japanese"
                            KinsokuSet="KinsokuTable/$ID/PhotoshopKinsokuHard"
                            KinsokuType="PushOut"
                            MojikumiTable="MojikumiTable/$ID/PhotoshopMojikumiSet4"
                            MojikumiSet="MojikumiSet/$ID/OldSet"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = s.paragraph_styles.get("ParagraphStyle/Japanese").unwrap();
        assert_eq!(
            p.kinsoku_set.as_deref(),
            Some("KinsokuTable/$ID/PhotoshopKinsokuHard")
        );
        assert_eq!(p.kinsoku_type.as_deref(), Some("PushOut"));
        assert_eq!(
            p.mojikumi_table.as_deref(),
            Some("MojikumiTable/$ID/PhotoshopMojikumiSet4")
        );
        assert_eq!(p.mojikumi_set.as_deref(), Some("MojikumiSet/$ID/OldSet"));
    }

    #[test]
    fn character_style_captures_ruby_and_kenten_attributes() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootCharacterStyleGroup>
            <CharacterStyle Self="CharacterStyle/RubyBase"
                            RubyFlag="true"
                            RubyType="GroupRuby"
                            RubyString="furigana"
                            RubyAlignment="Ruby1Aki"
                            KentenKind="KentenBlackCircle"
                            KentenFontSize="12"
                            KentenPlacement="3">
              <Properties>
                <RubyFont type="string">Kozuka Gothic Pr6N</RubyFont>
              </Properties>
            </CharacterStyle>
          </RootCharacterStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let c = s.character_styles.get("CharacterStyle/RubyBase").unwrap();
        assert_eq!(c.ruby_flag, Some(true));
        assert_eq!(c.ruby_type.as_deref(), Some("GroupRuby"));
        assert_eq!(c.ruby_string.as_deref(), Some("furigana"));
        assert_eq!(c.kenten_kind.as_deref(), Some("KentenBlackCircle"));
        assert_eq!(c.kenten_font_size, Some(12.0));
        assert_eq!(c.marks.ruby_alignment.as_deref(), Some("Ruby1Aki"));
        assert_eq!(c.marks.kenten_placement, Some(3.0));
        assert_eq!(c.marks.ruby_font.as_deref(), Some("Kozuka Gothic Pr6N"));
    }

    #[test]
    fn resolve_paragraph_propagates_kinsoku_through_based_on() {
        // Base style sets the kinsoku ref; child overrides only one
        // field. Cascade should pull the rest from BasedOn.
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
          <RootParagraphStyleGroup>
            <ParagraphStyle Self="ParagraphStyle/JpBase"
                            KinsokuSet="KinsokuTable/$ID/PhotoshopKinsokuHard"
                            KinsokuType="PushIn"
                            MojikumiTable="MojikumiTable/$ID/PhotoshopMojikumiSet4"/>
            <ParagraphStyle Self="ParagraphStyle/JpChild"
                            BasedOn="ParagraphStyle/JpBase"
                            KinsokuType="PushOut"/>
          </RootParagraphStyleGroup>
        </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/JpChild");
        // Local override wins for KinsokuType.
        assert_eq!(r.kinsoku_type.as_deref(), Some("PushOut"));
        // Other fields cascade from BasedOn.
        assert_eq!(
            r.kinsoku_set.as_deref(),
            Some("KinsokuTable/$ID/PhotoshopKinsokuHard")
        );
        assert_eq!(
            r.mojikumi_table.as_deref(),
            Some("MojikumiTable/$ID/PhotoshopMojikumiSet4")
        );
    }

    // ---- Track 4a: custom StrokeStyle parsing ----

    #[test]
    fn dashed_stroke_style_parses_pattern_into_floats() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
            <DashedStrokeStyle Self="StrokeStyle/u163" Name="Diag"
                               StartCap="ButtEndCap" CornerAdjustment="None"
                               GapColor="Swatch/None" GapTint="100"
                               Pattern="3.5 2 1 4"/>
            <DottedStrokeStyle Self="StrokeStyle/u164" Name="Tight"
                               GapColor="Swatch/None" GapTint="100"/>
          </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let dash = s.stroke_styles.get("StrokeStyle/u163").unwrap();
        assert_eq!(dash.kind, StrokeStyleKind::Dashed);
        assert_eq!(dash.name.as_deref(), Some("Diag"));
        assert_eq!(dash.pattern, vec![3.5, 2.0, 1.0, 4.0]);
        // `GapColor="Swatch/None"` normalises to None (no gap fill).
        assert_eq!(dash.gap_color, None);
        let dot = s.stroke_styles.get("StrokeStyle/u164").unwrap();
        assert_eq!(dot.kind, StrokeStyleKind::Dotted);
        assert!(dot.pattern.is_empty());
    }

    #[test]
    fn table_style_parses_alternating_row_and_column_fills() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
            <RootTableStyleGroup>
              <TableStyle Self="TableStyle/Alt" Name="Alt"
                          AlternatingFills="AlternatingRows"
                          StartRowFillColor="Color/Cyan" StartRowFillCount="2"
                          StartRowFillTint="40"
                          EndRowFillColor="Color/Gray" EndRowFillCount="1"
                          EndRowFillTint="100"
                          SkipFirstAlternatingFillRows="1"
                          SkipLastAlternatingFillRows="2"
                          StartColumnFillColor="Color/Blue" StartColumnFillCount="3"
                          StartColumnFillTint="55"
                          EndColumnFillColor="Color/None" EndColumnFillCount="1"
                          SkipFirstAlternatingFillColumns="0"
                          SkipLastAlternatingFillColumns="1"/>
            </RootTableStyleGroup>
          </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let t = s.table_styles.get("TableStyle/Alt").unwrap();
        assert_eq!(t.alternating_fills.as_deref(), Some("AlternatingRows"));
        // Row fields.
        assert_eq!(t.start_row_fill_color.as_deref(), Some("Color/Cyan"));
        assert_eq!(t.start_row_fill_count, Some(2));
        assert_eq!(t.start_row_fill_tint, Some(40.0));
        assert_eq!(t.end_row_fill_color.as_deref(), Some("Color/Gray"));
        assert_eq!(t.end_row_fill_count, Some(1));
        assert_eq!(t.end_row_fill_tint, Some(100.0));
        assert_eq!(t.skip_first_alternating_fill_rows, Some(1));
        assert_eq!(t.skip_last_alternating_fill_rows, Some(2));
        // Column fields.
        assert_eq!(t.start_column_fill_color.as_deref(), Some("Color/Blue"));
        assert_eq!(t.start_column_fill_count, Some(3));
        assert_eq!(t.start_column_fill_tint, Some(55.0));
        assert_eq!(t.end_column_fill_count, Some(1));
        assert_eq!(t.skip_last_alternating_fill_columns, Some(1));
    }

    #[test]
    fn resolve_table_walks_based_on_for_alternating_fills() {
        // Child overrides AlternatingFills + start fill; parent
        // supplies the end fill + skip counts. resolve_table walks the
        // BasedOn chain merging "self wins, parent fills the gaps".
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
            <RootTableStyleGroup>
              <TableStyle Self="TableStyle/Base" Name="Base"
                          AlternatingFills="AlternatingRows"
                          StartRowFillColor="Color/Cyan" StartRowFillCount="1"
                          EndRowFillColor="Color/Gray" EndRowFillCount="1"
                          SkipFirstAlternatingFillRows="2"/>
              <TableStyle Self="TableStyle/Child" Name="Child"
                          BasedOn="TableStyle/Base"
                          StartRowFillColor="Color/Magenta"/>
            </RootTableStyleGroup>
          </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_table("TableStyle/Child");
        // Override from child.
        assert_eq!(r.start_row_fill_color.as_deref(), Some("Color/Magenta"));
        // Inherited from base.
        assert_eq!(r.alternating_fills.as_deref(), Some("AlternatingRows"));
        assert_eq!(r.start_row_fill_count, Some(1));
        assert_eq!(r.end_row_fill_color.as_deref(), Some("Color/Gray"));
        assert_eq!(r.skip_first_alternating_fill_rows, Some(2));
    }

    #[test]
    fn dashed_stroke_style_keeps_real_gap_color() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
            <DashedStrokeStyle Self="StrokeStyle/u165" Name="GapDash"
                               GapColor="Color/Cyan" GapTint="60"
                               Pattern="6 4"/>
          </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let dash = s.stroke_styles.get("StrokeStyle/u165").unwrap();
        assert_eq!(dash.gap_color.as_deref(), Some("Color/Cyan"));
        assert_eq!(dash.gap_tint, Some(60.0));
    }

    // ---- W1.2: striped + wavy custom StrokeStyle parsing ----

    #[test]
    fn striped_stroke_style_parses_stripe_children() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
            <StripedStrokeStyle Self="StrokeStyle/u200" Name="ThickThin">
              <Stripe Left="0" Width="0.6"/>
              <Stripe Left="0.8" Width="0.2"/>
            </StripedStrokeStyle>
          </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let striped = s.stroke_styles.get("StrokeStyle/u200").unwrap();
        assert_eq!(striped.kind, StrokeStyleKind::Striped);
        assert_eq!(striped.name.as_deref(), Some("ThickThin"));
        assert_eq!(striped.stripes.len(), 2);
        assert_eq!(
            striped.stripes[0],
            StripeDef {
                left: 0.0,
                width: 0.6
            }
        );
        assert_eq!(
            striped.stripes[1],
            StripeDef {
                left: 0.8,
                width: 0.2
            }
        );
    }

    #[test]
    fn wavy_stroke_style_parses_width_and_wavelength() {
        let xml =
            br#"<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
            <WavyStrokeStyle Self="StrokeStyle/u201" Name="Wave"
                             Width="0.5" Wavelength="1.5"/>
          </idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let wavy = s.stroke_styles.get("StrokeStyle/u201").unwrap();
        assert_eq!(wavy.kind, StrokeStyleKind::Wavy);
        assert_eq!(wavy.wave_width, Some(0.5));
        assert_eq!(wavy.wave_length, Some(1.5));
        assert!(wavy.stripes.is_empty());
    }

    // ── W1.22 (engine gap 22) — NumberingList resources + NextStyle ──

    #[test]
    fn parses_numbering_list_resources() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <RootNumberingListGroup>
    <NumberingList Self="NumberingList/Steps"
                   Name="Steps"
                   ContinueNumbersAcrossStories="true"
                   ContinueNumbersAcrossDocuments="false"/>
  </RootNumberingListGroup>
  <NumberingList Self="NumberingList/Local"
                 Name="Local"
                 ContinueNumbersAcrossStories="false"/>
</idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        assert_eq!(s.numbering_lists.len(), 2);
        let steps = s.numbering_lists.get("NumberingList/Steps").unwrap();
        assert_eq!(steps.name.as_deref(), Some("Steps"));
        assert_eq!(steps.continue_across_stories, Some(true));
        assert_eq!(steps.continue_across_documents, Some(false));
        let local = s.numbering_lists.get("NumberingList/Local").unwrap();
        assert_eq!(local.continue_across_stories, Some(false));
    }

    #[test]
    fn parses_applied_numbering_list_and_next_style_on_paragraph_style() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <RootParagraphStyleGroup>
    <ParagraphStyle Self="ParagraphStyle/Step"
                    Name="Step"
                    AppliedNumberingList="NumberingList/Steps"
                    NextStyle="ParagraphStyle/Body"/>
    <ParagraphStyle Self="ParagraphStyle/NoneList"
                    Name="NoneList"
                    AppliedNumberingList="NumberingList/$ID/[No numbering list]"/>
    <ParagraphStyle Self="ParagraphStyle/Body" Name="Body"/>
  </RootParagraphStyleGroup>
</idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let step = s.paragraph_styles.get("ParagraphStyle/Step").unwrap();
        assert_eq!(
            step.applied_numbering_list.as_deref(),
            Some("NumberingList/Steps")
        );
        assert_eq!(step.next_style.as_deref(), Some("ParagraphStyle/Body"));
        // The "[No numbering list]" sentinel normalises to None so the
        // cascade can fall through.
        let none = s.paragraph_styles.get("ParagraphStyle/NoneList").unwrap();
        assert_eq!(none.applied_numbering_list, None);
    }

    #[test]
    fn next_style_and_applied_numbering_list_cascade_through_based_on() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <RootParagraphStyleGroup>
    <ParagraphStyle Self="ParagraphStyle/Base"
                    Name="Base"
                    AppliedNumberingList="NumberingList/Steps"
                    NextStyle="ParagraphStyle/Base"/>
    <ParagraphStyle Self="ParagraphStyle/Child"
                    Name="Child"
                    BasedOn="ParagraphStyle/Base"/>
  </RootParagraphStyleGroup>
</idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let r = s.resolve_paragraph("ParagraphStyle/Child");
        assert_eq!(
            r.applied_numbering_list.as_deref(),
            Some("NumberingList/Steps")
        );
        assert_eq!(r.next_style.as_deref(), Some("ParagraphStyle/Base"));
    }

    #[test]
    fn style_leading_is_read_from_the_attribute_and_the_typed_child() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/Tight" Name="Tight"><Properties><Leading type="unit">8.5</Leading></Properties></CharacterStyle></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/Attr" Name="Attr" Leading="14"/><ParagraphStyle Self="ParagraphStyle/Child" Name="Child" PointSize="6.5"><Properties><AppliedFont type="string">JetBrains Mono</AppliedFont><Leading type="unit">13</Leading></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Auto" Name="Auto"><Properties><Leading type="enumeration">Auto</Leading></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Derived" Name="Derived"><Properties><BasedOn type="object">ParagraphStyle/Child</BasedOn></Properties></ParagraphStyle></RootParagraphStyleGroup>
</idPkg:Styles>"#;
        let sheet = parse_stylesheet(xml).expect("parse");
        assert_eq!(
            sheet.paragraph_styles["ParagraphStyle/Attr"].leading,
            Some(14.0)
        );
        assert_eq!(
            sheet.paragraph_styles["ParagraphStyle/Child"].leading,
            Some(13.0)
        );
        assert_eq!(
            sheet.paragraph_styles["ParagraphStyle/Child"]
                .font
                .as_deref(),
            Some("JetBrains Mono")
        );
        // `Auto` is auto leading, spelled 0: it overrides a parent's.
        assert_eq!(
            sheet.paragraph_styles["ParagraphStyle/Auto"].leading,
            Some(0.0)
        );
        assert_eq!(
            sheet.character_styles["CharacterStyle/Tight"].leading,
            Some(8.5)
        );
        // … and it cascades.
        assert_eq!(
            sheet.resolve_paragraph("ParagraphStyle/Derived").leading,
            Some(13.0)
        );
        assert_eq!(
            sheet.resolve_paragraph("ParagraphStyle/Derived").point_size,
            Some(6.5)
        );
    }

    /// The probe document's paragraph style as InDesign 20.0.1 exported
    /// it (2026-10-12): a drop cap's style as the first nested style,
    /// two more nested styles, a GREP and a line style.
    const INDESIGN_NESTED: &str = r#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup Self="u7a">
<ParagraphStyle Self="ParagraphStyle/Nested" Name="Nested" EmptyNestedStyles="false" EmptyLineStyles="false" EmptyGrepStyles="false" DropCapCharacters="1" DropCapLines="2">
  <Properties>
    <BasedOn type="string">$ID/[No paragraph style]</BasedOn>
    <AllNestedStyles type="list">
      <ListItem type="record">
        <AppliedCharacterStyle type="object">CharacterStyle/DropMagenta</AppliedCharacterStyle>
        <Delimiter type="enumeration">Dropcap</Delimiter>
        <Repetition type="long">1</Repetition>
        <Inclusive type="boolean">true</Inclusive>
      </ListItem>
      <ListItem type="record">
        <AppliedCharacterStyle type="object">CharacterStyle/NestRed</AppliedCharacterStyle>
        <Delimiter type="enumeration">Tabs</Delimiter>
        <Repetition type="long">1</Repetition>
        <Inclusive type="boolean">true</Inclusive>
      </ListItem>
      <ListItem type="record">
        <AppliedCharacterStyle type="object">CharacterStyle/NestBlue</AppliedCharacterStyle>
        <Delimiter type="enumeration">AnyWord</Delimiter>
        <Repetition type="long">2</Repetition>
        <Inclusive type="boolean">false</Inclusive>
      </ListItem>
    </AllNestedStyles>
    <AllGREPStyles type="list">
      <ListItem type="record">
        <AppliedCharacterStyle type="object">CharacterStyle/GrepGreen</AppliedCharacterStyle>
        <GrepExpression type="string">\d+</GrepExpression>
      </ListItem>
    </AllGREPStyles>
    <AllLineStyles type="list">
      <ListItem type="record">
        <AppliedCharacterStyle type="object">CharacterStyle/LineUnder</AppliedCharacterStyle>
        <LineCount type="long">2</LineCount>
        <RepeatLast type="long">-1</RepeatLast>
      </ListItem>
    </AllLineStyles>
  </Properties>
</ParagraphStyle>
<ParagraphStyle Self="ParagraphStyle/Inherits" Name="Inherits" EmptyNestedStyles="false" EmptyLineStyles="false" EmptyGrepStyles="false"><Properties><BasedOn type="object">ParagraphStyle/Nested</BasedOn></Properties></ParagraphStyle>
<ParagraphStyle Self="ParagraphStyle/Cleared" Name="Cleared" EmptyNestedStyles="true" EmptyLineStyles="true" EmptyGrepStyles="true"><Properties><BasedOn type="object">ParagraphStyle/Nested</BasedOn></Properties></ParagraphStyle>
</RootParagraphStyleGroup></idPkg:Styles>"#;

    #[test]
    fn reads_nested_line_and_grep_styles_as_indesign_writes_them() {
        let s = parse_stylesheet(INDESIGN_NESTED.as_bytes()).unwrap();
        let p = &s.paragraph_styles["ParagraphStyle/Nested"];
        let delimiters: Vec<&NestedDelimiter> =
            p.nested_styles.iter().map(|n| &n.delimiter).collect();
        assert_eq!(
            delimiters,
            [
                &NestedDelimiter::Dropcap,
                &NestedDelimiter::Tab,
                &NestedDelimiter::Words
            ]
        );
        assert_eq!(
            p.nested_styles[2].applied_character_style,
            "CharacterStyle/NestBlue"
        );
        assert_eq!(p.nested_styles[2].repetition, 2);
        assert!(!p.nested_styles[2].inclusive);
        assert_eq!(p.grep_styles[0].grep_expression, r"\d+");
        assert_eq!(p.line_styles[0].line_count, 2);
        assert_eq!(p.line_styles[0].repeat_last, Some(-1));
        assert_eq!(p.empty_nested_styles, Some(false));
    }

    /// InDesign 20.0.1: a style that inherits its parent's lists writes
    /// none and `Empty*Styles="false"`; one that cleared them writes none
    /// and `"true"` — and then sets no nested style at all.
    #[test]
    fn an_inheriting_style_takes_its_parents_lists_and_a_cleared_one_none() {
        let s = parse_stylesheet(INDESIGN_NESTED.as_bytes()).unwrap();
        let inherits = s.resolve_paragraph("ParagraphStyle/Inherits");
        assert_eq!(inherits.nested_styles.len(), 3);
        assert_eq!(inherits.line_styles.len(), 1);
        assert_eq!(inherits.grep_styles.len(), 1);
        let cleared = s.resolve_paragraph("ParagraphStyle/Cleared");
        assert!(cleared.nested_styles.is_empty());
        assert!(cleared.line_styles.is_empty());
        assert!(cleared.grep_styles.is_empty());
    }

    /// The engine's older `<NestedStyle>` child still reads, for the
    /// packages written with it.
    #[test]
    fn legacy_nested_style_children_still_read() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup>
          <ParagraphStyle Self="ParagraphStyle/Old" Name="Old">
            <Properties><BasedOn type="string">$ID/[No paragraph style]</BasedOn></Properties>
            <NestedStyle AppliedCharacterStyle="CharacterStyle/Num" Delimiter="." Repetition="1" Inclusive="true"/>
            <NestedStyle AppliedCharacterStyle="CharacterStyle/Lead" Delimiter="Words" Repetition="2" Inclusive="false"/>
          </ParagraphStyle>
        </RootParagraphStyleGroup></idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let n = &s.paragraph_styles["ParagraphStyle/Old"].nested_styles;
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].delimiter, NestedDelimiter::Char('.'));
        assert_eq!(n[1].delimiter, NestedDelimiter::Words);
        assert!(!n[1].inclusive);
    }

    /// Where both spellings are present, InDesign's list wins.
    #[test]
    fn legacy_nested_style_children_yield_to_indesigns_list() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup>
          <ParagraphStyle Self="ParagraphStyle/Both" Name="Both">
            <NestedStyle AppliedCharacterStyle="CharacterStyle/Old" Delimiter="." Repetition="1" Inclusive="true"/>
            <Properties><AllNestedStyles type="list"><ListItem type="record"><AppliedCharacterStyle type="object">CharacterStyle/New</AppliedCharacterStyle><Delimiter type="enumeration">AnyWord</Delimiter><Repetition type="long">1</Repetition><Inclusive type="boolean">true</Inclusive></ListItem></AllNestedStyles></Properties>
          </ParagraphStyle>
        </RootParagraphStyleGroup></idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let n = &s.paragraph_styles["ParagraphStyle/Both"].nested_styles;
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].applied_character_style, "CharacterStyle/New");
    }

    /// InDesign 20.0.1 writes run formatting on paragraph styles
    /// (2026-10-12: `Ligatures="false" OTFHistorical="true"
    /// OTFStylisticAlternate="true" KerningMethod="$ID/Optical"`, …); the
    /// style carries it and it cascades through `BasedOn`.
    #[test]
    fn a_paragraph_style_reads_ligatures_kerning_and_opentype() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup>
          <ParagraphStyle Self="ParagraphStyle/OTF" Name="OTF" Ligatures="false" KerningMethod="$ID/Optical" OTFFigureStyle="ProportionalOldstyle" OTFOrdinal="true" OTFFraction="true" OTFDiscretionaryLigature="true" OTFTitling="true" OTFSlashedZero="true" OTFStylisticSets="5" OTFHistorical="true" OTFContextualAlternate="false" OTFSwash="true" OTFStylisticAlternate="true" OTFProportionalMetrics="true"/>
          <ParagraphStyle Self="ParagraphStyle/Child" Name="Child" OTFSwash="false"><Properties><BasedOn type="object">ParagraphStyle/OTF</BasedOn></Properties></ParagraphStyle>
        </RootParagraphStyleGroup></idPkg:Styles>"#;
        let s = parse_stylesheet(xml).unwrap();
        let p = &s.paragraph_styles["ParagraphStyle/OTF"];
        assert_eq!(p.ligatures_on, Some(false));
        assert_eq!(p.kerning_method.as_deref(), Some("$ID/Optical"));
        assert_eq!(p.otf.historical, Some(true));
        assert_eq!(p.otf.proportional_metrics, Some(true));
        assert_eq!(p.otf.stylistic_alternate, Some(true));
        assert_eq!(p.otf.contextual_alternates, Some(false));
        assert_eq!(p.otf.figure_style.as_deref(), Some("ProportionalOldstyle"));
        assert_eq!(p.otf.stylistic_sets, Some(5));
        let child = s.resolve_paragraph("ParagraphStyle/Child");
        assert_eq!(child.otf.swash, Some(false));
        assert_eq!(child.otf.historical, Some(true));
        assert_eq!(child.ligatures_on, Some(false));
        assert_eq!(child.kerning_method.as_deref(), Some("$ID/Optical"));
    }
}

#[cfg(test)]
mod object_style_text_frame_tests {
    use super::*;

    /// An object style's corners in InDesign 20.0.1's spelling: the four
    /// per-corner attributes (which InDesign reads) beside the pair (which
    /// it writes and ignores), and a style with only the pair.
    #[test]
    fn an_object_styles_four_corners_read_beside_its_pair() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x"><RootObjectStyleGroup>
<ObjectStyle Self="ObjectStyle/Corners" Name="Corners" TopLeftCornerOption="RoundedCorner" TopRightCornerOption="BevelCorner" BottomLeftCornerOption="RoundedCorner" BottomRightCornerOption="None" TopLeftCornerRadius="20" TopRightCornerRadius="8" BottomLeftCornerRadius="20" CornerRadius="20" CornerOption="RoundedCorner"/>
<ObjectStyle Self="ObjectStyle/Pair" Name="Pair" CornerRadius="20" CornerOption="RoundedCorner"/>
</RootObjectStyleGroup></idPkg:Styles>"#;
        let sheet = parse_stylesheet(xml).expect("styles");
        let c = &sheet.object_styles["ObjectStyle/Corners"];
        let spec = |option, radius| CornerSpec {
            option: Some(option),
            radius,
        };
        assert_eq!(
            c.corners,
            [
                spec(CornerOption::Rounded, Some(20.0)),
                spec(CornerOption::Bevel, Some(8.0)),
                spec(CornerOption::None, None),
                spec(CornerOption::Rounded, Some(20.0)),
            ]
        );
        assert_eq!(
            (c.corner_option.as_deref(), c.corner_radius),
            (Some("RoundedCorner"), Some(20.0))
        );
        let pair = &sheet.object_styles["ObjectStyle/Pair"];
        assert!(CornerSpec::all_unset(&pair.corners));
    }

    /// InDesign 20.0.1's own spelling of an object style's text-frame
    /// options: the flags on the style, the options on its
    /// `<TextFramePreference>`, the insets a typed `<InsetSpacing>` list.
    #[test]
    fn an_object_styles_text_frame_options_read_in_indesigns_spelling() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x"><RootObjectStyleGroup>
<ObjectStyle Self="ObjectStyle/Boxed" Name="Boxed" EnableTextFrameGeneralOptions="true" EnableTextFrameBaselineOptions="false">
<Properties><BasedOn type="object">ObjectStyle/$ID/[None]</BasedOn></Properties>
<TextFramePreference TextColumnCount="2" TextColumnGutter="12" FirstBaselineOffset="CapHeight" VerticalJustification="CenterAlign">
<Properties><InsetSpacing type="list"><ListItem type="unit">12</ListItem><ListItem type="unit">6</ListItem><ListItem type="unit">12</ListItem><ListItem type="unit">6</ListItem></InsetSpacing></Properties>
</TextFramePreference>
</ObjectStyle>
<ObjectStyle Self="ObjectStyle/Plain" Name="Plain"/>
</RootObjectStyleGroup></idPkg:Styles>"#;
        let mut sheet = parse_stylesheet(xml).expect("styles");
        read_object_style_text_frames(xml, &mut sheet.object_styles).expect("frames");
        let boxed = &sheet.object_styles["ObjectStyle/Boxed"];
        assert_eq!(boxed.enable.text_frame_general_options, Some(true));
        assert_eq!(boxed.enable.text_frame_baseline_options, Some(false));
        let tf = boxed.text_frame.as_ref().expect("options");
        assert_eq!(
            tf.vertical_justification,
            Some(VerticalJustification::Center)
        );
        assert_eq!(tf.inset_spacing, Some([12.0, 6.0, 12.0, 6.0]));
        assert_eq!((tf.column_count, tf.column_gutter), (Some(2), Some(12.0)));
        assert_eq!(
            tf.first_baseline_offset,
            Some(FirstBaselineOffset::CapHeight)
        );
        let plain = &sheet.object_styles["ObjectStyle/Plain"];
        assert!(plain.text_frame.is_none() && plain.enable.is_unset());
    }

    /// An object style's own text wrap and anchored-object options, as
    /// InDesign 20.0.1 writes them (ADR 143): the wrap's offsets a
    /// `<TextWrapOffset>` child, the anchoring every attribute.
    #[test]
    fn an_object_styles_wrap_and_anchoring_read_in_indesigns_spelling() {
        let xml = br#"<idPkg:Styles xmlns:idPkg="x"><RootObjectStyleGroup>
<ObjectStyle Self="ObjectStyle/W" Name="W" EnableTextWrapAndOthers="true" EnableAnchoredObjectOptions="true">
<TextWrapPreference Inverse="false" ApplyToMasterPageOnly="false" TextWrapSide="BothSides" TextWrapMode="BoundingBoxTextWrap">
<TextWrapOffset Top="4" Left="4" Bottom="4" Right="12"/>
<ContourOption ContourType="SameAsClipping" IncludeInsideEdges="false" ContourPathName="$ID/"/>
</TextWrapPreference>
<AnchoredObjectSetting AnchoredPosition="Anchored" SpineRelative="false" LockPosition="false" PinPosition="true" AnchorPoint="TopRightAnchor" HorizontalAlignment="RightAlign" HorizontalReferencePoint="TextFrame" VerticalAlignment="BottomAlign" VerticalReferencePoint="LineBaseline" AnchorXoffset="0" AnchorYoffset="6" AnchorSpaceAbove="0"/>
</ObjectStyle>
</RootObjectStyleGroup></idPkg:Styles>"#;
        let mut sheet = parse_stylesheet(xml).expect("styles");
        read_object_style_text_frames(xml, &mut sheet.object_styles).expect("frames");
        let w = &sheet.object_styles["ObjectStyle/W"];
        let wrap = w.own_text_wrap().expect("the style's wrap");
        assert_eq!(wrap.mode, paged_model::TextWrapMode::BoundingBoxTextWrap);
        assert_eq!(wrap.offsets, [4.0, 4.0, 4.0, 12.0]);
        assert_eq!(
            wrap.contour_type,
            Some(paged_model::ContourOptionType::SameAsClipping)
        );
        let a = w.own_anchored().expect("the style's anchoring");
        assert_eq!(a.anchored_position.as_deref(), Some("Anchored"));
        assert_eq!(a.anchor_point.as_deref(), Some("TopRightAnchor"));
        assert_eq!((a.anchor_x_offset, a.anchor_y_offset), (0.0, 6.0));
    }
}
