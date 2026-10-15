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

//! Tiny shared helpers used by every per-format parser in this crate.

/// Read an XML attribute by key. Returns `None` when absent or
/// non-UTF-8. Each parser submodule used to define its own copy of
/// this — `lib.rs` re-exports it here so they all share one.
pub(crate) fn attr(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| std::str::from_utf8(&a.value).ok().map(str::to_string))
}

/// Like [`attr`], but XML-NORMALIZES the value (entity unescaping) (`&quot;` → `"`,
/// `&amp;` → `&`, …). Required for free-text carriers — Label
/// `KeyValuePair` values hold JSON, which InDesign serialises with
/// escaped quotes. The plain [`attr`] stays raw because the numeric /
/// enum attributes it reads never contain entities.
pub(crate) fn attr_unescaped(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| {
            // IDML is XML 1.0 (every part declares it).
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

/// Decode InDesign's `<?AID xxxx?>` spelling of characters inside a
/// label value. InDesign 2025 writes every character outside the Basic
/// Multilingual Plane as one `<?AID xxxx?>` per UTF-16 code unit — an emoji
/// becomes `<?AID d83d?><?AID de00?>` (escaped `&lt;?AID d83d?&gt;` in the
/// attribute) — and decodes that form itself when it reads the label back
/// (the InDesign round-trip survey, 2026-10-06). A run of consecutive
/// escapes is decoded as UTF-16; a run that is not valid UTF-16 (a lone
/// surrogate) is kept verbatim rather than lost.
pub fn decode_aid_escapes(value: &str) -> std::borrow::Cow<'_, str> {
    const OPEN: &str = "<?AID ";
    if !value.contains(OPEN) {
        return std::borrow::Cow::Borrowed(value);
    }
    /// One `<?AID xxxx?>` at the start of `s`: (code unit, length).
    fn unit(s: &str) -> Option<(u16, usize)> {
        let rest = s.strip_prefix(OPEN)?;
        let end = rest.find("?>")?;
        let hex = &rest[..end];
        if hex.is_empty() || hex.len() > 4 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some((u16::from_str_radix(hex, 16).ok()?, OPEN.len() + end + 2))
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find(OPEN) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let mut units = Vec::new();
        let mut used = 0;
        while let Some((u, n)) = unit(&rest[used..]) {
            units.push(u);
            used += n;
        }
        if units.is_empty() {
            // `<?AID ` not followed by a code unit: literal text.
            out.push_str(OPEN);
            rest = &rest[OPEN.len()..];
            continue;
        }
        match String::from_utf16(&units) {
            Ok(text) => out.push_str(&text),
            Err(_) => out.push_str(&rest[..used]),
        }
        rest = &rest[used..];
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// A label `KeyValuePair`'s `Value`: XML-unescaped, then InDesign's
/// `<?AID xxxx?>` escapes decoded ([`decode_aid_escapes`]). Every label
/// reader (page items, pages, the designmap) goes through this one door.
pub(crate) fn label_value(e: &quick_xml::events::BytesStart) -> Option<String> {
    attr_unescaped(e, b"Value").map(|v| decode_aid_escapes(&v).into_owned())
}

/// The page-item kinds whose IDML `Name` is recorded (ADR 131: the six
/// `ElementId` page-item kinds, groups included).
pub(crate) const NAMED_ITEM_KINDS: [&[u8]; 6] = [
    b"TextFrame",
    b"Rectangle",
    b"Oval",
    b"Polygon",
    b"GraphicLine",
    b"Group",
];

/// Record `e`'s IDML `Name` under its `Self` when `e` is a named page
/// item. InDesign writes `Name="$ID/"` for an item nobody named; that, and
/// an empty name, record nothing.
pub(crate) fn record_item_name(
    e: &quick_xml::events::BytesStart,
    names: &mut std::collections::HashMap<String, String>,
) {
    if !NAMED_ITEM_KINDS.contains(&e.name().as_ref()) {
        return;
    }
    let (Some(self_id), Some(name)) = (attr(e, b"Self"), attr_unescaped(e, b"Name")) else {
        return;
    };
    if name.is_empty() || name == "$ID/" {
        return;
    }
    names.insert(self_id, name);
}

/// Parse an `f32` attribute by key. Returns `None` when the
/// attribute is absent, malformed, or non-finite. Convenience
/// wrapper used by the IDML effect parsers (XOffset, Size, Opacity,
/// Angle, etc.) to dedupe the `attr(...).and_then(|s| s.parse().ok())`
/// pattern that appeared 60+ times across the spread + styles
/// parsers.
pub(crate) fn parse_f(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<f32> {
    attr(e, key)?.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Interpret an IDML tint percentage SPELLING (`FillTint`, `GapTint`,
/// the table stroke tints).
///
/// Convention:
///   * `-1` → `None`. IDML's sentinel for "no tint override" — the same
///     document as writing no attribute at all.
///   * `0..=100` → `Some(pct)`; 100 = full strength.
///   * anything else (out of range, unparseable) → `None`, so a
///     malformed document can't silently distort the renderer's output.
///
/// Public, and the ONE place the rule lives, because both directions
/// need it. The parser reads it through [`parse_tint_attr`]. The
/// save-back writer replays it to answer a different question: "does the
/// model's `None` mean the same thing this source spelling does?" It
/// nearly always does — `FillTint="-1"` is 288 of the corpus's
/// attributes — and a writer that spells `None` as *delete the
/// attribute* silently rewrites 162 entries that nobody edited. A copy
/// of the range check on the writer's side would be two copies of one
/// decision, which is the defect one level up.
pub fn parse_tint(raw: &str) -> Option<f32> {
    let v: f32 = raw.parse().ok()?;
    if !(0.0..=100.0).contains(&v) {
        return None;
    }
    Some(v)
}

/// Parse an IDML tint percentage attribute by key. Absent ⇒ `None`
/// (no override), the same answer the `-1` sentinel gives; see
/// [`parse_tint`] for the whole rule.
/// A flag InDesign writes as a number: `RubyFlag="1"` on the run that
/// carries ruby (InDesign 20.0.1's own export). `true` / `false` read too,
/// the spelling older Paged builds and hand-written files use.
pub fn parse_flag(raw: &str) -> Option<bool> {
    match raw.trim() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

/// A paragraph's (or paragraph style's) place in a multi-level list:
/// `NumberingLevel` and `NumberingApplyRestartPolicy` (core ADR 158).
/// `<NumberingRestartPolicies>` is a `<Properties>` child, read by the
/// callers through [`restart_policy`].
pub(crate) fn list_level_attrs(e: &quick_xml::events::BytesStart) -> paged_model::ListLevel {
    paged_model::ListLevel {
        level: attr(e, b"NumberingLevel").and_then(|s| s.trim().parse().ok()),
        apply_restart: attr(e, b"NumberingApplyRestartPolicy").and_then(|s| s.trim().parse().ok()),
        restart_policy: None,
    }
}

/// `<NumberingRestartPolicies RestartPolicy="AnyPreviousLevel"
/// LowerLevel="0" UpperLevel="0"/>`, as InDesign writes it (core ADR 158).
pub(crate) fn restart_policy(
    e: &quick_xml::events::BytesStart,
) -> Option<paged_model::RestartPolicy> {
    let level = |key: &[u8]| {
        attr(e, key)
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };
    Some(paged_model::RestartPolicy {
        kind: paged_model::RestartPolicyKind::from_idml(&attr(e, b"RestartPolicy")?)?,
        lower: level(b"LowerLevel"),
        upper: level(b"UpperLevel"),
    })
}

/// A paragraph's (or paragraph style's) line-setting attributes:
/// `AutoLeading`, `SingleWordJustification`, `LastLineIndent` (core ADR
/// 153). `<BalanceRaggedLines>` is a typed child, read by the callers.
pub(crate) fn line_setting_attrs(e: &quick_xml::events::BytesStart) -> paged_model::LineSetting {
    paged_model::LineSetting {
        auto_leading: attr(e, b"AutoLeading").and_then(|s| s.parse().ok()),
        single_word: attr(e, b"SingleWordJustification")
            .and_then(|s| paged_model::SingleWordJustification::from_idml(&s)),
        last_line_indent: attr(e, b"LastLineIndent").and_then(|s| s.parse().ok()),
        balance: None,
    }
}

pub(crate) fn parse_tint_attr(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<f32> {
    parse_tint(&attr(e, key)?)
}
