# Status

What this repository ships and what it does not, read from the code at commit `6994ad1`
(`packages/publish-bundle/package.json` 0.1.1-canary.0, `packages/pdf-bundle/package.json`
0.2.6-canary.0). How the parts fit is in [`architecture.md`](architecture.md).

## Shipped

- **Open an IDML file.** A `.idml` file is routed to the `media.paged.publish` importer and
  replaces the active document. The parse is `idml-import`, linked into the engine. It
  reads layers, sections, hyperlinks, bookmarks and text variables; swatches and gradients;
  paragraph, character, object, cell and table styles and conditions; pages, master
  spreads, text frames, rectangles, ovals, polygons, lines, groups and guides; stories with
  runs, tables, footnotes and anchored frames.
- **Export IDML.** The `media.paged.publish` exporter returns a `.idml` written by
  `idml-export` inside the engine. Untouched entries keep their original bytes. The writer
  saves edits to page items and text; page items, pages and stories created after load;
  new swatches and styles; guides, text wrap and image links. Sections, hyperlinks,
  bookmarks, conditions and fonts are written in the spelling InDesign 20.0.1 was measured
  to read.
- **Save a `.paged` container.** The engine's save calls `write_paged`: the IDML parts,
  plugin and native-model parts under `paged/`, and a `manifest.json` with a hash of the
  IDML parts and an index of the container parts.
- **Open a PDF as an editable document.** The `media.paged.pdf` importer reads the PDF with
  PDFium and opens a native document with one page per PDF page: text frames with size and
  fill colour, vector shapes with fill, stroke and stroke width, and placed images.
- **Open a PDF as page images.** When the editable path fails, each page is rendered at
  150 dpi and opened as one full-page image.

## Pending release: core protocol 71

The `om/object-model` branch carries IDML export fixes that need core protocol 71 (core
branch `om/protocol-71`, not yet tagged). They do not build against the current core pin
(`f6a8c56`, protocol 70): the model field `Spread::item_names` and the store reader
`paged_store::from_parts` exist from protocol 71 on. Built and tested against a local
checkout of that core branch:

- **Hidden and locked page items.** `Visible` and `Locked` follow the model on items the
  source carried and on items created since load. Before, a hidden item came back visible.
- **Authored text variables.** A variable created in the model is written as a
  `<TextVariable>` definition in `designmap.xml`, in InDesign's own spelling; a changed one
  is rewritten and a deleted one dropped. An instance inserted into text is written as a
  `<TextVariableInstance>` in a range of its own, in a source story or in a story written
  whole. The importer reads both back.
- **Text on a path with no end bracket.** `EndBracket` is written as the host path's arc
  length. InDesign reads a missing `EndBracket` as 0 and shows no text.
- **Page-item names.** The `item-names` feature gate is removed: `Name` is always read
  into `Spread::item_names` and written for new and renamed items.
- **PDF import test.** The model part keeps its images as separate blob parts (pgm v4), so
  the round-trip test reads it with `paged_store::from_parts`.

InDesign 2025 (20.0.1) opened an export with all four changes, saved it as `.indd` and
reopened it. The hidden item stayed hidden, the locked frame stayed locked, the variable
showed `9.99`, the text sat on the path without overset, and the new name stayed. The
recording is `crates/idml-export/tests/fixtures/indesign-2025-object-model/verify.json`.
`crates/idml-export/tests/indesign/probe.sh` re-records it.

### Release order

The two repositories pin each other by git revision, so the steps run in this order:

1. **core:** merge `om/protocol-71` into `main` and tag `v0.71.0`. Call that commit `C`.
2. **plugin-publish:** in the workspace `Cargo.toml`, move the three core pins
   (`paged-model`, `paged-scene`, `paged-store`) to `rev = "C"` and update `Cargo.lock`. No
   local `[patch]` block may be committed. Run `cargo test --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`,
   then commit the pin together with this branch, so `main` never holds the ungated code on
   the old pin. Call the merged commit `P`.
3. **core:** move the eight `idml-import` / `idml-export` git pins
   (`paged-canvas` ×2, `paged-cli`, `paged-export-pdf`, `paged-gen`, `paged-introspect`,
   `paged-introspect-wasm`, `paged-mutate`) to `rev = "P"`. The pinned-defect assertions
   in `crates/paged-canvas/tests/idml_export_object_model_state.rs` then fail. Change them
   to assert the fix:
   - `a_hidden_item_is_visible_false_in_the_exported_spread` should assert
     `Visible="false"`;
   - `an_authored_text_variable_reaches_the_idml` should assert that both the definition
     and the instance are written.
   Then commit and push, and check that CI passes.
4. **core:** tag the patch release `v0.71.1`. The wire protocol does not change, so the
   plugin SDK's vendored contract stays as it is. Downstream, the editor moves its engine
   pins, and plugin-draw drops its `textPathEndBracketAbsent` finding, which now flips.

## Limits of what is shipped

- **The IDML code ships with the engine, not with the bundle.** `@paged-media/publish`
  carries no parser. A fix here reaches users only after the engine pins a new revision of
  this repository and is released.
- **IDML export leaves the plugin contract.** The exporter calls the raw editor handle
  (`host.editor.client.send`). It returns only the package bytes: the list of losses the
  engine sends with them is not passed on, a failed export returns `null` without a log
  line, and the file name is always `document.idml`. Images the model holds as bytes are
  embedded.
- **What the writer does not write.** A page removed from the model keeps its entry and its
  designmap reference. A master spread that has no source entry is not written, and the
  transparency pass does not run on master spreads. A run whose body holds other inline
  markup, such as a page-number marker, a text variable or an anchored frame, keeps its
  source text.
- **Byte identity has stated exceptions.** It holds for an unmutated source already in
  InDesign's spelling, with two exceptions: a story that no frame references is dropped,
  and `Resources/Fonts.xml` gains every applied face it does not declare. A source in the
  engine's older spelling is rewritten on every export.
- **The writer needs the original package.** It patches a source; it cannot produce an IDML
  package from a model alone.
- **PDF page caps.** The editable path reads at most 12 pages and the fallback at most 20.
  The importer does not log that pages were left out.
- **PDF text.** One text frame per line segment, one paragraph per frame. A run carries its
  text, size and fill colour; font family, bold and italic are never set.
- **PDF graphics.** Curve segments become straight polygon vertices. An image is placed in
  the bounding box of its transform, so rotation and skew are lost. Paths and images inside
  a form object are not read. The intermediate representation has no field for gradients,
  patterns, clipping, opacity or dashes, and a colour's partial transparency is dropped.
  On each page, text is stacked above all vectors and images.
- **Both PDF paths need PDFium.** If its wasm cannot load, the file does not open and an
  error is logged. No password is passed to PDFium.
- **The committed mapper wasm is older than its source.**
  `packages/pdf-bundle/bin/pdf_import_bg.wasm` was last committed on 2026-07-23;
  `crates/pdf-import` has changed since, most recently on 2026-10-01. No workflow rebuilds
  the file or compares it with the crate.
- **PDFium is vendored as a binary.** The 5.2 MB module is not declared in the manifest's
  `capabilities.wasm`, and the repository holds no notice file for it.
- **Manifests and comments lag.** The manifest versions (0.1.0-canary.0 and 0.2.3-canary.0)
  are behind the package versions. `readNative` is declared and unused. Comments in
  `crates/pdf-import`, `scripts/build-wasm.sh` and `packages/pdf-bundle/src/` still name
  pdf.js as the reader, and two call `bin/` gitignored; it is tracked.
- **Test coverage.** CI does not run InDesign (one recorded InDesign probe is asserted, see
  above), the ten corpus tests do not run in CI, the PDFium path has no test, and
  `packages/publish-bundle` has no tests.

## Not built

- PDF export. The PDF bundle contributes no exporter and nothing here writes PDF; the
  engine has its own exporter
  ([core ADR 119](https://github.com/paged-media/core/blob/main/docs/adr/119-pdf-export-backend.md)).
- A host door for IDML export, or the IDML crates shipped as wasm inside
  `@paged-media/publish`; the bundle's comments name both as the successors of the raw
  handle ([ADR 651](adr/651-idml-compiled-into-engine-wasm.md)).
- Merging PDF lines into flowing paragraphs: every line segment stays its own frame.
- A page-image background under a partly recovered PDF page: the field
  `background_png_b64` and the Rust code that draws it exist, but the bundle never sets it.
- Removing a page, or creating a master spread, on IDML export.
- A build of the shipped `pdf-import` artifact (`scripts/build-wasm.sh`, the `wasm-bindgen`
  output in `packages/pdf-bundle/bin/`) in CI or in the publish workflow. CI compiles the
  crate for `wasm32` as a check only.
