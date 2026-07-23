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

import { describe, expect, it } from "vitest";

import {
  groupLines,
  itemsToParagraphs,
  itemsToPositionedFrames,
  lineFrame,
  lineToParagraph,
  splitLineByGaps,
  textBBox,
  textCharCount,
  type PositionedItem,
} from "../src/extract";

/** Positioned-item factory (width defaults to 6pt/char). */
function pi(
  text: string,
  xPt: number,
  baselineTopY: number,
  fontSizePt = 12,
  o: Partial<PositionedItem> = {},
): PositionedItem {
  return {
    text,
    xPt,
    baselineTopY,
    widthPt: o.widthPt ?? text.length * 6,
    fontSizePt,
    bold: o.bold ?? false,
    italic: o.italic ?? false,
    fontFamily: o.fontFamily,
  };
}

describe("word/space recovery", () => {
  it("inserts a space across a kerning gap and merges same-style runs", () => {
    const line = [
      pi("Hello", 0, 100, 12, { widthPt: 30 }),
      pi("world", 35, 100, 12, { widthPt: 30 }),
    ];
    const p = lineToParagraph(line)!;
    expect(p.runs).toHaveLength(1);
    expect(p.runs[0].text).toBe("Hello world");
  });

  it("does not insert a space when items abut", () => {
    const line = [
      pi("Hel", 0, 100, 12, { widthPt: 18 }),
      pi("lo", 18, 100, 12, { widthPt: 12 }),
    ];
    const p = lineToParagraph(line)!;
    expect(p.runs).toHaveLength(1);
    expect(p.runs[0].text).toBe("Hello");
  });

  it("folds a whitespace-only item into a single separating space", () => {
    const line = [
      pi("Hello", 0, 100, 12, { widthPt: 30 }),
      pi(" ", 30, 100, 12, { widthPt: 4 }),
      pi("world", 34, 100, 12, { widthPt: 30 }),
    ];
    const p = lineToParagraph(line)!;
    expect(p.runs[0].text).toBe("Hello world");
  });
});

describe("style runs", () => {
  it("splits runs on a style change and carries bold/italic", () => {
    const line = [
      pi("Hello ", 0, 100, 12, { widthPt: 36, bold: true }),
      pi("world", 36, 100, 12, { widthPt: 30, italic: true }),
    ];
    const p = lineToParagraph(line)!;
    expect(p.runs).toHaveLength(2);
    expect(p.runs[0].text).toBe("Hello ");
    expect(p.runs[0].bold).toBe(true);
    expect(p.runs[1].text).toBe("world");
    expect(p.runs[1].italic).toBe(true);
    expect(p.runs[1].bold).toBeUndefined();
  });

  it("records point size per run", () => {
    const p = lineToParagraph([pi("Big", 0, 100, 24, { widthPt: 40 })])!;
    expect(p.runs[0].font_size_pt).toBe(24);
  });
});

describe("line grouping", () => {
  it("clusters by baseline and sorts within a line by x", () => {
    const items = [
      pi("b", 50, 100.5),
      pi("a", 0, 100),
      pi("c", 0, 120),
    ];
    const lines = groupLines(items);
    expect(lines).toHaveLength(2);
    expect(lines[0].map((i) => i.text)).toEqual(["a", "b"]);
    expect(lines[1][0].text).toBe("c");
  });

  it("produces one paragraph per visual line", () => {
    const items = [
      pi("line one", 0, 100, 12, { widthPt: 48 }),
      pi("line two", 0, 120, 12, { widthPt: 48 }),
    ];
    const paras = itemsToParagraphs(items);
    expect(paras).toHaveLength(2);
    expect(paras[0].runs[0].text).toBe("line one");
    expect(paras[1].runs[0].text).toBe("line two");
  });
});

describe("position-preserving frames", () => {
  it("places a line frame at the line's coordinates with the baseline restored", () => {
    // A 12pt line whose baseline sits 100pt from the page top, starting at x=72.
    const f = lineFrame(
      [pi("Hello world", 72, 100, 12, { widthPt: 60 })],
      612,
    )!;
    expect(f.kind).toBe("text");
    expect(f.x_pt).toBe(72);
    // top = baseline - 0.8*size = 100 - 9.6
    expect(f.y_pt).toBeCloseTo(90.4, 1);
    // width = text extent + slack, capped to page
    expect(f.width_pt).toBeGreaterThanOrEqual(60);
    expect(f.width_pt).toBeLessThanOrEqual(612 - 72);
    expect(f.paragraphs).toHaveLength(1);
    expect(f.paragraphs[0].runs[0].text).toBe("Hello world");
  });

  it("splits a shared-baseline line at a column gutter into separate frames", () => {
    const items = [
      pi("Title", 250, 80, 24, { widthPt: 80 }),
      pi("left column", 72, 120, 10, { widthPt: 55 }), // ends at 127
      pi("right column", 350, 120, 10, { widthPt: 60 }), // gap 223pt >> 3em
    ];
    const frames = itemsToPositionedFrames(items, 612, 792);
    // Title (1) + left column (1) + right column (1) = 3.
    expect(frames.length).toBe(3);
    const title = frames.find((f) => f.paragraphs[0].runs[0].text === "Title")!;
    expect(title.x_pt).toBe(250);
    expect(title.paragraphs[0].runs[0].font_size_pt).toBe(24);
    // The right column keeps its own x, not merged into the left.
    const rightCol = frames.find((f) => f.x_pt === 350)!;
    expect(rightCol.paragraphs[0].runs[0].text).toBe("right column");
  });

  it("keeps ordinary word spacing in one frame (no false column split)", () => {
    const items = [
      pi("The", 72, 100, 10, { widthPt: 18 }),
      pi("quick", 96, 100, 10, { widthPt: 28 }), // small gaps ~word spaces
      pi("fox", 128, 100, 10, { widthPt: 16 }),
    ];
    expect(splitLineByGaps(items)).toHaveLength(1);
  });
});

describe("geometry + confidence", () => {
  it("computes a padded text bbox clamped to the page", () => {
    const b = textBBox([pi("Hello", 10, 100, 12, { widthPt: 30 })], 600, 800)!;
    expect(b.x_pt).toBe(8); // 10 - 2 pad
    expect(b.y_pt).toBe(86); // (100 - 12) - 2 pad
    expect(b.width_pt).toBe(34); // (40 + 2) - 8
    expect(b.height_pt).toBe(19); // (103 + 2) - 86
  });

  it("counts only non-whitespace characters", () => {
    expect(textCharCount([pi("a b ", 0, 0)])).toBe(2);
    expect(textCharCount([pi("   ", 0, 0)])).toBe(0);
  });
});
