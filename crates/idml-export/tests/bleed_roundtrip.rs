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

/// The page setup on the same element, in InDesign's spelling: page size,
/// facing pages, binding.
const INDESIGN_PREFS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
	<DocumentPreference PageHeight="841.889763778" PageWidth="595.275590551" PagesPerDocument="1" FacingPages="true" DocumentBleedTopOffset="0" DocumentBleedBottomOffset="0" DocumentBleedInsideOrLeftOffset="0" DocumentBleedOutsideOrRightOffset="0" DocumentBleedUniformSize="true" SlugTopOffset="0" SlugBottomOffset="0" SlugInsideOrLeftOffset="0" SlugRightOrOutsideOffset="0" DocumentSlugUniformSize="false" PreserveLayoutWhenShuffling="true" AllowPageShuffle="true" OverprintBlack="true" PageBinding="LeftToRight" ColumnDirection="Horizontal" SpreadHiddenVisibility="false" Intent="PrintIntent" CreatePrimaryTextFrame="false" ColumnGuideLocked="true" MasterTextFrame="false" SnippetImportUsesOriginalLocation="false">
		<Properties>
			<ColumnGuideColor type="enumeration">Violet</ColumnGuideColor>
			<MarginGuideColor type="enumeration">Magenta</MarginGuideColor>
		</Properties>
	</DocumentPreference>
	<TextPreference TypographersQuotes="true" UseOpticalSize="true" />
</idPkg:Preferences>"#;

fn indesign_source() -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Resources/Preferences.xml", INDESIGN_PREFS),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

#[test]
fn the_page_setup_is_read_and_an_unchanged_one_round_trips_byte_identically() {
    let src = indesign_source();
    let doc = pkg::open(&src);
    let p = &doc.designmap.document_preference;
    assert!((p.page_width.unwrap() - 595.2756).abs() < 1e-3, "{p:?}");
    assert!((p.page_height.unwrap() - 841.8898).abs() < 1e-3);
    assert_eq!(p.facing_pages, Some(true));
    assert_eq!(p.page_binding.as_deref(), Some("LeftToRight"));
    let out = write_idml(&doc, &src).expect("write");
    pkg::assert_same_entries(&src, &out);
}

/// A bleed edit rewrites the element; the page setup the model read from
/// it is kept as InDesign spelled it (full precision included).
#[test]
fn a_bleed_edit_keeps_the_page_setup_as_spelled() {
    let src = indesign_source();
    let mut doc = pkg::open(&src);
    doc.designmap.document_preference.bleed_top = 9.0;
    let out = write_idml(&doc, &src).expect("write");
    let prefs = pkg::entry(&out, "Resources/Preferences.xml").expect("prefs");
    assert!(
        prefs.contains(r#"<DocumentPreference PageHeight="841.889763778" PageWidth="595.275590551" PagesPerDocument="1" FacingPages="true" DocumentBleedTopOffset="9""#),
        "{prefs}"
    );
    assert!(prefs.contains(r#"PageBinding="LeftToRight""#), "{prefs}");
}

#[test]
fn an_edited_page_setup_is_written_onto_the_element_and_reads_back() {
    let src = indesign_source();
    let mut doc = pkg::open(&src);
    let p = &mut doc.designmap.document_preference;
    p.page_width = Some(612.0);
    p.page_height = Some(792.0);
    p.facing_pages = Some(false);
    p.page_binding = Some("RightToLeft".into());
    let out = write_idml(&doc, &src).expect("write");
    let prefs = pkg::entry(&out, "Resources/Preferences.xml").expect("prefs");
    assert!(
        prefs.contains(r#"<DocumentPreference PageHeight="792" PageWidth="612" PagesPerDocument="1" FacingPages="false" DocumentBleedTopOffset="0""#),
        "in place, the neighbours untouched: {prefs}"
    );
    assert!(
        prefs.contains(
            r#"OverprintBlack="true" PageBinding="RightToLeft" ColumnDirection="Horizontal""#
        ),
        "{prefs}"
    );
    assert!(prefs.contains("<ColumnGuideColor type=\"enumeration\">Violet</ColumnGuideColor>"));
    let back = pkg::open(&out).designmap.document_preference;
    assert_eq!(back.page_width, Some(612.0));
    assert_eq!(back.page_height, Some(792.0));
    assert_eq!(back.facing_pages, Some(false));
    assert_eq!(back.page_binding.as_deref(), Some("RightToLeft"));
}
