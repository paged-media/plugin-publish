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

// Reconstruction heuristics — PURE, no pdf.js. Turns positioned text items
// (already normalized to points, top-left origin) into paragraphs of styled
// runs. Isolated here so the churning heuristics are unit-tested directly
// (test/extract.test.ts) without a PDF or a browser. `reconstruct.ts` owns the
// pdf.js I/O that produces the `PositionedItem`s.
//
// v1 scope (honest): line grouping by baseline, word/space recovery from
// kerning gaps, adjacent same-style run merging, and ONE paragraph per visual
// line (preserve-layout — no line-merging that could wrongly join paragraphs).
// Reflowable paragraph grouping (leading/indent detection) + columns are
// Phase 3.

import type { ParagraphIr, RectFields, RunIr, TextFrameIr } from "./ir";

/** A positioned text run in point coordinates, top-left origin (y-down).
 *  `xPt` is the run's left edge (baseline origin x); `baselineTopY` is the
 *  baseline's distance from the page top; `widthPt` is the advance width. */
export interface PositionedItem {
  text: string;
  xPt: number;
  baselineTopY: number;
  widthPt: number;
  fontSizePt: number;
  fontFamily?: string;
  bold: boolean;
  italic: boolean;
}

export interface ReconstructOptions {
  /** Insert a space when the inter-item gap exceeds this × font size. */
  spaceGapFactor: number;
  /** Two items share a line when their baselines differ by less than this ×
   *  the larger font size. */
  lineToleranceFactor: number;
  /** Font sizes within this many points are treated as equal for run merging. */
  sizeEqualityTolPt: number;
  /** Padding added around the recovered text bbox for the frame, in points. */
  framePaddingPt: number;
  /** Split a line into separate frames (columns / sidebars) when an intra-line
   *  gap exceeds this × the font size — well above a word space, so ordinary
   *  spacing stays one frame but a column gutter separates. */
  columnGapFactor: number;
}

export const DEFAULT_OPTIONS: ReconstructOptions = {
  spaceGapFactor: 0.25,
  lineToleranceFactor: 0.3,
  sizeEqualityTolPt: 0.6,
  framePaddingPt: 2,
  columnGapFactor: 3,
};

const isBlank = (s: string): boolean => s.trim().length === 0;

/** Group items into lines by baseline proximity, each line left-to-right. */
export function groupLines(
  items: PositionedItem[],
  opts: ReconstructOptions = DEFAULT_OPTIONS,
): PositionedItem[][] {
  const sorted = [...items].sort((a, b) => a.baselineTopY - b.baselineTopY);
  const lines: PositionedItem[][] = [];
  let current: PositionedItem[] = [];
  let lineY = Number.NaN;
  let lineSize = 0;

  for (const item of sorted) {
    const tol = opts.lineToleranceFactor * Math.max(lineSize, item.fontSizePt);
    if (current.length === 0 || Math.abs(item.baselineTopY - lineY) <= tol) {
      current.push(item);
      // Track the line's baseline as a running mean weighted by nothing fancy
      // — the first item anchors it; later items only extend it.
      lineY = Number.isNaN(lineY) ? item.baselineTopY : lineY;
      lineSize = Math.max(lineSize, item.fontSizePt);
    } else {
      lines.push(current);
      current = [item];
      lineY = item.baselineTopY;
      lineSize = item.fontSizePt;
    }
  }
  if (current.length > 0) lines.push(current);
  for (const line of lines) line.sort((a, b) => a.xPt - b.xPt);
  return lines;
}

const sameStyle = (
  a: PositionedItem,
  b: PositionedItem,
  opts: ReconstructOptions,
): boolean =>
  (a.fontFamily ?? "") === (b.fontFamily ?? "") &&
  a.bold === b.bold &&
  a.italic === b.italic &&
  Math.abs(a.fontSizePt - b.fontSizePt) <= opts.sizeEqualityTolPt;

const runOf = (item: PositionedItem, text: string): RunIr => {
  const run: RunIr = { text, font_size_pt: round2(item.fontSizePt) };
  if (item.fontFamily) run.font_family = item.fontFamily;
  if (item.bold) run.bold = true;
  if (item.italic) run.italic = true;
  return run;
};

/** Build one paragraph (its runs) from a single line's items, recovering
 *  spaces from kerning gaps and merging adjacent same-style items. */
export function lineToParagraph(
  line: PositionedItem[],
  opts: ReconstructOptions = DEFAULT_OPTIONS,
): ParagraphIr | null {
  const runs: RunIr[] = [];
  let cur: { item: PositionedItem; text: string } | null = null;
  let pendingSpace = false;
  let prevEndX = Number.NaN;

  for (const item of line) {
    const gap = Number.isNaN(prevEndX) ? 0 : item.xPt - prevEndX;
    const wantSpace = gap > opts.spaceGapFactor * item.fontSizePt;
    prevEndX = item.xPt + item.widthPt;

    // Whitespace-only items only signal a space; they never start a run.
    if (isBlank(item.text)) {
      if (item.text.length > 0) pendingSpace = true;
      continue;
    }
    if (wantSpace) pendingSpace = true;

    if (cur && sameStyle(cur.item, item, opts)) {
      const sep = pendingSpace && !endsWithSpace(cur.text) && !startsWithSpace(item.text) ? " " : "";
      cur.text += sep + item.text;
    } else {
      const hadPrev = cur !== null;
      if (cur) runs.push(runOf(cur.item, cur.text));
      const lead: string =
        pendingSpace && hadPrev && !startsWithSpace(item.text) ? " " : "";
      cur = { item, text: lead + item.text };
    }
    pendingSpace = false;
  }
  if (cur) runs.push(runOf(cur.item, cur.text));

  const nonEmpty = runs.filter((r) => r.text.length > 0);
  return nonEmpty.length > 0 ? { runs: nonEmpty } : null;
}

/** Full page reconstruction: items → one paragraph per line (reading order). */
export function itemsToParagraphs(
  items: PositionedItem[],
  opts: ReconstructOptions = DEFAULT_OPTIONS,
): ParagraphIr[] {
  return groupLines(items, opts)
    .map((line) => lineToParagraph(line, opts))
    .filter((p): p is ParagraphIr => p !== null && p.runs.length > 0);
}

/** Approximate ascent as a fraction of the font size (matches the engine's
 *  default first-baseline offset of `point_size × 0.8`), so a frame placed at
 *  `baseline − ASCENT×size` lands its recovered baseline back on the PDF's. */
const ASCENT_FACTOR = 0.8;
const DESCENT_FACTOR = 0.25;

/**
 * Position-preserving reconstruction: ONE text frame per visual line, placed at
 * the line's actual PDF coordinates, so the text lands where it was instead of
 * reflowing into a single page block. Each frame holds one paragraph (the
 * line's styled runs) and is sized to the line so it never re-wraps. This is
 * the faithful-reconstruction path (fidelity over flow-editability); lines stay
 * independently editable.
 */
export function itemsToPositionedFrames(
  items: PositionedItem[],
  pageWidthPt: number,
  _pageHeightPt: number,
  opts: ReconstructOptions = DEFAULT_OPTIONS,
): TextFrameIr[] {
  const frames: TextFrameIr[] = [];
  for (const line of groupLines(items, opts)) {
    // Split the line at column gutters so a body column and a sidebar sharing
    // a baseline become separate, correctly-placed frames instead of one run.
    for (const segment of splitLineByGaps(line, opts)) {
      const frame = lineFrame(segment, pageWidthPt, opts);
      if (frame) frames.push(frame);
    }
  }
  return frames;
}

/** Split a single (x-sorted) line into segments wherever an inter-item gap
 *  exceeds `columnGapFactor × fontSize` — a column gutter, not a word space. */
export function splitLineByGaps(
  line: PositionedItem[],
  opts: ReconstructOptions = DEFAULT_OPTIONS,
): PositionedItem[][] {
  const sorted = [...line].sort((a, b) => a.xPt - b.xPt);
  const segments: PositionedItem[][] = [];
  let cur: PositionedItem[] = [];
  let prevEnd = Number.NaN;
  for (const item of sorted) {
    const gap = Number.isNaN(prevEnd) ? 0 : item.xPt - prevEnd;
    if (cur.length > 0 && gap > opts.columnGapFactor * item.fontSizePt) {
      segments.push(cur);
      cur = [];
    }
    cur.push(item);
    prevEnd = item.xPt + item.widthPt;
  }
  if (cur.length > 0) segments.push(cur);
  return segments;
}

/** Build one positioned text frame from a single line, or null if the line has
 *  no renderable text. */
export function lineFrame(
  line: PositionedItem[],
  pageWidthPt: number,
  opts: ReconstructOptions = DEFAULT_OPTIONS,
): TextFrameIr | null {
  const para = lineToParagraph(line, opts);
  if (!para) return null;
  const real = line.filter((i) => !isBlank(i.text));
  if (real.length === 0) return null;

  const maxSize = Math.max(...real.map((i) => i.fontSizePt));
  // Items in a line share a baseline (grouped within tolerance); take the
  // lowest so mixed-size runs sit on a common line.
  const baseline = Math.max(...real.map((i) => i.baselineTopY));
  const left = Math.min(...real.map((i) => i.xPt));
  const right = Math.max(...real.map((i) => i.xPt + i.widthPt));

  const top = baseline - ASCENT_FACTOR * maxSize;
  const height = maxSize * (ASCENT_FACTOR + DESCENT_FACTOR + 0.2);
  // Width: the text extent plus slack so a substituted font (wider metrics
  // than the PDF's) doesn't force a wrap — capped so frames don't span the
  // page (which would make overlapping line-frames hard to select).
  const width = Math.min(
    Math.max(0, pageWidthPt - left),
    right - left + maxSize * 4,
  );

  return {
    kind: "text",
    x_pt: left,
    y_pt: Math.max(0, top),
    width_pt: width,
    height_pt: height,
    paragraphs: [para],
  };
}

/** Bounding box of the items (text ink), padded — the text frame's geometry.
 *  Top uses the tallest ascender (baseline − size), bottom a descender margin.
 *  Clamped to the page. Returns null for no items. */
export function textBBox(
  items: PositionedItem[],
  pageWidthPt: number,
  pageHeightPt: number,
  opts: ReconstructOptions = DEFAULT_OPTIONS,
): RectFields | null {
  const real = items.filter((i) => !isBlank(i.text));
  if (real.length === 0) return null;
  let left = Infinity;
  let right = -Infinity;
  let top = Infinity;
  let bottom = -Infinity;
  for (const i of real) {
    left = Math.min(left, i.xPt);
    right = Math.max(right, i.xPt + i.widthPt);
    top = Math.min(top, i.baselineTopY - i.fontSizePt);
    bottom = Math.max(bottom, i.baselineTopY + 0.25 * i.fontSizePt);
  }
  const pad = opts.framePaddingPt;
  const x = Math.max(0, left - pad);
  const y = Math.max(0, top - pad);
  return {
    x_pt: x,
    y_pt: y,
    width_pt: Math.min(pageWidthPt, right + pad) - x,
    height_pt: Math.min(pageHeightPt, bottom + pad) - y,
  };
}

/** Total non-whitespace characters — the page's text-confidence signal. */
export function textCharCount(items: PositionedItem[]): number {
  return items.reduce((n, i) => n + i.text.replace(/\s/g, "").length, 0);
}

function round2(n: number): number {
  return Math.round(n * 100) / 100;
}
function endsWithSpace(s: string): boolean {
  return /\s$/.test(s);
}
function startsWithSpace(s: string): boolean {
  return /^\s/.test(s);
}
