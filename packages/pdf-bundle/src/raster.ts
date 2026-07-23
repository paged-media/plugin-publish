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

// Phase 0 — rasterize each PDF page to a PNG with pdf.js.
//
// BROWSER-RUNTIME ONLY. pdf.js needs a canvas (OffscreenCanvas or a DOM
// <canvas>) to paint into, so this module is exercised at editor-integration
// time, NOT by the node vitest (which covers `idml-fallback.ts` instead).
//
// Worker strategy: we point `GlobalWorkerOptions.workerSrc` at pdf.js's own
// bundled worker via `new URL(<subpath>, import.meta.url)`. We keep it a
// RUNTIME expression rather than a bundler `?url` import so tsup/esbuild
// leaves it untouched; the editor's Vite build rewrites `new URL(pkg,
// import.meta.url)` to the hashed worker asset at load time. If the worker
// can't be created, pdf.js falls back to its main-thread "fake worker".

import * as pdfjsLib from "pdfjs-dist";

/** One rasterized page: point size at scale 1 + the PNG-encoded pixels. */
export interface PdfPageRaster {
  widthPt: number;
  heightPt: number;
  pngBytes: Uint8Array;
}

let workerConfigured = false;

function configureWorker(): void {
  if (workerConfigured) return;
  workerConfigured = true;
  try {
    pdfjsLib.GlobalWorkerOptions.workerSrc = new URL(
      "pdfjs-dist/build/pdf.worker.min.mjs",
      import.meta.url,
    ).toString();
  } catch {
    // Leave workerSrc unset — pdf.js will use its main-thread fake worker.
  }
}

/** A 2D context we can paint into and read a PNG blob back from. */
interface RasterTarget {
  ctx: CanvasRenderingContext2D;
  toPng: () => Promise<Uint8Array>;
}

function makeTarget(width: number, height: number): RasterTarget {
  // Prefer OffscreenCanvas (works off the main thread / in a worker); fall
  // back to a DOM <canvas> when it isn't available.
  if (typeof OffscreenCanvas !== "undefined") {
    const canvas = new OffscreenCanvas(width, height);
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("rasterizePdf: OffscreenCanvas 2d context unavailable");
    return {
      ctx: ctx as unknown as CanvasRenderingContext2D,
      toPng: async () => {
        const blob = await canvas.convertToBlob({ type: "image/png" });
        return new Uint8Array(await blob.arrayBuffer());
      },
    };
  }
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("rasterizePdf: canvas 2d context unavailable");
  return {
    ctx,
    toPng: () =>
      new Promise<Uint8Array>((resolve, reject) => {
        canvas.toBlob((blob) => {
          if (!blob) {
            reject(new Error("rasterizePdf: canvas.toBlob returned null"));
            return;
          }
          blob
            .arrayBuffer()
            .then((buf) => resolve(new Uint8Array(buf)))
            .catch(reject);
        }, "image/png");
      }),
  };
}

/**
 * Rasterize every page of a PDF to a PNG. `widthPt`/`heightPt` are the page's
 * viewport at scale 1 (PDF user units are points, 1/72 inch); the pixels are
 * rendered at `dpi/72` scale (default 150 DPI). Throws with a clear message on
 * failure — the importer catches and logs it.
 */
export async function rasterizePdf(
  bytes: Uint8Array,
  opts?: { dpi?: number },
): Promise<PdfPageRaster[]> {
  configureWorker();
  const dpi = opts?.dpi ?? 150;
  const scale = dpi / 72;

  let pdf: Awaited<ReturnType<typeof pdfjsLib.getDocument>["promise"]>;
  try {
    // `data` is consumed by pdf.js; hand it a copy so we never transfer /
    // detach the caller's buffer.
    pdf = await pdfjsLib.getDocument({ data: bytes.slice() }).promise;
  } catch (err) {
    throw new Error(`rasterizePdf: failed to parse PDF — ${String(err)}`);
  }

  try {
    const pages: PdfPageRaster[] = [];
    for (let n = 1; n <= pdf.numPages; n++) {
      const page = await pdf.getPage(n);
      const unit = page.getViewport({ scale: 1 });
      const viewport = page.getViewport({ scale });
      const target = makeTarget(
        Math.ceil(viewport.width),
        Math.ceil(viewport.height),
      );
      await page.render({
        // pdf.js accepts an Offscreen or DOM 2d context here; the cast in
        // makeTarget already normalised the type.
        canvasContext: target.ctx,
        viewport,
      }).promise;
      pages.push({
        widthPt: unit.width,
        heightPt: unit.height,
        pngBytes: await target.toPng(),
      });
      page.cleanup();
    }
    return pages;
  } catch (err) {
    throw new Error(`rasterizePdf: failed to render PDF pages — ${String(err)}`);
  } finally {
    await pdf.destroy();
  }
}
