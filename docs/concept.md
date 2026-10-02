# Concept

Why this repository exists, what it is for, and what it will never do. Each paragraph names
its source in a comment. The README describes an early state of the IDML half only; where it
and the code disagree, this page follows the code.

## Why it exists

Paged keeps a document in its own native model. That model, rendering and mutation live in
the engine repository, `paged-media/core`. This repository is import and export only: it maps
IDML packages to and from the native `Document`, and PDF files into it.
<!-- source: CONTRIBUTING.md:43-48; crates/pdf-import/src/lib.rs:15-17 -->

It began as the home of the IDML adapter, the import/export bridge that was moved out of the
engine ([ADR 022](adr/022-idml-relocates-to-plugin-publish.md)). The adapter crates depend
on the engine's model crates by a pinned git revision, and the engine consumes the adapter
crates back across the same boundary ([ADR 650](adr/650-mutual-git-revision-pins.md)).
PDF is the second foreign format here. It has its own crate and its own plugin bundle, and
it is import only: a PDF becomes a native document, and the bundle contributes no exporter.
<!-- source: README.md:3-4; README.md:12-15; crates/pdf-import/src/lib.rs:15-23; packages/pdf-bundle/manifest.json:18-20 -->

## What it is for

**Reading IDML.** `idml-import` opens an IDML ZIP package, parses `designmap.xml` and every
spread, story and resource part it references, and assembles a `Document`. The raw source
archive is returned next to the document, not inside it; the caller decides where it lives.
<!-- source: README.md:6-7; crates/idml-import/src/lib.rs:170-176 -->

**Writing IDML without losing what was not read.** The parser keeps a subset of each entry's
attributes, and most entries (fonts, preferences, tags, metadata, the XML backing store) are
not modelled at all. `idml-export` therefore takes the document and the original package,
copies untouched entries with their original bytes, and patches only what the model can
express. An unmutated package already in InDesign's spelling comes back byte for byte, with
two exceptions: a story no frame references is dropped, and `Resources/Fonts.xml` gains
every applied face it does not declare ([ADR 652](adr/652-idml-save-back-patches.md)).
<!-- source: README.md:8-10; crates/idml-export/src/lib.rs:20-47, 189-198; crates/idml-export/src/navigation.rs:60-66 -->

**Writing IDML that InDesign reads correctly.** The exporter's rules for document-level
resources are stated in the code as measurements of what InDesign 20.0.1 reads: "each is a
measured fact, not a guess". A source in the engine's older spelling is rewritten on export
([ADR 653](adr/653-indesign-is-the-oracle.md)).
<!-- source: crates/idml-export/src/navigation.rs:23-26, 60-66 -->

**Writing the `.paged` container.** A `.paged` file is a structurally valid IDML package at
all times. Plugin data and the native model ride along as extra ZIP entries that
`designmap.xml` does not reference. One writer produces both the `.idml` and the `.paged`.
<!-- source: crates/idml-export/src/paged.rs:17-21; crates/idml-export/src/lib.rs:325-341 -->

**Opening a PDF as an editable document.** The TypeScript bundle reads the PDF and
reconstructs text frames, vector shapes and images into a small intermediate representation;
the `pdf-import` crate turns that into a native document. When the editable path cannot run,
the importer falls back to one image per page.
<!-- source: crates/pdf-import/src/lib.rs:15-27; packages/pdf-bundle/src/index.ts:15-18; packages/pdf-bundle/src/io/pdf.ts:27-30 -->

**Reaching the user through the plugin host.** Two bundles register with the editor's plugin
host: `@paged-media/publish` for `.idml` and `@paged-media/pdf` for `.pdf`.
<!-- source: packages/publish-bundle/src/activate.ts:15-21; packages/publish-bundle/src/io/idml.ts:15-31; packages/pdf-bundle/src/index.ts:15-18; packages/pdf-bundle/src/io/pdf.ts:15-19 -->

## What it will never do

- **Own the model, rendering or mutation.** Those live in the engine repository.
- **Regenerate an IDML package from the model.** That would drop what the parser did not read.
- **Keep a second copy of a parser rule in the writer.** The parser reports what it dropped
  and what it split, and the writer looks the answer up.
- **Store the raw IDML package inside the native model.** The archive rides beside it.
- **Let an exported `.idml` carry `.paged` container parts.** `write_idml` drops
  `manifest.json` and everything under `paged/`.
- **Depend on anything that does not build for `wasm32`.** The IDML crates run inside the
  engine's wasm; they take no native-only dependency.
- **Put PDF knowledge into the Rust mapper.** Every heuristic stays in the TypeScript bundle.
- **Re-implement IDML in the plugin bundle.** The `.idml` bundle only routes files to the
  engine, which parses and writes with the crates above.
- **Fabricate a document, or throw past the host.** A mapper that cannot load resolves to
  `null`, and a failed import is logged.
<!-- source, in list order: CONTRIBUTING.md:43-48; crates/idml-export/src/lib.rs:22-27;
     crates/idml-import/src/story.rs:158-161, crates/idml-import/src/spread.rs:534-538;
     crates/idml-import/src/lib.rs:89-106; crates/idml-export/src/paged.rs:54-61;
     CONTRIBUTING.md:34-35, crates/idml-export/Cargo.toml:18-20; crates/pdf-import/src/lib.rs:24-27;
     packages/publish-bundle/src/io/idml.ts:19-21; packages/pdf-bundle/src/engine-loader.ts:21-26,
     packages/pdf-bundle/src/io/pdf.ts:33-34 -->
