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

//! The document's `<TextVariable>` definitions in `designmap.xml`.
//!
//! A variable authored in paged (`createTextVariable`) lived in the model
//! only: the designmap carried the definitions the source package already
//! had, so an instance of a new variable pointed at nothing and InDesign
//! dropped it. This pass brings the definitions in line with
//! `doc.designmap.text_variables`:
//!
//! * a definition the source lacks is written in InDesign's own spelling
//!   (a typed `…VariablePreference` child; a custom variable's value as
//!   `<Properties><Contents type="string">`, measured on InDesign 20.0.1
//!   packages), after the source's last `<TextVariable>` — or, with none,
//!   where InDesign puts them: after the preferences, before the tags,
//!   layers and spread references;
//! * a definition the model changed is rewritten whole;
//! * a definition the model deleted is dropped.
//!
//! An unchanged designmap passes through byte-identical.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use idml_import::TextVariable;

use crate::rewrite::{attr_value, emit_empty_with_attrs, emit_start_with_attrs};

type W = Writer<Cursor<Vec<u8>>>;

/// What the model says about a variable, for "did it change" — every
/// modelled field (the struct has no `PartialEq` of its own).
fn fingerprint(v: &TextVariable) -> String {
    format!("{v:?}")
}

/// The `Document` children that follow the text variables in InDesign's
/// own designmap order; a first definition goes before the first of them.
const AFTER_VARIABLES: &[&[u8]] = &[
    b"idPkg:Tags",
    b"Layer",
    b"idPkg:MasterSpread",
    b"idPkg:Spread",
    b"Section",
    b"idPkg:BackingStory",
    b"idPkg:Story",
];

/// Bring the designmap's `<TextVariable>` definitions in line with `vars`.
pub(crate) fn patch_text_variables(
    original: &[u8],
    vars: &[TextVariable],
) -> Result<Vec<u8>, quick_xml::Error> {
    let source = match idml_import::parse_designmap(original) {
        Ok(d) => d.text_variables,
        // Unreadable: leave it as it is.
        Err(_) => return Ok(original.to_vec()),
    };
    let src_by_id: HashMap<&str, &TextVariable> =
        source.iter().map(|v| (v.self_id.as_str(), v)).collect();
    let model_by_id: HashMap<&str, &TextVariable> =
        vars.iter().map(|v| (v.self_id.as_str(), v)).collect();
    let new: Vec<&TextVariable> = vars
        .iter()
        .filter(|v| !src_by_id.contains_key(v.self_id.as_str()))
        .collect();
    let dropped: HashSet<&str> = source
        .iter()
        .map(|v| v.self_id.as_str())
        .filter(|id| !model_by_id.contains_key(id))
        .collect();
    let changed: HashMap<&str, &TextVariable> = vars
        .iter()
        .filter(|v| {
            src_by_id
                .get(v.self_id.as_str())
                .is_some_and(|s| fingerprint(s) != fingerprint(v))
        })
        .map(|v| (v.self_id.as_str(), v))
        .collect();
    if new.is_empty() && dropped.is_empty() && changed.is_empty() {
        return Ok(original.to_vec());
    }

    let insert_at = insertion_point(original)?;
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut out = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut depth = 0usize;
    // Depth at which a skipped (dropped / rewritten) definition closes.
    let mut skip_until: Option<usize> = None;
    let mut inserted = new.is_empty();
    loop {
        let pos = reader.buffer_position();
        if !inserted && pos >= insert_at.0 {
            for v in &new {
                if insert_at.1 == Side::After {
                    newline(&mut out)?;
                }
                write_variable(&mut out, v)?;
                if insert_at.1 == Side::Before {
                    newline(&mut out)?;
                }
            }
            inserted = true;
        }
        let ev = reader.read_event_into(&mut buf)?;
        if matches!(ev, Event::Eof) {
            break;
        }
        if let Some(close) = skip_until {
            match &ev {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == close {
                        skip_until = None;
                    }
                }
                _ => {}
            }
            buf.clear();
            continue;
        }
        match ev {
            Event::Start(e) => {
                if depth == 1 && e.name().as_ref() == b"TextVariable" {
                    if let Some(id) = attr_value(&e, b"Self") {
                        if dropped.contains(id.as_str()) {
                            skip_until = Some(depth);
                            depth += 1;
                            buf.clear();
                            continue;
                        }
                        if let Some(v) = changed.get(id.as_str()) {
                            write_variable(&mut out, v)?;
                            skip_until = Some(depth);
                            depth += 1;
                            buf.clear();
                            continue;
                        }
                    }
                }
                depth += 1;
                out.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Empty(e) => {
                if depth == 1 && e.name().as_ref() == b"TextVariable" {
                    if let Some(id) = attr_value(&e, b"Self") {
                        if dropped.contains(id.as_str()) {
                            buf.clear();
                            continue;
                        }
                        if let Some(v) = changed.get(id.as_str()) {
                            write_variable(&mut out, v)?;
                            buf.clear();
                            continue;
                        }
                    }
                }
                out.write_event(Event::Empty(e.into_owned()))?;
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                out.write_event(Event::End(e.into_owned()))?;
            }
            other => out.write_event(other.into_owned())?,
        }
        buf.clear();
    }
    Ok(out.into_inner().into_inner())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// Right after the source's last definition.
    After,
    /// Right before the element that follows the definitions.
    Before,
}

fn newline(out: &mut W) -> Result<(), quick_xml::Error> {
    out.write_event(Event::Text(BytesText::from_escaped("\n\t")))?;
    Ok(())
}

/// The byte offset new definitions go at: after the last top-level
/// `<TextVariable>`, else before the first [`AFTER_VARIABLES`] element,
/// else before `</Document>`.
fn insertion_point(original: &[u8]) -> Result<(u64, Side), quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut after_last: Option<u64> = None;
    let mut before_first: Option<u64> = None;
    let mut doc_end: Option<u64> = None;
    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                if depth == 1
                    && before_first.is_none()
                    && AFTER_VARIABLES.contains(&e.name().as_ref())
                {
                    before_first = Some(pos);
                }
                depth += 1;
            }
            Event::Empty(e) => {
                if depth == 1 {
                    if e.name().as_ref() == b"TextVariable" {
                        after_last = Some(reader.buffer_position());
                    } else if before_first.is_none() && AFTER_VARIABLES.contains(&e.name().as_ref())
                    {
                        before_first = Some(pos);
                    }
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                if depth == 1 && e.name().as_ref() == b"TextVariable" {
                    after_last = Some(reader.buffer_position());
                }
                if depth == 0 && e.name().as_ref() == b"Document" {
                    doc_end = Some(pos);
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(match (after_last, before_first) {
        (Some(p), _) => (p, Side::After),
        (None, Some(p)) => (p, Side::Before),
        (None, None) => (doc_end.unwrap_or(original.len() as u64), Side::Before),
    })
}

/// The typed preference element InDesign writes for a variable type.
fn preference_element(variable_type: &str) -> Option<&'static str> {
    Some(match variable_type {
        "CustomTextType" => "CustomTextVariablePreference",
        "MatchParagraphStyleType" => "MatchParagraphStylePreference",
        "MatchCharacterStyleType" => "MatchCharacterStylePreference",
        "LastPageNumberType" => "PageNumberVariablePreference",
        "ChapterNumberType" => "ChapterNumberVariablePreference",
        "FileNameType" => "FileNameVariablePreference",
        "CreationDateType" | "ModificationDateType" | "OutputDateType" => "DateVariablePreference",
        _ => return None,
    })
}

/// One `<TextVariable>` definition in InDesign's spelling.
pub(crate) fn write_variable(out: &mut W, v: &TextVariable) -> Result<(), quick_xml::Error> {
    let vtype = v.variable_type.as_deref().unwrap_or("CustomTextType");
    let name = v.name.clone().unwrap_or_else(|| {
        v.self_id
            .strip_prefix("dTextVariablen")
            .unwrap_or(&v.self_id)
            .to_string()
    });
    let head = [
        ("Self", v.self_id.clone()),
        ("Name", name),
        ("VariableType", vtype.to_string()),
    ];
    let Some(pref) = preference_element(vtype) else {
        return emit_empty_with_attrs(out, "TextVariable", &head);
    };
    emit_start_with_attrs(out, "TextVariable", &head)?;
    let mut attrs: Vec<(&str, String)> = Vec::new();
    let deco = |attrs: &mut Vec<(&str, String)>| {
        attrs.push(("TextBefore", v.text_before.clone().unwrap_or_default()));
    };
    deco(&mut attrs);
    match pref {
        "MatchParagraphStylePreference" | "MatchCharacterStylePreference" => {
            let (key, style) = if pref == "MatchParagraphStylePreference" {
                ("AppliedParagraphStyle", &v.running_header_style)
            } else {
                ("AppliedCharacterStyle", &v.running_header_character_style)
            };
            if let Some(s) = style {
                attrs.push((key, s.clone()));
            }
            attrs.push((
                "SearchStrategy",
                v.running_header_use
                    .clone()
                    .unwrap_or_else(|| "FirstOnPage".to_string()),
            ));
            attrs.push((
                "ChangeCase",
                v.change_case.clone().unwrap_or_else(|| "None".to_string()),
            ));
            attrs.push(("DeleteEndPunctuation", v.delete_end_punctuation.to_string()));
        }
        "PageNumberVariablePreference" => {
            attrs.push((
                "Format",
                v.number_format
                    .clone()
                    .unwrap_or_else(|| "Current".to_string()),
            ));
            if let Some(s) = &v.page_number_scope {
                attrs.push(("Scope", s.clone()));
            }
        }
        "ChapterNumberVariablePreference" => {
            attrs.push((
                "Format",
                v.number_format
                    .clone()
                    .unwrap_or_else(|| "Current".to_string()),
            ));
        }
        "FileNameVariablePreference" => {
            attrs.push(("IncludePath", v.include_path.to_string()));
            attrs.push(("IncludeExtension", v.include_extension.to_string()));
        }
        "DateVariablePreference" => {
            if let Some(f) = &v.date_format {
                attrs.push(("Format", f.clone()));
            }
        }
        _ => {}
    }
    attrs.push(("TextAfter", v.text_after.clone().unwrap_or_default()));
    if pref == "CustomTextVariablePreference" {
        emit_start_with_attrs(out, pref, &attrs)?;
        out.write_event(Event::Start(BytesStart::new("Properties")))?;
        let mut c = BytesStart::new("Contents");
        c.push_attribute(("type", "string"));
        out.write_event(Event::Start(c))?;
        out.write_event(Event::Text(BytesText::new(
            v.contents.as_deref().unwrap_or(""),
        )))?;
        out.write_event(Event::End(BytesEnd::new("Contents")))?;
        out.write_event(Event::End(BytesEnd::new("Properties")))?;
        out.write_event(Event::End(BytesEnd::new(pref)))?;
    } else {
        emit_empty_with_attrs(out, pref, &attrs)?;
    }
    out.write_event(Event::End(BytesEnd::new("TextVariable")))?;
    Ok(())
}
