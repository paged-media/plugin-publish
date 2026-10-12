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

//! Custom stroke styles live in `Resources/Graphic.xml` in InDesign's
//! files; the importer reads them there as well as in Styles.xml.

#[path = "support/pkg.rs"]
mod pkg;

const GRAPHIC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" Name="Black"/>
<StrokeStyle Self="StrokeStyle/$ID/Solid" Name="$ID/Solid"/>
<DashedStrokeStyle Self="DashedStrokeStyle/Morse" DashArray="34.01574803149607 8.503937007874017 8.503937007874017 8.503937007874017" StrokeCornerAdjustment="DashesAndGaps" EndCap="ButtEndCap" Name="Morse" />
</idPkg:Graphic>"#;

#[test]
fn a_dashed_stroke_style_in_the_graphic_part_is_read() {
    let src = pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Resources/Graphic.xml", GRAPHIC),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ]);
    let doc = pkg::open(&src);
    let morse = doc
        .styles
        .stroke_styles
        .get("DashedStrokeStyle/Morse")
        .expect("the Graphic.xml stroke style");
    assert_eq!(morse.name.as_deref(), Some("Morse"));
    // InDesign's own element (exported by InDesign 20.0.1 from a style made
    // with dashArray [12, 3, 3, 3] mm): the dashes are `DashArray`, in pt.
    let pt: Vec<f32> = morse
        .pattern
        .iter()
        .map(|v| (v * 100.0).round() / 100.0)
        .collect();
    assert_eq!(pt, [34.02, 8.5, 8.5, 8.5]);
}
