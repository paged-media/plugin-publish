# plugin-publish

Home of the **IDML adapter** for [Paged](https://paged.media) — the
import/export bridge extracted out of the core engine (ADR-022, Option A).

- **`crates/idml-import`** — parses an IDML ZIP package into a Paged
  `Document` (the `import_idml*` orchestrator + the schema parsers).
- **`crates/idml-export`** — re-serialises a `Document` back to a valid IDML
  package (carry-through save-back: verbatim untouched entries + streaming
  attribute patches).

Both are **MPL-2.0 OR PMEL** and depend on the Paged model crates
(`paged-model`, `paged-scene`) from the public `paged-media/core` engine via
git (pinned rev). Core consumes these adapter crates back across the same
git boundary, so the shipped engine carries no IDML of its own.

> Status: Phase 1 (adapter libs extracted + building standalone against
> core). `idml-export`'s `paged-gen`/`paged-mutate` integration tests are
> deferred (re-homed in Phase 2). This repo will also grow the TS publishing
> bundle that surfaces import/export through the editor's plugin host.
