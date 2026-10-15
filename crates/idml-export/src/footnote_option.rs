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

//! The document's footnote options on `<FootnoteOption>` in
//! `Resources/Preferences.xml` (core ADR 162), in InDesign's spelling:
//! the numbers, texts and switches as attributes, the numbering style,
//! the restart and where the prefix and suffix show as typed
//! `<Properties>` enumeration children (InDesign 20.0.1 writes them so,
//! and reads them only so — core ADR 156).
//!
//! Byte-identical while the part already says what the model says, each
//! value compared as InDesign reads it (an absent value is its default);
//! otherwise each value that differs is rewritten in place, a missing one
//! is added, and a part with no element gains one when the model departs
//! from InDesign's defaults.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

const ROOT: &[u8] = b"idPkg:Preferences";
const ELEMENT: &[u8] = b"FootnoteOption";

/// What one attribute holds, compared as InDesign reads it.
#[derive(Clone, Copy)]
enum Kind {
    Text,
    Bool,
    Number,
}

fn same(kind: Kind, a: &str, b: &str) -> bool {
    match kind {
        Kind::Text => a == b,
        Kind::Bool => a.trim().parse::<bool>().ok() == b.trim().parse::<bool>().ok(),
        Kind::Number => match (a.trim().parse::<f32>(), b.trim().parse::<f32>()) {
            (Ok(x), Ok(y)) => (x - y).abs() < 1e-4,
            _ => a == b,
        },
    }
}

/// `(attribute, kind, the model's value as InDesign reads it)`.
fn attributes(fo: &paged_model::FootnoteOptions) -> Vec<(&'static str, Kind, String)> {
    let num = |v: Option<f32>, d: f32| crate::rewrite::format_f32(v.unwrap_or(d));
    vec![
        (
            "StartAt",
            Kind::Number,
            fo.start_at.unwrap_or(1).to_string(),
        ),
        ("Prefix", Kind::Text, fo.prefix.clone().unwrap_or_default()),
        ("Suffix", Kind::Text, fo.suffix.clone().unwrap_or_default()),
        (
            "EnableStraddling",
            Kind::Bool,
            fo.enable_straddling.unwrap_or(false).to_string(),
        ),
        (
            "NoSplitting",
            Kind::Bool,
            fo.no_splitting.unwrap_or(false).to_string(),
        ),
        ("SpaceBetween", Kind::Number, num(fo.space_between, 0.0)),
        ("Spacer", Kind::Number, num(fo.spacer, 0.0)),
        ("RuleOn", Kind::Bool, fo.rule_on_effective().to_string()),
        (
            "ContinuingRuleOn",
            Kind::Bool,
            fo.continuing_rule.on.unwrap_or(true).to_string(),
        ),
    ]
}

/// The attribute defaults InDesign reads when one is absent.
fn attribute_default(name: &str) -> &'static str {
    match name {
        "StartAt" => "1",
        "Prefix" | "Suffix" => "",
        "EnableStraddling" | "NoSplitting" => "false",
        "SpaceBetween" | "Spacer" => "0",
        _ => "true",
    }
}

/// `(child, the model's value)`: the typed enumeration children.
fn children(fo: &paged_model::FootnoteOptions) -> [(&'static str, String); 3] {
    [
        (
            "FootnoteNumberingStyle",
            fo.numbering_style
                .clone()
                .unwrap_or_else(|| "Arabic".into()),
        ),
        (
            "RestartNumbering",
            fo.restart_numbering
                .clone()
                .unwrap_or_else(|| "DontRestart".into()),
        ),
        (
            "ShowPrefixSuffix",
            fo.show_prefix_suffix
                .clone()
                .unwrap_or_else(|| "NoPrefixSuffix".into()),
        ),
    ]
}

fn child_default(name: &str) -> &'static str {
    match name {
        "FootnoteNumberingStyle" => "Arabic",
        "RestartNumbering" => "DontRestart",
        _ => "NoPrefixSuffix",
    }
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name.as_bytes())
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

/// The source element: its start tag, whether it is empty, and the text
/// of each enumeration child it has.
struct Source {
    start: BytesStart<'static>,
    children: Vec<(String, String)>,
}

fn scan(original: &[u8]) -> Result<Option<Source>, quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut found: Option<Source> = None;
    let mut depth = 0usize;
    let mut field: Option<String> = None;
    let mut text = String::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Empty(e) if found.is_none() && e.name().as_ref() == ELEMENT => {
                return Ok(Some(Source {
                    start: e.into_owned(),
                    children: Vec::new(),
                }));
            }
            Event::Start(e) if found.is_none() && e.name().as_ref() == ELEMENT => {
                found = Some(Source {
                    start: e.into_owned(),
                    children: Vec::new(),
                });
                depth = 1;
            }
            Event::Start(e) if found.is_some() => {
                depth += 1;
                if depth == 3 {
                    field = Some(String::from_utf8_lossy(e.name().as_ref()).into_owned());
                    text.clear();
                }
            }
            Event::Text(t) if field.is_some() => {
                text.push_str(
                    &t.xml_content(quick_xml::XmlVersion::Implicit1_0)
                        .unwrap_or_default(),
                );
            }
            Event::End(_) if found.is_some() => {
                if depth == 3 {
                    if let (Some(s), Some(f)) = (found.as_mut(), field.take()) {
                        s.children.push((f, text.trim().to_string()));
                    }
                }
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(found)
}

/// Patch `<FootnoteOption>` in the Preferences part to the model.
/// `may_insert` false keeps a part with no element as it is (another part
/// spells the options).
pub fn patch_footnote_option(
    original: &[u8],
    fo: &paged_model::FootnoteOptions,
    may_insert: bool,
) -> Result<Vec<u8>, quick_xml::Error> {
    let attrs = attributes(fo);
    let kids = children(fo);
    let source = scan(original)?;
    if source.is_none() && !may_insert {
        return Ok(original.to_vec());
    }
    // Which attributes and children the model departs on.
    let attr_differs = |src: Option<&BytesStart>, name: &str, kind: Kind, want: &str| {
        let have = src
            .and_then(|e| attr(e, name))
            .unwrap_or_else(|| attribute_default(name).to_string());
        !same(kind, &have, want)
    };
    let child_value = |src: Option<&Source>, name: &str| -> String {
        src.and_then(|s| {
            s.children
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                // InDesign's older attribute spelling of the enumeration.
                .or_else(|| attr(&s.start, name))
        })
        .unwrap_or_else(|| child_default(name).to_string())
    };
    let changed_attrs: Vec<&str> = attrs
        .iter()
        .filter(|(n, k, v)| attr_differs(source.as_ref().map(|s| &s.start), n, *k, v))
        .map(|(n, ..)| *n)
        .collect();
    let changed_kids: Vec<&str> = kids
        .iter()
        .filter(|(n, v)| child_value(source.as_ref(), n) != *v)
        .map(|(n, _)| *n)
        .collect();
    if changed_attrs.is_empty() && changed_kids.is_empty() {
        return Ok(original.to_vec());
    }

    let patched_start = |e: &BytesStart| -> BytesStart<'static> {
        let mut out = BytesStart::new("FootnoteOption");
        for a in e.attributes().flatten() {
            let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
            match attrs.iter().find(|(n, ..)| *n == key) {
                Some((_, _, v)) if changed_attrs.contains(&key.as_str()) => {
                    out.push_attribute((key.as_str(), v.as_str()));
                }
                _ => out.push_attribute((a.key.as_ref(), a.value.as_ref())),
            }
        }
        for (n, _, v) in &attrs {
            if changed_attrs.contains(n) && attr(e, n).is_none() {
                out.push_attribute((*n, v.as_str()));
            }
        }
        out
    };
    let write_child = |w: &mut Writer<Cursor<Vec<u8>>>,
                       name: &str,
                       value: &str|
     -> Result<(), quick_xml::Error> {
        let mut e = BytesStart::new(name);
        e.push_attribute(("type", "enumeration"));
        w.write_event(Event::Start(e))?;
        w.write_event(Event::Text(BytesText::new(value)))?;
        w.write_event(Event::End(BytesEnd::new(name)))?;
        Ok(())
    };
    let write_missing_children =
        |w: &mut Writer<Cursor<Vec<u8>>>, seen: &[String]| -> Result<(), quick_xml::Error> {
            for (n, v) in &kids {
                if changed_kids.contains(n) && !seen.iter().any(|s| s == n) {
                    write_child(w, n, v)?;
                }
            }
            Ok(())
        };

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    // Inside the source element: its depth, whether its <Properties> came,
    // the managed children seen, and a managed child being replaced.
    let mut depth = 0usize;
    let mut done = false;
    let mut props_seen = false;
    let mut seen: Vec<String> = Vec::new();
    let mut skipping: Option<String> = None;
    loop {
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Eof => break,
            Event::Empty(ref e) if !done && depth == 0 && e.name().as_ref() == ELEMENT => {
                done = true;
                if changed_kids.is_empty() {
                    writer.write_event(Event::Empty(patched_start(e)))?;
                } else {
                    writer.write_event(Event::Start(patched_start(e)))?;
                    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                    write_missing_children(&mut writer, &[])?;
                    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                    writer.write_event(Event::End(BytesEnd::new("FootnoteOption")))?;
                }
            }
            Event::Start(ref e) if !done && depth == 0 && e.name().as_ref() == ELEMENT => {
                depth = 1;
                writer.write_event(Event::Start(patched_start(e)))?;
            }
            Event::Start(ref e) if depth > 0 => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if depth == 1 && name == "Properties" {
                    props_seen = true;
                }
                if depth == 2 && kids.iter().any(|(n, _)| *n == name) {
                    seen.push(name.clone());
                    if changed_kids.contains(&name.as_str()) {
                        let value = &kids.iter().find(|(n, _)| *n == name).unwrap().1;
                        write_child(&mut writer, &name, value)?;
                        skipping = Some(name);
                        depth += 1;
                        buf.clear();
                        continue;
                    }
                }
                if skipping.is_none() {
                    writer.write_event(Event::Start(e.to_owned()))?;
                }
                depth += 1;
            }
            Event::End(ref e) if depth > 0 => {
                depth -= 1;
                if let Some(name) = skipping.as_ref() {
                    if depth == 2 && e.name().as_ref() == name.as_bytes() {
                        skipping = None;
                    }
                    buf.clear();
                    continue;
                }
                if depth == 1 && e.name().as_ref() == b"Properties" {
                    write_missing_children(&mut writer, &seen)?;
                }
                if depth == 0 {
                    // The element closes: a block it never opened goes in.
                    if !props_seen && !changed_kids.is_empty() {
                        writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                        write_missing_children(&mut writer, &[])?;
                        writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                    }
                    done = true;
                }
                writer.write_event(Event::End(e.to_owned()))?;
            }
            Event::End(ref e) if source.is_none() && !done && e.name().as_ref() == ROOT => {
                // No element anywhere: add one before the part closes.
                done = true;
                let empty = BytesStart::new("FootnoteOption");
                writer.write_event(Event::Start(patched_start(&empty)))?;
                writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                write_missing_children(&mut writer, &[])?;
                writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                writer.write_event(Event::End(BytesEnd::new("FootnoteOption")))?;
                writer.write_event(Event::End(e.to_owned()))?;
            }
            other => {
                if skipping.is_none() {
                    writer.write_event(other.into_owned())?;
                }
            }
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// InDesign 20.0.1's own spelling (a resave of core's
    /// `footnote-columns`, trimmed).
    const INDESIGN: &str = r#"<idPkg:Preferences xmlns:idPkg="x"><FootnoteOption EnableStraddling="false" StartAt="1" Prefix="" Suffix="" SeparatorText="&#x9;" SpaceBetween="0" Spacer="0" RuleOn="true" ContinuingRuleOn="true"><Properties><FootnoteNumberingStyle type="enumeration">Arabic</FootnoteNumberingStyle><RestartNumbering type="enumeration">DontRestart</RestartNumbering><ShowPrefixSuffix type="enumeration">NoPrefixSuffix</ShowPrefixSuffix><RuleColor type="object">Color/Black</RuleColor></Properties></FootnoteOption></idPkg:Preferences>"#;

    fn read(xml: &str) -> paged_model::FootnoteOptions {
        let mut dm = idml_import::DesignMap::default();
        idml_import::designmap::parse_preferences_part(xml.as_bytes(), &mut dm).expect("parse");
        dm.footnote_options
    }

    #[test]
    fn an_unchanged_option_keeps_its_bytes() {
        let fo = read(INDESIGN);
        assert_eq!(
            patch_footnote_option(INDESIGN.as_bytes(), &fo, true).unwrap(),
            INDESIGN.as_bytes()
        );
        // Defaults stated or left out read the same.
        let mut fo = fo;
        fo.rule_on = None;
        fo.enable_straddling = None;
        assert_eq!(
            patch_footnote_option(INDESIGN.as_bytes(), &fo, true).unwrap(),
            INDESIGN.as_bytes()
        );
    }

    #[test]
    fn a_changed_option_is_rewritten_in_place_and_reads_back() {
        let mut fo = read(INDESIGN);
        fo.restart_numbering = Some("PageRestart".into());
        fo.numbering_style = Some("LowerRoman".into());
        fo.prefix = Some("(".into());
        fo.enable_straddling = Some(true);
        fo.space_between = Some(6.0);
        let out = String::from_utf8(patch_footnote_option(INDESIGN.as_bytes(), &fo, true).unwrap())
            .unwrap();
        assert!(out.contains(r#"EnableStraddling="true""#), "{out}");
        assert!(out.contains(r#"Prefix="(""#), "{out}");
        assert!(out.contains(r#"SpaceBetween="6""#), "{out}");
        assert!(
            out.contains(r#"<RestartNumbering type="enumeration">PageRestart</RestartNumbering>"#),
            "{out}"
        );
        assert!(
            out.contains(
                r#"<FootnoteNumberingStyle type="enumeration">LowerRoman</FootnoteNumberingStyle>"#
            ),
            "{out}"
        );
        assert!(
            out.contains(r#"<RuleColor type="object">Color/Black</RuleColor>"#),
            "kept: {out}"
        );
        assert_eq!(out.matches("<RestartNumbering").count(), 1, "{out}");
        let back = read(&out);
        assert_eq!(back.restart_numbering.as_deref(), Some("PageRestart"));
        assert_eq!(back.numbering_style.as_deref(), Some("LowerRoman"));
        assert_eq!(back.prefix.as_deref(), Some("("));
        assert_eq!(back.enable_straddling, Some(true));
        assert_eq!(back.space_between, Some(6.0));
    }

    #[test]
    fn a_part_without_the_element_gains_one_only_when_the_model_departs() {
        let bare = r#"<idPkg:Preferences xmlns:idPkg="x"><TextPreference UseOpticalSize="false"/></idPkg:Preferences>"#;
        let default = paged_model::FootnoteOptions::default();
        assert_eq!(
            patch_footnote_option(bare.as_bytes(), &default, true).unwrap(),
            bare.as_bytes()
        );
        let fo = paged_model::FootnoteOptions {
            restart_numbering: Some("SectionRestart".into()),
            no_splitting: Some(true),
            ..Default::default()
        };
        let out =
            String::from_utf8(patch_footnote_option(bare.as_bytes(), &fo, true).unwrap()).unwrap();
        let back = read(&out);
        assert_eq!(back.restart_numbering.as_deref(), Some("SectionRestart"));
        assert_eq!(back.no_splitting, Some(true));
        assert!(
            out.ends_with("</FootnoteOption></idPkg:Preferences>"),
            "{out}"
        );
    }

    #[test]
    fn an_empty_element_opens_for_its_children() {
        let empty = r#"<idPkg:Preferences xmlns:idPkg="x"><FootnoteOption StartAt="1"/></idPkg:Preferences>"#;
        let fo = paged_model::FootnoteOptions {
            show_prefix_suffix: Some("PrefixSuffixBoth".into()),
            ..read(empty)
        };
        let out =
            String::from_utf8(patch_footnote_option(empty.as_bytes(), &fo, true).unwrap()).unwrap();
        assert!(
            out.contains(r#"<FootnoteOption StartAt="1"><Properties><ShowPrefixSuffix type="enumeration">PrefixSuffixBoth</ShowPrefixSuffix></Properties></FootnoteOption>"#),
            "{out}"
        );
    }
}
