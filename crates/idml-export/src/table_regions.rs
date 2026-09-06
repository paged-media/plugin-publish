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

//! A table style's REGION cell styles need the flag that turns the
//! region on.
//!
//! # What InDesign 20.0.1 actually reads (measured 2026-09-06)
//!
//! Naming a region's cell style is not enough. Asked to build a table
//! style with a header region cell style and export its own IDML,
//! InDesign wrote BOTH halves:
//!
//! ```text
//! <TableStyle … HeaderRegionSameAsBodyRegion="false"
//!               HeaderRegionCellStyle="CellStyle/TH" …>
//! ```
//!
//! Without the flag it reads the header as "same as the body" and the
//! region style is never applied — the annual's preflight table came
//! back with a WHITE header where the style asks for a warm fill, and
//! `cell.fillColor` reported none. Adding the one attribute to the same
//! file made InDesign resolve the header cell's fill to "Paper Warm"
//! and the alternating body rows to the same colour at tint 20.
//!
//! Only the header form is measured; the other three regions carry
//! InDesign's own property names, and an attribute InDesign does not
//! know is ignored rather than harmful.

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use std::io::Cursor;

const TABLE_STYLE: &[u8] = b"TableStyle";

/// The four `(region cell style, its enabling flag)` attribute pairs.
const REGIONS: [(&[u8], &str); 4] = [
    (b"HeaderRegionCellStyle", "HeaderRegionSameAsBodyRegion"),
    (b"FooterRegionCellStyle", "FooterRegionSameAsBodyRegion"),
    (
        b"LeftColumnRegionCellStyle",
        "LeftColumnRegionSameAsBodyRegion",
    ),
    (
        b"RightColumnRegionCellStyle",
        "RightColumnRegionSameAsBodyRegion",
    ),
];

fn has_attr(e: &BytesStart, key: &[u8]) -> bool {
    e.attributes()
        .flatten()
        .any(|a| a.key.as_ref().eq_ignore_ascii_case(key))
}

/// Add the missing `…SameAsBodyRegion="false"` to every `<TableStyle>`
/// that names a region cell style without it.
pub fn spell_region_flags(original: &[u8]) -> Result<Vec<u8>, quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match ev {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) if e.name().as_ref() == TABLE_STYLE => {
                let missing: Vec<&str> = REGIONS
                    .iter()
                    .filter(|(style, flag)| has_attr(e, style) && !has_attr(e, flag.as_bytes()))
                    .map(|(_, flag)| *flag)
                    .collect();
                if missing.is_empty() {
                    writer.write_event(ev.borrow())?;
                } else {
                    let name = e.name();
                    let name = std::str::from_utf8(name.as_ref()).unwrap_or("TableStyle");
                    let mut out = BytesStart::new(name);
                    out.extend_attributes(e.attributes().flatten());
                    for flag in missing {
                        out.push_attribute((flag, "false"));
                    }
                    match ev {
                        Event::Start(_) => writer.write_event(Event::Start(out))?,
                        _ => writer.write_event(Event::Empty(out))?,
                    }
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
    fn a_named_region_style_gets_the_flag_that_turns_it_on() {
        let src = br#"<idPkg:Styles xmlns:idPkg="x"><RootTableStyleGroup><TableStyle Self="TableStyle/A" Name="A" HeaderRegionCellStyle="CellStyle/TH" BodyRegionCellStyle="CellStyle/TD"/></RootTableStyleGroup></idPkg:Styles>"#;
        let out = spell_region_flags(src).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(r#"HeaderRegionSameAsBodyRegion="false""#), "{s}");
        // The body region is not a region in InDesign's sense and has
        // no flag of its own.
        assert!(!s.contains("BodyRegionSameAsBodyRegion"), "{s}");
    }

    #[test]
    fn an_existing_flag_is_left_alone() {
        let src = br#"<idPkg:Styles xmlns:idPkg="x"><TableStyle Self="TableStyle/A" HeaderRegionSameAsBodyRegion="true" HeaderRegionCellStyle="CellStyle/TH"/></idPkg:Styles>"#;
        let out = spell_region_flags(src).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(r#"HeaderRegionSameAsBodyRegion="true""#), "{s}");
        assert_eq!(s.matches("HeaderRegionSameAsBodyRegion").count(), 1, "{s}");
    }

    #[test]
    fn a_style_with_no_region_is_untouched() {
        let src = br#"<idPkg:Styles xmlns:idPkg="x"><TableStyle Self="TableStyle/A" Name="A"/></idPkg:Styles>"#;
        let out = spell_region_flags(src).unwrap();
        assert_eq!(out, src.to_vec());
    }
}
