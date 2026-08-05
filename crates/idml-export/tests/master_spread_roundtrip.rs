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

//! An edit to a MASTER page survives a save (and a reopen).
//!
//! # The defect
//!
//! `import_idml_archive` parses every `MasterSpreads/*.xml` with the same
//! `parse_spread` the ordinary spreads use and files the result under
//! `Document::master_spreads`. `write_idml` iterated `doc.spreads` only.
//! So a master part took the verbatim `raw_copy_file` path on every save,
//! and a user who edited a master — moved the running header, recoloured
//! the page frame — saved, reopened, and found the edit gone. No error,
//! no diagnostic: the model held the change and the writer never asked.
//!
//! Byte-identity could never have caught it. Copying an entry verbatim is
//! the *most* byte-identical thing a writer can do; the corpus sweep read
//! 0 gaps over all 316 master entries precisely BECAUSE nothing
//! transformed them. That is why the sweep ran the lane latently — it
//! measured the rewrite the writer *would* use, so "is `rewrite_spread`
//! safe to route masters through?" was answered (yes, 0 gaps, 0
//! malformed) before the hole was closed rather than after.
//!
//! # What this file pins
//!
//! The premise (the parser really does model the master's page items, so
//! the mutation below is a real one), the defect (the edit reaches the
//! saved package and survives a reopen), and the invariant that must not
//! regress on the way (an unmutated package is still byte-identical, and
//! a master's untouched neighbours are not re-minted).

use std::io::{Read, Write};

use idml_export::write_idml;
use paged_scene::Document;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

const MASTER_SRC: &str = "MasterSpreads/MasterSpread_uad.xml";
const SPREAD_SRC: &str = "Spreads/Spread_u1.xml";

const DESIGNMAP: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0" Self="doc">
<idPkg:Graphic src="Resources/Graphic.xml"/>
<idPkg:MasterSpread src="MasterSpreads/MasterSpread_uad.xml"/>
<idPkg:Spread src="Spreads/Spread_u1.xml"/>
</Document>"#;

const GRAPHIC: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" Name="Black"/>
<Color Self="Color/Paper" Model="Process" Space="CMYK" ColorValue="0 0 0 0" Name="Paper"/>
</idPkg:Graphic>"#;

/// A real `<MasterSpread>`: the element name InDesign uses for the part,
/// a page, and two items. `mr1` carries what the writer's `write_new_*`
/// emitters do NOT model (a `<TextWrapPreference>`, an unparsed
/// `LocalDisplaySetting`) so a re-mint would be visible as a loss.
const MASTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:MasterSpread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<MasterSpread Self="uad" Name="A-Master" NamePrefix="A" BaseName="Master">
<Page Self="mpg1" GeometricBounds="0 0 792 612" AppliedMaster="n"/>
<Rectangle Self="mr1" ItemTransform="1 0 0 1 10 10" GeometricBounds="0 0 50 50" FillColor="Color/Black" LocalDisplaySetting="HighResolution">
<TextWrapPreference Inverse="false" TextWrapMode="None"/>
</Rectangle>
<Oval Self="mo1" ItemTransform="1 0 0 1 30 30" GeometricBounds="0 0 70 70" FillColor="Color/Paper"/>
</MasterSpread>
</idPkg:MasterSpread>"#;

const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612" AppliedMaster="MasterSpread/uad"/>
<Rectangle Self="r1" ItemTransform="1 0 0 1 20 20" GeometricBounds="0 0 60 60" FillColor="Color/Paper"/>
</Spread>
</idPkg:Spread>"#;

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
        (MASTER_SRC, MASTER),
        (SPREAD_SRC, SPREAD),
    ] {
        zip.start_file(name, deflated).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn open(package: &[u8]) -> Document {
    idml_import::import_idml_doc(package).expect("import")
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

/// The one master in the fixture, by the `Self` id the importer derives
/// from the entry name.
fn master(doc: &Document) -> &paged_scene::ParsedMasterSpread {
    doc.master_spread("MasterSpread/uad")
        .expect("the master is in the model")
}

/// THE PREMISE, asserted rather than assumed: the importer really does
/// parse the master's page items onto the model, so the mutation below
/// is a real edit and not a write to a field nobody reads.
#[test]
fn the_importer_models_the_master_spreads_page_items() {
    let doc = open(&build_package());
    assert_eq!(doc.master_spreads.len(), 1, "one master parsed");
    let m = master(&doc);
    assert_eq!(m.src, MASTER_SRC);
    assert_eq!(m.spread.rectangles.len(), 1, "the master's rectangle");
    assert_eq!(m.spread.ovals.len(), 1, "the master's oval");
    assert_eq!(
        m.spread.rectangles[0].fill_color.as_deref(),
        Some("Color/Black")
    );
}

/// THE PRIME INVARIANT: routing masters through the rewrite must not
/// cost byte-identity on a document nobody touched.
#[test]
fn an_unmutated_package_with_a_master_round_trips_byte_identically() {
    let package = build_package();
    let doc = open(&package);
    let out = write_idml(&doc, &package).expect("write");
    assert_eq!(out, package, "an unmutated package must be byte-identical");
}

/// THE DEFECT: an edit to a master page item reaches the saved package
/// and survives a reopen. Before this, `write_idml` iterated
/// `doc.spreads` only and the master part was copied verbatim — the edit
/// vanished with no error.
#[test]
fn a_master_page_item_edit_survives_save_and_reopen() {
    let package = build_package();
    let mut doc = open(&package);

    let m = doc
        .master_spreads
        .get_mut("uad")
        .expect("the master is keyed by its derived id");
    m.spread.rectangles[0].fill_color = Some("Color/Paper".to_string());

    let out = write_idml(&doc, &package).expect("write");
    assert_ne!(
        entry(&out, MASTER_SRC),
        MASTER,
        "the master part must actually change — a verbatim copy IS the defect"
    );

    // Reopen: the only check that proves the user gets their edit back.
    let reopened = open(&out);
    assert_eq!(
        master(&reopened).spread.rectangles[0].fill_color.as_deref(),
        Some("Color/Paper"),
        "the reopened master must carry the edit"
    );

    // ...and the element was PATCHED, not re-minted: the two things the
    // model does not carry are still there, and nothing is duplicated.
    let xml = String::from_utf8(entry(&out, MASTER_SRC)).expect("utf8");
    assert!(
        xml.contains(r#"LocalDisplaySetting="HighResolution""#),
        "{xml}"
    );
    assert!(xml.contains("<TextWrapPreference"), "{xml}");
    for id in ["mpg1", "mr1", "mo1"] {
        assert_eq!(
            xml.matches(&format!(r#"Self="{id}""#)).count(),
            1,
            "{id} must appear exactly once: {xml}"
        );
    }
    // The ordinary spread is untouched by a master-only edit.
    assert_eq!(entry(&out, SPREAD_SRC), SPREAD);
}

/// A geometry edit takes the same lane — this is not a fill-colour
/// special case.
#[test]
fn a_master_item_transform_edit_survives_save_and_reopen() {
    let package = build_package();
    let mut doc = open(&package);
    doc.master_spreads
        .get_mut("uad")
        .expect("master")
        .spread
        .ovals[0]
        .item_transform = Some([1.0, 0.0, 0.0, 1.0, 44.0, 55.0]);

    let out = write_idml(&doc, &package).expect("write");
    let reopened = open(&out);
    assert_eq!(
        master(&reopened).spread.ovals[0].item_transform,
        Some([1.0, 0.0, 0.0, 1.0, 44.0, 55.0]),
        "the reopened master must carry the moved oval"
    );
}

/// The lane the latent measurement lived on, now a real one: every
/// `MasterSpreads/*.xml` in the corpus rewrites byte-identically when
/// nothing was mutated, and stays well-formed. Opt-in — the corpus is
/// private and gitignored, so this no-ops cleanly wherever it is absent.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test master_spread_roundtrip \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn corpus_master_spreads_rewrite_byte_identically() {
    let Some(root) = corpus::root() else { return };
    let mut checked = 0usize;
    let mut differed: Vec<String> = Vec::new();
    let mut packages = Vec::new();
    collect(&root, &mut packages);
    packages.sort();
    for package in &packages {
        for (name, body) in corpus::entries(package, "MasterSpreads/") {
            let spread = idml_import::parse_spread(&body).expect("parse");
            let out = idml_export::rewrite::rewrite_spread(&body, &spread).expect("rewrite");
            checked += 1;
            if out != body {
                differed.push(format!("{}#{name}", package.display()));
            }
        }
    }
    assert!(checked > 0, "the corpus had reachable master spreads");
    assert!(
        differed.is_empty(),
        "master spreads are written now, so a rewrite gap is a real save-back \
         defect, not a latent one: {differed:?}"
    );
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_symlink() {
            continue;
        }
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "idml") {
            out.push(p);
        }
    }
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
