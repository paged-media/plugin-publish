# ADR 656 — A PDF opens as a native document: editable first, page images as the fallback

- **Status:** Accepted. Recorded retroactively on 2026-10-02 from the code at `6994ad1`.
- **Scope:** `packages/pdf-bundle` (`@paged-media/pdf`, plugin id `media.paged.pdf`), and
  the packaging step in `crates/pdf-import/src/lib.rs`

## Context

The bundle registers an importer for `.pdf` through the host's importer door
([ADR 017](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/017-importer-exporter-door-shape.md)).
The door it opens the result with, `host.nativeDocument.open`, replaces the active document
with the package it is given (`plugin-sdk: packages/plugin-api/src/host.ts:1320-1322`). So
the importer has to produce a complete document.

The first version (commit `e1e5318`, 2026-07-23) rendered each page to a PNG and wrapped
the images in a minimal IDML package. The editable path was added on top of it, and the
importer's header keeps the image path "so a PDF always opens *something* real"
(`packages/pdf-bundle/src/io/pdf.ts:27-30`). The editable path produces the engine's native
model directly, "no IDML round-trip" (`Cargo.toml:26-27`). That is stated as a property.
The repository does not record why.

## Decision

The importer builds a whole package in the plugin and passes it to
`host.nativeDocument.open`. It tries the editable package first and falls back to page
images. It never throws past the host: failures are logged.

- **Editable.** `extractPdf` produces the Document IR and the mapper wasm turns it into a
  `.paged` package ([ADR 654](654-pdf-ir-and-mapper.md)). In the crate source the package
  is written by the engine's shared writer, `paged_store::package::wrap_document`: the
  model in `paged/core/model/document.pgm`, plus a one-page IDML skeleton the size of the
  first page. The committed mapper binary in `packages/pdf-bundle/bin/` predates that
  change (commit `7b30b3d`). The engine takes the model from the native part and does not
  parse the skeleton's spreads or stories; it still opens the package with the IDML
  reader and reads `Resources/Styles.xml` to fill style leading the native part lacks
  (`core: crates/paged-canvas/src/model.rs:1546`, `:9912-9929`).
- **Fallback.** When the mapper does not load, returns nothing, or extraction throws,
  `rasterizePdf` renders the pages and `buildIdmlFromRasters` builds an IDML package in
  TypeScript with JSZip: one spread per page, each a full-page rectangle with the PNG
  inline. The engine opens it through its ordinary IDML import.
- **Gates.** Registration requires `contribute.importer@1`; opening requires
  `document.openNative@1`. Without either the bundle logs a warning and does nothing.
  The bundle contributes no exporter: PDF is import-only here.

## Evidence

- `packages/pdf-bundle/src/io/pdf.ts:59-88`, `:93-110` — `importPdf`: the gate, the editable
  attempt, the fallback, the catch; `tryEditableImport` returns `null` instead of throwing
- `packages/pdf-bundle/src/engine-loader.ts:21-26`, `:95-125` — the mapper loader resolves
  to `null` on any failure
- `crates/pdf-import/src/lib.rs:49-63` — IR to `Document` to `wrap_document`
- `core: crates/paged-store/src/package.rs:15-23`, `:121-166` — the shared writer and its
  fallback skeleton
- `core: crates/paged-canvas/src/model.rs:1520-1558` — the load path prefers the native part
- `packages/pdf-bundle/src/idml-fallback.ts:15-33`, `:234` — the TypeScript IDML package
- `packages/pdf-bundle/manifest.json:7-20` — capabilities; one importer, no exporter

## Alternatives considered

A container writer private to the crate (`crates/pdf-import/src/ocf.rs`) was deleted in
commit `7b30b3d` in favour of the engine's writer, which is "shared with every native
producer, so the format cannot drift between plugins" (`crates/pdf-import/src/lib.rs:52-53`).
A full-page raster kept beneath low-confidence pages is still in the IR and the mapper
(`crates/pdf-import/src/ir.rs:35-44`); the current reader never supplies it.

## Consequences

Long documents are truncated without notice. `extractPdf` reads at most 12 pages and
`rasterizePdf` at most 20 unless given an option, and the importer calls both without
options (`packages/pdf-bundle/src/pdfium.ts:228`, `:351`;
`packages/pdf-bundle/src/io/pdf.ts:77`, `:100`). The log line reports the number of pages
imported; nothing reports that the PDF had more.

The fallback does not cover a failure of the reader. Both paths use PDFium
([ADR 655](655-pdfium-reader.md)); if it does not load, or cannot open the file, the
fallback throws as well, an error is logged and nothing opens.

The mapper has to be built against an engine revision whose native model encoding the
running engine can decode. When it cannot, the engine falls back to the IDML skeleton: one
blank page of the first page's size (`crates/pdf-import/src/lib.rs:52-56`,
`crates/pdf-import/tests/build_roundtrip.rs:209-254`). The committed mapper binary is
older than the crate ([ADR 654](654-pdf-ir-and-mapper.md)).

Comments describe earlier states. The doc comment on `importPdf`
(`packages/pdf-bundle/src/io/pdf.ts:51-58`) and the header of
`packages/pdf-bundle/src/activate.ts` (`:15-22`) describe the image path only;
`packages/pdf-bundle/src/io/pdf.ts:24-25` describes a per-page confidence gate that the
reader no longer implements. The manifest version (`0.2.3-canary.0`) is behind the package
version (`0.2.6-canary.0`).

## Related

- [ADR 654](654-pdf-ir-and-mapper.md), [ADR 655](655-pdfium-reader.md) — the mapper and the reader
- [ADR 118](https://github.com/paged-media/core/blob/main/docs/adr/118-paged-file-is-a-valid-idml-package.md) — the `.paged` container the editable path produces
- [ADR 119](https://github.com/paged-media/core/blob/main/docs/adr/119-pdf-export-backend.md) — PDF export, which is the engine's and not this bundle's
- [ADR 305](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/305-doors-always-present.md) — doors that are always present and report support
