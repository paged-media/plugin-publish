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

//! Document bleed save-back: InDesign keeps `DocumentPreference` in
//! `Resources/Preferences.xml`; a bleed edited in paged is written onto
//! that element and reads back, and an unchanged one leaves the part
//! byte-identical.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const PREFS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<DocumentPreference PageHeight="792" PageWidth="612" FacingPages="false" DocumentBleedTopOffset="9" DocumentBleedBottomOffset="9" DocumentBleedInsideOrLeftOffset="9" DocumentBleedOutsideOrRightOffset="9"/>
<TextPreference UseOpticalSize="true"/>
</idPkg:Preferences>"#;

fn source() -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Resources/Preferences.xml", PREFS),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

#[test]
fn indesigns_bleed_is_read_from_the_preferences_part() {
    let doc = pkg::open(&source());
    assert_eq!(doc.designmap.document_preference.bleed_top, 9.0);
}

#[test]
fn an_edited_bleed_reaches_the_preferences_part_and_reads_back() {
    let src = source();
    let mut doc = pkg::open(&src);
    doc.designmap.document_preference.bleed_top = 12.5;
    doc.designmap.document_preference.slug_bottom = 20.0;
    let out = write_idml(&doc, &src).expect("write");
    let prefs = pkg::entry(&out, "Resources/Preferences.xml").expect("prefs");
    assert!(
        prefs.contains(r#"PageHeight="792" PageWidth="612" FacingPages="false""#),
        "{prefs}"
    );
    let back = pkg::open(&out);
    assert_eq!(back.designmap.document_preference.bleed_top, 12.5);
    assert_eq!(back.designmap.document_preference.bleed_bottom, 9.0);
    assert_eq!(back.designmap.document_preference.slug_bottom, 20.0);
}

#[test]
fn an_unchanged_bleed_leaves_the_part_byte_identical() {
    let src = source();
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    assert_eq!(
        pkg::entry(&out, "Resources/Preferences.xml"),
        pkg::entry(&src, "Resources/Preferences.xml")
    );
}
