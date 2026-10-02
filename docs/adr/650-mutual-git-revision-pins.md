# ADR 650 — The IDML adapter and the engine pin each other by git revision

- **Status:** Accepted. Recorded retroactively on 2026-10-02 from the code at `6994ad1`.
- **Scope:** the workspace `Cargo.toml` and `Cargo.lock` here; on the engine side,
  `core: Cargo.toml` and every crate there that names `idml-import` or `idml-export`

## Context

[ADR 022](022-idml-relocates-to-plugin-publish.md) moved the IDML importer and exporter out
of the engine repository into this one. The model types they fill and read
(`paged-model`, `paged-scene`) stayed in the engine, and the engine still calls the adapter
(see [ADR 651](651-idml-compiled-into-engine-wasm.md)). So each repository needs crates
from the other.

The README states the arrangement: the crates "depend on the Paged model crates
(`paged-model`, `paged-scene`) from the public `paged-media/core` engine via git (pinned
rev). Core consumes these adapter crates back across the same git boundary"
(`README.md:12-15`). It does not say why a git revision was chosen over a crate registry.
The repository does not record why.

## Decision

Both directions are Cargo git dependencies pinned to one full commit hash. No crate is
published: `publish = false` is set for both workspaces.

- **This repo → engine.** `paged-model`, `paged-scene` and `paged-store` are workspace
  dependencies on `https://github.com/paged-media/core`, all at the same `rev`; the comment
  on `paged-store` says the shared `rev` keeps crate identity unified (`Cargo.toml:26-28`).
- **Engine → this repo.** Eleven engine crates name `idml-import` at one `rev` of this
  repo: six as a normal dependency (`paged-canvas`, `paged-renderer`, `paged-sdk`,
  `paged-cli`, `paged-export-pdf`, `paged-introspect-wasm`) and five as a dev-dependency.
  `paged-canvas` and `paged-renderer` also name `idml-export`.
- **One `Document` type.** The engine's manifest carries
  `[patch."https://github.com/paged-media/core"]`, which redirects the adapter's git
  dependencies on `paged-model`, `paged-scene`, `paged-flow` and `paged-composition` to the
  engine's local crates. Its comment gives the reason: without it Cargo treats the git
  copies as distinct from the local ones "(path source != git source), so `import_idml`'s
  Document wouldn't be core's Document".
- At the recorded commits this repo pins the engine at `bd2563a`; the engine at `9f933f1`
  pins this repo at `a88315d`, the parent of `6994ad1`.

## Evidence

- `Cargo.toml:10`, `:19-29` — `publish = false`; the three git dependencies on one `rev`,
  with the comment "the two-repo rev-pin dance"
- `core: Cargo.toml:41`, `:71-82` — `publish = false`; the `[patch]` block and its comment
- `core: crates/paged-canvas/Cargo.toml:22`, `:35`; `core: crates/paged-renderer/Cargo.toml:45`,
  `:55`; `core: crates/paged-sdk/Cargo.toml:26` — git dependencies on this repo by `rev`
- `.github/workflows/ci.yml:12-14`, `CONTRIBUTING.md:25-28` — CI fetches the engine over
  https with no authentication because both repositories are public
- `CONTRIBUTING.md:45-48` — a model change is made in the engine first, then the `rev` here
  is moved
- `crates/idml-import/src/story.rs:705-707`, `:1616-1620`;
  `crates/idml-import/src/styles.rs:1173-1176` — model literals end in `..Default::default()`
  so the model can grow "without breaking this crate at a pinned rev"

## Alternatives considered

ADR 022 describes the adapter as depending on a model crate published through a Rust
registry, and lists the choice of registry as an open question. No registry is used.

## Consequences

A model change crosses two repositories in order: the field lands in the engine, this repo
re-pins the engine and uses the field, the engine re-pins this repo, and this repo may
re-pin once more to follow the engine's main branch (commits `4a70ac5`, `2b906f3`).

The engine builds the adapter against its own local model crates, so an adapter struct
literal that names every field stops the engine's build as soon as the model gains a field.
Commits `412b884`, `780795b` and `71678a4` gave the importer's literals a default for
every field they do not name, for that reason.

Tests that need the engine's generator or mutation crates do not live here: those crates
depend on the adapter in turn, which the commit that created this repo calls "circular
across the git boundary" (`d3b84e8`). `idml-export` has no dev-dependencies; the save-back
tests that needed them were moved to the engine repository (commit `6babe02`).

For work that spans both repositories, commit messages describe a local-only `[patch]` in
this repo that points the model crates at a sibling checkout and is removed before a pin
is committed; commit `d8df609` records that with it "the pushed tree did not build at
all". No commit in this repo's history contains a `[patch]` section. Its comment header
remains at `Cargo.toml:40-42` with nothing under it.

Comments lag the code. `Cargo.toml:21` speaks of the engine building the adapter back as a
future step; it does so today. `Cargo.toml:41-42` mentions the engine's "own [patch] onto
this repo"; the engine's committed manifest has no patch onto this repository.

## Related

- [ADR 022](022-idml-relocates-to-plugin-publish.md) — the relocation; it names a registry, not this mechanism
- [ADR 651](651-idml-compiled-into-engine-wasm.md) — what the engine does with the adapter
- [ADR 021](https://github.com/paged-media/core/blob/main/docs/adr/021-paged-native-document-model-idml-as-format.md) — the native document model the adapter maps to
