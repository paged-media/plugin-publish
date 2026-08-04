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

//! Z-order save-back (C-23), end to end through the real [`write_idml`].
//!
//! Core's v59 `ReorderNode` permutes `Spread::frames_in_order`; IDML
//! spells that order as the document order of the page items inside
//! `<Spread>`. This drives the model exactly as the op does, writes the
//! package back out, and RE-IMPORTS it — the only check that proves an
//! Arrange the user performed is the Arrange they get back.
//!
//! The fixture's front item deliberately carries what the writer's
//! `write_new_*` emitters do NOT model — a placed `<Image>`, a
//! `<TextWrapPreference>`, an unparsed `LocalDisplaySetting` attribute.
//! Re-minting the element at its new slot would drop all three, which is
//! why the lane moves BYTES instead. These assertions are that promise.

use std::io::{Read, Write};

use idml_export::write_idml;
use paged_scene::{Document, ParsedSpread};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const SPREAD_SRC: &str = "Spreads/Spread_u1.xml";

const DESIGNMAP: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0" Self="doc">
<idPkg:Graphic src="Resources/Graphic.xml"/>
<idPkg:Spread src="Spreads/Spread_u1.xml"/>
</Document>"#;

const GRAPHIC: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" Name="Black"/>
<Color Self="Color/Paper" Model="Process" Space="CMYK" ColorValue="0 0 0 0" Name="Paper"/>
</idPkg:Graphic>"#;

/// Three top-level items plus a two-member group. `r1` is the rich one:
/// a placed image, a text-wrap preference, and an attribute the model
/// never reads.
const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Rectangle Self="r1" ItemTransform="1 0 0 1 10 10" GeometricBounds="0 0 50 50" FillColor="Color/Black" LocalDisplaySetting="HighResolution">
<TextWrapPreference Inverse="false" TextWrapMode="None"/>
<Image Self="img1" ItemTransform="1 0 0 1 0 0"><Link Self="lnk1" LinkResourceURI="file:///art.jpg"/></Image>
</Rectangle>
<Rectangle Self="r2" ItemTransform="1 0 0 1 20 20" GeometricBounds="0 0 60 60" FillColor="Color/Paper"/>
<Oval Self="o1" ItemTransform="1 0 0 1 30 30" GeometricBounds="0 0 70 70" FillColor="Color/Black"/>
<Group Self="g1" ItemTransform="1 0 0 1 0 0">
<Rectangle Self="m1" ItemTransform="1 0 0 1 1 1" GeometricBounds="0 0 10 10" FillColor="Color/Black"/>
<Rectangle Self="m2" ItemTransform="1 0 0 1 2 2" GeometricBounds="0 0 20 20" FillColor="Color/Paper"/>
</Group>
</Spread>
</idPkg:Spread>"#;

/// A minimal valid IDML package: stored `mimetype` first, then the
/// designmap and the parts it references.
fn build_package() -> Vec<u8> {
    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/vnd.adobe.indesign-idml-package")
        .unwrap();
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, body) in [
        ("designmap.xml", DESIGNMAP),
        ("Resources/Graphic.xml", GRAPHIC),
        (SPREAD_SRC, SPREAD),
    ] {
        zip.start_file(name, deflated).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Parse the package into the `Document` shape the writer consumes.
/// (The IDML→`Document` orchestrator lives in core's `paged-gen`, which
/// this repo can't depend on without a cycle.)
fn open(package: &[u8]) -> Document {
    let archive = idml_import::open_source_archive(package).expect("valid IDML");
    Document {
        designmap: idml_import::parse_designmap(&archive.designmap_raw).expect("designmap"),
        palette: idml_import::parse_graphic(GRAPHIC).expect("graphic"),
        spreads: vec![ParsedSpread {
            src: SPREAD_SRC.to_string(),
            spread: idml_import::parse_spread(SPREAD).expect("spread"),
        }],
        stories: Vec::new(),
        master_spreads: Default::default(),
        frame_for_story: Default::default(),
        text_frame_index: Default::default(),
        styles: Default::default(),
        anchors: Vec::new(),
    }
}

fn entry(package: &[u8], name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(package)).expect("zip");
    let mut e = zip
        .by_name(name)
        .unwrap_or_else(|_| panic!("{name} present"));
    let mut buf = Vec::new();
    e.read_to_end(&mut buf).expect("read");
    buf
}

/// The stacking order a REOPENED package reports: the `Self` ids of the
/// re-parsed spread's z table, back to front. This is the order the
/// renderer paints in, so it is the order the user sees.
fn reopened_z_order(package: &[u8]) -> Vec<String> {
    let spread = idml_import::parse_spread(&entry(package, SPREAD_SRC)).expect("re-parse");
    z_ids(&spread)
}

fn z_ids(spread: &idml_import::Spread) -> Vec<String> {
    use idml_import::FrameRef;
    spread
        .frames_in_order
        .iter()
        .map(|r| {
            let id = match *r {
                FrameRef::TextFrame(i) => spread.text_frames[i].self_id.clone(),
                FrameRef::Rectangle(i) => spread.rectangles[i].self_id.clone(),
                FrameRef::Oval(i) => spread.ovals[i].self_id.clone(),
                FrameRef::GraphicLine(i) => spread.graphic_lines[i].self_id.clone(),
                FrameRef::Polygon(i) => spread.polygons[i].self_id.clone(),
                FrameRef::Group(i) => spread.groups[i].self_id.clone(),
            };
            id.expect("every fixture item has a Self")
        })
        .collect()
}

/// THE PRIME INVARIANT, at package level: a document nobody reordered
/// writes back byte-identically. The z lane runs on every export, so it
/// has to be invisible on documents that never moved anything.
#[test]
fn unmutated_package_round_trips_byte_identically() {
    let package = build_package();
    let doc = open(&package);
    let out = write_idml(&doc, &package).expect("write");
    assert_eq!(out, package, "an unmutated package must be byte-identical");
    assert_eq!(reopened_z_order(&package), ["r1", "r2", "o1", "g1"]);
}

/// THE DEFECT, closed: bring-to-front survives an `.idml` export and
/// reopen. Before C-23 the writer re-emitted the source order and the
/// Arrange silently reverted.
#[test]
fn bring_to_front_survives_export_and_reimport() {
    let package = build_package();
    let mut doc = open(&package);

    // `Operation::ReorderNode { node: r1, target: Front }` — the op
    // permutes the spread's z table and nothing else.
    let z = &mut doc.spreads[0].spread.frames_in_order;
    let front = z.remove(0);
    z.push(front);

    let out = write_idml(&doc, &package).expect("write");
    assert_eq!(
        reopened_z_order(&out),
        ["r2", "o1", "g1", "r1"],
        "the reopened document must paint in the order the user arranged"
    );

    // ...and the moved element arrived intact. The `write_new_*`
    // emitters model none of these three, so their survival is the
    // proof that the element was MOVED, not re-minted.
    let xml = String::from_utf8(entry(&out, SPREAD_SRC)).expect("utf8");
    assert!(
        xml.contains(r#"LocalDisplaySetting="HighResolution""#),
        "{xml}"
    );
    assert!(xml.contains(r#"<Image Self="img1""#), "{xml}");
    assert!(
        xml.contains(r#"LinkResourceURI="file:///art.jpg""#),
        "{xml}"
    );
    assert!(xml.contains("<TextWrapPreference"), "{xml}");
    // Nothing duplicated: exactly one element per id.
    for id in ["r1", "r2", "o1", "g1", "m1", "m2", "img1"] {
        assert_eq!(
            xml.matches(&format!(r#"Self="{id}""#)).count(),
            1,
            "{id} must appear exactly once: {xml}"
        );
    }
}

/// Send-to-back is the same lane in the other direction, and a group
/// restacks as one unit — its members ride inside its element.
#[test]
fn send_to_back_moves_a_whole_group() {
    let package = build_package();
    let mut doc = open(&package);
    let z = &mut doc.spreads[0].spread.frames_in_order;
    let back = z.pop().expect("the group is on top");
    z.insert(0, back);

    let out = write_idml(&doc, &package).expect("write");
    assert_eq!(reopened_z_order(&out), ["g1", "r1", "r2", "o1"]);

    let spread = idml_import::parse_spread(&entry(&out, SPREAD_SRC)).expect("re-parse");
    let group = &spread.groups[0];
    assert_eq!(group.self_id.as_deref(), Some("g1"));
    assert_eq!(group.members.len(), 2, "the group kept both members");
    let xml = String::from_utf8(entry(&out, SPREAD_SRC)).expect("utf8");
    assert!(
        xml.find(r#"Self="m1""#) < xml.find(r#"Self="m2""#),
        "a SPREAD-level restack must not disturb the group's own order: {xml}"
    );
}

/// A reorder INSIDE a group saves back too — `ReorderNode` permutes
/// `Group::members` for a grouped node, and that list is the `<Group>`'s
/// element order.
#[test]
fn a_group_member_reorder_survives_export_and_reimport() {
    let package = build_package();
    let mut doc = open(&package);
    doc.spreads[0].spread.groups[0].members.reverse();

    let out = write_idml(&doc, &package).expect("write");
    let spread = idml_import::parse_spread(&entry(&out, SPREAD_SRC)).expect("re-parse");
    let members: Vec<String> = spread.groups[0]
        .members
        .iter()
        .map(|m| match *m {
            idml_import::FrameRef::Rectangle(i) => {
                spread.rectangles[i].self_id.clone().expect("Self")
            }
            other => panic!("unexpected member kind: {other:?}"),
        })
        .collect();
    assert_eq!(members, ["m2", "m1"]);
    // The top level is untouched by a group-internal move.
    assert_eq!(z_ids(&spread), ["r1", "r2", "o1", "g1"]);
}
