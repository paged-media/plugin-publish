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

//! Document IR → `paged_scene::Document`.
//!
//! One IR page becomes one spread with one page. Each page frame becomes a
//! model page item: an image frame is a `Rectangle` carrying inline
//! `image_bytes`; a text frame is a `TextFrame` + a `Story` whose paragraphs
//! and runs hold the recovered text. The two are linked by the model's one
//! hard contract — `TextFrame.parent_story` must equal the hosting
//! `ParsedStory.self_id` (string equality) — which `rebuild_indexes()` keys
//! `frame_for_story` on. Cross-kind draw order is recorded in
//! `Spread::frames_in_order` (raster background first, then reading order).
//!
//! `Rectangle`, `TextFrame`, and `Page` have no `Default`, so the `blank_*`
//! helpers spell out every field once (all-neutral) and the mappers override
//! only what the IR provides.

use std::collections::HashMap;

use base64::Engine as _;
use paged_model::{
    Bounds, CharacterRun, ColorEntry, ColorModel, ColorSpace, CornerSpec, DesignMap, FrameRef,
    Graphic, Page, Paragraph, PathAnchor, Polygon, Rectangle, Spread, SpreadRef, Story, StoryRef,
    TextFrame,
};
use paged_scene::{Document, ParsedSpread, ParsedStory};

use crate::ir::{DocumentIr, FrameIr, RectIr, TextFrameIr, VectorIr};
use crate::Error;

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Builds the document's colour palette, deduping PDF sRGB colours into
/// `ColorEntry` swatches the model resolves (`fill_color`/`stroke_color` refs).
struct Palette {
    graphic: Graphic,
    seen: HashMap<(u8, u8, u8), String>,
}
impl Palette {
    fn new() -> Self {
        Palette {
            graphic: Graphic::default(),
            seen: HashMap::new(),
        }
    }
    /// sRGB in 0..=1 → a deduped `"Color/pdf_r_g_b"` swatch id (model RGB is
    /// stored 0..=255).
    fn color_id(&mut self, rgb: [f32; 3]) -> String {
        let key = (to_u8(rgb[0]), to_u8(rgb[1]), to_u8(rgb[2]));
        if let Some(id) = self.seen.get(&key) {
            return id.clone();
        }
        let id = format!("Color/pdf_{}_{}_{}", key.0, key.1, key.2);
        self.graphic.colors.insert(
            id.clone(),
            ColorEntry {
                self_id: id.clone(),
                name: None,
                space: ColorSpace::Rgb,
                value: vec![key.0 as f32, key.1 as f32, key.2 as f32],
                model: ColorModel::Process,
                alternate_space: None,
                alternate_value: Vec::new(),
                tint: None,
                alpha: None,
            },
        );
        self.seen.insert(key, id.clone());
        id
    }
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Monotonic id source — the model only needs unique strings, and every id we
/// mint (frame/story/page) is internal to this from-scratch document.
struct IdGen {
    n: u32,
}
impl IdGen {
    fn new() -> Self {
        IdGen { n: 0 }
    }
    fn id(&mut self, prefix: &str) -> String {
        self.n += 1;
        format!("{prefix}{}", self.n)
    }
}

/// Map the whole IR into a native document. See the module docs.
pub fn build_document(ir: &DocumentIr) -> Result<Document, Error> {
    let mut ids = IdGen::new();
    let mut spreads = Vec::with_capacity(ir.pages.len());
    let mut stories = Vec::new();
    let mut palette = Palette::new();

    for (page_idx, page) in ir.pages.iter().enumerate() {
        let mut spread = Spread {
            self_id: Some(ids.id("spread")),
            item_transform: Some(IDENTITY),
            ..Spread::default()
        };
        spread.pages.push(Page {
            self_id: Some(ids.id("page")),
            bounds: page_bounds(page.width_pt, page.height_pt),
            applied_master: None,
            item_transform: Some(IDENTITY),
            master_page_transform: None,
            override_list: Vec::new(),
            name: Some((page_idx + 1).to_string()),
            show_master_items: Some(false),
        });

        // The full-page raster (if any) is drawn first so recovered frames
        // sit above it — the honest-degradation background.
        if let Some(b64) = &page.background_png_b64 {
            let bytes = decode_png(b64)?;
            push_image(
                &mut spread,
                &mut ids,
                RectIr {
                    x_pt: 0.0,
                    y_pt: 0.0,
                    width_pt: page.width_pt,
                    height_pt: page.height_pt,
                },
                bytes,
            );
        }

        for frame in &page.frames {
            match frame {
                FrameIr::Image(img) => {
                    let bytes = decode_png(&img.png_b64)?;
                    push_image(&mut spread, &mut ids, img.rect, bytes);
                }
                FrameIr::Text(tf) => {
                    push_text(&mut spread, &mut stories, &mut ids, &mut palette, tf);
                }
                FrameIr::Vector(v) => {
                    push_vector(&mut spread, &mut ids, &mut palette, v);
                }
            }
        }

        spreads.push(ParsedSpread {
            src: format!("Spreads/Spread_{}.xml", page_idx + 1),
            spread,
        });
    }

    // The shell derives page navigation + the "document loaded" state from the
    // designmap's ORDERED spread/story refs (not from Document.spreads, which
    // only the renderer + health reads). A parsed IDML gets these from
    // designmap.xml; our from-scratch pgm must fill them itself, or the editor
    // shows "No document loaded" despite a populated scene. The default section
    // ("Cover") is synthesized by the shell when sections are empty.
    let designmap = DesignMap {
        spreads: spreads
            .iter()
            .map(|s| SpreadRef { src: s.src.clone() })
            .collect(),
        stories: stories
            .iter()
            .map(|s| StoryRef { src: s.src.clone() })
            .collect(),
        document_name: Some("Imported PDF".to_string()),
        ..Default::default()
    };

    let mut document = Document {
        designmap,
        palette: palette.graphic,
        spreads,
        stories,
        master_spreads: HashMap::new(),
        frame_for_story: HashMap::new(),
        text_frame_index: HashMap::new(),
        styles: Default::default(),
        anchors: Vec::new(),
    };
    document.rebuild_indexes();
    Ok(document)
}

fn decode_png(b64: &str) -> Result<Vec<u8>, Error> {
    base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .map_err(Error::Base64)
}

/// Page rectangle in its own coords (identity page/spread transforms mean this
/// is also spread space). `[top, left, bottom, right]`, points.
fn page_bounds(width_pt: f32, height_pt: f32) -> Bounds {
    Bounds {
        top: 0.0,
        left: 0.0,
        bottom: height_pt,
        right: width_pt,
    }
}

/// IR rect (top-left origin, y-down, points) → model `Bounds`.
fn bounds_of(r: RectIr) -> Bounds {
    Bounds {
        top: r.y_pt,
        left: r.x_pt,
        bottom: r.y_pt + r.height_pt,
        right: r.x_pt + r.width_pt,
    }
}

/// Push an inline-image rectangle and record its draw order.
fn push_image(spread: &mut Spread, ids: &mut IdGen, rect: RectIr, bytes: Vec<u8>) {
    let mut r = blank_rectangle(ids.id("img"), bounds_of(rect));
    r.image_bytes = Some(bytes);
    r.has_image_element = true;
    // image_item_transform: None ⇒ stretch the image to the frame bounds,
    // which is exactly what a rasterized page/image wants.
    let idx = spread.rectangles.len();
    spread.rectangles.push(r);
    spread.frames_in_order.push(FrameRef::Rectangle(idx));
}

/// Build a text frame + its story from a text-frame IR, link them, and record
/// draw order. A frame whose paragraphs all reduce to empty is skipped (the
/// composer would drop empty runs/paragraphs anyway).
fn push_text(
    spread: &mut Spread,
    stories: &mut Vec<ParsedStory>,
    ids: &mut IdGen,
    palette: &mut Palette,
    tf: &TextFrameIr,
) {
    let mut story = Story::default();
    for p in &tf.paragraphs {
        let mut para = Paragraph::default();
        for run in &p.runs {
            if run.text.is_empty() {
                continue;
            }
            para.runs.push(CharacterRun {
                font: run.font_family.clone(),
                font_style: font_style(run.bold, run.italic),
                point_size: Some(run.font_size_pt),
                // Recovered fill colour → a swatch (defaults to the renderer's
                // black when the run carried no colour).
                fill_color: run.color_rgb.map(|c| palette.color_id(c)),
                text: run.text.clone(),
                ..Default::default()
            });
        }
        if !para.runs.is_empty() {
            story.paragraphs.push(para);
        }
    }
    if story.paragraphs.is_empty() {
        return;
    }

    let story_id = ids.id("story");
    let frame = blank_text_frame(ids.id("frame"), story_id.clone(), bounds_of(tf.rect));
    let idx = spread.text_frames.len();
    spread.text_frames.push(frame);
    spread.frames_in_order.push(FrameRef::TextFrame(idx));
    stories.push(ParsedStory {
        src: format!("Stories/Story_{story_id}.xml"),
        self_id: story_id,
        story,
    });
}

/// Build a Polygon from a vector IR: flatten its contours into a flat anchor
/// list + subpath boundaries, AABB bounds, and fill/stroke swatches.
fn push_vector(spread: &mut Spread, ids: &mut IdGen, palette: &mut Palette, v: &VectorIr) {
    let subs: Vec<_> = v.subpaths.iter().filter(|s| s.points.len() >= 2).collect();
    if subs.is_empty() {
        return;
    }
    let mut anchors: Vec<PathAnchor> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let mut opens: Vec<bool> = Vec::new();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for sub in &subs {
        starts.push(anchors.len());
        opens.push(!sub.closed);
        for p in &sub.points {
            anchors.push(PathAnchor {
                anchor: (p.x_pt, p.y_pt),
                left: (p.x_pt, p.y_pt),
                right: (p.x_pt, p.y_pt),
            });
            min_x = min_x.min(p.x_pt);
            min_y = min_y.min(p.y_pt);
            max_x = max_x.max(p.x_pt);
            max_y = max_y.max(p.y_pt);
        }
    }
    // Canonical single-contour form (matches the parser + renderer contract):
    // one contour → no explicit starts; a single CLOSED contour also drops the
    // open flags, a single OPEN one keeps `[true]`.
    if subs.len() == 1 {
        starts = Vec::new();
        if !opens[0] {
            opens = Vec::new();
        }
    }
    let bounds = Bounds {
        top: min_y,
        left: min_x,
        bottom: max_y,
        right: max_x,
    };
    let poly = blank_polygon(
        ids.id("vec"),
        bounds,
        anchors,
        starts,
        opens,
        v.fill_rgb.map(|c| palette.color_id(c)),
        v.stroke_rgb.map(|c| palette.color_id(c)),
        v.stroke_width_pt,
    );
    let idx = spread.polygons.len();
    spread.polygons.push(poly);
    spread.frames_in_order.push(FrameRef::Polygon(idx));
}

/// IDML expresses bold/italic through the font *style* string, not flags.
fn font_style(bold: bool, italic: bool) -> Option<String> {
    match (bold, italic) {
        (true, true) => Some("Bold Italic".to_string()),
        (true, false) => Some("Bold".to_string()),
        (false, true) => Some("Italic".to_string()),
        (false, false) => None,
    }
}

/// A neutral rectangle — every field at its no-op value. Callers override the
/// few they need (e.g. `image_bytes` + `has_image_element` for an image).
fn blank_rectangle(self_id: String, bounds: Bounds) -> Rectangle {
    Rectangle {
        self_id: Some(self_id),
        bounds,
        item_transform: Some(IDENTITY),
        fill_color: None,
        fill_tint: None,
        stroke_color: None,
        stroke_weight: None,
        drop_shadow: None,
        stroke_drop_shadow: None,
        image_link: None,
        has_image_element: false,
        has_inline_pdf: false,
        image_item_transform: None,
        image_bytes: None,
        image_clip: None,
        applied_object_style: None,
        text_wrap: None,
        frame_fitting: None,
        stroke_type: None,
        stroke_alignment: None,
        end_cap: None,
        end_join: None,
        miter_limit: None,
        stroke_gap_color: None,
        stroke_gap_tint: None,
        stroke_dash: Vec::new(),
        item_layer: None,
        corner_radius: None,
        corner_option: None,
        corners: [CornerSpec::default(); 4],
        is_anchored: false,
        opacity: None,
        blend_mode: None,
        effects: None,
        gradient_fill_angle: None,
        gradient_fill_length: None,
        gradient_stroke_angle: None,
        gradient_stroke_length: None,
        text_paths: Vec::new(),
        overprint_fill: false,
        overprint_stroke: false,
        nonprinting: false,
        visible: true,
        locked: false,
        anchors: Vec::new(),
        subpath_starts: Vec::new(),
        subpath_open: Vec::new(),
    }
}

/// A neutral text frame linked to `parent_story` — single, unthreaded, no
/// insets/columns overrides (the composer uses its defaults).
fn blank_text_frame(self_id: String, parent_story: String, bounds: Bounds) -> TextFrame {
    TextFrame {
        self_id: Some(self_id),
        parent_story: Some(parent_story),
        bounds,
        item_transform: Some(IDENTITY),
        fill_color: None,
        fill_tint: None,
        stroke_color: None,
        stroke_weight: None,
        stroke_type: None,
        stroke_gap_color: None,
        stroke_gap_tint: None,
        stroke_dash: Vec::new(),
        drop_shadow: None,
        stroke_drop_shadow: None,
        next_text_frame: None,
        vertical_justification: None,
        first_baseline_offset: None,
        minimum_first_baseline_offset: None,
        inset_spacing: None,
        auto_sizing: None,
        auto_sizing_reference_point: None,
        minimum_width_for_auto_sizing: None,
        minimum_height_for_auto_sizing: None,
        use_minimum_height_for_auto_sizing: None,
        column_count: None,
        column_gutter: None,
        column_balance: None,
        applied_object_style: None,
        text_wrap: None,
        item_layer: None,
        is_anchored: false,
        opacity: None,
        blend_mode: None,
        anchors: Vec::new(),
        subpath_starts: Vec::new(),
        subpath_open: Vec::new(),
        effects: None,
        gradient_fill_angle: None,
        gradient_fill_length: None,
        gradient_stroke_angle: None,
        gradient_stroke_length: None,
        applied_toc_style: None,
        overprint_fill: false,
        overprint_stroke: false,
        nonprinting: false,
        visible: true,
        locked: false,
    }
}

/// A neutral polygon — every field at its no-op value, with the caller's
/// geometry (anchors/subpaths), AABB bounds, and fill/stroke swatches.
fn blank_polygon(
    self_id: String,
    bounds: Bounds,
    anchors: Vec<PathAnchor>,
    subpath_starts: Vec<usize>,
    subpath_open: Vec<bool>,
    fill_color: Option<String>,
    stroke_color: Option<String>,
    stroke_weight: Option<f32>,
) -> Polygon {
    Polygon {
        self_id: Some(self_id),
        bounds,
        item_transform: Some(IDENTITY),
        fill_color,
        fill_tint: None,
        stroke_color,
        stroke_weight,
        stroke_type: None,
        stroke_alignment: None,
        end_join: None,
        miter_limit: None,
        stroke_gap_color: None,
        stroke_gap_tint: None,
        stroke_dash: Vec::new(),
        applied_object_style: None,
        anchors,
        subpath_starts,
        subpath_open,
        text_wrap: None,
        item_layer: None,
        effects: None,
        gradient_fill_angle: None,
        gradient_fill_length: None,
        gradient_stroke_angle: None,
        gradient_stroke_length: None,
        opacity: None,
        blend_mode: None,
        text_paths: Vec::new(),
        image_link: None,
        has_image_element: false,
        has_inline_pdf: false,
        image_item_transform: None,
        image_bytes: None,
        image_clip: None,
        overprint_fill: false,
        overprint_stroke: false,
        nonprinting: false,
        visible: true,
        locked: false,
    }
}
