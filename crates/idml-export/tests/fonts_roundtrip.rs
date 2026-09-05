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

//! `Resources/Fonts.xml` declares every face the document applies —
//! styles through their cascade, run overrides, the host's registered
//! faces — in InDesign's spelling, leaving what it already declares
//! byte-identical.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::{write_idml, write_idml_with, ExportOptions, FontFace};

const STYLES_WITH_FONTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]"/><CharacterStyle Self="CharacterStyle/Emphasis" Name="Emphasis" FontStyle="Italic"><Properties><AppliedFont type="string">EB Garamond</AppliedFont></Properties></CharacterStyle></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]"/><ParagraphStyle Self="ParagraphStyle/Code" Name="Code" FontStyle="Bold"><Properties><AppliedFont type="string">JetBrains Mono</AppliedFont></Properties></ParagraphStyle><ParagraphStyle Self="ParagraphStyle/Code Light" Name="Code Light" FontStyle="Light"><Properties><BasedOn type="object">ParagraphStyle/Code</BasedOn></Properties></ParagraphStyle></RootParagraphStyleGroup>
</idPkg:Styles>"#;

const FONTS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Fonts xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><FontFamily Self="FontFamily/JetBrainsMono" Name="JetBrains Mono"><Font Self="Font/JetBrainsMono" FontFamily="JetBrains Mono" Name="JetBrains Mono" PostScriptName="JetBrainsMono" Status="Installed" FontStyleName="Regular" FontType="TrueType"/></FontFamily></idPkg:Fonts>"#;

const BODY: &str = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Code"><CharacterStyleRange><Content>let x = 1;</Content></CharacterStyleRange><CharacterStyleRange FontStyle="Semibold Italic"><Properties><AppliedFont type="string">Fraunces</AppliedFont></Properties><Content> // note</Content></CharacterStyleRange></ParagraphStyleRange>"#;

fn source(with_fonts: bool) -> Vec<u8> {
    let dm_body = if with_fonts {
        r#"<idPkg:Fonts src="Resources/Fonts.xml"/>
"#
    } else {
        ""
    };
    let mut parts: Vec<(&str, String)> = vec![
        ("designmap.xml", pkg::designmap(dm_body)),
        ("Resources/Styles.xml", STYLES_WITH_FONTS.to_string()),
        ("Spreads/Spread_s1.xml", pkg::SPREAD.to_string()),
        ("Stories/Story_st1.xml", pkg::story(BODY)),
    ];
    if with_fonts {
        parts.insert(1, ("Resources/Fonts.xml", FONTS.to_string()));
    }
    let refs: Vec<(&str, &str)> = parts.iter().map(|(n, b)| (*n, b.as_str())).collect();
    pkg::package(&refs)
}

#[test]
fn every_applied_face_is_declared_as_an_instance_and_family_only_entries_are_rewritten() {
    let src = source(true);
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    let fonts = pkg::entry(&out, "Resources/Fonts.xml").expect("fonts part");
    // The generator's family-only regular entry is REWRITTEN as the
    // instance (InDesign 20.0.1 substitutes the family-only form), its
    // `Self` kept.
    assert!(fonts.contains(r#"<Font Self="Font/JetBrainsMono" FontFamily="JetBrains Mono" Name="JetBrains Mono Regular" PostScriptName="JetBrainsMono-Regular" Status="Installed" FontStyleName="Regular" FontType="TrueType" WritingScript="0" FullName="JetBrains Mono Regular" FullNameNative="JetBrains Mono Regular" FontStyleNameNative="Regular" PlatformName="$ID/" TypekitID="$ID/"/>"#), "{fonts}");
    assert!(
        !fonts.contains(r#"Name="JetBrains Mono" "#),
        "no family-only entry survives:\n{fonts}"
    );
    // The paragraph style's face joins its family, in InDesign's
    // attribute set and order.
    assert!(fonts.contains(r#"<Font Self="FontFamily/JetBrainsMonoFontnJetBrains Mono Bold" FontFamily="JetBrains Mono" Name="JetBrains Mono Bold" PostScriptName="JetBrainsMono-Bold" Status="Installed" FontStyleName="Bold" FontType="TrueType" WritingScript="0" FullName="JetBrains Mono Bold" FullNameNative="JetBrains Mono Bold" FontStyleNameNative="Bold" PlatformName="$ID/" TypekitID="$ID/"/>"#), "{fonts}");
    // A style based on it inherits the family and applies its own style.
    assert!(
        fonts.contains(r#"Name="JetBrains Mono Light" PostScriptName="JetBrainsMono-Light""#),
        "{fonts}"
    );
    // A character style's face; a run-level override's face.
    assert!(fonts.contains(r#"<FontFamily Self="FontFamily/EBGaramond" Name="EB Garamond"><Font Self="FontFamily/EBGaramondFontnEB Garamond Italic" FontFamily="EB Garamond" Name="EB Garamond Italic" PostScriptName="EBGaramond-Italic""#), "{fonts}");
    assert!(
        fonts.contains(
            r#"Name="Fraunces Semibold Italic" PostScriptName="Fraunces-SemiboldItalic""#
        ),
        "{fonts}"
    );
    assert_eq!(fonts.matches("<Font ").count(), 5, "{fonts}");
    // Only the fonts part changed.
    for name in pkg::names(&src) {
        if name != "Resources/Fonts.xml" {
            assert_eq!(pkg::entry(&src, &name), pkg::entry(&out, &name), "{name}");
        }
    }
    // Stable: a second export of the re-read package changes nothing.
    let re = pkg::open(&out);
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

#[test]
fn a_host_face_derived_from_bytes_names_the_instance_with_its_axes() {
    let src = source(true);
    let doc = pkg::open(&src);
    let known = vec![
        FontFace {
            family: "JetBrains Mono".into(),
            style: "Regular".into(),
            postscript_name: "JetBrainsMonoRoman-Regular".into(),
            full_name: "JetBrains Mono Regular".into(),
            font_type: "OpenTypeTT".into(),
            version: Some("Version 2.211".into()),
            axes: vec![("Weight".into(), 400.0)],
        },
        FontFace {
            family: "JetBrains Mono".into(),
            style: "Bold".into(),
            postscript_name: "JetBrainsMonoRoman-Bold".into(),
            full_name: "JetBrains Mono Bold".into(),
            font_type: "OpenTypeTT".into(),
            version: Some("Version 2.211".into()),
            axes: vec![("Weight".into(), 700.0)],
        },
        // Registered but applied nowhere: not declared.
        FontFace::synthesized("Space Grotesk", "Medium"),
    ];
    let out = write_idml_with(
        &doc,
        &src,
        &ExportOptions {
            fonts: known,
            ..Default::default()
        },
    )
    .expect("write");
    let fonts = pkg::entry(&out.bytes, "Resources/Fonts.xml").expect("fonts part");
    // The family-only regular entry is rewritten from the bytes, with
    // the `Roman` infix only the file's fvar records carry.
    assert!(fonts.contains(r#"<Font Self="Font/JetBrainsMono" FontFamily="JetBrains Mono" Name="JetBrains Mono Regular" PostScriptName="JetBrainsMonoRoman-Regular" Status="Installed" FontStyleName="Regular" FontType="OpenTypeTT" WritingScript="0" FullName="JetBrains Mono Regular" FullNameNative="JetBrains Mono Regular" FontStyleNameNative="Regular" PlatformName="$ID/" Version="Version 2.211" TypekitID="$ID/" NumDesignAxes="1" DesignAxesName="Weight" DesignAxesValues="400"/>"#), "{fonts}");
    assert!(fonts.contains(r#"Name="JetBrains Mono Bold" PostScriptName="JetBrainsMonoRoman-Bold" Status="Installed" FontStyleName="Bold" FontType="OpenTypeTT" WritingScript="0" FullName="JetBrains Mono Bold" FullNameNative="JetBrains Mono Bold" FontStyleNameNative="Bold" PlatformName="$ID/" Version="Version 2.211" TypekitID="$ID/" NumDesignAxes="1" DesignAxesName="Weight" DesignAxesValues="700"/>"#), "{fonts}");
    assert!(!fonts.contains("Space Grotesk"), "{fonts}");
    // Stable on re-export with the same registry.
    let re = pkg::open(&out.bytes);
    let again = write_idml_with(
        &re,
        &out.bytes,
        &ExportOptions {
            fonts: vec![],
            ..Default::default()
        },
    )
    .expect("re-write");
    pkg::assert_same_package(&out.bytes, &again.bytes);
}

#[test]
fn a_package_without_a_fonts_part_gets_one_referenced_from_the_designmap() {
    let src = source(false);
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    let fonts = pkg::entry(&out, "Resources/Fonts.xml").expect("minted fonts part");
    assert!(fonts.starts_with(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Fonts xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><FontFamily Self="FontFamily/EBGaramond" Name="EB Garamond">"#), "{fonts}");
    assert_eq!(fonts.matches("<Font ").count(), 4, "{fonts}");
    let dm = pkg::entry(&out, "designmap.xml").unwrap();
    let fonts_ref = pkg::pos(&dm, r#"<idPkg:Fonts src="Resources/Fonts.xml"/>"#);
    let spread_ref = pkg::pos(&dm, "<idPkg:Spread ");
    assert!(fonts_ref < spread_ref, "before the spreads:\n{dm}");
    let re = pkg::open(&out);
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

#[test]
fn a_document_applying_no_face_leaves_a_fontless_package_alone() {
    let src = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    assert_eq!(out, src);
    assert!(pkg::entry(&out, "Resources/Fonts.xml").is_none());
}
