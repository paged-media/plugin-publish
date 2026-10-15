//! The attributes of a `<ParagraphStyle>` / `<CharacterStyle>` /
//! `<ObjectStyle>` — the
//! definition InDesign reads, spelled once for the emitter (a style the
//! source lacks) and the patch lane (a style it has). A style minted or
//! edited by mutation used to reach the part with its name, size,
//! colour and face only: the annual's "Field Note" (8.5 pt, tracking 8,
//! 6.5 pt before and after, flush left, next style Annual Body) came
//! back to InDesign — and to the export's own twin — as an 8.5 pt slate
//! Annual Body (measured 2026-09-06). Typed `<Properties>` children
//! (BasedOn, AppliedFont, Leading, TabList, BulletChar, NumberingFormat,
//! AppliedNumberingList, NumberingExpression) are not attributes and are
//! spelled by their own passes (`based_on`, `face`, `paragraph_props`).

use idml_import::styles::{CharacterStyleDef, ObjectStyleDef, ParagraphStyleDef};

use crate::emit::{
    grid_alignment_attrs, line_setting_attrs, list_level_attrs, paragraph_rule_attrs,
    span_column_attrs, RULE_ABOVE, RULE_BELOW,
};
use crate::rewrite::{
    composer_patch, corner_attr_extras, corner_attr_patch, corner_attrs_of, format_f32,
    grid_alignment_patch, line_setting_patch, list_level_patch, numbering_list_patch,
    opt_bool_patch, opt_i32_patch, opt_string_patch, opt_u32_patch, paragraph_rule_patch,
    preserving_f32_patch, preserving_tint_patch, span_column_patch, Patch,
};

fn push_strs(out: &mut Vec<(&'static str, String)>, attrs: &[(&'static str, &Option<String>)]) {
    for (k, v) in attrs {
        if let Some(v) = v {
            out.push((k, v.clone()));
        }
    }
}

fn push_f32s(out: &mut Vec<(&'static str, String)>, attrs: &[(&'static str, Option<f32>)]) {
    for (k, v) in attrs {
        if let Some(v) = v {
            out.push((k, format_f32(*v)));
        }
    }
}

fn push_bools(out: &mut Vec<(&'static str, String)>, attrs: &[(&'static str, Option<bool>)]) {
    for (k, v) in attrs {
        if let Some(b) = v {
            out.push((k, b.to_string()));
        }
    }
}

/// The discrete `OTF*` toggles, as attributes (InDesign spells them so on
/// a `<ParagraphStyle>` as on a `<CharacterStyle>`).
fn push_otf(out: &mut Vec<(&'static str, String)>, otf: &idml_import::OtfFeatures) {
    push_bools(
        out,
        &[
            ("OTFFraction", otf.fraction),
            ("OTFOrdinal", otf.ordinal),
            ("OTFSwash", otf.swash),
            ("OTFDiscretionaryLigature", otf.discretionary_ligatures),
            ("OTFSlashedZero", otf.slashed_zero),
            ("OTFTitling", otf.titling),
            ("OTFContextualAlternate", otf.contextual_alternates),
            ("OTFStylisticAlternate", otf.stylistic_alternate),
            ("OTFHistorical", otf.historical),
        ],
    );
    push_strs(out, &[("OTFFigureStyle", &otf.figure_style)]);
    if let Some(n) = otf.stylistic_sets {
        out.push(("OTFStylisticSets", n.to_string()));
    }
}

/// A boolean the model owns, keeping the source's spelling whenever it
/// already says the model's value (or the model has none and the source
/// spells something the parser could not read).
fn preserving_bool_patch(raw: Option<&str>, v: Option<bool>) -> Patch {
    match (raw.map(|r| r.parse::<bool>()), v) {
        (Some(Ok(r)), Some(m)) if r == m => Patch::Keep,
        (Some(Err(_)), None) => Patch::Keep,
        _ => opt_bool_patch(v),
    }
}

/// Every attribute a `<ParagraphStyle>` carries, from the model.
pub(crate) fn paragraph_style_attrs(s: &ParagraphStyleDef) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = vec![("Self", s.self_id.clone())];
    push_strs(
        &mut out,
        &[
            ("Name", &s.name),
            ("FontStyle", &s.font_style),
            ("FillColor", &s.fill_color),
            ("StrokeColor", &s.stroke_color),
            ("Capitalization", &s.capitalization),
            ("Position", &s.position),
        ],
    );
    push_f32s(
        &mut out,
        &[
            ("PointSize", s.point_size),
            ("FillTint", s.fill_tint),
            ("StrokeWeight", s.stroke_weight),
            ("BaselineShift", s.baseline_shift),
            ("HorizontalScale", s.horizontal_scale),
            ("VerticalScale", s.vertical_scale),
            ("Skew", s.skew),
            ("Tracking", s.tracking),
        ],
    );
    if let Some(j) = s.justification {
        out.push(("Justification", j.as_idml().to_string()));
    }
    push_f32s(
        &mut out,
        &[
            ("FirstLineIndent", s.first_line_indent),
            ("LeftIndent", s.left_indent),
            ("RightIndent", s.right_indent),
            ("SpaceBefore", s.space_before),
            ("SpaceAfter", s.space_after),
        ],
    );
    push_bools(
        &mut out,
        &[
            ("Underline", s.underline),
            ("StrikeThru", s.strikethru),
            ("OverprintFill", s.overprint_fill),
            ("OverprintStroke", s.overprint_stroke),
            ("Ligatures", s.ligatures_on),
        ],
    );
    push_strs(&mut out, &[("KerningMethod", &s.kerning_method)]);
    push_otf(&mut out, &s.otf);
    // `true` on a style whose nested / line / GREP list is empty — one
    // that cleared its parent's; the lists themselves are `<Properties>`
    // children (see `nested_styles`).
    push_bools(
        &mut out,
        &[
            ("EmptyNestedStyles", s.empty_nested_styles),
            ("EmptyLineStyles", s.empty_line_styles),
            ("EmptyGrepStyles", s.empty_grep_styles),
        ],
    );
    // NOT here: `NumberingFormat`, `AppliedNumberingList`,
    // `NumberingExpression` — InDesign reads them only as typed
    // `<Properties>` children (measured 2026-09-06) — nor the marker
    // character styles, which it ignores as attributes (`list-overrides`,
    // 2026-10-01; `BulletsAndNumberingDigitsCharacterStyle` was a name of
    // ours). `paragraph_props` spells all of them as children.
    push_strs(
        &mut out,
        &[
            ("BulletsAndNumberingListType", &s.bullets_list_type),
            ("BulletsTextAfter", &s.bullets_text_after),
        ],
    );
    if let Some(n) = s.numbering_start_at {
        out.push(("NumberingStartAt", n.to_string()));
    }
    push_bools(&mut out, &[("NumberingContinue", s.numbering_continue)]);
    push_strs(&mut out, &[("NextStyle", &s.next_style)]);
    push_bools(&mut out, &[("Hyphenation", s.hyphenation)]);
    push_f32s(&mut out, &[("HyphenationZone", s.hyphenation_zone)]);
    if let Some(c) = &s.composer {
        out.push(("Composer", c.as_idml().to_string()));
    }
    push_strs(&mut out, &[("AppliedLanguage", &s.applied_language)]);
    push_f32s(
        &mut out,
        &[
            ("MinimumWordSpacing", s.minimum_word_spacing),
            ("DesiredWordSpacing", s.desired_word_spacing),
            ("MaximumWordSpacing", s.maximum_word_spacing),
            ("MinimumLetterSpacing", s.minimum_letter_spacing),
            ("DesiredLetterSpacing", s.desired_letter_spacing),
            ("MaximumLetterSpacing", s.maximum_letter_spacing),
            ("MinimumGlyphScaling", s.minimum_glyph_scaling),
            ("DesiredGlyphScaling", s.desired_glyph_scaling),
            ("MaximumGlyphScaling", s.maximum_glyph_scaling),
        ],
    );
    if let Some(n) = s.drop_cap_characters {
        out.push(("DropCapCharacters", n.to_string()));
    }
    if let Some(n) = s.drop_cap_lines {
        out.push(("DropCapLines", n.to_string()));
    }
    if let Some(n) = s.drop_cap_detail {
        // InDesign's spelling, lowercase c.
        out.push(("DropcapDetail", n.to_string()));
    }
    push_strs(
        &mut out,
        &[
            ("KinsokuSet", &s.kinsoku_set),
            ("KinsokuType", &s.kinsoku_type),
            ("MojikumiTable", &s.mojikumi_table),
            ("MojikumiSet", &s.mojikumi_set),
        ],
    );
    span_column_attrs(&mut out, &s.span_columns);
    line_setting_attrs(&mut out, &s.line_setting);
    list_level_attrs(&mut out, &s.list_level);
    grid_alignment_attrs(&mut out, s.grid_alignment, s.grid_align_first_line_only);
    paragraph_rule_attrs(&mut out, &RULE_ABOVE, &s.rule_above);
    paragraph_rule_attrs(&mut out, &RULE_BELOW, &s.rule_below);
    out
}

/// Every attribute a `<CharacterStyle>` carries, from the model.
pub(crate) fn character_style_attrs(s: &CharacterStyleDef) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = vec![("Self", s.self_id.clone())];
    push_strs(
        &mut out,
        &[
            ("Name", &s.name),
            ("FontStyle", &s.font_style),
            ("FillColor", &s.fill_color),
            ("StrokeColor", &s.stroke_color),
            ("Capitalization", &s.capitalization),
            ("Position", &s.position),
        ],
    );
    push_f32s(
        &mut out,
        &[
            ("PointSize", s.point_size),
            ("FillTint", s.fill_tint),
            ("StrokeWeight", s.stroke_weight),
            ("BaselineShift", s.baseline_shift),
            ("HorizontalScale", s.horizontal_scale),
            ("VerticalScale", s.vertical_scale),
            ("Skew", s.skew),
            ("Tracking", s.tracking),
        ],
    );
    push_bools(
        &mut out,
        &[
            ("Underline", s.underline),
            ("StrikeThru", s.strikethru),
            ("OverprintFill", s.overprint_fill),
            ("OverprintStroke", s.overprint_stroke),
            ("Ligatures", s.ligatures_on),
            ("NoBreak", s.no_break),
        ],
    );
    // InDesign writes the ruby flag as a number (`RubyFlag="1"`).
    if let Some(f) = s.ruby_flag {
        out.push(("RubyFlag", if f { "1" } else { "0" }.to_string()));
    }
    push_strs(
        &mut out,
        &[
            ("RubyType", &s.ruby_type),
            ("RubyString", &s.ruby_string),
            ("KentenKind", &s.kenten_kind),
            ("KentenCustomCharacter", &s.kenten_character),
            ("KerningMethod", &s.kerning_method),
            ("RubyPosition", &s.marks.ruby_position),
            ("RubyAlignment", &s.marks.ruby_alignment),
            ("RubyParentSpacing", &s.marks.ruby_parent_spacing),
            ("RubyParentOverhangAmount", &s.marks.ruby_overhang_amount),
            ("KentenPosition", &s.marks.kenten_position),
            ("KentenAlignment", &s.marks.kenten_alignment),
        ],
    );
    let m = &s.marks;
    push_f32s(
        &mut out,
        &[
            ("KentenFontSize", s.kenten_font_size),
            ("RubyFontSize", m.ruby_font_size),
            ("RubyXScale", m.ruby_x_scale),
            ("RubyYScale", m.ruby_y_scale),
            ("RubyXOffset", m.ruby_x_offset),
            ("RubyYOffset", m.ruby_y_offset),
            ("KentenXScale", m.kenten_x_scale),
            ("KentenYScale", m.kenten_y_scale),
            ("KentenPlacement", m.kenten_placement),
            ("RubyParentScalingPercent", m.ruby_parent_scaling_percent),
        ],
    );
    push_bools(&mut out, &[("RubyAutoScaling", m.ruby_auto_scaling)]);
    push_otf(&mut out, &s.otf);
    out
}

/// A swatch reference the parser reads as "none" (`Swatch/None`)
/// keeps its bytes; a `None` model never deletes that spelling.
fn swatch_patch(raw: Option<&str>, v: &Option<String>) -> Patch {
    match v {
        Some(s) if raw == Some(s.as_str()) => Patch::Keep,
        Some(s) => Patch::Set(s.clone()),
        None => match raw {
            Some("Swatch/None") | Some("n") => Patch::Keep,
            Some(_) => Patch::Remove,
            None => Patch::Remove,
        },
    }
}

/// The `<ParagraphStyle>` attributes the model owns; anything else
/// (`Self`, the OTF flags and `Empty*Styles` — no setter writes them —,
/// the border and shading families, a `BasedOn` or `AppliedFont`
/// attribute another pass converts) passes through.
pub(crate) fn paragraph_style_attr_patch(
    key: &[u8],
    raw: &[u8],
    s: &ParagraphStyleDef,
) -> Option<Patch> {
    let raw = std::str::from_utf8(raw).ok();
    match key {
        b"Name" => Some(opt_string_patch(&s.name)),
        b"FontStyle" => Some(opt_string_patch(&s.font_style)),
        b"PointSize" => Some(preserving_f32_patch(raw, s.point_size)),
        b"FillColor" => Some(swatch_patch(raw, &s.fill_color)),
        b"FillTint" => Some(preserving_tint_patch(raw, s.fill_tint)),
        b"StrokeColor" => Some(swatch_patch(raw, &s.stroke_color)),
        b"StrokeWeight" => Some(preserving_f32_patch(raw, s.stroke_weight)),
        b"Capitalization" => Some(opt_string_patch(&s.capitalization)),
        b"BaselineShift" => Some(preserving_f32_patch(raw, s.baseline_shift)),
        b"HorizontalScale" => Some(preserving_f32_patch(raw, s.horizontal_scale)),
        b"VerticalScale" => Some(preserving_f32_patch(raw, s.vertical_scale)),
        b"Skew" => Some(preserving_f32_patch(raw, s.skew)),
        b"Position" => Some(opt_string_patch(&s.position)),
        b"Tracking" => Some(preserving_f32_patch(raw, s.tracking)),
        b"Justification" => Some(match s.justification {
            Some(j) if raw == Some(j.as_idml()) => Patch::Keep,
            Some(j) => Patch::Set(j.as_idml().to_string()),
            None => Patch::Remove,
        }),
        b"FirstLineIndent" => Some(preserving_f32_patch(raw, s.first_line_indent)),
        b"LeftIndent" => Some(preserving_f32_patch(raw, s.left_indent)),
        b"RightIndent" => Some(preserving_f32_patch(raw, s.right_indent)),
        b"SpaceBefore" => Some(preserving_f32_patch(raw, s.space_before)),
        b"SpaceAfter" => Some(preserving_f32_patch(raw, s.space_after)),
        b"Underline" => Some(opt_bool_patch(s.underline)),
        b"StrikeThru" => Some(opt_bool_patch(s.strikethru)),
        b"OverprintFill" => Some(opt_bool_patch(s.overprint_fill)),
        b"OverprintStroke" => Some(opt_bool_patch(s.overprint_stroke)),
        // A paragraph style's own setters write these two.
        b"Ligatures" => Some(preserving_bool_patch(raw, s.ligatures_on)),
        b"KerningMethod" => Some(opt_string_patch(&s.kerning_method)),
        b"BulletsAndNumberingListType" => Some(opt_string_patch(&s.bullets_list_type)),
        b"BulletsTextAfter" => Some(opt_string_patch(&s.bullets_text_after)),
        // Attribute spellings of what InDesign reads as children: an
        // older file of ours may carry them; patched in place, never
        // added (`paragraph_props` spells the children).
        b"BulletsCharacterStyle" => Some(opt_string_patch(&s.bullets_character_style)),
        b"BulletsAndNumberingDigitsCharacterStyle" => Some(opt_string_patch(
            &s.bullets_and_numbering_digits_character_style,
        )),
        b"NumberingFormat" => Some(opt_string_patch(&s.numbering_format)),
        b"NumberingExpression" => Some(opt_string_patch(&s.numbering_expression)),
        b"AppliedNumberingList" => Some(numbering_list_patch(raw, &s.applied_numbering_list)),
        b"NumberingStartAt" => Some(opt_i32_patch(raw, s.numbering_start_at)),
        b"NumberingContinue" => Some(opt_bool_patch(s.numbering_continue)),
        b"NextStyle" => Some(opt_string_patch(&s.next_style)),
        b"Hyphenation" => Some(opt_bool_patch(s.hyphenation)),
        b"HyphenationZone" => Some(preserving_f32_patch(raw, s.hyphenation_zone)),
        b"Composer" => Some(composer_patch(raw, &s.composer)),
        b"AppliedLanguage" => Some(opt_string_patch(&s.applied_language)),
        b"MinimumWordSpacing" => Some(preserving_f32_patch(raw, s.minimum_word_spacing)),
        b"DesiredWordSpacing" => Some(preserving_f32_patch(raw, s.desired_word_spacing)),
        b"MaximumWordSpacing" => Some(preserving_f32_patch(raw, s.maximum_word_spacing)),
        b"MinimumLetterSpacing" => Some(preserving_f32_patch(raw, s.minimum_letter_spacing)),
        b"DesiredLetterSpacing" => Some(preserving_f32_patch(raw, s.desired_letter_spacing)),
        b"MaximumLetterSpacing" => Some(preserving_f32_patch(raw, s.maximum_letter_spacing)),
        b"MinimumGlyphScaling" => Some(preserving_f32_patch(raw, s.minimum_glyph_scaling)),
        b"DesiredGlyphScaling" => Some(preserving_f32_patch(raw, s.desired_glyph_scaling)),
        b"MaximumGlyphScaling" => Some(preserving_f32_patch(raw, s.maximum_glyph_scaling)),
        b"DropCapCharacters" => Some(opt_u32_patch(raw, s.drop_cap_characters)),
        b"DropCapLines" => Some(opt_u32_patch(raw, s.drop_cap_lines)),
        b"DropcapDetail" => Some(opt_i32_patch(raw, s.drop_cap_detail)),
        // The spelling older Paged builds wrote: the value is written again
        // in InDesign's spelling above, so the old key goes.
        b"DropCapDetail" => Some(Patch::Remove),
        b"KinsokuSet" => Some(opt_string_patch(&s.kinsoku_set)),
        b"KinsokuType" => Some(opt_string_patch(&s.kinsoku_type)),
        b"MojikumiTable" => Some(opt_string_patch(&s.mojikumi_table)),
        b"MojikumiSet" => Some(opt_string_patch(&s.mojikumi_set)),
        _ => span_column_patch(key, raw, &s.span_columns)
            .or_else(|| line_setting_patch(key, raw, &s.line_setting))
            .or_else(|| list_level_patch(key, raw, &s.list_level))
            .or_else(|| {
                grid_alignment_patch(key, raw, s.grid_alignment, s.grid_align_first_line_only)
            })
            .or_else(|| paragraph_rule_patch(key, raw, &RULE_ABOVE, &s.rule_above))
            .or_else(|| paragraph_rule_patch(key, raw, &RULE_BELOW, &s.rule_below)),
    }
}

/// The `<CharacterStyle>` attributes the model owns.
pub(crate) fn character_style_attr_patch(
    key: &[u8],
    raw: &[u8],
    s: &CharacterStyleDef,
) -> Option<Patch> {
    let raw = std::str::from_utf8(raw).ok();
    match key {
        b"Name" => Some(opt_string_patch(&s.name)),
        b"FontStyle" => Some(opt_string_patch(&s.font_style)),
        b"PointSize" => Some(preserving_f32_patch(raw, s.point_size)),
        b"FillColor" => Some(swatch_patch(raw, &s.fill_color)),
        b"FillTint" => Some(preserving_tint_patch(raw, s.fill_tint)),
        b"StrokeColor" => Some(swatch_patch(raw, &s.stroke_color)),
        b"StrokeWeight" => Some(preserving_f32_patch(raw, s.stroke_weight)),
        b"Capitalization" => Some(opt_string_patch(&s.capitalization)),
        b"BaselineShift" => Some(preserving_f32_patch(raw, s.baseline_shift)),
        b"HorizontalScale" => Some(preserving_f32_patch(raw, s.horizontal_scale)),
        b"VerticalScale" => Some(preserving_f32_patch(raw, s.vertical_scale)),
        b"Skew" => Some(preserving_f32_patch(raw, s.skew)),
        b"Position" => Some(opt_string_patch(&s.position)),
        b"Tracking" => Some(preserving_f32_patch(raw, s.tracking)),
        b"Underline" => Some(opt_bool_patch(s.underline)),
        b"StrikeThru" => Some(opt_bool_patch(s.strikethru)),
        b"OverprintFill" => Some(opt_bool_patch(s.overprint_fill)),
        b"OverprintStroke" => Some(opt_bool_patch(s.overprint_stroke)),
        // A source spelling of the same value (`1` or `true`) keeps its
        // bytes; a change is written as InDesign writes it.
        b"RubyFlag" => Some(match s.ruby_flag {
            Some(f) if raw.and_then(idml_import::parse_flag) == Some(f) => Patch::Keep,
            Some(f) => Patch::Set(if f { "1" } else { "0" }.to_string()),
            None => Patch::Remove,
        }),
        b"Ligatures" => Some(opt_bool_patch(s.ligatures_on)),
        b"NoBreak" => Some(preserving_bool_patch(raw, s.no_break)),
        b"RubyType" => Some(opt_string_patch(&s.ruby_type)),
        b"RubyString" => Some(opt_string_patch(&s.ruby_string)),
        b"KentenKind" => Some(opt_string_patch(&s.kenten_kind)),
        b"KentenCustomCharacter" => Some(opt_string_patch(&s.kenten_character)),
        b"KerningMethod" => Some(opt_string_patch(&s.kerning_method)),
        b"KentenFontSize" => Some(preserving_f32_patch(raw, s.kenten_font_size)),
        b"RubyFontSize" => Some(preserving_f32_patch(raw, s.marks.ruby_font_size)),
        b"RubyXScale" => Some(preserving_f32_patch(raw, s.marks.ruby_x_scale)),
        b"RubyYScale" => Some(preserving_f32_patch(raw, s.marks.ruby_y_scale)),
        b"RubyXOffset" => Some(preserving_f32_patch(raw, s.marks.ruby_x_offset)),
        b"RubyYOffset" => Some(preserving_f32_patch(raw, s.marks.ruby_y_offset)),
        b"RubyPosition" => Some(opt_string_patch(&s.marks.ruby_position)),
        b"RubyAlignment" => Some(opt_string_patch(&s.marks.ruby_alignment)),
        b"RubyParentSpacing" => Some(opt_string_patch(&s.marks.ruby_parent_spacing)),
        b"RubyParentOverhangAmount" => Some(opt_string_patch(&s.marks.ruby_overhang_amount)),
        b"RubyAutoScaling" => Some(preserving_bool_patch(raw, s.marks.ruby_auto_scaling)),
        b"RubyParentScalingPercent" => Some(preserving_f32_patch(
            raw,
            s.marks.ruby_parent_scaling_percent,
        )),
        b"KentenXScale" => Some(preserving_f32_patch(raw, s.marks.kenten_x_scale)),
        b"KentenYScale" => Some(preserving_f32_patch(raw, s.marks.kenten_y_scale)),
        b"KentenPlacement" => Some(preserving_f32_patch(raw, s.marks.kenten_placement)),
        b"KentenPosition" => Some(opt_string_patch(&s.marks.kenten_position)),
        b"KentenAlignment" => Some(opt_string_patch(&s.marks.kenten_alignment)),
        _ => None,
    }
}

/// Every attribute an `<ObjectStyle>` carries, from the model: fill,
/// stroke, and the corners — the all-corner pair and the four per-corner
/// attributes, which are the ones InDesign reads (ADR 149). The `Enable…`
/// switches have no setter and are not written for a new style.
pub(crate) fn object_style_attrs(s: &ObjectStyleDef) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    push_strs(&mut out, &[("Name", &s.name), ("FillColor", &s.fill_color)]);
    push_f32s(&mut out, &[("FillTint", s.fill_tint)]);
    push_strs(&mut out, &[("StrokeColor", &s.stroke_color)]);
    push_f32s(
        &mut out,
        &[
            ("StrokeTint", s.stroke_tint),
            ("StrokeWeight", s.stroke_weight),
        ],
    );
    out.extend(corner_attr_extras(&corner_attrs_of(
        s.corner_radius,
        &s.corner_option,
        &s.corners,
    )));
    out
}

/// The `<ObjectStyle>` attributes the model owns. An edited source style
/// used to pass through with its source attributes, so a fill, stroke or
/// corner set on it was lost on save.
pub(crate) fn object_style_attr_patch(key: &[u8], raw: &[u8], s: &ObjectStyleDef) -> Option<Patch> {
    let text = std::str::from_utf8(raw).ok();
    match key {
        b"Name" => Some(opt_string_patch(&s.name)),
        b"FillColor" => Some(swatch_patch(text, &s.fill_color)),
        b"FillTint" => Some(preserving_tint_patch(text, s.fill_tint)),
        b"StrokeColor" => Some(swatch_patch(text, &s.stroke_color)),
        b"StrokeTint" => Some(preserving_tint_patch(text, s.stroke_tint)),
        b"StrokeWeight" => Some(preserving_f32_patch(text, s.stroke_weight)),
        _ => corner_attr_patch(
            key,
            raw,
            &corner_attrs_of(s.corner_radius, &s.corner_option, &s.corners),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_level_is_a_style_attribute() {
        // Core ADR 158: a paragraph style's NumberingLevel and
        // NumberingApplyRestartPolicy; the policy is a `<Properties>`
        // child, not here.
        let p = ParagraphStyleDef {
            self_id: "ParagraphStyle/P".into(),
            list_level: idml_import::ListLevel {
                level: Some(2),
                apply_restart: Some(false),
                restart_policy: Some(idml_import::RestartPolicy {
                    kind: idml_import::RestartPolicyKind::AnyPreviousLevel,
                    lower: 0,
                    upper: 0,
                }),
            },
            ..Default::default()
        };
        let attrs = paragraph_style_attrs(&p);
        for (k, v) in [
            ("NumberingLevel", "2"),
            ("NumberingApplyRestartPolicy", "false"),
        ] {
            assert!(
                attrs.iter().any(|(a, b)| *a == k && b == v),
                "{k}: {attrs:?}"
            );
        }
        assert!(!attrs.iter().any(|(a, _)| *a == "NumberingRestartPolicies"));
        assert!(matches!(
            paragraph_style_attr_patch(b"NumberingLevel", b"2", &p),
            Some(Patch::Keep)
        ));
        assert!(matches!(
            paragraph_style_attr_patch(b"NumberingApplyRestartPolicy", b"true", &p),
            Some(Patch::Set(v)) if v == "false"
        ));
    }

    #[test]
    fn line_setting_and_no_break_are_style_attributes() {
        // Core ADR 153: a paragraph style's AutoLeading,
        // SingleWordJustification and LastLineIndent, a character
        // style's NoBreak; balance is a `<Properties>` child, not here.
        let p = ParagraphStyleDef {
            self_id: "ParagraphStyle/P".into(),
            line_setting: idml_import::LineSetting {
                auto_leading: Some(140.0),
                single_word: Some(idml_import::SingleWordJustification::LeftAlign),
                last_line_indent: Some(12.5),
                balance: Some(idml_import::BalanceRaggedLines::FullyBalanced),
            },
            ..Default::default()
        };
        let attrs = paragraph_style_attrs(&p);
        for (k, v) in [
            ("AutoLeading", "140"),
            ("SingleWordJustification", "LeftAlign"),
            ("LastLineIndent", "12.5"),
        ] {
            assert!(
                attrs.iter().any(|(a, b)| *a == k && b == v),
                "{k}: {attrs:?}"
            );
        }
        assert!(!attrs.iter().any(|(a, _)| *a == "BalanceRaggedLines"));
        assert!(matches!(
            paragraph_style_attr_patch(b"AutoLeading", b"140", &p),
            Some(Patch::Keep)
        ));
        assert!(matches!(
            paragraph_style_attr_patch(b"LastLineIndent", b"3", &p),
            Some(Patch::Set(v)) if v == "12.5"
        ));

        let c = CharacterStyleDef {
            self_id: "CharacterStyle/C".into(),
            no_break: Some(true),
            ..Default::default()
        };
        assert!(character_style_attrs(&c)
            .iter()
            .any(|(a, b)| *a == "NoBreak" && b == "true"));
        assert!(matches!(
            character_style_attr_patch(b"NoBreak", b"true", &c),
            Some(Patch::Keep)
        ));
    }

    #[test]
    fn ruby_flag_is_written_as_indesign_writes_it() {
        // InDesign 20.0.1 writes `RubyFlag="1"`.
        let on = CharacterStyleDef {
            ruby_flag: Some(true),
            ..Default::default()
        };
        assert!(character_style_attrs(&on).contains(&("RubyFlag", "1".to_string())));
        // A source spelling of the same value keeps its bytes…
        for raw in [&b"1"[..], b"true"] {
            assert!(matches!(
                character_style_attr_patch(b"RubyFlag", raw, &on),
                Some(Patch::Keep)
            ));
        }
        // …and a change is written InDesign's way.
        let off = CharacterStyleDef {
            ruby_flag: Some(false),
            ..Default::default()
        };
        assert!(matches!(
            character_style_attr_patch(b"RubyFlag", b"1", &off),
            Some(Patch::Set(ref v)) if v == "0"
        ));
    }

    /// A ruby's parent spacing, overhang and auto-scaling go out as
    /// InDesign spells them, and an unchanged source spelling stays.
    #[test]
    fn ruby_parent_settings_are_style_attributes() {
        let s = CharacterStyleDef {
            marks: paged_model::MarkSettings {
                ruby_parent_spacing: Some("RubyParentEqualAki".into()),
                ruby_overhang_amount: Some("RubyOverhangNoLimit".into()),
                ruby_auto_scaling: Some(true),
                ruby_parent_scaling_percent: Some(80.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let attrs = character_style_attrs(&s);
        for (k, v) in [
            ("RubyParentSpacing", "RubyParentEqualAki"),
            ("RubyParentOverhangAmount", "RubyOverhangNoLimit"),
            ("RubyAutoScaling", "true"),
            ("RubyParentScalingPercent", "80"),
        ] {
            assert!(attrs.contains(&(k, v.to_string())), "{k}: {attrs:?}");
        }
        assert!(matches!(
            character_style_attr_patch(b"RubyParentScalingPercent", b"80.0", &s),
            Some(Patch::Keep)
        ));
        assert!(matches!(
            character_style_attr_patch(b"RubyParentSpacing", b"RubyParent121Aki", &s),
            Some(Patch::Set(ref v)) if v == "RubyParentEqualAki"
        ));
    }

    #[test]
    fn drop_cap_detail_is_written_in_indesigns_spelling() {
        let s = ParagraphStyleDef {
            drop_cap_detail: Some(2),
            ..Default::default()
        };
        assert!(paragraph_style_attrs(&s).contains(&("DropcapDetail", "2".to_string())));
        assert!(!paragraph_style_attrs(&s)
            .iter()
            .any(|(k, _)| *k == "DropCapDetail"));
        // The camel-cased key older Paged builds wrote is dropped on save;
        // the value goes out again in InDesign's spelling.
        assert!(matches!(
            paragraph_style_attr_patch(b"DropCapDetail", b"2", &s),
            Some(Patch::Remove)
        ));
        assert!(matches!(
            paragraph_style_attr_patch(b"DropcapDetail", b"2", &s),
            Some(Patch::Keep)
        ));
    }
}
