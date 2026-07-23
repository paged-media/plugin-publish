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

//! The `.paged` OCF container the load path expects.
//!
//! The engine's load sniff (`paged-canvas::CanvasModel::load` →
//! `idml_import::open_source_archive`) requires a ZIP with a STORED-first
//! `mimetype` == the IDML-package constant **and** a `designmap.xml` entry,
//! then looks up `paged/core/model/document.pgm`; when
//! `paged_store::from_bytes` decodes that part it uses the `Document`
//! directly with **no IDML/designmap parse**. So the PDF import wraps its
//! reconstructed model in exactly this skeleton — the pgm carries the truth,
//! and the designmap/resources are a *valid-but-empty* fallback that only
//! matters if the pgm ever fails to decode (version drift), in which case the
//! load degrades to a blank one-page document rather than a parse error.
//!
//! The skeleton mirrors `paged-canvas::blank::blank_idml` (which this crate
//! cannot call — `paged-canvas` isn't a dependency). Keep the two in sync.

use std::io::{Cursor, Write};

use paged_scene::Document;

/// IDML/OCF package mimetype. MUST be the first ZIP entry and STORED.
const MIME: &str = "application/vnd.adobe.indesign-idml-package";
const NS: &str = "http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging";

fn xml(body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{body}")
}

fn empty_pkg(tag: &str) -> String {
    xml(&format!(
        "<idPkg:{tag} xmlns:idPkg=\"{NS}\" DOMVersion=\"20.0\"/>"
    ))
}

fn container() -> String {
    xml(
        "<container xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\" version=\"1.0\">\
<rootfiles><rootfile full-path=\"designmap.xml\" media-type=\"text/xml\"/></rootfiles></container>",
    )
}

fn graphic() -> String {
    xml(&format!(
        "<idPkg:Graphic xmlns:idPkg=\"{NS}\" DOMVersion=\"20.0\">\
<Color Self=\"Color/Black\" Model=\"Process\" Space=\"CMYK\" ColorValue=\"0 0 0 100\" Name=\"Black\"/>\
<Swatch Self=\"Swatch/None\" Name=\"None\"/></idPkg:Graphic>"
    ))
}

fn styles() -> String {
    xml(&format!(
        "<idPkg:Styles xmlns:idPkg=\"{NS}\" DOMVersion=\"20.0\">\
<RootCharacterStyleGroup Self=\"rcs\">\
<CharacterStyle Self=\"CharacterStyle/$ID/[No character style]\" Name=\"$ID/[No character style]\"/>\
</RootCharacterStyleGroup>\
<RootParagraphStyleGroup Self=\"rps\">\
<ParagraphStyle Self=\"ParagraphStyle/$ID/[No paragraph style]\" Name=\"$ID/[No paragraph style]\"/>\
</RootParagraphStyleGroup></idPkg:Styles>"
    ))
}

fn backing() -> String {
    xml(&format!(
        "<idPkg:BackingStory xmlns:idPkg=\"{NS}\" DOMVersion=\"20.0\">\
<XmlStory Self=\"backing\"/></idPkg:BackingStory>"
    ))
}

fn designmap() -> String {
    xml(&format!(
        "<?aid style=\"50\" type=\"document\" readerVersion=\"6.0\" featureSet=\"257\" product=\"20.0(32)\"?>\n\
<Document xmlns:idPkg=\"{NS}\" DOMVersion=\"20.0\" Self=\"d\" StoryList=\"\" Name=\"Imported.pdf\">\n\
<idPkg:Graphic src=\"Resources/Graphic.xml\"/>\n\
<idPkg:Fonts src=\"Resources/Fonts.xml\"/>\n\
<idPkg:Styles src=\"Resources/Styles.xml\"/>\n\
<idPkg:Preferences src=\"Resources/Preferences.xml\"/>\n\
<idPkg:MasterSpread src=\"MasterSpreads/MasterSpread_um.xml\"/>\n\
<idPkg:Spread src=\"Spreads/Spread_us.xml\"/>\n\
<idPkg:BackingStory src=\"XML/BackingStory.xml\"/>\n\
</Document>"
    ))
}

fn master_spread(bounds: &str) -> String {
    xml(&format!(
        "<idPkg:MasterSpread xmlns:idPkg=\"{NS}\" DOMVersion=\"20.0\">\
<MasterSpread Self=\"um\" Name=\"A\">\
<Page Self=\"ump\" Name=\"A\" GeometricBounds=\"{bounds}\" ItemTransform=\"1 0 0 1 0 0\"/>\
</MasterSpread></idPkg:MasterSpread>"
    ))
}

fn spread(bounds: &str) -> String {
    xml(&format!(
        "<idPkg:Spread xmlns:idPkg=\"{NS}\" DOMVersion=\"20.0\">\n\
<Spread Self=\"us\" PageCount=\"1\" ItemTransform=\"1 0 0 1 0 0\">\n\
<Page Self=\"usp\" Name=\"1\" GeometricBounds=\"{bounds}\" ItemTransform=\"1 0 0 1 0 0\" AppliedMaster=\"um\"/>\n\
</Spread></idPkg:Spread>"
    ))
}

/// Wrap a reconstructed [`Document`] in the `.paged` OCF container the engine
/// load path accepts. `fallback_width_pt` × `fallback_height_pt` size the
/// empty skeleton page that is only ever parsed if the pgm fails to decode.
pub fn wrap_document(
    doc: &Document,
    fallback_width_pt: f32,
    fallback_height_pt: f32,
) -> Result<Vec<u8>, crate::Error> {
    let pgm = paged_store::to_bytes(doc).map_err(crate::Error::Pgm)?;
    // InDesign's GeometricBounds order is "y0 x0 y1 x1".
    let bounds = format!("0 0 {fallback_height_pt} {fallback_width_pt}");

    let mut zip = zip::write::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // In-memory writes are infallible; `expect` documents the invariant.
    let mut put_text = |name: &str, body: &str, stored_entry: bool| {
        let opts = if stored_entry { stored } else { deflated };
        zip.start_file(name, opts).expect("zip start_file");
        zip.write_all(body.as_bytes()).expect("zip write_all");
    };

    // mimetype first + STORED (OCF convention) — the sniff keys on it.
    put_text("mimetype", MIME, true);
    put_text("designmap.xml", &designmap(), false);
    put_text("META-INF/container.xml", &container(), false);
    put_text("Resources/Graphic.xml", &graphic(), false);
    put_text("Resources/Fonts.xml", &empty_pkg("Fonts"), false);
    put_text("Resources/Styles.xml", &styles(), false);
    put_text(
        "Resources/Preferences.xml",
        &empty_pkg("Preferences"),
        false,
    );
    put_text(
        "MasterSpreads/MasterSpread_um.xml",
        &master_spread(&bounds),
        false,
    );
    put_text("Spreads/Spread_us.xml", &spread(&bounds), false);
    put_text("XML/BackingStory.xml", &backing(), false);

    // The native model part — this is what the load path actually uses.
    zip.start_file(paged_store::DOCUMENT_PGM_PATH, deflated)
        .expect("zip start_file pgm");
    zip.write_all(&pgm).expect("zip write_all pgm");

    Ok(zip.finish().expect("zip finish").into_inner())
}
