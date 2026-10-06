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

//! The document's own labels in `designmap.xml`.
//!
//! A label on the DOCUMENT (`DesignMap::labels` — plugin state that
//! belongs to no page item) lives where an item's does, on the element
//! itself: `<Document><Properties><Label><KeyValuePair Key Value/>`.
//! One streaming pass: a designmap whose labels already agree with the
//! model passes through byte-identical; otherwise the document's
//! `<Label>` is replaced wholesale (other `<Properties>` children are
//! kept), synthesised as the `<Document>`'s first child when there was
//! none, and dropped — with a `<Properties>` it leaves empty — when the
//! model has no labels.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use crate::rewrite::emit_empty_with_attrs;

type W = Writer<Cursor<Vec<u8>>>;

fn write_label(w: &mut W, labels: &[(String, String)]) -> Result<(), quick_xml::Error> {
    if labels.is_empty() {
        return Ok(());
    }
    w.write_event(Event::Start(BytesStart::new("Label")))?;
    for (k, v) in labels {
        // Through `emit_empty_with_attrs` (its attribute escaping keeps
        // a newline in a JSON value as a character reference).
        emit_empty_with_attrs(
            w,
            "KeyValuePair",
            &[("Key", k.clone()), ("Value", v.clone())],
        )?;
    }
    w.write_event(Event::End(BytesEnd::new("Label")))?;
    Ok(())
}

fn write_properties(w: &mut W, labels: &[(String, String)]) -> Result<(), quick_xml::Error> {
    if labels.is_empty() {
        return Ok(());
    }
    w.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_label(w, labels)?;
    w.write_event(Event::End(BytesEnd::new("Properties")))?;
    Ok(())
}

/// Bring the `<Document>`'s own `<Label>` in line with `labels`.
pub(crate) fn patch_document_labels(
    original: &[u8],
    labels: &[(String, String)],
) -> Result<Vec<u8>, quick_xml::Error> {
    if idml_import::parse_designmap(original).is_ok_and(|d| d.labels.as_slice() == labels) {
        return Ok(original.to_vec());
    }
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut out = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    // Depth of open elements (0 = outside `<Document>`).
    let mut depth = 0usize;
    // Set once the document's labels have been written (or placed).
    let mut done = false;
    // While inside the Document's direct `<Properties>`: its events go
    // to this side writer, so the wrapper can be dropped if nothing but
    // the old `<Label>` was in it.
    let mut props: Option<(W, BytesStart<'static>, bool)> = None;
    // Depth at which a skipped `<Label>` subtree closes.
    let mut skip_until: Option<usize> = None;

    loop {
        let ev = reader.read_event_into(&mut buf)?;
        if matches!(ev, Event::Eof) {
            break;
        }
        // The old Label's subtree: swallowed.
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
                depth += 1;
                let name = e.name().as_ref().to_vec();
                if depth == 1 && name == b"Document" {
                    out.write_event(Event::Start(e.into_owned()))?;
                    // Synthesise a block unless the Document carries a
                    // `<Properties>` of its own (found below).
                    if !document_has_properties(original) {
                        write_properties(&mut out, labels)?;
                        done = true;
                    }
                } else if depth == 2 && name == b"Properties" && !done {
                    props = Some((Writer::new(Cursor::new(Vec::new())), e.into_owned(), false));
                } else if depth == 3 && props.is_some() && name == b"Label" {
                    skip_until = Some(2);
                } else if let Some((w, _, kept)) = props.as_mut() {
                    *kept = true;
                    w.write_event(Event::Start(e.into_owned()))?;
                } else {
                    out.write_event(Event::Start(e.into_owned()))?;
                }
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                if depth == 1 && name == b"Properties" && !done {
                    // `<Properties/>` — nothing in it to keep.
                    write_properties(&mut out, labels)?;
                    done = true;
                } else if depth == 2 && props.is_some() && name == b"Label" {
                    // An empty `<Label/>`: replaced below.
                } else if let Some((w, _, kept)) = props.as_mut() {
                    *kept = true;
                    w.write_event(Event::Empty(e.into_owned()))?;
                } else {
                    out.write_event(Event::Empty(e.into_owned()))?;
                }
            }
            Event::End(e) => {
                depth -= 1;
                if depth == 1 && props.is_some() && e.name().as_ref() == b"Properties" {
                    let (w, start, kept) = props.take().expect("checked");
                    if kept || !labels.is_empty() {
                        out.write_event(Event::Start(start))?;
                        out.get_mut().get_mut().extend(w.into_inner().into_inner());
                        write_label(&mut out, labels)?;
                        out.write_event(Event::End(e.into_owned()))?;
                    }
                    done = true;
                } else if let Some((w, _, _)) = props.as_mut() {
                    w.write_event(Event::End(e.into_owned()))?;
                } else {
                    out.write_event(Event::End(e.into_owned()))?;
                }
            }
            other => {
                if let Some((w, _, _)) = props.as_mut() {
                    w.write_event(other.into_owned())?;
                } else {
                    out.write_event(other.into_owned())?;
                }
            }
        }
        buf.clear();
    }
    Ok(out.into_inner().into_inner())
}

/// Whether the `<Document>` has a `<Properties>` child of its own.
fn document_has_properties(original: &[u8]) -> bool {
    let mut reader = Reader::from_reader(original);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                if depth == 2 && e.name().as_ref() == b"Properties" {
                    return true;
                }
            }
            Ok(Event::Empty(e)) => {
                if depth == 1 && e.name().as_ref() == b"Properties" {
                    return true;
                }
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Eof) | Err(_) => return false,
            _ => {}
        }
        buf.clear();
    }
}
