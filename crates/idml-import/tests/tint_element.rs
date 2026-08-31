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

//! A named tint swatch is a `<Tint>` element, and we must read it.
//!
//! # The measurement came first
//!
//! A 134-page document was exported from the editor, opened in real
//! InDesign 2026 and re-exported. Every panel painted "Vermilion 20%"
//! came back FULL STRENGTH — red where the document says pink. The
//! swatch had been written as
//!
//! ```text
//! <Color … ColorValue="0 85 90 5" Name="Vermilion 20%" TintValue="20"/>
//! ```
//!
//! and InDesign's own re-export of it reads
//!
//! ```text
//! <Color … ColorValue="0 85 90 5" … Name="Vermilion 20%" … />
//! ```
//!
//! — the `TintValue` simply gone. A `TintValue` attribute on a
//! `<Color>` is our private convention: `ColorEntry::effective_cmyk`
//! multiplies it into the channels, so the document looks right in
//! every paged surface and round-trips through us perfectly. Adobe
//! discards it.
//!
//! The interchange spelling is a `<Tint>` element carrying `BaseColor`
//! and `TintValue`. We did not parse that element at all, which is the
//! same defect pointing the other way: an InDesign file using named
//! tints lost every one of them on the way in.

use idml_import::graphic::parse_graphic;

const GRAPHIC: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
  <Color Self="Color/Vermilion" Model="Spot" Space="CMYK" ColorValue="0 85 90 5"
         Name="Vermilion" AlternateSpace="CMYK" AlternateColorValue="0 85 90 5"/>
  <Tint Self="Color/Vermilion20" Name="Vermilion 20%" BaseColor="Color/Vermilion"
        TintValue="20"/>
</idPkg:Graphic>"#;

#[test]
fn a_tint_element_is_read_and_resolves_through_its_base() {
    let g = parse_graphic(GRAPHIC).expect("parse Graphic.xml");

    let tint = g
        .colors
        .get("Color/Vermilion20")
        .expect("the <Tint> element must land in the swatch table — a file \
                 using named tints must not lose them on the way in");
    assert_eq!(tint.tint, Some(20.0), "the tint value is read");

    // The channels come from the base, and the spot MODEL comes with
    // them: a tint of a spot is still that ink, and must separate onto
    // its plate rather than flattening to process.
    let cmyk = tint
        .effective_cmyk()
        .expect("a spot tint resolves through its CMYK alternate");
    let expect = [0.0, 85.0 * 0.2, 90.0 * 0.2, 5.0 * 0.2];
    for (got, want) in cmyk.iter().zip(expect.iter()) {
        assert!(
            (got - want).abs() < 0.001,
            "20% of the base ink, channel-wise: got {cmyk:?}, want {expect:?}"
        );
    }
}

#[test]
fn the_legacy_color_tintvalue_spelling_still_reads() {
    // Documents we already wrote carry the old private spelling. They
    // must keep opening exactly as before — the fix is about what we
    // WRITE, and dropping the read would strand every existing file.
    const LEGACY: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Graphic xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
  <Color Self="Color/V20" Model="Process" Space="CMYK" ColorValue="0 85 90 5"
         Name="Vermilion 20%" TintValue="20"/>
</idPkg:Graphic>"#;
    let g = parse_graphic(LEGACY).expect("parse legacy Graphic.xml");
    let c = g.colors.get("Color/V20").expect("legacy swatch still read");
    assert_eq!(c.tint, Some(20.0));
    let cmyk = c.effective_cmyk().expect("process CMYK resolves");
    assert!((cmyk[1] - 17.0).abs() < 0.001, "still 20% of 85: {cmyk:?}");
}
