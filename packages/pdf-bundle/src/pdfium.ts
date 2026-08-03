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

// The PDF reader engine — Google's PDFium compiled to wasm (from
// paulocoutinhox/pdfium-lib). Replaces pdf.js: PDFium exposes a structured
// CONTENT model (typed page objects with matrices, colours, path segments, and
// ORIGINAL encoded image bytes) rather than a rendering op-list, which is
// exactly what faithful decomposition wants — and it reads images as their
// original bytes with no re-decode (the thing that stalled pdf.js).
//
// BROWSER-only: the 5.2 MB wasm is lazy-loaded on first PDF open (bundle `bin/`,
// glued via the `?url` asset the editor's Vite serves), and we call the FPDF C
// API through Emscripten `cwrap` + the HEAP views.

import type {
  DocumentIr,
  ImageFrameIr,
  PageIr,
  PointIr,
  SubpathIr,
  TextFrameIr,
  VectorIr,
} from "./ir";
import {
  DEFAULT_OPTIONS,
  itemsToPositionedFrames,
  type PositionedItem,
} from "./extract";
import type { PdfPageRaster } from "./idml-fallback";

// FPDF_PAGEOBJ_* object types.
const OBJ_PATH = 2;
const OBJ_IMAGE = 3;
const OBJ_FORM = 4;
// FPDF_SEGMENT_* path-segment types.
const SEG_LINETO = 0;
const SEG_BEZIERTO = 1;
const SEG_MOVETO = 2;

/** Minimal shape of the Emscripten module + the wrapped FPDF calls we use. */
interface Fpdf {
  M: {
    HEAPU8: Uint8Array;
    HEAPU32: Uint32Array;
    HEAPF32: Float32Array;
    HEAPF64: Float64Array;
    // The raw wasm exports — used directly (no `cwrap` marshaling) in the hot
    // per-char text loop. Every FPDF* export takes/returns plain numbers.
    wasmExports: {
      malloc: (n: number) => number;
      free: (p: number) => void;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      [fn: string]: (...a: number[]) => any;
    };
  };
  malloc: (n: number) => number;
  free: (p: number) => void;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [fn: string]: any;
}

let cached: Promise<Fpdf | null> | undefined;

/** Load + memoize the PDFium wasm engine, or `null` if it can't load. */
export async function loadPdfium(): Promise<Fpdf | null> {
  if (cached) return cached;
  cached = (async (): Promise<Fpdf | null> => {
    try {
      // @ts-ignore — committed Emscripten glue (bin/, gitignored from tsc).
      const factory = (await import("../bin/pdfium.esm.js")).default as (
        arg: Record<string, unknown>,
      ) => Promise<unknown>;
      // @ts-ignore — `?url` asset the bundler serves; hand the bytes in so the
      // module never does its own relative fetch (which 404s under Vite).
      const wasmUrl = (await import("../bin/pdfium.esm.wasm?url")).default as string;
      const wasmBinary = new Uint8Array(await (await fetch(wasmUrl)).arrayBuffer());
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const M = (await factory({ wasmBinary })) as any;
      const cw = (n: string, ret: string | null, args: string[]) => M.cwrap(n, ret, args);
      const f: Fpdf = {
        M,
        malloc: (n: number) => M.wasmExports.malloc(n),
        free: (p: number) => M.wasmExports.free(p),
        InitLibrary: cw("FPDF_InitLibrary", null, []),
        LoadMemDocument: cw("FPDF_LoadMemDocument", "number", ["number", "number", "number"]),
        LoadPage: cw("FPDF_LoadPage", "number", ["number", "number"]),
        ClosePage: cw("FPDF_ClosePage", null, ["number"]),
        CloseDoc: cw("FPDF_CloseDocument", null, ["number"]),
        PageCount: cw("FPDF_GetPageCount", "number", ["number"]),
        PageWidth: cw("FPDF_GetPageWidthF", "number", ["number"]),
        PageHeight: cw("FPDF_GetPageHeightF", "number", ["number"]),
        CountObjects: cw("FPDFPage_CountObjects", "number", ["number"]),
        GetObject: cw("FPDFPage_GetObject", "number", ["number", "number"]),
        ObjType: cw("FPDFPageObj_GetType", "number", ["number"]),
        GetMatrix: cw("FPDFPageObj_GetMatrix", "number", ["number", "number"]),
        GetBounds: cw("FPDFPageObj_GetBounds", "number", [
          "number",
          "number",
          "number",
          "number",
          "number",
        ]),
        GetFillColor: cw("FPDFPageObj_GetFillColor", "number", [
          "number",
          "number",
          "number",
          "number",
          "number",
        ]),
        GetStrokeColor: cw("FPDFPageObj_GetStrokeColor", "number", [
          "number",
          "number",
          "number",
          "number",
          "number",
        ]),
        GetStrokeWidth: cw("FPDFPageObj_GetStrokeWidth", "number", ["number", "number"]),
        PathCount: cw("FPDFPath_CountSegments", "number", ["number"]),
        PathSeg: cw("FPDFPath_GetPathSegment", "number", ["number", "number"]),
        SegPoint: cw("FPDFPathSegment_GetPoint", "number", ["number", "number", "number"]),
        SegType: cw("FPDFPathSegment_GetType", "number", ["number"]),
        SegClose: cw("FPDFPathSegment_GetClose", "number", ["number"]),
        TextLoadPage: cw("FPDFText_LoadPage", "number", ["number"]),
        TextClosePage: cw("FPDFText_ClosePage", null, ["number"]),
        // Char-level text API (Unicode via ToUnicode; per-object text is
        // glyph-encoded/empty, so we extract chars + group them ourselves).
        TextCountChars: cw("FPDFText_CountChars", "number", ["number"]),
        TextUnicode: cw("FPDFText_GetUnicode", "number", ["number", "number"]),
        TextCharBox: cw("FPDFText_GetCharBox", null, [
          "number",
          "number",
          "number",
          "number",
          "number",
          "number",
        ]),
        TextCharSize: cw("FPDFText_GetFontSize", "number", ["number", "number"]),
        TextCharFill: cw("FPDFText_GetFillColor", "number", [
          "number",
          "number",
          "number",
          "number",
          "number",
          "number",
        ]),
        ImgDataRaw: cw("FPDFImageObj_GetImageDataRaw", "number", ["number", "number", "number"]),
        ImgFilterCount: cw("FPDFImageObj_GetImageFilterCount", "number", ["number"]),
        ImgFilter: cw("FPDFImageObj_GetImageFilter", "number", [
          "number",
          "number",
          "number",
          "number",
        ]),
        // Page → bitmap raster (the fallback path; replaces pdf.js render).
        BitmapCreate: cw("FPDFBitmap_Create", "number", ["number", "number", "number"]),
        BitmapFillRect: cw("FPDFBitmap_FillRect", "number", [
          "number",
          "number",
          "number",
          "number",
          "number",
          "number",
        ]),
        RenderPageBitmap: cw("FPDF_RenderPageBitmap", null, [
          "number",
          "number",
          "number",
          "number",
          "number",
          "number",
          "number",
          "number",
        ]),
        BitmapGetBuffer: cw("FPDFBitmap_GetBuffer", "number", ["number"]),
        BitmapGetStride: cw("FPDFBitmap_GetStride", "number", ["number"]),
        BitmapGetWidth: cw("FPDFBitmap_GetWidth", "number", ["number"]),
        BitmapGetHeight: cw("FPDFBitmap_GetHeight", "number", ["number"]),
        BitmapGetFormat: cw("FPDFBitmap_GetFormat", "number", ["number"]),
        ImgGetBitmap: cw("FPDFImageObj_GetBitmap", "number", ["number"]),
        BitmapDestroy: cw("FPDFBitmap_Destroy", null, ["number"]),
      };
      f.InitLibrary();
      return f;
    } catch {
      return null;
    }
  })();
  return cached;
}

/** For tests — drop the memoized engine. */
export function _resetPdfium(): void {
  cached = undefined;
}

export interface RasterOptions {
  /** Render DPI (PDF user units are points, 1/72"). Default 150. */
  dpi?: number;
  /** Cap on pages rendered (safety for huge PDFs). Default 20. */
  maxPages?: number;
}

/**
 * Rasterize each PDF page to a PNG via PDFium's `FPDF_RenderPageBitmap` — the
 * Phase-0 image fallback (used when the editable reconstruction can't run).
 * Replaces the pdf.js renderer; PDFium renders BGRA, which we swap to RGBA and
 * encode to PNG through a canvas. `widthPt`/`heightPt` are the page size in
 * points; pixels are rendered at `dpi/72` scale.
 */
export async function rasterizePdf(
  bytes: Uint8Array,
  opts: RasterOptions = {},
): Promise<PdfPageRaster[]> {
  const f = await loadPdfium();
  if (!f) throw new Error("rasterizePdf: PDFium wasm failed to load");
  const scale = (opts.dpi ?? 150) / 72;
  const maxPages = opts.maxPages ?? 20;

  const buf = f.malloc(bytes.length);
  f.M.HEAPU8.set(bytes, buf);
  const doc = f.LoadMemDocument(buf, bytes.length, 0);
  if (!doc) {
    f.free(buf);
    throw new Error("rasterizePdf: PDFium could not open the document");
  }
  try {
    const count = Math.min(f.PageCount(doc), maxPages);
    const pages: PdfPageRaster[] = [];
    for (let n = 0; n < count; n++) {
      const page = f.LoadPage(doc, n);
      const widthPt = f.PageWidth(page);
      const heightPt = f.PageHeight(page);
      const w = Math.max(1, Math.round(widthPt * scale));
      const h = Math.max(1, Math.round(heightPt * scale));
      // alpha=1 → BGRA; fill opaque white, then render the page on top.
      const bitmap = f.BitmapCreate(w, h, 1);
      f.BitmapFillRect(bitmap, 0, 0, w, h, 0xffffffff);
      // flags=0x10 FPDF_ANNOT off; use FPDF_LCD_TEXT(2)|FPDF_ANNOT(1)? keep 0.
      f.RenderPageBitmap(bitmap, page, 0, 0, w, h, 0, 0);
      const ptr = f.BitmapGetBuffer(bitmap);
      const stride = f.BitmapGetStride(bitmap);
      const heap = f.M.HEAPU8;
      const rgba = new Uint8ClampedArray(w * h * 4);
      for (let y = 0; y < h; y++) {
        let src = ptr + y * stride;
        let dst = y * w * 4;
        for (let x = 0; x < w; x++) {
          rgba[dst] = heap[src + 2]; // R ← B
          rgba[dst + 1] = heap[src + 1]; // G
          rgba[dst + 2] = heap[src]; // B ← R
          rgba[dst + 3] = heap[src + 3]; // A
          src += 4;
          dst += 4;
        }
      }
      f.BitmapDestroy(bitmap);
      f.ClosePage(page);
      pages.push({ widthPt, heightPt, pngBytes: await encodePng(rgba, w, h) });
    }
    return pages;
  } finally {
    f.CloseDoc(doc);
    f.free(buf);
  }
}

/** Encode RGBA pixels to PNG bytes through a canvas (OffscreenCanvas when
 *  available — worker-safe — else a DOM `<canvas>`). Uses `createImageData` +
 *  `.data.set` rather than the `ImageData` constructor to sidestep the
 *  generic-typed-array mismatch on the constructor overload. */
async function encodePng(
  rgba: Uint8ClampedArray,
  w: number,
  h: number,
): Promise<Uint8Array> {
  const paint = (
    ctx: OffscreenCanvasRenderingContext2D | CanvasRenderingContext2D,
  ): void => {
    const img = ctx.createImageData(w, h);
    img.data.set(rgba);
    ctx.putImageData(img, 0, 0);
  };
  if (typeof OffscreenCanvas !== "undefined") {
    const canvas = new OffscreenCanvas(w, h);
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("rasterizePdf: OffscreenCanvas 2d unavailable");
    paint(ctx);
    const blob = await canvas.convertToBlob({ type: "image/png" });
    return new Uint8Array(await blob.arrayBuffer());
  }
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("rasterizePdf: canvas 2d unavailable");
  paint(ctx);
  return await new Promise<Uint8Array>((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (!blob) {
        reject(new Error("rasterizePdf: canvas.toBlob returned null"));
        return;
      }
      blob
        .arrayBuffer()
        .then((b) => resolve(new Uint8Array(b)))
        .catch(reject);
    }, "image/png");
  });
}

export interface ExtractOptions {
  maxPages?: number;
}

/** Read a PDFium 3×2 matrix (FS_MATRIX: a,b,c,d,e,f floats) out of a pointer. */
function readMatrix(f: Fpdf, p: number): number[] {
  const h = f.M.HEAPF32;
  const i = p >> 2;
  return [h[i], h[i + 1], h[i + 2], h[i + 3], h[i + 4], h[i + 5]];
}

/** Apply a PDFium matrix to an object-space point → top-left page points. */
function toPage(m: number[], x: number, y: number, pageH: number): PointIr {
  const px = m[0] * x + m[2] * y + m[4];
  const py = m[1] * x + m[3] * y + m[5];
  return { x_pt: px, y_pt: pageH - py };
}

/**
 * Extract a PDF into the Document IR by walking PDFium's page objects: text
 * objects → positioned text frames; path objects → vector shapes with exact
 * fill/stroke colours; image objects → placed images (original encoded bytes).
 */
export async function extractPdf(
  bytes: Uint8Array,
  opts: ExtractOptions = {},
): Promise<DocumentIr> {
  const f = await loadPdfium();
  if (!f) throw new Error("extractPdf: PDFium wasm failed to load");
  const maxPages = opts.maxPages ?? 12;

  const buf = f.malloc(bytes.length);
  f.M.HEAPU8.set(bytes, buf);
  const doc = f.LoadMemDocument(buf, bytes.length, 0);
  if (!doc) {
    f.free(buf);
    throw new Error("extractPdf: PDFium could not open the document");
  }

  // scratch out-param pointers (reused across the whole extraction)
  const p = {
    a: f.malloc(4),
    b: f.malloc(4),
    c: f.malloc(4),
    d: f.malloc(4),
    mat: f.malloc(24),
    dbl: f.malloc(32), // 4 doubles (FPDFText_GetCharBox out-params)
  };

  try {
    const total = f.PageCount(doc);
    const count = Math.min(total, maxPages);
    const pages: PageIr[] = [];
    for (let n = 0; n < count; n++) {
      const page = f.LoadPage(doc, n);
      const widthPt = f.PageWidth(page);
      const heightPt = f.PageHeight(page);
      const textPage = f.TextLoadPage(page);
      const frames: (TextFrameIr | VectorIr | ImageFrameIr)[] = [];

      // Vectors + images come from the page objects (paint order, behind text).
      // Consecutive path objects that share a paint style (fill/stroke/width)
      // are coalesced into ONE compound polygon (many subpaths) — a halftone
      // map or dot pattern is thousands of identical fills that render the same
      // whether drawn as 1 shape or 1000, so one editable object is both
      // faithful and far cheaper for the engine to build. An image/form breaks
      // the run (a real z-order boundary the merge must not cross).
      const objN = f.CountObjects(page);
      let runVector: VectorIr | null = null;
      let runKey = "";
      for (let i = 0; i < objN; i++) {
        const obj = f.GetObject(page, i);
        const type = f.ObjType(obj);
        if (type === OBJ_PATH) {
          const v = extractPath(f, obj, p, heightPt);
          if (!v) continue;
          const key = vectorStyleKey(v);
          if (runVector && key === runKey) {
            for (const sp of v.subpaths) runVector.subpaths.push(sp);
          } else {
            frames.push(v);
            runVector = v;
            runKey = key;
          }
        } else if (type === OBJ_IMAGE) {
          const img = await extractImage(f, obj, p, heightPt);
          if (img) frames.push(img);
          runVector = null; // z-order boundary — never merge across an image
          runKey = "";
        } else if (type === OBJ_FORM) {
          runVector = null; // opaque nested content — treat as a paint boundary
          runKey = "";
        }
      }

      // Text comes from the char-level API (proper Unicode) → grouped into
      // positioned lines via the shared heuristics, then drawn on top.
      const items = extractTextItems(f, textPage, p, heightPt);
      frames.push(
        ...(itemsToPositionedFrames(items, widthPt, heightPt, DEFAULT_OPTIONS) as TextFrameIr[]),
      );

      f.TextClosePage(textPage);
      f.ClosePage(page);
      pages.push({ width_pt: widthPt, height_pt: heightPt, frames });
    }
    return { pages };
  } finally {
    Object.values(p).forEach(f.free);
    f.CloseDoc(doc);
    f.free(buf);
  }
}

/** Read an object's fill or stroke colour (RGBA 0..255) as sRGB 0..1, or null
 *  when unset / fully transparent. */
function readColor(
  f: Fpdf,
  get: (o: number, r: number, g: number, b: number, a: number) => number,
  obj: number,
  p: { a: number; b: number; c: number; d: number },
): [number, number, number] | null {
  if (!get(obj, p.a, p.b, p.c, p.d)) return null;
  const h = f.M.HEAPU32;
  const alpha = h[p.d >> 2];
  if (alpha === 0) return null;
  return [h[p.a >> 2] / 255, h[p.b >> 2] / 255, h[p.c >> 2] / 255];
}

/** A paint-style fingerprint for a vector: two vectors with the same key fill
 *  and stroke identically, so a run of them can safely coalesce into one shape. */
function vectorStyleKey(v: VectorIr): string {
  const c = (a?: [number, number, number]): string =>
    a ? a.map((x) => Math.round(x * 255)).join(",") : "-";
  return `f${c(v.fill_rgb)}|s${c(v.stroke_rgb)}|w${v.stroke_width_pt ?? 0}`;
}

function extractPath(
  f: Fpdf,
  obj: number,
  p: { a: number; b: number; c: number; d: number; mat: number },
  pageH: number,
): VectorIr | null {
  f.GetMatrix(obj, p.mat);
  const m = readMatrix(f, p.mat);
  const segN = f.PathCount(obj);
  if (segN <= 0) return null;

  const subpaths: SubpathIr[] = [];
  let cur: SubpathIr | null = null;
  for (let j = 0; j < segN; j++) {
    const seg = f.PathSeg(obj, j);
    if (!seg) continue;
    f.SegPoint(seg, p.a, p.b);
    const x = f.M.HEAPF32[p.a >> 2];
    const y = f.M.HEAPF32[p.b >> 2];
    const st = f.SegType(seg);
    const pt = toPage(m, x, y, pageH);
    if (st === SEG_MOVETO) {
      if (cur && cur.points.length > 0) subpaths.push(cur);
      cur = { points: [pt], closed: false };
    } else if (st === SEG_LINETO || st === SEG_BEZIERTO) {
      // Béziers flatten to their endpoint (curve fidelity is a refinement).
      if (cur) cur.points.push(pt);
    }
    if (f.SegClose(seg) && cur) cur.closed = true;
  }
  if (cur && cur.points.length > 0) subpaths.push(cur);
  if (subpaths.every((s) => s.points.length < 2)) return null;

  const fill = readColor(f, f.GetFillColor, obj, p);
  const stroke = readColor(f, f.GetStrokeColor, obj, p);
  if (!fill && !stroke) return null;
  const v: VectorIr = { kind: "vector", subpaths };
  if (fill) v.fill_rgb = fill;
  if (stroke) {
    v.stroke_rgb = stroke;
    f.GetStrokeWidth(obj, p.a);
    v.stroke_width_pt = Math.max(0.1, f.M.HEAPF32[p.a >> 2]);
  }
  return v;
}

/** Char-level text extraction: each glyph → a positioned item (proper Unicode
 *  via ToUnicode, exact box + size + fill colour). The shared heuristics then
 *  group these into lines/runs. Control chars (newlines pdfium inserts) skip. */
function extractTextItems(
  f: Fpdf,
  textPage: number,
  p: { a: number; b: number; c: number; d: number; dbl: number },
  pageH: number,
): PositionedItem[] {
  // The per-char loop runs thousands of times per page, so call the RAW wasm
  // exports directly — `cwrap`'s per-call marshaling made this ~4× slower.
  const raw = f.M.wasmExports;
  const n = raw.FPDFText_CountChars(textPage);
  const items: PositionedItem[] = [];
  const h64 = f.M.HEAPF64;
  const hu = f.M.HEAPU32;
  const dbl = p.dbl >> 3;
  const L = p.dbl;
  const R = p.dbl + 8;
  const B = p.dbl + 16;
  const T = p.dbl + 24;
  for (let i = 0; i < n; i++) {
    const u = raw.FPDFText_GetUnicode(textPage, i);
    if (u < 32) continue; // skip newlines/control (grouping re-derives lines)
    raw.FPDFText_GetCharBox(textPage, i, L, R, B, T); // doubles: left,right,bottom,top
    const left = h64[dbl];
    const right = h64[dbl + 1];
    const bottom = h64[dbl + 2];
    const top = h64[dbl + 3];
    const size = raw.FPDFText_GetFontSize(textPage, i);
    let colorRgb: [number, number, number] | undefined;
    if (raw.FPDFText_GetFillColor(textPage, i, p.a, p.b, p.c, p.d) && hu[p.d >> 2] !== 0) {
      colorRgb = [hu[p.a >> 2] / 255, hu[p.b >> 2] / 255, hu[p.c >> 2] / 255];
    }
    items.push({
      text: String.fromCharCode(u),
      xPt: left,
      baselineTopY: pageH - bottom, // box bottom ≈ baseline (top-down)
      widthPt: Math.max(0, right - left),
      fontSizePt: size > 1 ? size : Math.max(1, top - bottom),
      bold: false,
      italic: false,
      colorRgb,
    });
  }
  return items;
}

async function extractImage(
  f: Fpdf,
  obj: number,
  p: { a: number; b: number; c: number; d: number; mat: number },
  pageH: number,
): Promise<ImageFrameIr | null> {
  // Placement: the image's unit square [0,1]² mapped by its matrix → page.
  f.GetMatrix(obj, p.mat);
  const m = readMatrix(f, p.mat);
  const corners = [
    toPage(m, 0, 0, pageH),
    toPage(m, 1, 0, pageH),
    toPage(m, 1, 1, pageH),
    toPage(m, 0, 1, pageH),
  ];
  const xs = corners.map((c) => c.x_pt);
  const ys = corners.map((c) => c.y_pt);
  const left = Math.min(...xs);
  const right = Math.max(...xs);
  const top = Math.min(...ys);
  const bottom = Math.max(...ys);
  if (right - left <= 0.5 || bottom - top <= 0.5) return null;

  // JPEG (DCTDecode) streams pass straight through — the engine decodes
  // them natively. Every OTHER filter (Flate, LZW, RunLength, JBIG2, …)
  // goes through PDFium's own decode: FPDFImageObj_GetBitmap → RGBA →
  // PNG. That closes the "non-JPEG images drop" gap the coverage spec
  // named as the real remaining import item.
  if (!isJpeg(f, obj)) {
    const png = await imageBitmapToPng(f, obj);
    if (!png) return null;
    return {
      kind: "image",
      x_pt: left,
      y_pt: top,
      width_pt: right - left,
      height_pt: bottom - top,
      png_b64: toBase64(png),
    };
  }
  const len = f.ImgDataRaw(obj, 0, 0);
  if (len <= 0) return null;
  const bufP = f.malloc(len);
  f.ImgDataRaw(obj, bufP, len);
  const raw = f.M.HEAPU8.slice(bufP, bufP + len);
  f.free(bufP);

  return {
    kind: "image",
    x_pt: left,
    y_pt: top,
    width_pt: right - left,
    height_pt: bottom - top,
    png_b64: toBase64(raw), // JPEG bytes; the engine sniffs the format
  };
}

/** Decode a non-JPEG image object through PDFium's bitmap lane and encode
 *  PNG. Formats per FPDFBitmap_GetFormat: 1=Gray, 2=BGR, 3=BGRx, 4=BGRA
 *  (the same swizzle the page-raster fallback uses). Returns null when
 *  PDFium can't produce a bitmap (the honest skip — never a fake frame). */
async function imageBitmapToPng(f: Fpdf, obj: number): Promise<Uint8Array | null> {
  const bitmap = f.ImgGetBitmap(obj);
  if (!bitmap) return null;
  try {
    const w = f.BitmapGetWidth(bitmap) as number;
    const h = f.BitmapGetHeight(bitmap) as number;
    const stride = f.BitmapGetStride(bitmap) as number;
    const format = f.BitmapGetFormat(bitmap) as number;
    const buf = f.BitmapGetBuffer(bitmap) as number;
    if (w <= 0 || h <= 0 || !buf) return null;
    const heap = f.M.HEAPU8;
    const rgba = new Uint8ClampedArray(w * h * 4);
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const dst = (y * w + x) * 4;
        if (format === 1) {
          const g = heap[buf + y * stride + x];
          rgba[dst] = g;
          rgba[dst + 1] = g;
          rgba[dst + 2] = g;
          rgba[dst + 3] = 255;
        } else if (format === 2) {
          const src = buf + y * stride + x * 3;
          rgba[dst] = heap[src + 2];
          rgba[dst + 1] = heap[src + 1];
          rgba[dst + 2] = heap[src];
          rgba[dst + 3] = 255;
        } else if (format === 3 || format === 4) {
          const src = buf + y * stride + x * 4;
          rgba[dst] = heap[src + 2];
          rgba[dst + 1] = heap[src + 1];
          rgba[dst + 2] = heap[src];
          rgba[dst + 3] = format === 4 ? heap[src + 3] : 255;
        } else {
          return null; // unknown format — honest skip
        }
      }
    }
    return await encodePng(rgba, w, h);
  } finally {
    f.BitmapDestroy(bitmap);
  }
}

/** True when the image's (last) filter is DCTDecode — i.e. a JPEG stream we can
 *  pass straight through as `image_bytes`. */
function isJpeg(f: Fpdf, obj: number): boolean {
  const n = f.ImgFilterCount(obj);
  if (n <= 0) return false;
  const cap = 32;
  const bufP = f.malloc(cap);
  const len = f.ImgFilter(obj, n - 1, bufP, cap);
  let name = "";
  const heap = f.M.HEAPU8;
  for (let i = 0; i + 1 < len - 2; i += 2) {
    name += String.fromCharCode(heap[bufP + i] | (heap[bufP + i + 1] << 8));
  }
  f.free(bufP);
  return name === "DCTDecode";
}


function toBase64(bytes: Uint8Array): string {
  if (typeof Buffer !== "undefined") return Buffer.from(bytes).toString("base64");
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}
