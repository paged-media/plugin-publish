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

// The Document IR — the TypeScript twin of the Rust `pdf-import` crate's
// `ir.rs`. This is the exact JSON shape `pdf_ir_to_paged_wasm` deserializes,
// so the two MUST stay in lockstep (a mismatch is a serde error at the wasm
// boundary, surfaced by the round-trip test on the Rust side). Everything is
// reading-ordered, in POINTS, top-left origin (y grows downward).

/** A rectangle in point coordinates, top-left origin. Flattened into the
 *  frame objects on the wire (matches serde `#[serde(flatten)]`). */
export interface RectFields {
  x_pt: number;
  y_pt: number;
  width_pt: number;
  height_pt: number;
}

/** A styled text run — the smallest unit of uniform character attributes. */
export interface RunIr {
  text: string;
  font_size_pt: number;
  font_family?: string;
  bold?: boolean;
  italic?: boolean;
  /** sRGB fill 0..1 per channel; carried for the Phase 2 swatch mapping. */
  color_rgb?: [number, number, number];
}

/** A paragraph = a sequence of styled runs. */
export interface ParagraphIr {
  runs: RunIr[];
}

/** An editable text frame: geometry + its paragraphs. `kind` tags the union
 *  (serde `#[serde(tag = "kind")]`). */
export interface TextFrameIr extends RectFields {
  kind: "text";
  paragraphs: ParagraphIr[];
}

/** An image frame carrying an inline PNG (base64, no data-URI prefix). */
export interface ImageFrameIr extends RectFields {
  kind: "image";
  png_b64: string;
}

/** A point in page points, top-left origin. */
export interface PointIr {
  x_pt: number;
  y_pt: number;
}

/** One contour of a vector path. `closed` contours are filled/closed shapes;
 *  open ones are strokes (a line/polyline). Curves are flattened to points. */
export interface SubpathIr {
  points: PointIr[];
  closed: boolean;
}

/** A vector shape recovered from the PDF's path ops — one or more contours
 *  (compound path) with an optional fill and/or stroke. Colours are sRGB
 *  0..1; `build` registers them as swatches. */
export interface VectorIr {
  kind: "vector";
  subpaths: SubpathIr[];
  fill_rgb?: [number, number, number];
  stroke_rgb?: [number, number, number];
  stroke_width_pt?: number;
}

export type FrameIr = TextFrameIr | ImageFrameIr | VectorIr;

/** One page: size in points, an optional full-page raster background (base64
 *  PNG) kept beneath low-confidence content, and frames in reading order. */
export interface PageIr {
  width_pt: number;
  height_pt: number;
  background_png_b64?: string;
  frames: FrameIr[];
}

/** The whole reconstructed document. */
export interface DocumentIr {
  pages: PageIr[];
}
