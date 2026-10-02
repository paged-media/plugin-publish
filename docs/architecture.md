# Architecture

How this repository is built: foreign-format import and export for Paged, IDML in both
directions and PDF as import. It describes the code at commit `6994ad1`; the reason behind
each choice is in an ADR under [`adr/`](adr/README.md). A path starting with `src/` is inside
the crate or package under discussion. A path in the engine repository is written
`core: <path>` and was read at `core` commit `9f933f1`.

## Crates and packages

The repository is two workspaces side by side: a Cargo workspace with three crates under
`crates/` and a pnpm workspace with two packages under `packages/`. The Rust toolchain is
pinned to 1.94.1 with the `wasm32-unknown-unknown` target (`rust-toolchain.toml`). All three
crates are version `0.0.0` and `publish = false`.

**`crates/idml-import`** reads an IDML package into the engine's model. `src/lib.rs` holds
the ZIP reader (`open_source_archive`) and the orchestrator (`import_idml`,
`import_idml_doc`, `import_idml_archive`); one module per part parses the XML:
`designmap.rs`, `graphic.rs`, `styles.rs`, `spread.rs`, `story.rs`. The model types
themselves are defined in the engine's `paged-model` crate and re-exported from here.

**`crates/idml-export`** writes a `Document` back as an IDML package (`write_idml`,
`write_idml_with`) or as a `.paged` container (`write_paged`). It depends on `idml-import`
by path and re-parses source parts with it. `src/lib.rs` drives one writer, `write_package`;
`src/rewrite.rs` is the streaming rewrite of spread and story XML; `src/emit.rs` builds whole
parts for objects created after load; `src/paged.rs` adds the container manifest; each other
module owns one lane (fonts, images, guides, text wrap, transparency, navigation, resources).

**`crates/pdf-import`** (`cdylib` and `rlib`) maps a JSON intermediate representation to a
native document and returns `.paged` bytes (`pdf_ir_to_paged`). It contains no PDF reader.
The `wasm-bindgen` export `pdf_ir_to_paged_wasm` is compiled only for `wasm32`.

**`packages/publish-bundle`** is the plugin `media.paged.publish`, published as
`@paged-media/publish`. Three TypeScript files, no wasm: it registers an `.idml` importer
and an `.idml` exporter and forwards both to the engine.

**`packages/pdf-bundle`** is the plugin `media.paged.pdf`, published as `@paged-media/pdf`
(`dist`, `bin` and `manifest.json`). It holds the PDF reader (`src/pdfium.ts`), the text
reconstruction heuristics (`src/extract.ts`), the TypeScript copy of the intermediate
representation (`src/ir.ts`), the wasm loader (`src/engine-loader.ts`), the image fallback
(`src/idml-fallback.ts`) and the importer (`src/io/pdf.ts`). `bin/` holds two committed wasm
modules with their JavaScript glue: the built `pdf-import` mapper and a prebuilt PDFium. The
two bundles do not import each other and the editor loads them as two plugins
([ADR 657](adr/657-one-bundle-per-format.md)).

```
core (engine)                                    this repository
  paged-model, paged-scene, paged-store  <-- git rev --  idml-import <-- idml-export
                                                         pdf-import
  paged-canvas, paged-renderer,
  paged-sdk, paged-cli, ...              -- git rev -->  idml-import, idml-export
        | built into the engine wasm
        v
editor  <-- npm --  @paged-media/publish   TypeScript only
        <-- npm --  @paged-media/pdf       TypeScript + pdf-import wasm + PDFium wasm
```

- The crates take `paged-scene`, `paged-model` and `paged-store` from the engine repository
  as git dependencies on one revision (`Cargo.toml:24-29`): `idml-import` the first two,
  `idml-export` the first, `pdf-import` all three.
- The engine depends back on `idml-import` and `idml-export` by git revision, and patches
  the adapter's git dependencies onto its own local crates so both sides share one
  `Document` type (`core: Cargo.toml:71-82`). At that commit, six engine crates link
  `idml-import` as a normal dependency (`paged-canvas`, `paged-renderer`, `paged-sdk`,
  `paged-cli`, `paged-export-pdf`, `paged-introspect-wasm`) and two of them, `paged-canvas`
  and `paged-renderer`, also link `idml-export`. The revision the engine pins need not be
  this repository's head: at `core` `9f933f1` it is `a88315d`, the parent of the commit
  described here. See [ADR 650](adr/650-mutual-git-revision-pins.md).
- So the IDML code reaches a user inside the engine build, not inside
  `@paged-media/publish`. Only `pdf-import` is compiled to a wasm module that a bundle here
  ships. See [ADR 651](adr/651-idml-compiled-into-engine-wasm.md).
- No TypeScript is generated from Rust: the intermediate representation is written twice,
  in `crates/pdf-import/src/ir.rs` and `packages/pdf-bundle/src/ir.ts`.
- Towards the host, `@paged-media/plugin-api` and `@paged-media/plugin-sdk` are peer
  dependencies of both bundles. The PDF bundle also uses `jszip`, inlined into its build.

## Opening an IDML file

1. `@paged-media/publish` registers an importer for the extension `.idml` and the MIME type
   `application/vnd.adobe.indesign-idml-package`. The importer passes the file's bytes to
   `host.nativeDocument.open` (`packages/publish-bundle/src/io/idml.ts:55-70`).
2. The engine opens the bytes with `open_source_archive`: every ZIP entry is decompressed
   into memory, the `mimetype` entry must be the IDML one, and `designmap.xml` must exist.
   If the archive carries a native model part (`paged/core/model/document.pgm`) that decodes,
   the engine uses it; otherwise it calls `import_idml_archive`
   (`core: crates/paged-canvas/src/model.rs:1529-1561`).
3. `import_idml_archive` parses `designmap.xml`, then `Resources/Graphic.xml` and
   `Resources/Styles.xml` (each optional), reads conditions from both the designmap and the
   stylesheet, parses the master spreads, spreads and stories the designmap lists, and
   builds a `paged_scene::Document`. A listed part that the archive lacks is an error.
4. The raw archive stays with the caller, beside the model. Its entries are excluded from
   the model's serialisation (`crates/idml-import/src/lib.rs:89-107`).

## Exporting IDML

1. The exporter of `@paged-media/publish` sends the engine message `exportIdml` through the
   raw editor handle, `host.editor.client.send`, and returns the bytes of the `idmlExported`
   reply as `document.idml` (`packages/publish-bundle/src/io/idml.ts:83-90`).
2. The engine calls `idml_export::write_idml_with` with its document, the source package
   bytes it kept at load, and the font faces it has registered
   (`core: crates/paged-canvas/src/model.rs:4398-4408`, `:4477-4483`).
3. `write_package` (`crates/idml-export/src/lib.rs:342-944`) builds a replacement body for
   each part the model can change, and keeps it only if it differs from the source:
   - a spread runs through the frame-preference, guide, page-item, image, text-wrap and
     transparency passes; a master spread through the same passes except transparency;
   - a story gets its text destinations injected, then the range and text rewrite, then
     the font-face, dangling-style, cell-inset and row-attribute passes;
   - `Resources/Fonts.xml`, `Resources/Preferences.xml`, `designmap.xml`,
     `Resources/Graphic.xml` and `Resources/Styles.xml` are patched for fonts, sections,
     hyperlinks, bookmarks, conditions, new swatches and new styles.
4. A spread or story that has no source entry was created after load. It is written as a
   whole new part by `crates/idml-export/src/emit.rs` and referenced from `designmap.xml`.
5. The writer then walks the source archive in its original order. An entry with a
   replacement body is written deflated; every other entry is copied with its original
   compressed bytes (`raw_copy_file`). New parts are appended. A story that no frame
   references is left out together with its designmap reference.
6. With no link base given, which is how the bundle calls it, the bytes of placed images
   that the model holds are embedded in the package (`crates/idml-export/src/images.rs`).

To line source XML up with model items, the rewriters call `parse_spread_with_provenance`
and `parse_story_with_provenance` on the source part and look each element up: a page item
by its `Self` id, a text range by its byte offset. An element the parser did not model passes
through unchanged. See [ADR 652](adr/652-idml-save-back-patches.md); what InDesign reads is
[ADR 653](adr/653-indesign-is-the-oracle.md).

## The `.paged` container

`write_paged` (`crates/idml-export/src/paged.rs:200-280`) runs the same `write_package`
with container parts kept, then rewrites `manifest.json`: it merges into the existing
manifest the fields `v`, `format`, `pagedProtocol`, `idmlPartsHash` (FNV-1a over the IDML
parts), `domVersion` and a `parts` index of every entry under `paged/` with its owning
plugin, length and hash. `write_idml` is the same writer with those entries dropped. The
engine calls `write_paged` on save and adds its native model part
(`core: crates/paged-canvas/src/model.rs:4662-4682`). The format decision is recorded in
the engine repository as
[ADR 118](https://github.com/paged-media/core/blob/main/docs/adr/118-paged-file-is-a-valid-idml-package.md).
A second `.paged` writer exists in the engine, `paged_store::package::wrap_document`, for
documents that have no IDML source; the `pdf-import` crate source calls it.

## Opening a PDF

1. `@paged-media/pdf` registers an importer for `.pdf` and `application/pdf`
   (`packages/pdf-bundle/src/io/pdf.ts:117-139`).
2. Editable path. `loadPdfMapper` loads the `pdf-import` wasm on first use and keeps it.
   `extractPdf` loads PDFium on first use and walks each page: path objects become vector
   shapes (consecutive paths with the same fill, stroke and width merge into one), image
   objects become image frames, and the page's characters become text items.
   `itemsToPositionedFrames` groups the items into lines by baseline, splits a line at wide
   gaps, recovers spaces, merges items of one style into runs, and emits one text frame per
   line segment. The result is a `DocumentIr`: pages, each a list of text, image and vector
   frames in points with a top-left origin.
3. The bundle passes the IR as a JSON string to `pdf_ir_to_paged_wasm`.
   `build_document` (`crates/pdf-import/src/build.rs`) creates one single-page spread per
   IR page, a rectangle with inline image bytes per image, a text frame and a story per
   text frame, a polygon per vector shape, and one RGB swatch per distinct colour. It also
   fills the designmap's spread and story lists. In the crate source, `wrap_document`
   packages the model as a `.paged` file whose IDML half is a one-page skeleton; the
   committed binary in `bin/` was built before that call was introduced (commit `7b30b3d`).
4. The importer hands those bytes to `host.nativeDocument.open`.
5. Fallback path. If the mapper does not load, or extraction or mapping fails,
   `rasterizePdf` renders each page with PDFium at 150 dpi and `buildIdmlFromRasters`
   (`packages/pdf-bundle/src/idml-fallback.ts`) builds an IDML package in TypeScript with
   one full-page image rectangle per page, opened through the same door and parsed as IDML.

Decisions: the split between TypeScript and Rust, [ADR 654](adr/654-pdf-ir-and-mapper.md);
the reader, [ADR 655](adr/655-pdfium-reader.md); the two paths,
[ADR 656](adr/656-pdf-opens-as-native-document.md).

## Where data is stored

Neither bundle stores anything: no plugin metadata, no container parts, no host storage.
Each turns bytes into a package and hands it to the host. What the crates define is the
layout of the files they write: an `.idml` package (source entries, patched or copied, plus
new parts) and a `.paged` container (the same, plus `manifest.json` and entries under
`paged/`: `paged/<plugin>/...` for plugins, `paged/core/model/document.pgm` for the native
model). In memory, the engine keeps the original package bytes and the decompressed
`SourceArchive` beside the model while a document is open; the writer patches the former.

## Boundary to other repositories

| Door or API | Used by | For |
|---|---|---|
| `defineBundle` (`@paged-media/plugin-sdk`) | both bundles | the exported `publishBundle` and `pdfBundle` |
| `host.supports` | both bundles | probing `contribute.importer@1`, `contribute.exporter@1`, `document.openNative@1` |
| `host.contribute.importer` | both bundles | `.idml` and `.pdf` |
| `host.contribute.exporter` | publish bundle | `.idml` |
| `host.nativeDocument.open` | both bundles | replace the active document with a package |
| `host.editor.client.send` | publish bundle | the engine message `exportIdml`; the raw editor handle |
| `host.log` | both bundles | activation, a missing door, and the result or failure of an import |

Neither bundle contributes a panel, a command or a menu entry, and neither calls
`host.document`. The manifests declare `document.read: "broad"` and `openNative: true`; the
publish bundle also declares `readNative: true`, which no code path uses. The PDF manifest
declares one wasm module, `pdf-import` (at most 4 MiB). The PDFium module is not declared.

In the other direction this repository offers the engine a Rust API, not a door: the
functions named above, the model re-exports of `idml-import`, and `PAGED_PREFIX`, which the
engine uses for the plugin namespace of container parts (`core: crates/paged-canvas/src/model.rs:4606-4613`).

## Build and test

- `cargo build --workspace --all-targets` fetches the engine repository over https.
  `scripts/build-wasm.sh` builds `pdf-import` for `wasm32`, runs `wasm-bindgen --target web`
  and, if installed, `wasm-opt -Oz`, and writes `packages/pdf-bundle/bin/`. That output is
  committed; no workflow runs the script.
- `pnpm -r build` runs tsup in both packages (ESM plus type declarations). The PDF bundle
  keeps `?url` imports external, so the consuming bundler serves the two wasm files.
- `.github/workflows/ci.yml` has three jobs: format check, clippy, a native build and a
  `wasm32` build of all three crates; typecheck and build of both bundles plus manifest
  validation with `@paged-media/plugin-cli`; and `cargo nextest` over the workspace plus
  vitest in each package that declares a test script, which fails if any test failed.
  `.github/workflows/publish.yml` runs on every push to `main` and publishes each package
  whose version is not yet on npm under the dist-tag `canary`. No crate is published.
- Rust tests: unit tests inside `idml-import` and `idml-export`; three integration files in
  `crates/idml-import/tests/`, 44 in `crates/idml-export/tests/` and one in
  `crates/pdf-import/tests/`. One reads a package exported by InDesign 20.0.1,
  `crates/idml-export/tests/fixtures/indesign-20.0.1-navigation.idml`. Ten tests in
  `idml-export` are `#[ignore]`d: they read a private document corpus when
  `PAGED_IDML_CORPUS` is set and do not run in CI.
- TypeScript tests: two vitest files in `packages/pdf-bundle/test/` cover the text heuristics
  and the fallback builder. The PDFium path is browser-only and untested; the IDML bundle has no tests.
