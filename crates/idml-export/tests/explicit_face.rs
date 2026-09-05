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

//! A paragraph style or run that names an `AppliedFont` without a
//! `FontStyle` is instanced by point size in InDesign 20.0.1 (measured
//! 2026-09-05: `Optical size-9.500`, `12pt` — SUBSTITUTED for a variable
//! font with an optical-size axis, NOT_AVAILABLE for a static face), so
//! the export spells the face the engine composed with; and the part
//! that states the optical-size preference the engine composes under.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]"/><CharacterStyle Self="CharacterStyle/Code Inline" Name="Code Inline"><Properties><AppliedFont type="string">JetBrains Mono</AppliedFont></Properties></CharacterStyle></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]"/><ParagraphStyle Self="ParagraphStyle/Body" Name="Body" PointSize="9.5"><Properties><AppliedFont type="string">Source Serif 4</AppliedFont></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Deck" Name="Deck" FontStyle="Italic" PointSize="14"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Deck Small" Name="Deck Small" PointSize="11"><Properties><BasedOn type="object">ParagraphStyle/Deck</BasedOn><AppliedFont type="string">EB Garamond</AppliedFont></Properties></ParagraphStyle></RootParagraphStyleGroup>
</idPkg:Styles>"#;

const BODY: &str = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange><Content>body </Content></CharacterStyleRange><CharacterStyleRange PointSize="8"><Properties><AppliedFont type="string">Noto Sans Hebrew</AppliedFont></Properties><Content>hebrew</Content></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Deck"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Code Inline"><Content>code</Content></CharacterStyleRange><CharacterStyleRange><Properties><AppliedFont type="string">Space Grotesk</AppliedFont></Properties><Content>deck</Content></CharacterStyleRange></ParagraphStyleRange>"#;

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
        ("Resources/Styles.xml", STYLES),
        ("Resources/Preferences.xml", PREFERENCES),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(BODY)),
    ])
}

#[test]
fn every_carrier_naming_a_font_spells_the_face_the_engine_composed_with() {
    let src = source();
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");

    let styles = pkg::entry(&out, "Resources/Styles.xml").expect("styles part");
    // Body names a font with nothing in its cascade → Regular; Deck Small
    // inherits Deck's Italic through BasedOn; Deck already spells it;
    // the root names no font. Character styles are left alone — a run
    // inherits its face through them correctly (measured).
    assert!(styles.contains(r#"<ParagraphStyle Self="ParagraphStyle/Body" Name="Body" PointSize="9.5" FontStyle="Regular">"#), "{styles}");
    assert!(styles.contains(r#"<ParagraphStyle Self="ParagraphStyle/Deck Small" Name="Deck Small" PointSize="11" FontStyle="Italic">"#), "{styles}");
    assert!(styles.contains(r#"<ParagraphStyle Self="ParagraphStyle/Deck" Name="Deck" FontStyle="Italic" PointSize="14">"#), "{styles}");
    assert!(styles.contains(r#"<ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]"/>"#), "{styles}");
    assert!(
        styles.contains(
            r#"<CharacterStyle Self="CharacterStyle/Code Inline" Name="Code Inline"><Properties>"#
        ),
        "{styles}"
    );

    let story = pkg::entry(&out, "Stories/Story_st1.xml").expect("story part");
    // The Hebrew run in a Body paragraph → Regular; the Space Grotesk
    // run in a Deck paragraph → Italic (the engine's cascade); runs
    // naming no font are untouched.
    assert!(story.contains(r#"<CharacterStyleRange PointSize="8" FontStyle="Regular"><Properties><AppliedFont type="string">Noto Sans Hebrew</AppliedFont></Properties><Content>hebrew</Content>"#), "{story}");
    assert!(story.contains(r#"<CharacterStyleRange FontStyle="Italic"><Properties><AppliedFont type="string">Space Grotesk</AppliedFont></Properties><Content>deck</Content>"#), "{story}");
    assert!(
        story.contains(r#"<CharacterStyleRange><Content>body </Content></CharacterStyleRange>"#),
        "{story}"
    );
    assert!(story.contains(r#"<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/Code Inline"><Content>code</Content>"#), "{story}");

    // The empty preferences part states the composition preference.
    let prefs = pkg::entry(&out, "Resources/Preferences.xml").expect("preferences part");
    assert!(prefs.contains(r#"<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><TextPreference UseOpticalSize="false"/></idPkg:Preferences>"#), "{prefs}");
}

#[test]
fn a_source_that_spells_every_face_round_trips_byte_identically() {
    let src = source();
    let doc = pkg::open(&src);
    let once = write_idml(&doc, &src).expect("write");
    // Re-open what was written: every carrier now spells its face and
    // the preference is stated — nothing left to change.
    let doc2 = pkg::open(&once);
    let twice = write_idml(&doc2, &once).expect("write again");
    pkg::assert_same_package(&once, &twice);
}
