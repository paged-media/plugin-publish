# ADR 654 — PDF understanding lives in TypeScript; a PDF-blind mapper builds the model

- **Status:** Accepted. Recorded retroactively on 2026-10-02 from the code at `6994ad1`.
- **Scope:** `crates/pdf-import`, and in `packages/pdf-bundle` the files `src/ir.ts`,
  `src/extract.ts`, `src/pdfium.ts` and `src/engine-loader.ts`

## Context

Opening a PDF as an editable document needs two kinds of work. One is reading the PDF and
guessing structure from it: which glyphs form a line, where a space was, which paths belong
together. The other is building a document in the engine's model types, which are Rust
types that change with the engine ([ADR 650](650-mutual-git-revision-pins.md)).

The crate header gives the reason for where the line was drawn. The mapper is
"Deliberately tiny" and "knows nothing about PDF"; the reader and the heuristics live in
the TypeScript bundle, "so the wasm stays small and rebuilds rarely, and a model-shape
change is a compile error here rather than silent data loss in hand-rolled JSON"
(`crates/pdf-import/src/lib.rs:24-27`).

## Decision

PDF import is cut in two at one intermediate representation, the Document IR. TypeScript
produces it; Rust consumes it and knows nothing else about the source.

- **TypeScript side.** `extractPdf` walks the PDF ([ADR 655](655-pdfium-reader.md)) and
  returns a `DocumentIr`. The text heuristics are in `extract.ts`, a module with no reader
  dependency: line grouping by baseline, space recovery from gaps, merging of same-style
  runs, splitting a line at column gaps, and snapping per-glyph sizes to the dominant size
  of a segment. Consecutive paths with the same paint are merged in `pdfium.ts`.
- **The IR.** Pages in order; each page has a size and an ordered list of frames tagged
  `text`, `image` or `vector`. Units are points, the origin is top-left and y grows
  downward. It has no version field.
- **Rust side.** `pdf_ir_to_paged(ir_json)` deserialises the JSON, builds a
  `paged_scene::Document` (one single-page spread per IR page; text frames with
  stories, rectangles with image bytes, polygons with swatches) and packages it
  ([ADR 656](656-pdf-opens-as-native-document.md)).
- **The boundary** is one call, `pdf_ir_to_paged_wasm(JSON.stringify(ir))`, which returns
  bytes or throws. The IR is declared twice, in `ir.ts` and `ir.rs`, and kept in step by
  hand.

## Evidence

- `crates/pdf-import/src/lib.rs:15-27`, `:47-63`, `:69-75` — the header and its reason; the
  single entry; the wasm export
- `crates/pdf-import/src/ir.rs:15-25`, `:29-57` — the IR as the seam; its top-level shape
- `packages/pdf-bundle/src/ir.ts:15-19` — the TypeScript twin, which "MUST stay in lockstep"
- `packages/pdf-bundle/src/extract.ts:15-25`, `:72-100`, `:182-258` — the heuristics module
- `packages/pdf-bundle/src/pdfium.ts:345-428` — extraction into the IR; paths merged at `:382-405`
- `packages/pdf-bundle/src/engine-loader.ts:106-108`, `crates/pdf-import/src/build.rs:104-198`
  — the boundary call; IR to `Document`
- `packages/pdf-bundle/test/extract.test.ts:17-28`,
  `crates/pdf-import/tests/build_roundtrip.rs:30-59` — each side is tested on its own input

## Alternatives considered

Writing the native model as JSON by hand from TypeScript is the alternative the header
names and rejects. When the PDF reader was replaced, commit `85a24a6` recorded that the cut
held: "only the TS extraction layer changed; the pdf-import Rust mapper + the editor are
untouched."

## Consequences

A change to the IR is two edits. `ir.ts` says a mismatch "is a serde error at the wasm
boundary". That holds for a missing required field or a wrong type. The Rust types do not
reject unknown fields and many fields have a serde default, so a field that exists on one
side only can pass without an error. No test feeds the output of the TypeScript extractor
to the Rust mapper: the vitest specs import only `extract.ts` and `idml-fallback.ts`, and
the Rust test uses JSON written by hand.

The IR carries more than the current reader fills. `pdfium.ts` sets `bold` and `italic`
to `false` for every glyph, never sets a font family and never sets `background_png_b64`;
the Rust side still honours all four (`packages/pdf-bundle/src/pdfium.ts:539-548`,
`crates/pdf-import/src/build.rs:128-143`, `:256-258`). Every line and Bézier segment
contributes one point (`packages/pdf-bundle/src/pdfium.ts:483-485`), so vector shapes
arrive as polylines.

The promise of "a compile error here" covers the crate, not the shipped binary. The mapper
wasm in `packages/pdf-bundle/bin/` is a committed product of `scripts/build-wasm.sh`, last
changed in commit `0c64e7d` (2026-07-23). `crates/pdf-import` has five later commits and
no workflow rebuilds the binary. `scripts/build-wasm.sh:12` and
`packages/pdf-bundle/src/engine-loader.ts:38` call `bin/` gitignored; it is tracked.

Comments still name the earlier reader: `crates/pdf-import/src/lib.rs:16`, `:25`,
`crates/pdf-import/src/ir.rs:16-22`, `crates/pdf-import/Cargo.toml:8` and
`packages/pdf-bundle/src/extract.ts:18-19` refer to pdf.js or to `reconstruct.ts`, a file
that no longer exists.

## Related

- [ADR 655](655-pdfium-reader.md), [ADR 656](656-pdf-opens-as-native-document.md) — the reader that fills the IR; what happens to the mapper's output
- [ADR 308](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/308-plugin-wasm.md) — how plugin wasm is declared, shipped and loaded
- [ADR 314](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/314-plugin-shape.md) — the plugin shape shared by the plugin family; here the heuristics are in TypeScript
