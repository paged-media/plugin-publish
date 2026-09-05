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

//! `BasedOn` save-back: a style's parent spelled the way InDesign reads
//! it — a typed `<Properties>` child, never an attribute.
//!
//! # What InDesign 20.0.1 actually reads (measured 2026-09-05)
//!
//! The engine's older spelling put the parent on the start tag:
//! `<ParagraphStyle … BasedOn="ParagraphStyle/Annual Body">`. InDesign
//! ignores that attribute outright — every one of the annual's 35
//! paragraph styles reported `basedOn = [No paragraph style]`, "Body
//! First" (based on the 9.5 pt Source Serif body) composed at 12 pt in
//! the root's face, 4,449 characters fell to the application default,
//! and the book's line breaks, overset count and worst visual pages all
//! followed from it. The same styles with
//! `<Properties><BasedOn type="object">ParagraphStyle/Annual Body</BasedOn>`
//! resolved every cascade (140 → 133 overset stories, nothing else
//! changed). InDesign's own files spell it that way (`type="object"`
//! carrying the parent's `Self`; `type="string"` with the `$ID/…` name
//! for its reserved roots — both are read).
//!
//! This pass rewrites the attribute into the child on every style kind
//! that cascades (`ParagraphStyle`, `CharacterStyle`, `ObjectStyle`,
//! `CellStyle`, `TableStyle`), as the FIRST child of the element's
//! `<Properties>` block — creating the block, and opening a self-closing
//! element, when there is none. A part that already spells every parent
//! as a child passes through byte-identical.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use crate::resources::write_based_on_child;

const BASED_ON: &[u8] = b"BasedOn";
const PROPERTIES: &[u8] = b"Properties";

fn cascades(name: &[u8]) -> bool {
    matches!(
        name,
        b"ParagraphStyle" | b"CharacterStyle" | b"ObjectStyle" | b"CellStyle" | b"TableStyle"
    )
}

/// The start tag without its `BasedOn` attribute, plus the attribute's
/// decoded value when it carried one.
fn without_based_on(e: &BytesStart) -> (BytesStart<'static>, Option<String>) {
    let mut parent = None;
    let mut out = BytesStart::new(String::from_utf8_lossy(e.name().as_ref()).into_owned());
    for a in e.attributes().flatten() {
        if a.key.as_ref() == BASED_ON {
            parent = std::str::from_utf8(&a.value)
                .ok()
                .map(|v| v.to_string())
                .filter(|v| !v.trim().is_empty());
        } else {
            out.push_attribute((a.key.as_ref(), a.value.as_ref()));
        }
    }
    (out, parent)
}

/// Rewrite every `BasedOn="…"` attribute on a cascading style element
/// into `<Properties><BasedOn type="object">…</BasedOn>` (first child).
/// Byte-identical when the part carries no such attribute.
pub fn patch_based_on(original: &[u8]) -> Result<Vec<u8>, quick_xml::Error> {
    let has_attr = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut found = false;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(e) | Event::Empty(e)
                    if cascades(e.name().as_ref())
                        && e.attributes().flatten().any(|a| a.key.as_ref() == BASED_ON) =>
                {
                    found = true;
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        found
    };
    if !has_attr {
        return Ok(original.to_vec());
    }

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    // A parent lifted off the start tag just written, waiting for the
    // element's `<Properties>` (or for the first sign there is none).
    let mut pending: Option<String> = None;
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        // Anything but the element's own `<Properties>` start means the
        // element has no block to join: open one before the event.
        if let Some(parent) = pending.take() {
            let joins_block = matches!(&ev, Event::Start(e) if e.name().as_ref() == PROPERTIES);
            if joins_block {
                if let Event::Start(e) = &ev {
                    writer.write_event(Event::Start(e.to_owned()))?;
                }
                write_based_on_child(&mut writer, &parent)?;
                buf.clear();
                continue;
            }
            writer.write_event(Event::Start(BytesStart::new("Properties")))?;
            write_based_on_child(&mut writer, &parent)?;
            writer.write_event(Event::End(BytesEnd::new("Properties")))?;
        }
        match ev {
            Event::Eof => break,
            Event::Start(ref e) if cascades(e.name().as_ref()) => {
                let (start, parent) = without_based_on(e);
                writer.write_event(Event::Start(start))?;
                pending = parent;
            }
            Event::Empty(ref e) if cascades(e.name().as_ref()) => {
                let (start, parent) = without_based_on(e);
                match parent {
                    Some(parent) => {
                        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        writer.write_event(Event::Start(start))?;
                        writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                        write_based_on_child(&mut writer, &parent)?;
                        writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                        writer.write_event(Event::End(BytesEnd::new(name)))?;
                    }
                    None => writer.write_event(ev.borrow())?,
                }
            }
            other => writer.write_event(other.borrow())?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attribute_parent_becomes_the_first_properties_child() {
        let src = r#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/Body First" Name="Body First" BasedOn="ParagraphStyle/Annual Body" FirstLineIndent="0" NextStyle="ParagraphStyle/Annual Body"><Properties><Leading type="unit">13</Leading></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Body Small" Name="Body Small" BasedOn="ParagraphStyle/Annual Body" PointSize="8"/><ParagraphStyle Self="ParagraphStyle/Bare" Name="Bare" BasedOn="ParagraphStyle/Annual Body" PointSize="8"><Properties/></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Nested" Name="Nested" BasedOn="ParagraphStyle/Annual Body"><NestedStyle Delimiter=":" Repetition="1"/></ParagraphStyle></RootParagraphStyleGroup><RootObjectStyleGroup Self="u9f"><ObjectStyle Self="ObjectStyle/Sidebar" Name="Sidebar" BasedOn="ObjectStyle/Callout" FillColor="Color/Black"/></RootObjectStyleGroup></idPkg:Styles>"#;
        let out = String::from_utf8(patch_based_on(src.as_bytes()).unwrap()).unwrap();
        assert!(out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Body First" Name="Body First" FirstLineIndent="0" NextStyle="ParagraphStyle/Annual Body"><Properties><BasedOn type="object">ParagraphStyle/Annual Body</BasedOn><Leading type="unit">13</Leading></Properties></ParagraphStyle>"#), "{out}");
        assert!(out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Body Small" Name="Body Small" PointSize="8"><Properties><BasedOn type="object">ParagraphStyle/Annual Body</BasedOn></Properties></ParagraphStyle>"#), "{out}");
        // A self-closing `<Properties/>` is not a block to join: it is
        // replaced by one that holds the parent.
        assert!(out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Bare" Name="Bare" PointSize="8"><Properties><BasedOn type="object">ParagraphStyle/Annual Body</BasedOn></Properties><Properties/></ParagraphStyle>"#), "{out}");
        assert!(out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Nested" Name="Nested"><Properties><BasedOn type="object">ParagraphStyle/Annual Body</BasedOn></Properties><NestedStyle Delimiter=":" Repetition="1"/></ParagraphStyle>"#), "{out}");
        assert!(out.contains(r#"<ObjectStyle Self="ObjectStyle/Sidebar" Name="Sidebar" FillColor="Color/Black"><Properties><BasedOn type="object">ObjectStyle/Callout</BasedOn></Properties></ObjectStyle>"#), "{out}");
        assert!(!out.contains(r#"BasedOn=""#), "{out}");
    }

    #[test]
    fn a_part_spelling_every_parent_as_a_child_is_byte_identical() {
        let src = r#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/Body First" Name="Body First"><Properties><BasedOn type="object">ParagraphStyle/Annual Body</BasedOn></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/$ID/NormalParagraphStyle" Name="$ID/NormalParagraphStyle"><Properties><BasedOn type="string">$ID/[No paragraph style]</BasedOn></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Plain" Name="Plain"/></RootParagraphStyleGroup></idPkg:Styles>"#;
        assert_eq!(patch_based_on(src.as_bytes()).unwrap(), src.as_bytes());
    }
}
