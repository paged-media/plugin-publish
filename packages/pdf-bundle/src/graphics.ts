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

// Vector extraction from a PDF page's operator list — the "decompose a mixed
// page into editable objects" path (slice 2). One walk over the op list,
// tracking the graphics state (CTM, fill/stroke colour, line width) so every
// painted path becomes a `VectorIr` (a real editable shape with colours), in
// paint order. Curves flatten to points (curve fidelity is a later
// refinement); clip paths are ignored (approximation). Images are only COUNTED
// here (not decomposed — in-browser XObject decode is too slow to do inline);
// the caller rasterizes an image-dominant page instead.

import * as pdfjsLib from "pdfjs-dist";

import type { PointIr, SubpathIr, VectorIr } from "./ir";

/** Result of a page graphics walk: the vector shapes + a count of image ops
 *  (so the caller can rasterize an image-dominant page). */
export interface GraphicsResult {
  vectors: VectorIr[];
  imageOps: number;
}

type Mat = number[]; // [a,b,c,d,e,f]
const IDENTITY: Mat = [1, 0, 0, 1, 0, 0];

/** Above this many operators a page is too complex to decompose cheaply — the
 *  caller rasterizes it instead. */
const MAX_OPS = 24000;

function mul(m1: Mat, m2: Mat): Mat {
  return [
    m1[0] * m2[0] + m1[2] * m2[1],
    m1[1] * m2[0] + m1[3] * m2[1],
    m1[0] * m2[2] + m1[2] * m2[3],
    m1[1] * m2[2] + m1[3] * m2[3],
    m1[0] * m2[4] + m1[2] * m2[5] + m1[4],
    m1[1] * m2[4] + m1[3] * m2[5] + m1[5],
  ];
}

/** Transform a path-space point by the CTM into top-left page points. */
function tp(ctm: Mat, x: number, y: number, pageH: number): PointIr {
  const px = ctm[0] * x + ctm[2] * y + ctm[4];
  const py = ctm[1] * x + ctm[3] * y + ctm[5];
  return { x_pt: px, y_pt: pageH - py };
}

/** Uniform-ish scale of a matrix (for line-width mapping). */
function matScale(m: Mat): number {
  const sx = Math.hypot(m[0], m[1]);
  const sy = Math.hypot(m[2], m[3]);
  return (sx + sy) / 2 || 1;
}

type Rgb = [number, number, number];

interface GState {
  ctm: Mat;
  fill: Rgb | null;
  stroke: Rgb | null;
  lineWidth: number;
}

function cloneGState(g: GState): GState {
  return { ctm: g.ctm.slice(), fill: g.fill, stroke: g.stroke, lineWidth: g.lineWidth };
}

const clamp01 = (n: number): number => (n < 0 ? 0 : n > 1 ? 1 : n);
const rgb255 = (r: number, g: number, b: number): Rgb => [
  clamp01(r / 255),
  clamp01(g / 255),
  clamp01(b / 255),
];
const gray = (g: number): Rgb => [clamp01(g), clamp01(g), clamp01(g)];
const cmyk = (c: number, m: number, y: number, k: number): Rgb => [
  clamp01((1 - c) * (1 - k)),
  clamp01((1 - m) * (1 - k)),
  clamp01((1 - y) * (1 - k)),
];

/** pdf.js colour args arrive as an array-like {0,1,2,…}. */
function argNums(a: unknown): number[] {
  if (Array.isArray(a)) return a as number[];
  if (a && typeof a === "object") return Object.values(a as Record<string, number>);
  return [];
}

/** Parse one `constructPath(subops, coords)` op into contours, transformed to
 *  top-left page points by `ctm`. Curves flatten to their endpoints. */
function parsePath(
  subops: number[],
  coords: number[],
  ctm: Mat,
  pageH: number,
): SubpathIr[] {
  const OPS = pdfjsLib.OPS;
  const subpaths: SubpathIr[] = [];
  let cur: SubpathIr | null = null;
  let ci = 0;
  const T = (x: number, y: number) => tp(ctm, x, y, pageH);
  for (const op of subops) {
    if (op === OPS.moveTo) {
      if (cur && cur.points.length > 0) subpaths.push(cur);
      cur = { points: [T(coords[ci], coords[ci + 1])], closed: false };
      ci += 2;
    } else if (op === OPS.lineTo) {
      if (cur) cur.points.push(T(coords[ci], coords[ci + 1]));
      ci += 2;
    } else if (op === OPS.curveTo) {
      if (cur) cur.points.push(T(coords[ci + 4], coords[ci + 5]));
      ci += 6;
    } else if (op === OPS.curveTo2 || op === OPS.curveTo3) {
      if (cur) cur.points.push(T(coords[ci + 2], coords[ci + 3]));
      ci += 4;
    } else if (op === OPS.closePath) {
      if (cur) cur.closed = true;
    } else if (op === OPS.rectangle) {
      const x = coords[ci];
      const y = coords[ci + 1];
      const w = coords[ci + 2];
      const h = coords[ci + 3];
      ci += 4;
      if (cur && cur.points.length > 0) subpaths.push(cur);
      subpaths.push({
        points: [T(x, y), T(x + w, y), T(x + w, y + h), T(x, y + h)],
        closed: true,
      });
      cur = null;
    }
  }
  if (cur && cur.points.length > 0) subpaths.push(cur);
  return subpaths;
}

/** Emit a vector shape for the current path with the current fill/stroke. */
function emitVector(
  path: SubpathIr[],
  g: GState,
  wantFill: boolean,
  wantStroke: boolean,
): VectorIr | null {
  const subpaths = path.filter((s) => s.points.length >= 2);
  if (subpaths.length === 0) return null;
  const v: VectorIr = { kind: "vector", subpaths };
  if (wantFill && g.fill) v.fill_rgb = g.fill;
  if (wantStroke && g.stroke) {
    v.stroke_rgb = g.stroke;
    v.stroke_width_pt = Math.max(0.1, g.lineWidth * matScale(g.ctm));
  }
  if (!v.fill_rgb && !v.stroke_rgb) return null; // invisible (e.g. clip-only)
  return v;
}

/**
 * Extract every painted vector shape + image on the page as ordered editable
 * objects (paint order). Returns [] on any failure — the caller falls back.
 */
export async function extractGraphics(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  page: any,
  _pageWidthPt: number,
  pageHeightPt: number,
): Promise<GraphicsResult | null> {
  let ops: { fnArray: number[]; argsArray: unknown[][] };
  try {
    ops = await page.getOperatorList();
  } catch {
    return null; // can't read ops → let the caller rasterize
  }
  const OPS = pdfjsLib.OPS;
  // A pathologically complex page (huge op list — gradient meshes, thousands of
  // path segments) would produce tens of thousands of objects and stall the
  // walk/mapper. `null` tells the caller to rasterize instead (still faithful).
  if (ops.fnArray.length > MAX_OPS) return null;

  const out: VectorIr[] = [];
  let imageOps = 0;
  let g: GState = { ctm: IDENTITY.slice(), fill: [0, 0, 0], stroke: [0, 0, 0], lineWidth: 1 };
  const stack: GState[] = [];
  let path: SubpathIr[] = [];

  for (let i = 0; i < ops.fnArray.length; i++) {
    const fn = ops.fnArray[i];
    const a = ops.argsArray[i];
    if (fn === OPS.save) {
      stack.push(cloneGState(g));
    } else if (fn === OPS.restore) {
      const prev = stack.pop();
      if (prev) g = prev;
    } else if (fn === OPS.transform) {
      g.ctm = mul(g.ctm, a as Mat);
    } else if (fn === OPS.paintFormXObjectBegin) {
      stack.push(cloneGState(g));
      const m = a[0];
      if (Array.isArray(m)) g.ctm = mul(g.ctm, m as Mat);
    } else if (fn === OPS.paintFormXObjectEnd) {
      const prev = stack.pop();
      if (prev) g = prev;
    } else if (fn === OPS.setLineWidth) {
      g.lineWidth = (a as number[])[0] ?? g.lineWidth;
    } else if (fn === OPS.setFillRGBColor) {
      const [r, gg, b] = argNums(a);
      g.fill = rgb255(r, gg, b);
    } else if (fn === OPS.setStrokeRGBColor) {
      const [r, gg, b] = argNums(a);
      g.stroke = rgb255(r, gg, b);
    } else if (fn === OPS.setFillGray) {
      g.fill = gray(argNums(a)[0] ?? 0);
    } else if (fn === OPS.setStrokeGray) {
      g.stroke = gray(argNums(a)[0] ?? 0);
    } else if (fn === OPS.setFillCMYKColor) {
      const [c, m, y, k] = argNums(a);
      g.fill = cmyk(c, m, y, k);
    } else if (fn === OPS.setStrokeCMYKColor) {
      const [c, m, y, k] = argNums(a);
      g.stroke = cmyk(c, m, y, k);
    } else if (fn === OPS.constructPath) {
      const subops = a[0] as number[];
      const coords = a[1] as number[];
      path.push(...parsePath(subops, coords, g.ctm, pageHeightPt));
    } else if (fn === OPS.fill || fn === OPS.eoFill) {
      const v = emitVector(path, g, true, false);
      if (v) out.push(v);
      path = [];
    } else if (fn === OPS.stroke || fn === OPS.closeStroke) {
      const v = emitVector(path, g, false, true);
      if (v) out.push(v);
      path = [];
    } else if (fn === OPS.fillStroke || fn === OPS.eoFillStroke) {
      const v = emitVector(path, g, true, true);
      if (v) out.push(v);
      path = [];
    } else if (fn === OPS.endPath) {
      path = [];
    } else if (
      fn === OPS.paintImageXObject ||
      fn === OPS.paintInlineImageXObject ||
      fn === OPS.paintImageMaskXObject
    ) {
      // Images aren't decomposed as individual objects (in-browser XObject
      // decode is too costly to do inline); the caller rasterizes an
      // image-dominant page instead. We only COUNT them for that decision.
      imageOps++;
    }
  }
  return { vectors: out, imageOps };
}
