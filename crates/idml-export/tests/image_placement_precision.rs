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

//! A placed picture comes back from an export exactly where the engine
//! put it.
//!
//! Three losses, all on pictures the model places with no transform of
//! its own (by a named fit, or stretched into the frame):
//!
//! * the transform was written at four decimals — a 2200 px picture at
//!   `0.19636364` came back at `0.1964`, 0.08 pt wider, and the twin of
//!   every photograph differed in a grid of resampled rows and columns;
//! * a TIFF's size was never read, so its `<Image>` kept the frame box
//!   and the identity transform — read back as one point per pixel, the
//!   twin showed the picture's top-left corner at full size: a blank
//!   tile;
//! * a source part an older export wrote kept that transform verbatim
//!   on every later save, whatever the model said.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const SPREAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
<Rectangle Self="r1" GeometricBounds="366 48 654.43 480" ItemTransform="1 0 0 1 0 0"/>
</Spread></idPkg:Spread>"#;

fn source(spread: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

/// A PNG header declaring `w` × `h` — all the export reads of a picture.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 2, 0, 0, 0]);
    b
}

/// A little-endian TIFF whose first directory declares `w` × `h`.
fn tiff(w: u32, h: u32) -> Vec<u8> {
    let mut t = b"II*\0".to_vec();
    t.extend_from_slice(&8u32.to_le_bytes());
    t.extend_from_slice(&2u16.to_le_bytes());
    for (tag, v) in [(256u16, w), (257u16, h)] {
        t.extend_from_slice(&tag.to_le_bytes());
        t.extend_from_slice(&4u16.to_le_bytes());
        t.extend_from_slice(&1u32.to_le_bytes());
        t.extend_from_slice(&v.to_le_bytes());
    }
    t.extend_from_slice(&[0; 4]);
    t
}

fn r1(doc: &paged_scene::Document) -> &idml_import::Rectangle {
    &doc.spreads[0].spread.rectangles[0]
}

fn fill_proportionally() -> idml_import::FrameFittingOption {
    idml_import::FrameFittingOption {
        fitting_on_empty_frame: Some("FillProportionally".into()),
        ..Default::default()
    }
}

/// The engine's own cover fit, to the bit.
#[test]
fn a_named_fit_comes_back_at_full_precision() {
    let src = source(SPREAD);
    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.rectangles[0].image_bytes = Some(png(2200, 1469));
    doc.spreads[0].spread.rectangles[0].frame_fitting = Some(fill_proportionally());
    let want = paged_model::fitting::fit_transform(
        paged_model::fitting::FitMode::FillProportionally,
        None,
        r1(&doc).bounds,
        2200.0,
        1469.0,
    )
    .expect("a cover fit");
    assert_ne!(
        format!("{:.4}", want[0]).parse::<f32>().unwrap(),
        want[0],
        "the case needs a scale four decimals cannot hold"
    );
    let out = write_idml(&doc, &src).expect("write");
    assert_eq!(r1(&pkg::open(&out)).image_item_transform, Some(want));
}

/// A TIFF is measured like the other formats: its picture box, and the
/// transform that stretches it into the frame.
#[test]
fn a_tiff_is_placed_by_its_size() {
    let src = source(SPREAD);
    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.rectangles[0].image_bytes = Some(tiff(800, 534));
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<GraphicBounds Left="0" Top="0" Right="800" Bottom="534"/>"#),
        "the picture's own box:\n{xml}"
    );
    let b = r1(&doc).bounds;
    let t = r1(&pkg::open(&out))
        .image_item_transform
        .expect("a transform");
    assert_eq!(
        t,
        [
            (b.right - b.left) / 800.0,
            0.0,
            0.0,
            (b.bottom - b.top) / 534.0,
            b.left,
            b.top
        ]
    );
}

/// The source part an older export wrote: the identity transform and the
/// frame box for a TIFF it could not measure, four decimals for a fit.
/// Both are brought in line with the model's placement.
#[test]
fn a_placement_an_older_export_wrote_is_corrected() {
    let older = SPREAD.replace(
        r#"<Rectangle Self="r1" GeometricBounds="366 48 654.43 480" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<Rectangle Self="r1" GeometricBounds="366 48 654.43 480" ItemTransform="1 0 0 1 0 0"><Image Self="r1Image" ItemTransform="1 0 0 1 0 0"><Properties><GraphicBounds Left="48" Top="366" Right="480" Bottom="654.43"/></Properties><Link Self="r1Link" LinkResourceURI="r1.tif" StoredState="Normal"/></Image></Rectangle>"#,
    );
    let src = source(&older);
    let mut doc = pkg::open(&src);
    assert_eq!(
        r1(&doc).image_item_transform,
        Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    );
    // The engine's model of that frame: bytes, no transform of its own.
    doc.spreads[0].spread.rectangles[0].image_item_transform = None;
    doc.spreads[0].spread.rectangles[0].image_bytes = Some(tiff(800, 534));
    doc.spreads[0].spread.rectangles[0].image_link = Some("r1.tif".into());
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<GraphicBounds Left="0" Top="0" Right="800" Bottom="534"/>"#),
        "{xml}"
    );
    let t = r1(&pkg::open(&out))
        .image_item_transform
        .expect("a transform");
    assert_eq!(t[0], 432.0 / 800.0);
    assert_eq!((t[4], t[5]), (48.0, 366.0));

    let rounded = SPREAD.replace(
        r#"<Rectangle Self="r1" GeometricBounds="366 48 654.43 480" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<Rectangle Self="r1" GeometricBounds="366 48 654.43 480" ItemTransform="1 0 0 1 0 0"><Image Self="r1Image" ItemTransform="0.1964 0 0 0.1964 48 366"><Properties><GraphicBounds Left="0" Top="0" Right="2200" Bottom="1469"/></Properties><Link Self="r1Link" LinkResourceURI="r1.png" StoredState="Normal"/></Image></Rectangle>"#,
    );
    let src = source(&rounded);
    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.rectangles[0].image_item_transform = None;
    doc.spreads[0].spread.rectangles[0].image_bytes = Some(png(2200, 1469));
    doc.spreads[0].spread.rectangles[0].frame_fitting = Some(fill_proportionally());
    let want = paged_model::fitting::fit_transform(
        paged_model::fitting::FitMode::FillProportionally,
        None,
        r1(&doc).bounds,
        2200.0,
        1469.0,
    );
    let out = write_idml(&doc, &src).expect("write");
    assert_eq!(r1(&pkg::open(&out)).image_item_transform, want);
}

/// A transform the model holds — InDesign's own, at full precision —
/// keeps its bytes, and so does the whole package.
#[test]
fn a_transform_the_model_holds_keeps_its_bytes() {
    let indesign = SPREAD.replace(
        r#"<Rectangle Self="r1" GeometricBounds="366 48 654.43 480" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<Rectangle Self="r1" GeometricBounds="366 48 654.43 480" ItemTransform="1 0 0 1 0 0"><Image Self="u7d8" ItemTransform="0.13835145741383767 0 0 0.13835145741383767 -128.48500711668567 665.7193966347146"><Properties><GraphicBounds Left="0" Top="0" Right="1277" Bottom="1858" /></Properties><Link Self="u7db" LinkResourceURI="file:/Links/photo.png" StoredState="Normal" /></Image></Rectangle>"#,
    );
    let src = source(&indesign);
    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.rectangles[0].image_bytes = Some(png(1277, 1858));
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<Image Self="u7d8" ItemTransform="0.13835145741383767 0 0 0.13835145741383767 -128.48500711668567 665.7193966347146"><Properties><GraphicBounds Left="0" Top="0" Right="1277" Bottom="1858" />"#),
        "{xml}"
    );
    let doc = pkg::open(&src);
    pkg::assert_same_package(&src, &write_idml(&doc, &src).expect("write"));
}
