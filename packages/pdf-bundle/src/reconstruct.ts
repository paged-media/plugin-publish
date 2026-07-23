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

// Phase 1 reconstruction — pdf.js → Document IR.
//
// BROWSER-RUNTIME ONLY (the low-confidence fallback rasterizes via a canvas,
// like raster.ts). One `getDocument` parse; per page a CONFIDENCE GATE decides
// the honest representation:
//
//   · enough positioned, unrotated text  → an editable TEXT frame (the pure
//       heuristics in extract.ts turn glyph runs into paragraphs of runs). No
//       raster beneath it — the text IS the content.
//   · otherwise (scanned / image-only / rotated page) → the page rasterized as
//       a full-page background image (the Phase 0 behaviour, per page).
//
// So a text PDF opens editable, a scanned PDF opens as images, and a mixed
// document picks per page — never faking editable text it couldn't recover.
// Images/vectors embedded in a text page are deferred to Phase 2 (getOperatorList).

import * as pdfjsLib from "pdfjs-dist";

import type { DocumentIr, PageIr } from "./ir";
import {
  DEFAULT_OPTIONS,
  itemsToParagraphs,
  textBBox,
  textCharCount,
  type PositionedItem,
  type ReconstructOptions,
} from "./extract";
import { ensureWorker, renderPageToPng } from "./raster";

/** Below this many non-whitespace characters a page is treated as non-text
 *  (rasterized rather than reconstructed). */
const MIN_TEXT_CHARS = 4;

export interface ReconstructPdfOptions extends Partial<ReconstructOptions> {
  /** DPI for the raster fallback of non-text pages (default 150). */
  dpi?: number;
  /** Min non-whitespace chars for a page to be treated as text (default 4). */
  minTextChars?: number;
}

/** Reconstruct a PDF into a Document IR (editable text where recoverable,
 *  raster background otherwise). Throws with a clear message on a parse
 *  failure — the importer catches + falls back to the Phase 0 path. */
export async function reconstructPdf(
  bytes: Uint8Array,
  opts: ReconstructPdfOptions = {},
): Promise<DocumentIr> {
  await ensureWorker();
  const heur: ReconstructOptions = { ...DEFAULT_OPTIONS, ...opts };
  const dpi = opts.dpi ?? 150;
  const minChars = opts.minTextChars ?? MIN_TEXT_CHARS;

  let pdf: Awaited<ReturnType<typeof pdfjsLib.getDocument>["promise"]>;
  try {
    pdf = await pdfjsLib.getDocument({ data: bytes.slice() }).promise;
  } catch (err) {
    throw new Error(`reconstructPdf: failed to parse PDF — ${String(err)}`);
  }

  try {
    const pages: PageIr[] = [];
    for (let n = 1; n <= pdf.numPages; n++) {
      const page = await pdf.getPage(n);
      const vp = page.getViewport({ scale: 1 });
      const widthPt = vp.width;
      const heightPt = vp.height;
      const rotated = normalizeRotation(page.rotate) !== 0;

      const pageIr: PageIr = { width_pt: widthPt, height_pt: heightPt, frames: [] };

      if (!rotated) {
        const tc = await page.getTextContent();
        const items = normalizeItems(tc, heightPt);
        if (textCharCount(items) >= minChars) {
          const paragraphs = itemsToParagraphs(items, heur);
          const bbox = textBBox(items, widthPt, heightPt, heur);
          if (paragraphs.length > 0 && bbox) {
            pageIr.frames.push({ kind: "text", ...bbox, paragraphs });
          }
        }
      }

      // No editable text recovered → keep the page as a raster background.
      if (pageIr.frames.length === 0) {
        const png = await renderPageToPng(page, dpi);
        pageIr.background_png_b64 = toBase64(png);
      }

      pages.push(pageIr);
      page.cleanup();
    }
    return { pages };
  } finally {
    await pdf.destroy();
  }
}

/** pdf.js text content → point-space `PositionedItem`s (top-left origin). Only
 *  valid for unrotated pages (the caller gates on rotation). */
function normalizeItems(
  tc: { items: unknown[]; styles?: Record<string, { fontFamily?: string }> },
  pageHeightPt: number,
): PositionedItem[] {
  const out: PositionedItem[] = [];
  for (const raw of tc.items) {
    const it = raw as {
      str?: unknown;
      transform?: unknown;
      width?: unknown;
      fontName?: string;
    };
    if (typeof it.str !== "string" || !Array.isArray(it.transform)) continue;
    const t = it.transform as number[];
    const fontSizePt = Math.hypot(t[0], t[1]);
    if (!(fontSizePt > 0)) continue;

    const style = it.fontName ? tc.styles?.[it.fontName] : undefined;
    const nameBlob = `${it.fontName ?? ""} ${style?.fontFamily ?? ""}`;
    out.push({
      text: it.str,
      xPt: t[4],
      // pdf.js user space is y-up from the bottom-left; flip to y-down top.
      baselineTopY: pageHeightPt - t[5],
      widthPt: typeof it.width === "number" ? it.width : 0,
      fontSizePt,
      fontFamily: cleanFamily(style?.fontFamily),
      bold: /bold|black|heavy|semibold/i.test(nameBlob),
      italic: /italic|oblique/i.test(nameBlob),
    });
  }
  return out;
}

/** Strip an `ABCDEF+` subset prefix; drop bare CSS generics (the engine's
 *  default face is a better fallback than "sans-serif"). */
function cleanFamily(family: string | undefined): string | undefined {
  if (!family) return undefined;
  const stripped = family.replace(/^[A-Z]{6}\+/, "").trim();
  if (/^(sans-serif|serif|monospace)$/i.test(stripped)) return undefined;
  return stripped.length > 0 ? stripped : undefined;
}

function normalizeRotation(rotate: number): number {
  return (((rotate % 360) + 360) % 360) as number;
}

/** Portable, chunked base64 (browser + node) for PNG bytes. */
function toBase64(bytes: Uint8Array): string {
  if (typeof Buffer !== "undefined") {
    return Buffer.from(bytes).toString("base64");
  }
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}
