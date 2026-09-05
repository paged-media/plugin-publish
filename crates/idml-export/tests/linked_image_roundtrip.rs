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

//! A LINKED image (`image_link`, no bytes) on a frame whose element has
//! no placed-image child reaches the export as `<Image>` + `<Link
//! LinkResourceURI=…>` and reads back; an existing link is patched in
//! place, and left alone byte-for-byte when it agrees.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const SPREAD_WITH_RECT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
<Rectangle Self="r1" GeometricBounds="450 100 600 300" ItemTransform="1 0 0 1 0 0"/>
</Spread></idPkg:Spread>"#;

fn source(spread: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

#[test]
fn a_link_placed_on_a_source_frame_is_written_and_reads_back() {
    let src = source(SPREAD_WITH_RECT);
    let mut doc = pkg::open(&src);
    let r = &mut doc.spreads[0].spread.rectangles[0];
    assert_eq!(r.self_id.as_deref(), Some("r1"));
    r.image_link = Some("assets/photos/apples.jpg".into());
    r.image_item_transform = Some([0.5, 0.0, 0.0, 0.5, 10.0, 20.0]);

    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let rect = pkg::pos(&xml, "<Rectangle Self=\"r1\"");
    let image = pkg::pos(
        &xml,
        r#"<Image Self="r1Image" ItemTransform="0.5 0 0 0.5 10 20"><Properties><GraphicBounds Left="100" Top="450" Right="300" Bottom="600"/></Properties><Link Self="r1Link" LinkResourceURI="assets/photos/apples.jpg" LinkResourceFormat="$ID/JPEG" StoredState="Normal" LinkClassID="35906" LinkClientID="257""#,
    );
    let end = pkg::pos(&xml, "</Rectangle>");
    assert!(rect < image && image < end, "inside the frame:\n{xml}");

    let re = pkg::open(&out);
    let r = &re.spreads[0].spread.rectangles[0];
    assert_eq!(r.image_link.as_deref(), Some("assets/photos/apples.jpg"));
    assert!(r.has_image_element);
    assert_eq!(
        r.image_item_transform,
        Some([0.5, 0.0, 0.0, 0.5, 10.0, 20.0])
    );
    assert_eq!(
        (r.bounds.top, r.bounds.left, r.bounds.bottom, r.bounds.right),
        (450.0, 100.0, 600.0, 300.0)
    );
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

#[test]
fn an_existing_link_is_patched_in_place_and_kept_when_equal() {
    let spread = SPREAD_WITH_RECT.replace(
        r#"<Rectangle Self="r1" GeometricBounds="450 100 600 300" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<Rectangle Self="r1" GeometricBounds="450 100 600 300" ItemTransform="1 0 0 1 0 0"><Image Self="u7d8" ItemTransform="1 0 0 1 0 0"><Properties><GraphicBounds Left="0" Top="0" Right="200" Bottom="150" /></Properties><Link Self="u7db" AssetURL="$ID/" LinkResourceURI="file:/old/apples.png" LinkResourceFormat="$ID/Portable Network Graphics (PNG)" StoredState="Normal" /></Image></Rectangle>"#,
    );
    let src = source(&spread);
    let doc = pkg::open(&src);
    assert_eq!(
        doc.spreads[0].spread.rectangles[0].image_link.as_deref(),
        Some("file:/old/apples.png")
    );
    assert_eq!(
        src,
        write_idml(&doc, &src).expect("write"),
        "unchanged: byte-identical"
    );

    let mut doc = doc;
    doc.spreads[0].spread.rectangles[0].image_link = Some("file:/new/apples.png".into());
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(xml.contains(r#"<Link Self="u7db" AssetURL="$ID/" LinkResourceURI="file:/new/apples.png" LinkResourceFormat="$ID/Portable Network Graphics (PNG)" StoredState="Normal"/>"#), "patched in place, other attributes kept:\n{xml}");
    assert_eq!(xml.matches("<Image ").count(), 1, "no second image:\n{xml}");
    assert_eq!(
        pkg::open(&out).spreads[0].spread.rectangles[0]
            .image_link
            .as_deref(),
        Some("file:/new/apples.png")
    );
}

#[test]
fn an_inserted_frame_with_a_link_carries_its_image() {
    let src = source(SPREAD_WITH_RECT);
    let mut doc = pkg::open(&src);
    let mut r = doc.spreads[0].spread.rectangles[0].clone();
    r.self_id = Some("r2".into());
    r.image_link = Some("assets/cover.tif".into());
    doc.spreads[0].spread.rectangles.push(r);
    doc.spreads[0]
        .spread
        .frames_in_order
        .push(idml_import::FrameRef::Rectangle(1));
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let rect = pkg::pos(&xml, "<Rectangle Self=\"r2\"");
    let image = pkg::pos(
        &xml,
        r#"<Link Self="r2Link" LinkResourceURI="assets/cover.tif" LinkResourceFormat="$ID/TIFF""#,
    );
    assert!(rect < image, "{xml}");
    let re = pkg::open(&out);
    let r2 = re.spreads[0]
        .spread
        .rectangles
        .iter()
        .find(|r| r.self_id.as_deref() == Some("r2"))
        .expect("r2");
    assert_eq!(r2.image_link.as_deref(), Some("assets/cover.tif"));
}

#[test]
fn a_frame_holding_bytes_and_a_link_still_gets_its_link() {
    let src = source(SPREAD_WITH_RECT);
    let mut doc = pkg::open(&src);
    let r = &mut doc.spreads[0].spread.rectangles[0];
    r.image_link = Some("assets/photos/apples.jpg".into());
    r.image_bytes = Some(vec![1, 2, 3, 4]);
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<Link Self="r1Link" LinkResourceURI="assets/photos/apples.jpg""#),
        "{xml}"
    );
    let re = pkg::open(&out);
    let r = &re.spreads[0].spread.rectangles[0];
    assert_eq!(r.image_link.as_deref(), Some("assets/photos/apples.jpg"));
    assert!(r.image_bytes.is_none(), "IDML carries no pixels");
}
