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

//! A picture frame's `<FrameFittingOption>` survives an export, its
//! crops in the units each side measures them in.
//!
//! InDesign's crop is the distance from the picture's edge to the
//! frame's, in the picture's own units, positive where the frame cuts
//! into it; the model's is the frame-space inset from the frame's edge to
//! the picture's. The reference below is an InDesign-authored package's
//! letterboxed `Proportionally` frame: 250.3 pt wide around a picture
//! 1277 units wide at scale 0.13835, carrying `LeftCrop="-266.07743371484"`
//! — its 36.81 pt band over the scale.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{FrameFittingOption, FrameRef};

/// The InDesign frame's own numbers: its path, its fitting option and
/// its picture's transform and box.
const INDESIGN_FRAME: &str = r#"<Rectangle Self="u7cc" ContentType="GraphicType" ItemTransform="1 0 0 1 -417.2787401557688 -700.0913385790545"><Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="-141.99685039313124 360.6425196844698" LeftDirection="-141.99685039313124 360.6425196844698" RightDirection="-141.99685039313124 360.6425196844698" /><PathPointType Anchor="-141.99685039313124 617.6995275593802" LeftDirection="-141.99685039313124 617.6995275593802" RightDirection="-141.99685039313124 617.6995275593802" /><PathPointType Anchor="108.30236220310329 617.6995275593802" LeftDirection="108.30236220310329 617.6995275593802" RightDirection="108.30236220310329 617.6995275593802" /><PathPointType Anchor="108.30236220310329 360.6425196844698" LeftDirection="108.30236220310329 360.6425196844698" RightDirection="108.30236220310329 360.6425196844698" /></PathPointArray></GeometryPathType></PathGeometry></Properties><FrameFittingOption LeftCrop="-266.07743371484" RightCrop="-266.07743371484" FittingOnEmptyFrame="Proportionally" /><Image Self="u7d8" ItemTransform="0.13835145741383767 0 0 0.13835145741383764 -105.18464965374933 360.6425196844698"><Properties><GraphicBounds Left="0" Top="0" Right="1277" Bottom="1858" /></Properties><Link Self="u7db" LinkResourceURI="file:/Links/photo.jpg" StoredState="Normal" /></Image></Rectangle>"#;

const LEFT_CROP: f64 = -266.07743371484;
const SCALE: f64 = 0.13835145741383767;
const FRAME_LEFT: f64 = -141.99685039313124;
const IMAGE_LEFT: f64 = -105.18464965374933;

fn source(item: &str) -> Vec<u8> {
    let spread = pkg::SPREAD.replace("</Spread>", &format!("{item}\n</Spread>"));
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

fn rect<'a>(doc: &'a paged_scene::Document, id: &str) -> &'a idml_import::Rectangle {
    doc.spreads[0]
        .spread
        .rectangles
        .iter()
        .find(|r| r.self_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("rectangle {id}"))
}

fn option_tag<'a>(xml: &'a str, after: &str) -> &'a str {
    let at = pkg::pos(xml, after);
    let open = at + pkg::pos(&xml[at..], "<FrameFittingOption");
    let close = open + xml[open..].find("/>").expect("tag end") + 2;
    &xml[open..close]
}

fn attr(tag: &str, k: &str) -> f64 {
    let at = pkg::pos(tag, &format!(r#" {k}=""#)) + k.len() + 3;
    tag[at..at + tag[at..].find('"').unwrap()].parse().unwrap()
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * a.abs().max(b.abs()).max(1.0)
}

/// Reading the InDesign frame gives the model the band between the frame
/// and the picture, in points, positive: the picture's edge sits 36.81 pt
/// inside the frame's — measured off the file's own path and transform.
#[test]
fn indesign_crops_read_as_the_frame_space_band() {
    let doc = pkg::open(&source(INDESIGN_FRAME));
    let ff = rect(&doc, "u7cc").frame_fitting.as_ref().expect("fitting");
    let band = IMAGE_LEFT - FRAME_LEFT;
    assert!(
        close(band, -LEFT_CROP * SCALE, 1e-6),
        "the file agrees with itself"
    );
    assert!(
        close(ff.left_crop.unwrap() as f64, band, 1e-5),
        "{ff:?} vs {band}"
    );
    assert!(close(ff.right_crop.unwrap() as f64, band, 1e-4), "{ff:?}");
    assert_eq!(ff.top_crop, None);
    assert_eq!(ff.fitting_on_empty_frame.as_deref(), Some("Proportionally"));
}

/// The same frame, unedited, keeps its option byte for byte.
#[test]
fn an_indesign_frame_saves_back_byte_identical() {
    let src = source(INDESIGN_FRAME);
    let doc = pkg::open(&src);
    pkg::assert_same_package(&src, &write_idml(&doc, &src).expect("write"));
}

/// A frame the export mints with that placement is written with
/// InDesign's own crop for it.
#[test]
fn a_minted_frame_carries_indesigns_number_for_the_same_placement() {
    let src = source(INDESIGN_FRAME);
    let mut doc = pkg::open(&src);
    {
        let spread = &mut doc.spreads[0].spread;
        let mut r = rect_mut(spread, "u7cc").clone();
        r.self_id = Some("minted".into());
        spread.rectangles.push(r);
        let idx = spread.rectangles.len() - 1;
        spread.frames_in_order.push(FrameRef::Rectangle(idx));
    }
    doc.rebuild_indexes();
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let tag = option_tag(&xml, r#"Self="minted""#);
    assert!(close(attr(tag, "LeftCrop"), LEFT_CROP, 1e-5), "{tag}");
    assert!(close(attr(tag, "RightCrop"), LEFT_CROP, 1e-4), "{tag}");
    assert!(
        tag.contains(r#"FittingOnEmptyFrame="Proportionally""#),
        "{tag}"
    );
    let back = pkg::open(&out);
    let (a, b) = (
        rect(&back, "minted").frame_fitting.clone().unwrap(),
        rect(&doc, "minted").frame_fitting.clone().unwrap(),
    );
    assert!(close(
        a.left_crop.unwrap() as f64,
        b.left_crop.unwrap() as f64,
        1e-6
    ));
}

fn rect_mut<'a>(spread: &'a mut idml_import::Spread, id: &str) -> &'a mut idml_import::Rectangle {
    spread
        .rectangles
        .iter_mut()
        .find(|r| r.self_id.as_deref() == Some(id))
        .unwrap()
}

/// A PNG header declaring `w` × `h`.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 2, 0, 0, 0]);
    b
}

/// The engine's cover: a picture placed through the authoring doors, its
/// box grown past the frame by negative model crops, no transform of its
/// own. InDesign gets a positive crop (the frame cuts into the picture)
/// in picture units, and the reader takes it back to the model's.
#[test]
fn an_engine_cover_is_written_as_a_cropped_picture_and_reads_back() {
    let src = source(
        r#"<Rectangle Self="r1" GeometricBounds="150 60 300 250" ItemTransform="1 0 0 1 0 0"/>"#,
    );
    let mut doc = pkg::open(&src);
    {
        let r = rect_mut(&mut doc.spreads[0].spread, "r1");
        r.image_bytes = Some(png(2400, 1543));
        r.frame_fitting = Some(FrameFittingOption {
            left_crop: Some(-21.655865),
            top_crop: Some(0.0),
            right_crop: Some(-21.655865),
            bottom_crop: Some(0.0),
            fitting_on_empty_frame: Some("FillProportionally".into()),
            ..Default::default()
        });
    }
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let tag = option_tag(&xml, r#"Self="r1""#);
    // The box is 190 + 2 × 21.66 pt over 2400 px.
    let scale = (190.0 + 2.0 * 21.655865) / 2400.0;
    assert!(
        close(attr(tag, "LeftCrop"), 21.655865 / scale, 1e-5),
        "{tag}"
    );
    assert!(
        pkg::pos(&xml, "<FrameFittingOption") < pkg::pos(&xml, "<Image "),
        "the option precedes the picture, in InDesign's order:\n{xml}"
    );
    let back = pkg::open(&out);
    let ff = rect(&back, "r1").frame_fitting.clone().expect("fitting");
    assert!(
        close(ff.left_crop.unwrap() as f64, -21.655865, 1e-5),
        "{ff:?}"
    );
    assert!(
        close(ff.right_crop.unwrap() as f64, -21.655865, 1e-5),
        "{ff:?}"
    );
    assert_eq!(ff.top_crop, Some(0.0));
    assert_eq!(
        ff.fitting_on_empty_frame.as_deref(),
        Some("FillProportionally")
    );
    // A second save of what was written changes nothing.
    pkg::assert_same_package(&out, &write_idml(&back, &out).expect("again"));
}

/// The fit, its reference point and auto-fit on a frame with no picture;
/// with no picture transform the crops convert by nothing on either side.
#[test]
fn fitting_options_on_an_empty_frame_round_trip() {
    let src = source(
        r#"<Rectangle Self="r1" GeometricBounds="150 60 300 250" ItemTransform="1 0 0 1 0 0"/>"#,
    );
    let mut doc = pkg::open(&src);
    rect_mut(&mut doc.spreads[0].spread, "r1").frame_fitting = Some(FrameFittingOption {
        left_crop: Some(4.0),
        fitting_on_empty_frame: Some("FitContentToFrame".into()),
        reference_point: Some("BottomRightPoint".into()),
        auto_fit: Some(true),
        ..Default::default()
    });
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<FrameFittingOption AutoFit="true" LeftCrop="4" FittingOnEmptyFrame="FitContentToFrame" FittingAlignment="BottomRightPoint"/>"#),
        "{xml}"
    );
    let back = pkg::open(&out);
    let ff = rect(&back, "r1").frame_fitting.clone().expect("fitting");
    assert_eq!(ff.left_crop, Some(4.0));
    assert_eq!(ff.reference_point.as_deref(), Some("BottomRightPoint"));
    assert_eq!(ff.auto_fit, Some(true));
}
