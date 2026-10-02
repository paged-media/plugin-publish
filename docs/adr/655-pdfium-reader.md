# ADR 655 — PDFium is the PDF reader

- **Status:** Accepted. Recorded retroactively on 2026-10-02 from the code at `6994ad1`.
- **Scope:** `packages/pdf-bundle/src/pdfium.ts` and the two files it loads,
  `packages/pdf-bundle/bin/pdfium.esm.js` and `packages/pdf-bundle/bin/pdfium.esm.wasm`

## Context

The PDF bundle has to read a PDF in the browser twice over: to extract text, paths and
images for the editable import ([ADR 654](654-pdf-ir-and-mapper.md)), and to render whole
pages for the image fallback ([ADR 656](656-pdf-opens-as-native-document.md)).

The first versions, on 2026-07-23, used pdf.js (`pdfjs-dist`). Commit `85a24a6`, one day
later, replaced it. The reader's header gives the reason: PDFium "exposes a structured
CONTENT model (typed page objects with matrices, colours, path segments, and ORIGINAL
encoded image bytes) rather than a rendering op-list, which is exactly what faithful
decomposition wants — and it reads images as their original bytes with no re-decode (the
thing that stalled pdf.js)" (`packages/pdf-bundle/src/pdfium.ts:16-20`).

## Decision

The bundle reads PDFs with PDFium compiled to WebAssembly, taken as a prebuilt binary from
the `paulocoutinhox/pdfium-lib` project and committed under `packages/pdf-bundle/bin/`.
It is the only reader: it serves both the extraction and the page raster.

- **Loading.** `loadPdfium` imports the Emscripten glue, fetches the wasm through a
  bundler `?url` asset and passes the bytes in as `wasmBinary`. It runs on the first PDF
  opened, is memoised, and resolves to `null` when anything fails.
- **Calls.** The FPDF C API is wrapped with Emscripten's `cwrap`. The per-character text
  loop calls the raw wasm exports instead, because "`cwrap`'s per-call marshaling made
  this ~4× slower".
- **Text** comes from the character-level API: Unicode value, box, font size and fill
  colour per character. Lines and runs are rebuilt by the bundle's heuristics.
- **Paths** are read segment by segment with their matrix, fill colour, stroke colour and
  stroke width.
- **Images.** A JPEG stream (last filter `DCTDecode`) is passed through as its raw bytes.
  Any other image is decoded by PDFium to a bitmap and encoded to PNG through a canvas.
- **Page raster.** `rasterizePdf` renders a page with `FPDF_RenderPageBitmap` at 150 dpi
  by default and encodes it to PNG the same way.

## Evidence

- `packages/pdf-bundle/src/pdfium.ts:15-24` — the header: origin, reason, browser-only
- `packages/pdf-bundle/src/pdfium.ts:76-200` — `loadPdfium` and the wrapped calls
- `packages/pdf-bundle/src/pdfium.ts:508-551` — the character loop; raw exports at `:514-516`
- `packages/pdf-bundle/src/pdfium.ts:459-503` — path extraction
- `packages/pdf-bundle/src/pdfium.ts:576-608`, `:614-673` — JPEG passthrough, bitmap decode,
  the filter test
- `packages/pdf-bundle/src/pdfium.ts:221-276` — the page raster
- `packages/pdf-bundle/package.json:18-20`, `:33-37` — `jszip` is the only runtime
  dependency; `bin` is in the published file list
- `packages/pdf-bundle/manifest.json:9-16` — the one declared wasm artifact, `pdf-import`

## Alternatives considered

pdf.js was used and removed. Commit `85a24a6` deleted `raster.ts`, `reconstruct.ts` and
`graphics.ts` and the `pdfjs-dist` dependency, and its message says "the bundle is now
single-engine".

## Consequences

PDFium is a third-party binary in the repository and in the published package. The
repository contains no licence or notice file for it: the only licence files are `LICENSE`
and `LICENSE.md`, and neither mentions PDFium. The glue file carries no licence header. A
licence is named for it only in the message of commit `85a24a6`. Neither the code nor that
message records which release of `pdfium-lib` or which PDFium version the binary is.

The binary is not declared in the manifest. `capabilities.wasm` lists `pdf-import` only,
with a `maxBytes` of 4 194 304; `pdfium.esm.wasm` is 5 218 943 bytes and is loaded without a
declaration. [ADR 308](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/308-plugin-wasm.md)
records that nothing compares what a bundle loads with what it declares.

The reader runs only in a browser: it uses `fetch`, a `?url` asset that needs a bundler,
and a canvas for PNG encoding. No test in the repository imports `pdfium.ts`; the vitest
specs cover the heuristics and the fallback package only.

Not everything PDFium exposes is read. A form object contributes no path or image; it only
ends a run of merged paths (`packages/pdf-bundle/src/pdfium.ts:411-413`). No font name,
weight or style is read for text. An image whose bitmap PDFium cannot produce, or whose
bitmap format is not one of the four handled, is skipped.

Comments elsewhere still name pdf.js as the reader:
`packages/pdf-bundle/src/io/pdf.ts:22`, `packages/pdf-bundle/src/activate.ts:15-19`,
`scripts/build-wasm.sh:5-6`.

## Related

- [ADR 654](654-pdf-ir-and-mapper.md) — the representation this reader fills
- [ADR 656](656-pdf-opens-as-native-document.md) — the two import paths that call it
- [ADR 308](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/308-plugin-wasm.md) — plugin wasm as a declared capability under one size budget
