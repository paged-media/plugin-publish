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

//! Master spreads made, renamed and removed by the model (`CreateMaster`,
//! `RenameMaster`, `DeleteMaster`).
//!
//! A minted master is written whole by [`crate::emit::master_spread_part`];
//! this module renames a source master in place and keeps the designmap's
//! `<idPkg:MasterSpread>` references in step with the parts written.

use std::io::Cursor;

use quick_xml::events::Event;
use quick_xml::{Reader, Writer};

use crate::rewrite::{self, attr_value, Patch};

/// Set a source master's `Name` (and `BaseName`, which InDesign shows)
/// to the model's. Byte-identical when the name is unchanged; a cleared
/// name leaves the source's spelling alone, since InDesign requires one.
pub(crate) fn rewrite_master_name(
    original: &[u8],
    name: Option<&str>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let Some(name) = name else {
        return Ok(original.to_vec());
    };
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut changed = false;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) if e.name().as_ref() == b"MasterSpread" => {
                if attr_value(&e, b"Name").as_deref() == Some(name) {
                    return Ok(original.to_vec());
                }
                changed = true;
                let patched = rewrite::patch_start(
                    &e,
                    |k, _| match k {
                        b"Name" | b"BaseName" => Some(Patch::Set(name.to_string())),
                        _ => None,
                    },
                    &[("Name", name.to_string())],
                )?;
                writer.write_event(Event::Start(patched))?;
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    if !changed {
        return Ok(original.to_vec());
    }
    Ok(writer.into_inner().into_inner())
}

/// Reference the minted masters' parts and drop the removed masters'
/// references. New references follow the last existing master reference
/// (before the first spread reference when there is none); nothing else
/// moves.
pub(crate) fn patch_designmap_masters(
    original: &[u8],
    minted: &[String],
    dropped: &[String],
) -> Result<Vec<u8>, quick_xml::Error> {
    if minted.is_empty() && dropped.is_empty() {
        return Ok(original.to_vec());
    }
    let total = {
        let mut reader = Reader::from_reader(original);
        let mut buf = Vec::new();
        let mut n = 0usize;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"idPkg:MasterSpread" => {
                    n += 1
                }
                _ => {}
            }
            buf.clear();
        }
        n
    };
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut seen = 0usize;
    let mut placed = false;
    let place = |w: &mut Writer<Cursor<Vec<u8>>>| -> Result<(), quick_xml::Error> {
        for src in minted {
            rewrite::emit_empty_with_attrs(w, "idPkg:MasterSpread", &[("src", src.clone())])?;
        }
        Ok(())
    };
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Empty(e) if e.name().as_ref() == b"idPkg:MasterSpread" => {
                seen += 1;
                let drop = attr_value(&e, b"src").is_some_and(|s| dropped.contains(&s));
                if !drop {
                    writer.write_event(Event::Empty(e.into_owned()))?;
                }
                if seen == total && !placed {
                    place(&mut writer)?;
                    placed = true;
                }
            }
            Event::Empty(e) if e.name().as_ref() == b"idPkg:Spread" && !placed => {
                place(&mut writer)?;
                placed = true;
                writer.write_event(Event::Empty(e.into_owned()))?;
            }
            Event::End(e) if e.name().as_ref() == b"Document" && !placed => {
                place(&mut writer)?;
                placed = true;
                writer.write_event(Event::End(e.into_owned()))?;
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_patches_name_and_base_name_only() {
        let xml = br#"<idPkg:MasterSpread><MasterSpread Self="uA" Name="A-Master" NamePrefix="A" BaseName="Master" PageCount="1"><Page Self="p"/></MasterSpread></idPkg:MasterSpread>"#;
        let out =
            String::from_utf8(rewrite_master_name(xml, Some("Title Slide")).unwrap()).unwrap();
        assert!(
            out.contains(r#"Name="Title Slide" NamePrefix="A" BaseName="Title Slide""#),
            "{out}"
        );
        assert_eq!(
            rewrite_master_name(xml, Some("A-Master")).unwrap(),
            xml.to_vec()
        );
        assert_eq!(rewrite_master_name(xml, None).unwrap(), xml.to_vec());
    }

    #[test]
    fn master_references_are_added_after_the_last_and_dropped() {
        let xml = br#"<Document><idPkg:MasterSpread src="MasterSpreads/MasterSpread_uA.xml"/><idPkg:MasterSpread src="MasterSpreads/MasterSpread_uB.xml"/><idPkg:Spread src="Spreads/Spread_s.xml"/></Document>"#;
        let out = patch_designmap_masters(
            xml,
            &["MasterSpreads/MasterSpread_uC.xml".into()],
            &["MasterSpreads/MasterSpread_uA.xml".into()],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            r#"<Document><idPkg:MasterSpread src="MasterSpreads/MasterSpread_uB.xml"/><idPkg:MasterSpread src="MasterSpreads/MasterSpread_uC.xml"/><idPkg:Spread src="Spreads/Spread_s.xml"/></Document>"#
        );
        assert_eq!(
            patch_designmap_masters(xml, &[], &[]).unwrap(),
            xml.to_vec()
        );
    }
}
