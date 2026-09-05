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

//! Text no style reaches composes in the engine's fallback face; the
//! export says so (`<TextDefault>`, the face declared) and drops the
//! references to styles the document never defined, so InDesign does
//! the same instead of binding them to its application default
//! (measured 2026-09-05: 4,449 Minion Pro characters → 6).

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::{write_idml, write_idml_with, ExportOptions, FontFace};

const BODY: &str = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/docx-Default"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/docx-auto-c1"><Content>lost styles</Content></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange><Content>no style at all</Content></CharacterStyleRange></ParagraphStyleRange>"#;

const PREFERENCES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"/>"#;

fn source() -> Vec<u8> {
    pkg::package(&[
        (
            "designmap.xml",
            &pkg::designmap(
                r#"<idPkg:Preferences src="Resources/Preferences.xml"/>
"#,
            ),
        ),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Resources/Preferences.xml", PREFERENCES),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(BODY)),
    ])
}

#[test]
fn the_default_face_is_stated_and_declared_and_dangling_references_dropped() {
    let src = source();
    let doc = pkg::open(&src);
    let opts = ExportOptions {
        default_face: Some(FontFace::synthesized("Inter", "Regular")),
        ..Default::default()
    };
    let out = write_idml_with(&doc, &src, &opts).expect("write").bytes;

    let prefs = pkg::entry(&out, "Resources/Preferences.xml").expect("preferences part");
    assert!(prefs.contains(r#"<TextDefault FontStyle="Regular" PointSize="12"><Properties><AppliedFont type="string">Inter</AppliedFont><Leading type="enumeration">Auto</Leading></Properties></TextDefault>"#), "{prefs}");

    let fonts = pkg::entry(&out, "Resources/Fonts.xml").expect("fonts part");
    assert!(fonts.contains(r#"FontFamily="Inter" Name="Inter Regular""#), "{fonts}");

    let story = pkg::entry(&out, "Stories/Story_st1.xml").expect("story part");
    assert!(story.contains(r#"<ParagraphStyleRange><CharacterStyleRange><Content>lost styles</Content>"#), "{story}");
    assert!(!story.contains("docx-"), "{story}");

    // Reopened, the export is its own word: a second save changes nothing.
    let doc2 = pkg::open(&out);
    let twice = write_idml_with(&doc2, &out, &opts).expect("write again").bytes;
    pkg::assert_same_package(&out, &twice);
}

#[test]
fn without_a_default_face_only_the_dangling_references_go() {
    let src = source();
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    let prefs = pkg::entry(&out, "Resources/Preferences.xml").expect("preferences part");
    assert!(!prefs.contains("TextDefault"), "{prefs}");
    assert!(prefs.contains(r#"<TextPreference UseOpticalSize="false"/>"#), "{prefs}");
    let story = pkg::entry(&out, "Stories/Story_st1.xml").expect("story part");
    assert!(!story.contains("docx-"), "{story}");
}
