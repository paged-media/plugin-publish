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

//! Minimal in-memory IDML packages for the navigation / guide / table
//! round-trip tests: a one-spread, one-story document whose parts the
//! test supplies (or takes from the defaults here).

#![allow(dead_code)]

use std::io::{Cursor, Read, Write};

pub const PKG: &str = r#"xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging""#;

pub fn designmap(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Document DOMVersion="20.0" Self="d1" {PKG}>
<Layer Self="ub6" Name="Layer 1" Visible="true" Locked="false" Printable="true"/>
<idPkg:Spread src="Spreads/Spread_s1.xml"/>
{body}<idPkg:Story src="Stories/Story_st1.xml"/>
</Document>"#
    )
}

pub const SPREAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
</Spread></idPkg:Spread>"#;

pub fn story(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Story Self="st1">{body}</Story></idPkg:Story>"#
    )
}

pub const STORY_BODY: &str = r#"<ParagraphStyleRange><CharacterStyleRange><Content>Hello world</Content></CharacterStyleRange></ParagraphStyleRange>"#;

pub const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]"/></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]"/></RootParagraphStyleGroup>
</idPkg:Styles>"#;

/// Build a package from `(entry name, body)` parts; `mimetype` is added
/// first + stored, everything else deflated (the shape a real `.idml`
/// ships in).
pub fn package(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/vnd.adobe.indesign-idml-package")
            .unwrap();
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in parts {
            zip.start_file(*name, deflated).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

/// The default one-spread / one-story package with the given designmap
/// body (between the spread and story includes), story body and styles.
pub fn simple(designmap_body: &str, story_body: &str, styles: &str) -> Vec<u8> {
    package(&[
        ("designmap.xml", &designmap(designmap_body)),
        ("Resources/Styles.xml", styles),
        ("Spreads/Spread_s1.xml", SPREAD),
        ("Stories/Story_st1.xml", &story(story_body)),
    ])
}

pub fn entry(package: &[u8], name: &str) -> Option<String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(package)).expect("zip");
    let mut e = zip.by_name(name).ok()?;
    let mut out = String::new();
    e.read_to_string(&mut out).unwrap();
    Some(out)
}

pub fn names(package: &[u8]) -> Vec<String> {
    let zip = zip::ZipArchive::new(Cursor::new(package)).expect("zip");
    zip.file_names().map(|n| n.to_string()).collect()
}

pub fn open(package: &[u8]) -> paged_scene::Document {
    idml_import::import_idml_doc(package).expect("package parses")
}

/// Byte offset of the first occurrence of `needle` in `hay` (panics when
/// absent, naming it).
pub fn pos(hay: &str, needle: &str) -> usize {
    hay.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not found in:\n{hay}"))
}

/// Assert two packages are byte-identical, naming the first differing
/// entry and showing the bytes around the divergence (a raw `assert_eq!`
/// on two ZIPs prints numbers nobody can read).
pub fn assert_same_package(expected: &[u8], actual: &[u8]) {
    if expected == actual {
        return;
    }
    for name in names(expected) {
        let a = entry(expected, &name);
        let b = entry(actual, &name);
        if a != b {
            let a = a.unwrap_or_default();
            let b = b.unwrap_or_default();
            let common = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
            let lo = common.saturating_sub(200);
            panic!(
                "{name} differs at byte {common}:\n--- expected\n{}\n--- actual\n{}",
                &a[lo..(common + 300).min(a.len())],
                &b[lo..(common + 300).min(b.len())]
            );
        }
    }
    let extra: Vec<String> = names(actual)
        .into_iter()
        .filter(|n| entry(expected, n).is_none())
        .collect();
    panic!("packages differ; entries only in actual: {extra:?}");
}

/// Assert two packages carry the same entries with byte-identical
/// bodies, ignoring the ZIP framing around them.
pub fn assert_same_entries(expected: &[u8], actual: &[u8]) {
    let mut a = names(expected);
    let mut b = names(actual);
    a.sort();
    b.sort();
    assert_eq!(a, b, "entry sets differ");
    for name in a {
        let x = entry(expected, &name).unwrap_or_default();
        let y = entry(actual, &name).unwrap_or_default();
        if x != y {
            let common = x.bytes().zip(y.bytes()).take_while(|(p, q)| p == q).count();
            let lo = common.saturating_sub(200);
            panic!(
                "{name} differs at byte {common}:\n--- expected\n{}\n--- actual\n{}",
                &x[lo..(common + 300).min(x.len())],
                &y[lo..(common + 300).min(y.len())]
            );
        }
    }
}
