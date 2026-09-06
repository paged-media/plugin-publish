//! The attributes of a `<ParagraphStyle>` / `<CharacterStyle>` — the
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

use idml_import::styles::{CharacterStyleDef, ParagraphStyleDef};

use crate::emit::{paragraph_rule_attrs, RULE_ABOVE, RULE_BELOW};
use crate::rewrite::{
    format_f32, numbering_list_patch, opt_bool_patch, opt_string_patch, opt_u32_patch,
    paragraph_rule_patch, preserving_f32_patch, preserving_tint_patch, Patch,
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
        ],
    );
    // NOT here: `NumberingFormat`, `AppliedNumberingList`,
    // `NumberingExpression` — InDesign reads them only as typed
    // `<Properties>` children (measured 2026-09-06).
    push_strs(
        &mut out,
        &[
            ("BulletsAndNumberingListType", &s.bullets_list_type),
            ("BulletsTextAfter", &s.bullets_text_after),
            ("BulletsCharacterStyle", &s.bullets_character_style),
            (
                "BulletsAndNumberingDigitsCharacterStyle",
                &s.bullets_and_numbering_digits_character_style,
            ),
        ],
    );
    if let Some(n) = s.numbering_start_at {
        out.push(("NumberingStartAt", n.to_string()));
    }
    push_bools(&mut out, &[("NumberingContinue", s.numbering_continue)]);
    push_strs(&mut out, &[("NextStyle", &s.next_style)]);
    push_bools(&mut out, &[("Hyphenation", s.hyphenation)]);
    push_f32s(&mut out, &[("HyphenationZone", s.hyphenation_zone)]);
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
        out.push(("DropCapDetail", n.to_string()));
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
            ("RubyFlag", s.ruby_flag),
            ("Ligatures", s.ligatures_on),
        ],
    );
    push_strs(
        &mut out,
        &[
            ("RubyType", &s.ruby_type),
            ("RubyString", &s.ruby_string),
            ("KentenKind", &s.kenten_kind),
            ("KentenCharacter", &s.kenten_character),
            ("KerningMethod", &s.kerning_method),
        ],
    );
    push_f32s(&mut out, &[("KentenFontSize", s.kenten_font_size)]);
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

fn opt_i32_patch(raw: Option<&str>, v: Option<i32>) -> Patch {
    match v {
        Some(n) => {
            if raw.and_then(|s| s.trim().parse::<i32>().ok()) == Some(n) {
                Patch::Keep
            } else {
                Patch::Set(n.to_string())
            }
        }
        None => Patch::Remove,
    }
}

/// The `<ParagraphStyle>` attributes the model owns; anything else
/// (`Self`, the OTF flags, the border and shading families, a `BasedOn`
/// or `AppliedFont` attribute another pass converts) passes through.
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
        b"BulletsAndNumberingListType" => Some(opt_string_patch(&s.bullets_list_type)),
        b"BulletsTextAfter" => Some(opt_string_patch(&s.bullets_text_after)),
        b"BulletsCharacterStyle" => Some(opt_string_patch(&s.bullets_character_style)),
        b"BulletsAndNumberingDigitsCharacterStyle" => Some(opt_string_patch(
            &s.bullets_and_numbering_digits_character_style,
        )),
        // Attribute spellings of what InDesign reads as children: an
        // older file of ours may carry them; patched in place, never
        // added (`paragraph_props` spells the children).
        b"NumberingFormat" => Some(opt_string_patch(&s.numbering_format)),
        b"NumberingExpression" => Some(opt_string_patch(&s.numbering_expression)),
        b"AppliedNumberingList" => Some(numbering_list_patch(raw, &s.applied_numbering_list)),
        b"NumberingStartAt" => Some(opt_i32_patch(raw, s.numbering_start_at)),
        b"NumberingContinue" => Some(opt_bool_patch(s.numbering_continue)),
        b"NextStyle" => Some(opt_string_patch(&s.next_style)),
        b"Hyphenation" => Some(opt_bool_patch(s.hyphenation)),
        b"HyphenationZone" => Some(preserving_f32_patch(raw, s.hyphenation_zone)),
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
        b"DropCapDetail" => Some(opt_i32_patch(raw, s.drop_cap_detail)),
        b"KinsokuSet" => Some(opt_string_patch(&s.kinsoku_set)),
        b"KinsokuType" => Some(opt_string_patch(&s.kinsoku_type)),
        b"MojikumiTable" => Some(opt_string_patch(&s.mojikumi_table)),
        b"MojikumiSet" => Some(opt_string_patch(&s.mojikumi_set)),
        _ => paragraph_rule_patch(key, raw, &RULE_ABOVE, &s.rule_above)
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
        b"RubyFlag" => Some(opt_bool_patch(s.ruby_flag)),
        b"Ligatures" => Some(opt_bool_patch(s.ligatures_on)),
        b"RubyType" => Some(opt_string_patch(&s.ruby_type)),
        b"RubyString" => Some(opt_string_patch(&s.ruby_string)),
        b"KentenKind" => Some(opt_string_patch(&s.kenten_kind)),
        b"KentenCharacter" => Some(opt_string_patch(&s.kenten_character)),
        b"KerningMethod" => Some(opt_string_patch(&s.kerning_method)),
        b"KentenFontSize" => Some(preserving_f32_patch(raw, s.kenten_font_size)),
        _ => None,
    }
}
