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

//! The document's inks: InDesign writes one `<Ink>` per process and spot
//! ink in `Resources/Graphic.xml`. They are read into the palette, pass
//! through byte-identical when the part is rewritten for another reason,
//! and a package whose part lacks them gains the model's.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

/// InDesign's spelling (sample-3.idml's process inks, attribute for
/// attribute, ` />` included) plus a spot ink.
const INKS: [&str; 5] = [
    r#"<Ink Self="Ink/$ID/Process Cyan" Name="$ID/Process Cyan" Angle="71.5651" ConvertToProcess="false" Frequency="63.2456" NeutralDensity="0.61" PrintInk="true" TrapOrder="1" InkType="Normal" />"#,
    r#"<Ink Self="Ink/$ID/Process Magenta" Name="$ID/Process Magenta" Angle="18.4349" ConvertToProcess="false" Frequency="63.2456" NeutralDensity="0.76" PrintInk="true" TrapOrder="2" InkType="Normal" />"#,
    r#"<Ink Self="Ink/$ID/Process Yellow" Name="$ID/Process Yellow" Angle="0" ConvertToProcess="false" Frequency="66.6667" NeutralDensity="0.16" PrintInk="true" TrapOrder="3" InkType="Normal" />"#,
    r#"<Ink Self="Ink/$ID/Process Black" Name="$ID/Process Black" Angle="45" ConvertToProcess="false" Frequency="70.7107" NeutralDensity="1.7" PrintInk="true" TrapOrder="4" InkType="Normal" />"#,
    r#"<Ink Self="Ink/PANTONE 286 C" Name="PANTONE 286 C" Angle="45" ConvertToProcess="true" Frequency="70" NeutralDensity="1.0532" PrintInk="true" TrapOrder="5" InkType="Normal" />"#,
];

fn graphic(inks: &[&str]) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<idPkg:Graphic xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\" DOMVersion=\"20.0\">\n\
\t<Color Self=\"Color/Black\" Model=\"Process\" Space=\"CMYK\" ColorValue=\"0 0 0 100\" Name=\"Black\" />\n\
{}\
\t<Swatch Self=\"Swatch/None\" Name=\"None\" />\n\
</idPkg:Graphic>",
        inks.iter().map(|i| format!("\t{i}\n")).collect::<String>()
    )
}

fn source(graphic: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Resources/Graphic.xml", graphic),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

fn graphic_part(package: &[u8]) -> String {
    pkg::entry(package, "Resources/Graphic.xml").expect("graphic part")
}

#[test]
fn the_inks_are_read_in_document_order() {
    let doc = pkg::open(&source(&graphic(&INKS)));
    let inks = &doc.palette.inks;
    let ids: Vec<&str> = inks.iter().map(|i| i.self_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "Ink/$ID/Process Cyan",
            "Ink/$ID/Process Magenta",
            "Ink/$ID/Process Yellow",
            "Ink/$ID/Process Black",
            "Ink/PANTONE 286 C",
        ]
    );
    assert_eq!(inks[0].angle, Some(71.5651));
    assert_eq!(inks[0].frequency, Some(63.2456));
    assert_eq!(inks[3].neutral_density, Some(1.7));
    assert_eq!(inks[4].convert_to_process, Some(true));
    assert_eq!(inks[4].trap_order, Some(5));
    assert_eq!(inks[4].ink_type.as_deref(), Some("Normal"));
}

#[test]
fn an_unchanged_palette_leaves_the_part_byte_identical() {
    let src = source(&graphic(&INKS));
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    assert_eq!(graphic_part(&out), graphic_part(&src));
}

/// A new swatch rewrites the part; every `<Ink>` passes through as it was.
#[test]
fn the_inks_pass_through_a_rewritten_part_byte_identically() {
    let src = source(&graphic(&INKS));
    let mut doc = pkg::open(&src);
    let mut red = doc.palette.colors["Color/Black"].clone();
    red.self_id = "Color/Red".into();
    red.name = Some("Red".into());
    red.value = vec![0.0, 100.0, 100.0, 0.0];
    doc.palette.colors.insert(red.self_id.clone(), red);
    let out = write_idml(&doc, &src).expect("write");
    let part = graphic_part(&out);
    assert!(part.contains(r#"Self="Color/Red""#), "rewritten: {part}");
    for ink in INKS {
        assert!(part.contains(ink), "{ink} kept verbatim in:\n{part}");
        assert_eq!(part.matches(ink).count(), 1, "and not written twice");
    }
    assert_eq!(pkg::open(&out).palette.inks, doc.palette.inks);
}

/// The model's inks reach a package whose part does not declare them,
/// and read back.
#[test]
fn a_part_without_the_inks_gains_the_model_s() {
    let with = pkg::open(&source(&graphic(&INKS)));
    let src = source(&graphic(&INKS[..2]));
    let mut doc = pkg::open(&src);
    doc.palette.inks = with.palette.inks.clone();
    let out = write_idml(&doc, &src).expect("write");
    let part = graphic_part(&out);
    for ink in &INKS[..2] {
        assert_eq!(part.matches(ink).count(), 1, "{ink} kept, once:\n{part}");
    }
    assert!(
        part.contains(r#"<Ink Self="Ink/$ID/Process Black" Name="$ID/Process Black" Angle="45" ConvertToProcess="false" Frequency="70.7107" NeutralDensity="1.7" PrintInk="true" TrapOrder="4" InkType="Normal"/>"#),
        "{part}"
    );
    assert_eq!(pkg::open(&out).palette.inks, with.palette.inks);
}
