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

//! The drift guard + the linkage contract: an IR → `.paged` OCF → decode the
//! pgm back → assert the model reconstructs with editable text whose frame is
//! linked to its story, and inline-image rectangles. If the `paged-store`
//! serde shape ever drifts from this crate's pinned core rev, `from_bytes`
//! returns `None` and this test fails loudly rather than the editor silently
//! opening an empty document.

use std::io::Read;

use pdf_import::pdf_ir_to_paged;

// A real 1×1 PNG (any base64 works — `build` only base64-decodes + stores).
const PNG_1X1_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

fn sample_ir_json() -> String {
    format!(
        r#"{{
      "pages": [
        {{
          "width_pt": 612.0, "height_pt": 792.0,
          "frames": [
            {{ "kind": "text", "x_pt": 72.0, "y_pt": 72.0, "width_pt": 468.0, "height_pt": 200.0,
               "paragraphs": [
                 {{ "runs": [
                    {{ "text": "Hello ", "font_size_pt": 18.0, "font_family": "Helvetica", "bold": true }},
                    {{ "text": "world",  "font_size_pt": 18.0, "font_family": "Helvetica", "italic": true }}
                 ] }},
                 {{ "runs": [ {{ "text": "Second paragraph.", "font_size_pt": 12.0 }} ] }}
               ] }}
          ]
        }},
        {{
          "width_pt": 612.0, "height_pt": 792.0,
          "background_png_b64": "{png}",
          "frames": [
            {{ "kind": "image", "x_pt": 100.0, "y_pt": 100.0, "width_pt": 200.0, "height_pt": 150.0,
               "png_b64": "{png}" }}
          ]
        }}
      ]
    }}"#,
        png = PNG_1X1_B64
    )
}

/// Read one ZIP entry's bytes.
fn zip_entry(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut f = zip.by_name(name).ok()?;
    let mut out = Vec::new();
    f.read_to_end(&mut out).ok()?;
    Some(out)
}

#[test]
fn ir_maps_to_paged_ocf_with_editable_text_and_images() {
    let ocf = pdf_ir_to_paged(&sample_ir_json()).expect("map IR → .paged");

    // --- container shape the load sniff requires ---
    // mimetype must be the FIRST entry and STORED.
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&ocf)).unwrap();
    {
        let first = zip.by_index(0).unwrap();
        assert_eq!(first.name(), "mimetype", "mimetype must be first");
        assert_eq!(
            first.compression(),
            zip::CompressionMethod::Stored,
            "mimetype must be STORED"
        );
    }
    let mimetype = zip_entry(&ocf, "mimetype").unwrap();
    assert_eq!(mimetype, b"application/vnd.adobe.indesign-idml-package");
    assert!(
        zip_entry(&ocf, "designmap.xml").is_some(),
        "sniff needs designmap.xml"
    );

    // --- the native model part is what the load path actually uses ---
    let pgm = zip_entry(&ocf, paged_store::DOCUMENT_PGM_PATH).expect("pgm part present");
    let doc = paged_store::from_bytes(&pgm)
        .expect("pgm decodes — if this is None the pinned core rev drifted");

    // Two pages → two spreads, each one page.
    assert_eq!(doc.spreads.len(), 2);
    assert_eq!(doc.spreads[0].spread.pages.len(), 1);
    assert_eq!(doc.spreads[1].spread.pages.len(), 1);

    // --- page 1: an editable text frame linked to its story ---
    let s0 = &doc.spreads[0].spread;
    assert_eq!(s0.text_frames.len(), 1);
    assert!(s0.rectangles.is_empty(), "page 1 has no image");
    let frame = &s0.text_frames[0];
    let story_id = frame.parent_story.clone().expect("frame links a story");

    // The linkage contract: rebuild_indexes keyed frame_for_story on the
    // story id — if this resolves, the text will flow into the frame.
    assert!(
        doc.frame_for_story.contains_key(&story_id),
        "frame↔story link must resolve"
    );

    let story = doc
        .stories
        .iter()
        .find(|s| s.self_id == story_id)
        .expect("story present");
    assert_eq!(story.story.paragraphs.len(), 2);
    let p0 = &story.story.paragraphs[0];
    assert_eq!(p0.runs.len(), 2);
    assert_eq!(p0.runs[0].text, "Hello ");
    assert_eq!(p0.runs[0].font_style.as_deref(), Some("Bold"));
    assert_eq!(p0.runs[0].point_size, Some(18.0));
    assert_eq!(p0.runs[1].text, "world");
    assert_eq!(p0.runs[1].font_style.as_deref(), Some("Italic"));
    assert_eq!(
        doc.stories[0].story.paragraphs[1].runs[0].text,
        "Second paragraph."
    );

    // --- page 2: a full-page raster background + an image frame ---
    let s1 = &doc.spreads[1].spread;
    assert_eq!(s1.text_frames.len(), 0);
    assert_eq!(s1.rectangles.len(), 2, "background + placed image");
    for r in &s1.rectangles {
        assert!(r.image_bytes.is_some(), "inline image bytes present");
        assert!(r.has_image_element, "flagged as an image frame");
    }
    // Draw order recorded across kinds.
    assert_eq!(s1.frames_in_order.len(), 2);
}

#[test]
fn vector_maps_to_polygon_with_swatch() {
    // A single filled rectangle (red) + a stroked open line (blue) on one page.
    let json = r#"{
      "pages": [{
        "width_pt": 612.0, "height_pt": 792.0,
        "frames": [
          { "kind": "vector",
            "subpaths": [{ "points": [
              {"x_pt":10.0,"y_pt":10.0},{"x_pt":100.0,"y_pt":10.0},
              {"x_pt":100.0,"y_pt":50.0},{"x_pt":10.0,"y_pt":50.0}], "closed": true }],
            "fill_rgb": [1.0, 0.0, 0.0] },
          { "kind": "vector",
            "subpaths": [{ "points": [{"x_pt":10.0,"y_pt":70.0},{"x_pt":200.0,"y_pt":70.0}], "closed": false }],
            "stroke_rgb": [0.0, 0.0, 1.0], "stroke_width_pt": 2.0 }
        ]
      }]
    }"#;
    let ir: pdf_import::ir::DocumentIr = serde_json::from_str(json).expect("parse IR");
    let doc = pdf_import::build::build_document(&ir).expect("build");
    let s = &doc.spreads[0].spread;
    assert_eq!(s.polygons.len(), 2, "two vector shapes → two polygons");

    // The filled rect: a red swatch fill, closed single contour (no explicit
    // starts/opens), 4 anchors.
    let rect = &s.polygons[0];
    assert_eq!(rect.fill_color.as_deref(), Some("Color/pdf_255_0_0"));
    assert!(rect.stroke_color.is_none());
    assert_eq!(rect.anchors.len(), 4);
    assert!(rect.subpath_starts.is_empty());
    assert!(rect.subpath_open.is_empty(), "single closed contour");

    // The open line: a blue stroke, single OPEN contour keeps `[true]`.
    let line = &s.polygons[1];
    assert_eq!(line.stroke_color.as_deref(), Some("Color/pdf_0_0_255"));
    assert_eq!(line.stroke_weight, Some(2.0));
    assert_eq!(line.subpath_open, vec![true]);

    // Both colours registered as RGB swatches (0..255).
    let red = doc
        .palette
        .colors
        .get("Color/pdf_255_0_0")
        .expect("red swatch");
    assert_eq!(red.value, vec![255.0, 0.0, 0.0]);
    assert!(doc.palette.colors.contains_key("Color/pdf_0_0_255"));
}

#[test]
fn pgm_round_trips_are_stable() {
    // Drift guard proper: build once, serialize, deserialize, and confirm the
    // primary fields survive (the #[serde(skip)] caches are rebuilt, so we
    // compare on spreads/stories, not the indexes).
    let ir: pdf_import::ir::DocumentIr = serde_json::from_str(&sample_ir_json()).expect("parse IR");
    let doc = pdf_import::build::build_document(&ir).expect("build");
    let bytes = paged_store::to_bytes(&doc).expect("to_bytes");
    let back = paged_store::from_bytes(&bytes).expect("from_bytes round-trips");
    assert_eq!(back.spreads.len(), doc.spreads.len());
    assert_eq!(back.stories.len(), doc.stories.len());
    assert_eq!(back.stories[0].story.paragraphs[0].runs[0].text, "Hello ");
}
