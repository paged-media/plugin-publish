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

//! `write_idml` must produce a PURE IDML package.
//!
//! The carry-through writer copied EVERY source entry verbatim — including
//! the `.paged` container's non-IDML parts (`manifest.json`,
//! `paged/core/model/document.pgm`, `paged/<plugin>/…`). An `.idml`
//! exported from a loaded `.paged` was therefore the container under
//! another name: the engine's load sniff then preferred the carried
//! `document.pgm` and never parsed the IDML parts, so an IDML-parity gate
//! that re-opened the export compared the model with itself and reported
//! zero differing pages. `write_paged` must keep carrying those parts —
//! that lane is the container.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use idml_export::{is_container_part, write_idml, write_paged, MANIFEST_NAME};

const DESIGNMAP: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document DOMVersion="20.0" Self="d1" xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<idPkg:Spread src="Spreads/Spread_s1.xml"/>
<idPkg:Story src="Stories/Story_st1.xml"/>
</Document>"#;

const SPREAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
</Spread></idPkg:Spread>"#;

const STORY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Story Self="st1"><ParagraphStyleRange><CharacterStyleRange><Content>Hello</Content></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;

/// A package: the IDML core plus `extra` entries (deflated).
fn package(extra: &[(&str, &[u8])]) -> Vec<u8> {
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
        for (name, body) in [
            ("designmap.xml", DESIGNMAP.as_bytes()),
            ("Spreads/Spread_s1.xml", SPREAD.as_bytes()),
            ("Stories/Story_st1.xml", STORY.as_bytes()),
        ] {
            zip.start_file(name, deflated).unwrap();
            zip.write_all(body).unwrap();
        }
        for (name, body) in extra {
            zip.start_file(*name, deflated).unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

fn entry_names(package: &[u8]) -> Vec<String> {
    let zip = zip::ZipArchive::new(Cursor::new(package)).expect("zip");
    zip.file_names().map(|n| n.to_string()).collect()
}

fn entry(package: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(package)).expect("zip");
    let mut e = zip.by_name(name).ok()?;
    let mut out = Vec::new();
    e.read_to_end(&mut out).unwrap();
    Some(out)
}

const CONTAINER_PARTS: &[(&str, &[u8])] = &[
    (MANIFEST_NAME, br#"{"format":"paged-container","v":1}"#),
    ("paged/core/model/document.pgm", b"not-a-real-pgm"),
    ("paged/demo/state.json", b"{\"plugin\":\"demo\"}"),
];

#[test]
fn write_idml_drops_the_container_parts() {
    let source = package(CONTAINER_PARTS);
    let doc = idml_import::import_idml_doc(&source).expect("source parses");

    let idml = write_idml(&doc, &source).expect("write");
    let names = entry_names(&idml);
    for (name, _) in CONTAINER_PARTS {
        assert!(
            !names.iter().any(|n| n == name),
            "{name} must not survive into a pure .idml; got {names:?}"
        );
    }
    assert!(
        names.iter().all(|n| !is_container_part(n)),
        "every remaining entry must be an IDML part: {names:?}"
    );
    // The IDML parts are all there, byte-for-byte, and the product opens.
    for name in [
        "mimetype",
        "designmap.xml",
        "Spreads/Spread_s1.xml",
        "Stories/Story_st1.xml",
    ] {
        assert_eq!(entry(&idml, name), entry(&source, name), "{name}");
    }
    assert_eq!(names[0], "mimetype", "mimetype stays first");
    let reopened = idml_import::import_idml_doc(&idml).expect("pure idml re-parses");
    assert_eq!(
        reopened.stories[0].story.paragraphs[0].runs[0].text,
        "Hello"
    );
}

#[test]
fn write_paged_keeps_carrying_the_container_parts() {
    let source = package(CONTAINER_PARTS);
    let doc = idml_import::import_idml_doc(&source).expect("source parses");

    let paged = write_paged(&doc, &source, &BTreeMap::new(), 61).expect("write_paged");
    let names = entry_names(&paged);
    for (name, body) in CONTAINER_PARTS {
        if *name == MANIFEST_NAME {
            // Refreshed each save (the data-loss guard hash), but present.
            assert!(names.iter().any(|n| n == name), "{name} present: {names:?}");
            continue;
        }
        assert_eq!(
            entry(&paged, name).as_deref(),
            Some(*body),
            "{name} carried through verbatim by the container lane"
        );
    }
}

#[test]
fn a_source_without_container_parts_still_round_trips_byte_identically() {
    let source = package(&[]);
    let doc = idml_import::import_idml_doc(&source).expect("source parses");
    let idml = write_idml(&doc, &source).expect("write");
    assert_eq!(source, idml, "no container parts ⇒ the filter is a no-op");
}
