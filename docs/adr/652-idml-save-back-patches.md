# ADR 652 — IDML save-back patches the source package

- **Status:** Accepted. Recorded retroactively on 2026-10-02 from the code at `6994ad1`.
- **Scope:** `crates/idml-export`, and the provenance API of `crates/idml-import`
  (`parse_story_with_provenance`, `parse_spread_with_provenance`)

## Context

The importer reads only part of an IDML package. The writer's module header states what
follows: "The parser keeps a *subset* of every entry's attributes; most entries (fonts,
preferences, tags, metadata, the XML backing store) are not modeled at all. Regenerating
those from the model would silently drop everything the parser didn't read"
(`crates/idml-export/src/lib.rs:22-25`).

A patching writer has to know which source element became which model item, and the
parser does not keep one item per element: it drops a character range with no text, drops
a page item with no geometry, and splits a range that holds a text variable. Matching
ranges by counting patched the wrong runs (`crates/idml-import/src/story.rs:144-156`).
Reading "id not in the model" as "the user deleted it" removed two polygons on a save that
changed nothing (`crates/idml-export/src/rewrite.rs:2494-2496`,
`crates/idml-import/src/spread.rs:527-532`).

## Decision

`idml-export` never regenerates a package. `write_idml(doc, original)` takes the model and
the bytes it was parsed from, copies what the model did not change, patches what it did,
and generates a whole part only for an object with no source entry. The link from a source
element to a model item comes from the parser.

- **Copy.** The source ZIP is walked in its original order. An entry is copied with its
  compressed bytes (`raw_copy_file`) unless a patched body exists for it. A rewrite equal
  to the source is discarded, so an unmutated document that is already in InDesign's
  spelling keeps every entry byte for byte, with the two exceptions in
  [ADR 653](653-indesign-is-the-oracle.md).
- **Patch.** Spreads, master spreads and stories go through a quick-xml reader-to-writer
  pass that replaces only the attributes and text the model owns. `designmap.xml` and four
  `Resources` parts (`Graphic`, `Styles`, `Fonts`, `Preferences`) are patched the same way.
- **Numbers.** An untouched `ItemTransform` or `StrokeWeight` keeps its source spelling;
  the model stores `f32` and the writer prints four decimals, so re-deriving truncates it.
- **Generate.** A story or spread created after load is written as a full part, appended
  after the source entries and referenced by a minimal insertion in `designmap.xml`.
- **Provenance.** `parse_story_with_provenance` maps the byte offset of each style range
  to the model paragraph or runs it produced. `parse_spread_with_provenance` returns the
  `Self` ids of page items the parser declined to model. The writer derives both from the
  bytes it is streaming; a style range with no record, and a page item the spread record
  lists, pass through verbatim.

## Evidence

- `crates/idml-export/src/lib.rs:20-47`, `:211-224` — the strategy and its reason; the API
  takes the original bytes
- `crates/idml-export/src/lib.rs:360-372`, `:895-937`, `crates/idml-export/src/emit.rs:15-32`
  — kept rewrites; the walk (`raw_copy_file` at `:907`, `:927`); parts created after load
- `crates/idml-export/src/rewrite.rs:45-50`, `:61-66`, `:3455-3470`, `:3993-3995` — source
  spelling for an untouched transform or stroke weight
- `crates/idml-import/src/story.rs:138-197`, `crates/idml-import/src/spread.rs:514-561` — the
  two provenance records and why each is keyed as it is
- `crates/idml-export/src/rewrite.rs:2507-2510`, `:4435-4438`, `:5400-5449` — the writer
  parses the entry it rewrites and resolves each element through the record

## Alternatives considered

Regenerating the package from the model: rejected in the module header quoted above.
Teaching the writer the parser's drop rule: "that would be two copies of one decision,
which is the defect one level up" (`crates/idml-import/src/story.rs:158-161`). Ordinals for
story ranges and byte offsets for page items were both rejected
(`crates/idml-import/src/story.rs:163-175`, `crates/idml-import/src/spread.rs:540-547`).

## Consequences

The engine holds the source package bytes while the document is open and passes them to
the writer (`core: crates/paged-canvas/src/model.rs:1119-1127`, `:4402-4406`); `original`
must be the package the model was parsed from or one with the same entries and `Self` ids
(`crates/idml-export/src/lib.rs:183-184`). Export parses each rewritten spread and story
again, and both crates must read a part with the same reader configuration or the offsets
disagree (`crates/idml-export/src/rewrite.rs:4410-4416`, `crates/idml-import/src/story.rs:168-172`).

The writer's header lists what does not save: a removed page keeps its entry and its
`designmap.xml` reference; a master spread with no source entry is not written; an item
that changes parent is rebuilt and loses source-only children; a dissolved group and a
group's own transform are not written back
(`crates/idml-export/src/rewrite.rs:159-203`, `crates/idml-export/src/lib.rs:482-487`).

Comments lag the code. `crates/idml-export/src/lib.rs:29-30` names only spreads, master
spreads and stories as patched, and `:743-893` patches five more entries; `:52-53` says
`Document` retains the original entries, and the importer returns the archive separately
(`crates/idml-import/src/lib.rs:170-176`). `crates/idml-export/src/rewrite.rs:34-39` says
story ranges are matched positionally; `:204-209` says opacity on an existing item is not
saved, and `crates/idml-export/src/lib.rs:428-434` runs a transparency pass over spreads
that have a source entry.

## Related

- [ADR 653](653-indesign-is-the-oracle.md), [ADR 651](651-idml-compiled-into-engine-wasm.md) — what decides the spelling the writer emits; where the writer runs
- [ADR 007](https://github.com/paged-media/core/blob/main/docs/adr/007-carry-through-rendering-honesty.md) — the carry-through principle
- [ADR 118](https://github.com/paged-media/core/blob/main/docs/adr/118-paged-file-is-a-valid-idml-package.md) — the `.paged` container, written by the same `write_package`
