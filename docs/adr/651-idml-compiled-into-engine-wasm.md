# ADR 651 — IDML is compiled into the engine wasm; the bundle is a registration shim

- **Status:** Accepted. Recorded retroactively on 2026-10-02 from the code at `6994ad1`.
- **Scope:** `packages/publish-bundle` (`@paged-media/publish`, plugin id
  `media.paged.publish`) and the way `crates/idml-import` and `crates/idml-export` reach a
  running editor

## Context

[ADR 022](022-idml-relocates-to-plugin-publish.md) moved the IDML parser and writer into
this repository. The code could then run inside the plugin bundle, or inside the engine
that already called it.

The bundle's header says which: "Under ADR-022 Option A the engine (canvas-wasm) still
performs the actual IDML parse/write via the out-of-repo adapter crates; this bundle does
NOT re-implement IDML. It only routes the file/registry flow to the engine"
(`packages/publish-bundle/src/io/idml.ts:19-21`). The engine takes the adapter crates as
git dependencies ([ADR 650](650-mutual-git-revision-pins.md)).

## Decision

The Rust crates in this repository are linked by the engine; the `paged.publish` bundle
contains no parser, no writer and no wasm. It registers one importer and one exporter for
`.idml` and forwards both to the engine.

- **Engine side.** `paged-canvas`, the crate behind the editor's wasm, depends on
  `idml-import` and `idml-export`. Its load path opens every document through
  `idml_import::open_source_archive` and runs `import_idml_archive` when the package
  carries no native model part, or one it cannot decode. Its `export_idml` calls
  `idml_export::write_idml_with`. The viewer SDK crate depends on `idml-import`.
- **Import.** The importer claims `.idml` and passes the file bytes to
  `host.nativeDocument.open`, gated on `document.openNative@1`. A failure is logged, not
  thrown.
- **Export.** The exporter calls `host.editor.client.send({ kind: "exportIdml",
  payload: {} })` and returns the bytes of an `idmlExported` reply under the fixed file name
  `document.idml`. Any other reply yields `null`.
- **Build constraint.** Because the adapter runs inside the engine wasm, CI builds
  `idml-import` and `idml-export` for `wasm32-unknown-unknown`, and `zip` is used with
  default features off and `deflate` only.

## Evidence

- `packages/publish-bundle/src/io/idml.ts:55-70` — import through `host.nativeDocument.open`
- `packages/publish-bundle/src/io/idml.ts:74-90` — export through `host.editor.client.send`
- `packages/publish-bundle/manifest.json:7-13` — capabilities and contributions; no `wasm` entry
- `core: crates/paged-canvas/Cargo.toml:22`, `:31-35`; `core: crates/paged-canvas-wasm/Cargo.toml:24`
  — the wasm crate depends on `paged-canvas`, which depends on both adapter crates
- `core: crates/paged-canvas/src/model.rs:1529-1558`, `:4398-4408` — the load sniff and the
  export call into the adapter
- `core: crates/paged-canvas/src/channel.rs:1206-1209`, `:1790-1814` — the wire kinds
  `ExportIdml` and `IdmlExported`
- `core: crates/paged-sdk/Cargo.toml:26`, `core: crates/paged-sdk/src/lib.rs:203` — the viewer
  SDK imports IDML with `idml_import::import_idml_doc`
- `.github/workflows/ci.yml:37-40`, `Cargo.toml:33`, `crates/idml-export/Cargo.toml:18-21` —
  the wasm32 build and the `zip` feature set

## Alternatives considered

The exporter's comment names two successors to the raw editor handle: "a first-class export
door (or the adapter-wasm path)" (`packages/publish-bundle/src/io/idml.ts:79-80`). The
bundle uses neither. ADR 022 records the other placements of the adapter that were weighed.

## Consequences

The engine cannot be built without this repository. A change to IDML import or export
reaches users through an engine release, not through a release of `@paged-media/publish`.

Export uses `host.editor`, which the bundle calls "a transitional bridge"
(`packages/publish-bundle/src/io/idml.ts:78-79`). The plugin contract marks that member as
one that "does not survive the isolate boundary"
(`plugin-sdk: packages/plugin-api/src/host.ts:1673-1679`), so the exporter works only while
bundles run in the editor's own realm. Import already uses the isolate-safe door.

The exporter reads only `idmlBytes` from the reply. The reply's `lost` list, which names
native constructs the IDML writer could not carry, and its `links` are not read, and the
request carries no `linkBase`.

The manifest declares `readNative: true`; no code in the bundle calls `readModel`,
`readComposition` or `listParts`. The package has no test script, and the manifest version
(`0.1.0-canary.0`) is behind the package version (`0.1.1-canary.0`). The README says "the
shipped engine carries no IDML of its own" (`README.md:14-15`); that describes where the
source lives, and the adapter is compiled into the shipped engine wasm.

## Related

- [ADR 022](022-idml-relocates-to-plugin-publish.md) — the relocation and its Option A
- [ADR 650](650-mutual-git-revision-pins.md) — how the engine obtains these crates
- [ADR 652](652-idml-save-back-patches.md) — what the linked writer does
- [ADR 017](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/017-importer-exporter-door-shape.md) — the importer and exporter doors the bundle registers through
- [ADR 319](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/319-trust-line.md) — in-process bundles and the isolate boundary
