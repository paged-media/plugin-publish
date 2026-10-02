# ADR 022 — IDML relocates to plugin-publish; the model self-owns natively (amends ADR-021)

**2026-07-20 · decision record · status: ACCEPTED (ratified 2026-07-20).** Amends
[ADR-021](https://github.com/paged-media/core/blob/main/docs/adr/021-paged-native-document-model-idml-as-format.md): keeps its thesis (Paged-native model,
IDML demoted to import/export) but **reverses its placement clause** — *"IDML read/write stays
core-resident"* / *"plugin-publish is NOT the home for IDML"* — now that the three walls that forced
that clause are down. Paired with an internal design memo on the document-model direction.

**Sources:** [ADR-021](https://github.com/paged-media/core/blob/main/docs/adr/021-paged-native-document-model-idml-as-format.md) (the decision being amended);
the self-ownership slices merged to core (**N1–N4** — `paged-store` codec + `DOCUMENT_PGM_PATH`,
native-first `CanvasModel::load` sniff, auto-embed on `export_paged`; core PRs #14/#15/#16/#17); the
model-extraction slices (**N5.1–N5.4** — `core: crates/paged-model`, 40 types; PRs #18/#19/#20/#21); the
mechanics finding (no cross-repo Rust dependency exists in the ecosystem — plugins never Cargo-dep
core; core crates are all `publish = false`, only npm/wasm is published);
[ADR-021 §8](https://github.com/paged-media/core/blob/main/docs/adr/021-paged-native-document-model-idml-as-format.md) (which anticipated the missing
whole-document-read host door).

## The decision

**Relocate IDML (the parser + writer) OUT of the core engine into `plugin-publish`, as an adapter
under the same open licence as the engine (see `LICENSE.md`), shared by the open viewer and the
publish plugin. The core ENGINE becomes zero-IDML. Execution
= reclaim-and-evolve: the current in-memory model becomes the Paged model in place; IDML becomes
import/export only.**

This reverses ADR-021's "IDML stays in core" clause. It does not contradict ADR-021's *reasoning* — it
executes the world ADR-021 §8 anticipated ("`plugin-publish` needs the whole-document-read host door
that doesn't exist yet — platform work"). The three walls ADR-021 rejected its "Option Y" on are now
down:

| ADR-021 wall | How it comes down |
|---|---|
| **1 Technical** — no neutral model + the plugin API can't read the whole document | N1–N4 made the native model authoritative (the document self-owns; load reconstructs with no IDML parse). The other half — the plugin can't read the whole doc — is closed by a new isolate-safe **`host.nativeDocument`** door (reads already-native parts + native `open` → adds ZERO IDML to core). |
| **2 Doctrinal** — IDML is the readability floor | The floor is now native (the container format v2 design, §5.3 ladder: native part → render-cache SceneLayer → derived pixels). IDML export becomes lossy interop, not the floor. **Precondition:** the render path must never route native→IDML→raster — enforced by the C1 "render with no IDML" CI gate. |
| **3 Licensing** — IDML read = the embeddable open flagship | The IDML adapter stays **under the engine's open licence (see `LICENSE.md`) and shared**: the open viewer AND the publish plugin both consume it, so the "PDF.js for IDML" / embeddable-open-reader positioning survives. |

### The chosen mechanics (the forks ADR-021 left open)
- **Adapter home = out of core (Option A).** The adapter Rust lives in `plugin-publish`, depending on a
  published `paged-model` via a Rust registry. This is the literal "adapter in plugin-publish."
  **Recorded cost:** the first cross-repo Rust dependency in the ecosystem, semver on a still-churning
  model, a new registry + `deny.toml` allowance, and a core→plugin dependency inversion for the viewer.
  The pragmatic alternative — **B/C** (adapter stays an un-wired, openly licensed crate in the core
  *workspace*, shipped as wasm; "zero-IDML" = the engine graph, not the repo) — is recorded as the
  fallback if the registry cost proves premature (defer Option A to model stabilisation).
- **Execution = reclaim-and-evolve (not clean-model-and-flip).** The current model structure becomes
  the Paged model in place (re-own the types, invert the parser dependency), NOT a from-scratch clean
  model proven via a render-parity flip. Low fidelity risk — rendering is unchanged.

### The sequence
N5 (extract `paged-model`) → N6 (de-inherent the `X::parse` methods; orphan rule) → N7 (break the
`Container` coupling) → N8 (flip dependents `paged_parse`→`paged_model`, leaf-first) → N9 (extract the
parse orchestrator out of `paged-scene`; rename `paged-parse`→idml-import, `paged-write`→idml-export) →
N10 (publish `paged-model` + the adapter; the `host.nativeDocument` door + `PROTOCOL_VERSION` 51→52; the
`plugin-publish` bundle; retire the editor's static IDML export target). Each core slice stays
byte-identical + corpus-fidelity-gated.

## Open questions (UNRESOLVED — each blocks its corresponding step)

1. **The lossless-IDML round-trip promise + the provenance sidecar — BIGGEST.** Approach A drops raw
   IDML provenance from core (`#[serde(skip)]`). "Paged never destroys your IDML" then depends on an
   adapter-owned **provenance sidecar** — the unmodeled attributes/subtrees/parts (fonts, preferences,
   tags, backing store) captured at import and replayed at export. **DESIGN NOTE WRITTEN 2026-07-20**
   (an internal design note) — adapter-owned part
   (never core), original bytes keyed per source part, re-scoped promise (lossless for IDML-origin
   whose provenance travels, honest otherwise,
   [ADR-007](https://github.com/paged-media/core/blob/main/docs/adr/007-carry-through-rendering-honesty.md)),
   and it unblocks Q4 (the IDML parts become a
   derived projection once the sidecar carries provenance). Not yet built.
2. **`document.pgm` versioning / schema churn.** N4 auto-embeds the native part on EVERY save and N3
   **prefers** it on load, but the format had no version field and N5–N9 *rename its types* (changing
   the serde shape). **RESOLVED 2026-07-20 (core PR #22):** a versioned envelope
   (`{ format_version, model }` + `PGM_FORMAT_VERSION`) — `from_bytes` returns `None` on an
   incompatible/unparseable part and the load sniff **falls back to the IDML import**, so a stale
   `.pgm` degrades to the IDML parse (never a corrupt reload or failed open). Discipline: bump the
   version on any serde-shape change.
3. **Reclaim (B) vs the clean-model specs.** `document.pgm` serialises the *current IDML-shaped*
   structure; an internal publishing-format design and the engine repository's composition format
   reference describe a *cleaned* rope model. Decide
   whether `document.pgm` IS the format going forward (schema-freeze risk) or a stepping stone toward
   the specs.
4. **Dropping the IDML parts.** N4 makes a `.paged` carry BOTH IDML and the native model (≈2× on disk).
   "Stop writing IDML" is unsequenced — keeping the container foreign-openable means IDML must become a
   *regenerated derived projection* (needs the generative export adapter; tangled with N10).
5. **Native render-path fidelity.** All corpus fixtures are `.idml` → they take the parse path, so the
   corpus gate has **never exercised the native `document.pgm` render end-to-end**. Build the
   native-round-trip corpus extension before trusting the native path.
6. **Cross-repo publishing mechanism.** Which registry (crates.io vs private), how the open viewer
   consumes a plugin-published adapter (the dependency inversion), and the publish/version lockstep are
   undecided.
7. **InDesign data-loss-guard — an unverified load-bearing assumption.** The container truth-flip
   assumes InDesign silently drops the `paged/core/…` parts on a round-trip; never empirically verified
   (no InDesign on the dev machine).
8. **Mutation-vocabulary re-noun.** The 215-variant `NodeId`/`PropertyPath`
   (`core: crates/paged-mutate/src/operation.rs`) crosses the wasm ABI the editor consumes; "retarget-before-rename"
   is stated but the editor migration + protocol choreography is unplanned.

## Status of the work (2026-07-20)
Merged to core `main`: **N1–N4** (self-ownership) + **N5.1–N5.4** (`paged-model` extraction, 40 types,
`spread.rs` done). The internal feature registry records the self-ownership capability
(`package-anatomy.native-model-part` / `.composition-part`, `round-tripping.native-reserialization`).
This ADR closes the governance gap (ADR-021's placement clause) and pins the open questions above as
explicit, blocking design items — several must be settled *before* their step ships, especially (1) and
(2).

## Amendment — 2026-10-02

Checked against the code at `6994ad1`, and against the engine repository (`core`) at `9f933f1`.
The relocation was carried out on 2026-07-23 (commit `d3b84e8` here, `337b78e` in core): the
parser and the writer are `crates/idml-import` and `crates/idml-export` in this repository
(`Cargo.toml:3`), the engine's workspace no longer has them as members
(`core: Cargo.toml:3-31`), and the parser depends on the model, not the model on the parser
(`crates/idml-import/Cargo.toml:15-17`). The text above no longer matches the code in two places.

**1. There is no registry. The two repositories pin each other by git revision.** See
[ADR 650](650-mutual-git-revision-pins.md).

- `Cargo.toml:10` — `publish = false` for every crate of this workspace
  (`crates/idml-import/Cargo.toml:7`, `crates/idml-export/Cargo.toml:7` inherit it).
- `Cargo.toml:24-25`, `:29` — `paged-model`, `paged-scene` and `paged-store` are git dependencies
  on `https://github.com/paged-media/core`, all at one `rev`. The comment at `:20-23` names the
  procedure "the two-repo rev-pin dance". `Cargo.lock:251`, `:259`, `:273` record the git source.
- `core: Cargo.toml:41` — `publish = false` there as well; `paged-model` inherits it
  (`core: crates/paged-model/Cargo.toml:7`).
- `core: crates/paged-canvas/Cargo.toml:22`, `:35` — the engine depends back on `idml-import` and
  `idml-export` as git dependencies on this repository, at one `rev`.
- `core: Cargo.toml:78-82` — a `[patch."https://github.com/paged-media/core"]` block redirects
  the adapter's git dependencies on `paged-model`, `paged-scene`, `paged-flow` and
  `paged-composition` to the engine's local crates. The comment at `:71-77` gives the reason: to
  Cargo a path source and a git source are different crates, so without the patch the `Document`
  the importer returns would not be the engine's `Document`.
- `core: deny.toml:62` — crates.io is the only allowed registry; `:79` allows this repository as
  a git source (comment at `:66-68`).

This supersedes, in "The chosen mechanics", "depending on a published `paged-model` via a Rust
registry" and, in the recorded cost, "semver on a still-churning model" and "a new registry +
`deny.toml` allowance": the crates stay at version `0.0.0` (`Cargo.toml:6`,
`core: Cargo.toml:37`), a model change is followed by a `rev` bump (`Cargo.toml:23`), and the
allowance in `core: deny.toml` is for a git source. It also supersedes "publish `paged-model` +
the adapter" in "The sequence", and answers open question 6's "Which registry (crates.io vs
private)" with neither. The other two recorded costs did arrive: this is a cross-repo Rust
dependency, and the viewer SDK depends on a crate from this repository
(`core: crates/paged-sdk/Cargo.toml:26`). The repository does not record why git revisions were
chosen over a registry.

**2. The IDML parser and writer left the engine's source tree, not the engine.** They are
ordinary dependencies of the engine's crates and are compiled into the published wasm. See
[ADR 651](651-idml-compiled-into-engine-wasm.md).

- `core: crates/paged-canvas/Cargo.toml:22`, `:35` — both adapter crates are unconditional
  dependencies of the canvas crate; the comment at `:31-34` says the export path compiles into
  the wasm worker.
- `core: crates/paged-canvas/src/model.rs:1529`, `:1550`, `:1556` — `CanvasModel::load` opens
  every package with `idml_import::open_source_archive` and, when the package carries no native
  model part that decodes, builds the model with `idml_import::import_idml_archive`.
- `core: crates/paged-canvas/src/model.rs:4402`, `:4676` — IDML export calls
  `idml_export::write_idml_with`; `.paged` export calls `idml_export::write_paged`.
- `core: crates/paged-canvas-wasm/Cargo.toml:24`, `core: .github/workflows/publish-wasm.yml:220`,
  `:228` — the editor's wasm, `@paged-media/canvas-wasm`, is `paged-canvas-wasm`, which depends
  on `paged-canvas`.
- `core: crates/paged-sdk/Cargo.toml:26`, `core: crates/paged-sdk/src/lib.rs:203` — the viewer
  SDK's `load` calls `idml_import::import_idml_doc`; it is published as `@paged-media/sdk`
  (`core: .github/workflows/publish-wasm.yml:276`, `:288`).
- `idml-import` is also an unconditional dependency of `core: crates/paged-renderer/Cargo.toml:45`
  (with `idml-export` at `:55`; the `paged-inspect` binary calls both,
  `core: crates/paged-renderer/src/bin/inspect.rs:286`, `:946`),
  `core: crates/paged-cli/Cargo.toml:38` (`core: crates/paged-cli/src/options.rs:272`),
  `core: crates/paged-export-pdf/Cargo.toml:14` and
  `core: crates/paged-introspect-wasm/Cargo.toml:24` (`core: crates/paged-introspect-wasm/src/lib.rs:78`).
- `.github/workflows/ci.yml:37-40` — for that reason this repository's CI builds the adapter
  crates for `wasm32-unknown-unknown`.
- `packages/publish-bundle/src/io/idml.ts:63`, `:84` — the `paged.publish` bundle contains no
  IDML code and no wasm. Import hands the file's bytes to `host.nativeDocument.open`; export
  sends `exportIdml` to the engine through `host.editor`
  (`core: crates/paged-canvas/src/channel.rs:1206`). The comment at
  `packages/publish-bundle/src/io/idml.ts:74-82` calls that a transitional bridge. The manifest
  declares no wasm (`packages/publish-bundle/manifest.json:7-9`).

This supersedes, in "The decision", "The core ENGINE becomes zero-IDML." What the code shows is
the reverse of the reading the fallback gives ("the engine graph, not the repo"): the parser and
the writer are out of the engine's repository and in the engine's dependency graph. In the walls
table it supersedes two phrases. Row 3's "the open viewer AND the publish plugin both consume
it" holds for the viewer; the plugin bundle does not consume the adapter, the engine does. Row
1's "load reconstructs with no IDML parse" holds for the model when a native part decodes
(`core: crates/paged-canvas/src/model.rs:1539-1547`), but that load still opens the package with
the adapter's reader (`:1529`) and parses `Resources/Styles.xml` with
`idml_import::styles::parse_stylesheet` to fill style leading that an older native part lacks
(`:1546`, `:9912-9929`). `README.md:14-15` in this repository ("the shipped engine carries no
IDML of its own") is true only in that the parser and the writer crates are no longer in the
engine's source tree.

The engine repository also still writes IDML packages itself, not through `idml-export`: the
blank document behind File ▸ New (`core: crates/paged-canvas/src/blank.rs:17-27`), the one-page
fallback skeleton inside a natively produced `.paged`
(`core: crates/paged-store/src/package.rs:15-23`), and the generator of IDML test packages
(`core: crates/paged-gen/src/lib.rs:15-21`).
