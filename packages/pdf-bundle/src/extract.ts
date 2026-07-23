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

import type { ParagraphIr, RectFields, RunIr } from "./ir";

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
}

export const DEFAULT_OPTIONS: ReconstructOptions = {
  spaceGapFactor: 0.25,
  lineToleranceFactor: 0.3,
  sizeEqualityTolPt: 0.6,
  framePaddingPt: 2,
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
