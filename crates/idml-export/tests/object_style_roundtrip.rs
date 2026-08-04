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

//! Object-style save-back, end to end through the real [`write_idml`].
//!
//! Builds a synthetic-but-real-shaped IDML package, mutates the model
//! the way the engine's `CreateObjectStyle` + `SetProperty(
//! AppliedObjectStyle)` operations do, writes the package back out, and
//! RE-IMPORTS it — the only check that proves the exported package is
//! self-consistent (definition present, reference resolving to it).
//!
//! `paged-mutate` can't drive this from here: core's `paged-scene` /
//! `paged-mutate` / `paged-gen` all depend on `idml-import` across the
//! git boundary, so depending on them back would make the dep cycle
//! (this is why `6babe02` re-homed the save-back integration tests into
//! core). The two ops are reproduced exactly instead — `style_crud!`'s
//! create arm builds a `default()` def carrying only `self_id` / `name`
//! / `based_on`, and the property setter writes the item's
//! `applied_object_style` field.

use std::io::{Read, Write};

use idml_export::write_idml;
use paged_scene::{Document, ParsedSpread};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const STYLES_SRC: &str = "Resources/Styles.xml";
const SPREAD_SRC: &str = "Spreads/Spread_u1.xml";

const DESIGNMAP: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0" Self="doc">
<idPkg:Graphic src="Resources/Graphic.xml"/>
<idPkg:Styles src="Resources/Styles.xml"/>
<idPkg:Spread src="Spreads/Spread_u1.xml"/>
</Document>"#;

const GRAPHIC: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" Name="Black"/>
<Color Self="Color/Paper" Model="Process" Space="CMYK" ColorValue="0 0 0 0" Name="Paper"/>
</idPkg:Graphic>"#;

const STYLES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]" Imported="false"/></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]" Imported="false"/></RootParagraphStyleGroup>
<RootObjectStyleGroup Self="u9f"><ObjectStyle Self="ObjectStyle/$ID/[None]" Name="$ID/[None]" Imported="false"/><ObjectStyle Self="ObjectStyle/Callout" Name="Callout" FillColor="Color/Black" FillTint="20" StrokeColor="Color/Paper" StrokeWeight="1.5"/></RootObjectStyleGroup>
</idPkg:Styles>"#;

const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1"><Page Self="pg1" GeometricBounds="0 0 792 612"/><Rectangle Self="r1" AppliedObjectStyle="ObjectStyle/$ID/[None]" ItemTransform="1 0 0 1 10 10" GeometricBounds="0 0 50 50" FillColor="Color/Black"/></Spread>
</idPkg:Spread>"#;

/// A minimal valid IDML package: stored `mimetype` first, then the
/// designmap and the three parts it references.
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
        (STYLES_SRC, STYLES),
        (SPREAD_SRC, SPREAD),
    ] {
        zip.start_file(name, deflated).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Parse the package into the `Document` shape the writer consumes.
/// (The IDML→`Document` orchestrator lives in core's `paged-gen`, which
/// this repo can't depend on; the fields are all public, so the parts
/// this test exercises are assembled directly.)
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
        styles: idml_import::parse_stylesheet(STYLES).expect("styles"),
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

fn entry_names(package: &[u8]) -> Vec<String> {
    let zip = zip::ZipArchive::new(std::io::Cursor::new(package)).expect("zip");
    zip.file_names().map(str::to_string).collect()
}

/// THE PRIME INVARIANT, at package level: a document nobody mutated
/// writes back byte-identically. The object-style lanes must not
/// disturb the lazy-verbatim posture.
#[test]
fn unmutated_package_round_trips_byte_identically() {
    let package = build_package();
    let doc = open(&package);
    let out = write_idml(&doc, &package).expect("write");
    assert_eq!(out, package, "an unmutated package must be byte-identical");
}

/// The whole point: an object style created by the engine and applied
/// to a page item survives export AND re-import — the definition lands
/// in `<RootObjectStyleGroup>`, the item points at it, and the pointer
/// resolves.
#[test]
fn created_object_style_survives_export_and_reimport() {
    let package = build_package();
    let mut doc = open(&package);

    // `Operation::CreateObjectStyle { name, based_on }` — the def is a
    // `default()` body carrying only these three fields; everything
    // else cascades through BasedOn.
    doc.styles.object_styles.insert(
        "ObjectStyle/u0".to_string(),
        idml_import::ObjectStyleDef {
            self_id: "ObjectStyle/u0".to_string(),
            name: Some("Sidebar".to_string()),
            based_on: Some("ObjectStyle/Callout".to_string()),
            stroke_weight: Some(3.0),
            corner_option: Some("RoundedCorner".to_string()),
            corner_radius: Some(6.0),
            ..Default::default()
        },
    );
    // `SetProperty(AppliedObjectStyle)` on the rectangle.
    doc.spreads[0].spread.rectangles[0].applied_object_style = Some("ObjectStyle/u0".to_string());

    let out = write_idml(&doc, &package).expect("write");

    // --- re-import ---------------------------------------------------
    let styles = idml_import::parse_stylesheet(&entry(&out, STYLES_SRC)).expect("styles re-parse");
    let def = styles
        .object_styles
        .get("ObjectStyle/u0")
        .expect("the created object style is DEFINED in the exported package");
    assert_eq!(def.name.as_deref(), Some("Sidebar"));
    assert_eq!(def.based_on.as_deref(), Some("ObjectStyle/Callout"));
    assert_eq!(def.stroke_weight, Some(3.0));
    assert_eq!(def.corner_option.as_deref(), Some("RoundedCorner"));
    assert_eq!(def.corner_radius, Some(6.0));

    // The source styles are still there, exactly once each — injection
    // must not duplicate what the part already carried.
    assert_eq!(styles.object_styles.len(), 3, "None + Callout + u0");
    let styles_xml = String::from_utf8(entry(&out, STYLES_SRC)).expect("utf8");
    assert_eq!(
        styles_xml
            .matches(r#"Self="ObjectStyle/$ID/[None]""#)
            .count(),
        1,
        "the reserved style must not be duplicated: {styles_xml}"
    );

    // The reference resolves — no dangling `AppliedObjectStyle`.
    let spread = idml_import::parse_spread(&entry(&out, SPREAD_SRC)).expect("spread re-parse");
    let applied = spread.rectangles[0]
        .applied_object_style
        .as_deref()
        .expect("the item still carries a reference");
    assert_eq!(applied, "ObjectStyle/u0");
    assert!(
        styles.object_styles.contains_key(applied),
        "the applied style must be defined in the same package"
    );

    // ...and the BasedOn cascade is intact: `FillColor` / `FillTint`
    // come from Callout, `StrokeWeight` from the derived style itself.
    let resolved = styles.resolve_object(applied);
    assert_eq!(resolved.fill_color.as_deref(), Some("Color/Black"));
    assert_eq!(resolved.fill_tint, Some(20.0));
    assert_eq!(resolved.stroke_weight, Some(3.0));
}

/// A style DEFINED but not REGISTERED is the sibling failure mode: the
/// definition rides in an existing part, so `designmap.xml` must keep
/// pointing at it (and must not have been rewritten to add anything).
#[test]
fn the_styles_part_stays_registered_in_the_designmap() {
    let package = build_package();
    let mut doc = open(&package);
    doc.styles.object_styles.insert(
        "ObjectStyle/u0".to_string(),
        idml_import::ObjectStyleDef {
            self_id: "ObjectStyle/u0".to_string(),
            name: Some("Sidebar".to_string()),
            ..Default::default()
        },
    );
    let out = write_idml(&doc, &package).expect("write");

    assert_eq!(
        entry(&out, "designmap.xml"),
        DESIGNMAP,
        "no new part was minted, so the designmap is untouched"
    );
    assert_eq!(
        entry_names(&out),
        entry_names(&package),
        "the package's entry set is unchanged"
    );
    // And the manifest still names the part the definition landed in.
    let designmap = idml_import::parse_designmap(&entry(&out, "designmap.xml")).expect("designmap");
    let _ = designmap;
    assert!(
        String::from_utf8_lossy(&entry(&out, "designmap.xml")).contains(STYLES_SRC),
        "the styles part must stay referenced"
    );
}
