# ADR 657 — One repo for foreign formats, one bundle per format

- **Status:** Accepted. Recorded retroactively on 2026-10-02 from the code at `6994ad1`.
- **Scope:** the repository layout: the Cargo workspace under `crates/`, the pnpm workspace
  under `packages/`, both manifests, and the two workflows in `.github/workflows/`

## Context

The repository was created on 2026-07-23 as the home of the IDML adapter
([ADR 022](022-idml-relocates-to-plugin-publish.md)). The same day it gained a plugin
bundle for IDML (commit `c8ef82e`) and a second bundle for PDF import (commit `e1e5318`).

The title's term, foreign formats, follows
[ADR 021](https://github.com/paged-media/core/blob/main/docs/adr/021-paged-native-document-model-idml-as-format.md),
which scopes this repository to formats other than the native one, reached through the
importer and exporter doors.

Commit `e1e5318` calls the PDF bundle "a child of plugin-publish" and says it mirrors the
IDML bundle's scaffold and registration. It does not say why PDF was placed in this
repository, or why it became a bundle of its own. The README predates it: it expects "the
TS publishing bundle", one bundle, and does not mention PDF (`README.md:17-20`).
The repository does not record why.

## Decision

All foreign-format code lives in one repository with one Cargo workspace and one pnpm
workspace. Each format is a separate plugin bundle with its own manifest, plugin id, npm
package and version.

| Format | Package directory | Plugin id | npm package | Contributes |
|---|---|---|---|---|
| IDML | `packages/publish-bundle` | `media.paged.publish` | `@paged-media/publish` | one importer, one exporter |
| PDF | `packages/pdf-bundle` | `media.paged.pdf` | `@paged-media/pdf` | one importer |

- The bundles do not depend on each other. Neither `package.json` names the other package
  and neither imports from the other's source.
- The workflows treat them alike. CI typechecks, builds and validates every package under
  `packages/*`; the publish workflow publishes each one whose version is new.
- The Rust crates do not map one-to-one onto the bundles. `idml-import` and `idml-export`
  are linked by the engine, not by the IDML bundle
  ([ADR 651](651-idml-compiled-into-engine-wasm.md)); `pdf-import` is compiled to the PDF
  bundle's wasm ([ADR 654](654-pdf-ir-and-mapper.md)).

## Evidence

- `Cargo.toml:3`, `pnpm-workspace.yaml:1-2` — the two workspaces
- `packages/publish-bundle/manifest.json:2-3`, `:10-13`;
  `packages/pdf-bundle/manifest.json:2-3`, `:18-20` — two ids; what each contributes
- `packages/publish-bundle/package.json:2-4`, `:17-27`;
  `packages/pdf-bundle/package.json:2-4`, `:18-32` — two packages; neither lists the other
- `packages/publish-bundle/src/activate.ts:51-54`,
  `packages/pdf-bundle/src/activate.ts:52-55` — one `defineBundle` each
- `.github/workflows/ci.yml:57-65`, `.github/workflows/publish.yml:43-57` — the loops over
  all bundles
- `editor: apps/canvas/src/main.tsx:96-97`, `:1332-1335`;
  `editor: apps/canvas/package.json:35`, `:38` — the editor imports, pins and loads the two
  bundles separately

## Alternatives considered

None recorded in the repository.

## Consequences

A format can be released, pinned and loaded without the other. The editor pins the two
packages at different versions and loads each with its own call.

The scope is uneven. IDML is handled in both directions; PDF is import-only, and PDF export
is the engine's ([ADR 119](https://github.com/paged-media/core/blob/main/docs/adr/119-pdf-export-backend.md)).
The IDML bundle ships no wasm and no format code of its own; the PDF bundle ships two wasm
modules and all of its reading logic.

The bundles share one lockfile, one `tsconfig.base.json` and one set of workflows. The
publish workflow typechecks and builds every package before it publishes any
(`.github/workflows/publish.yml:41-42`), so a failure in one bundle stops the release of
the other. The PDF side depends on the IDML crates in one place:
`pdf-import` takes `idml-import` as a dev-dependency to test that its output opens the
way the engine opens it (`crates/pdf-import/Cargo.toml:32-37`).

Nothing in the repository says whether a further format belongs here as a third bundle.
The README and `CONTRIBUTING.md` describe the repository as the IDML adapter only
(`README.md:3-4`, `CONTRIBUTING.md:3-5`, `:43-46`); the CI comment at
`.github/workflows/ci.yml:42-43` names one bundle. Both manifests carry a version behind
their `package.json`.

## Related

- [ADR 022](022-idml-relocates-to-plugin-publish.md) — why the IDML adapter is in this repository
- [ADR 651](651-idml-compiled-into-engine-wasm.md), [ADR 656](656-pdf-opens-as-native-document.md) — what each bundle does
- [ADR 306](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/306-canary-releases.md), [ADR 307](https://github.com/paged-media/plugin-sdk/blob/main/docs/adr/307-contract-as-peer-dependency.md) — how bundles are published and how they take the plugin contract
- [ADR 201](https://github.com/paged-media/editor/blob/main/docs/adr/201-plugins-as-pinned-packages.md) — the editor compiles first-party plugins in as pinned packages
