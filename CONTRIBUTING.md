# Contributing to plugin-publish

Thanks for your interest. This repository is the **IDML adapter** for
[paged](https://paged.media) — the `idml-import` / `idml-export` crates
extracted out of the core engine (ADR-022, Option A). It is **open**
(MPL-2.0 OR PMEL); the engine (`paged-media/core`) is also MPL-2.0 OR PMEL,
and the editor (`paged-media/editor`) is AGPL-3.0 OR PMEL.

## License of contributions

`plugin-publish` is dual-licensed — **MPL-2.0 OR the Paged Media Enterprise
License (PMEL)**. By contributing you agree to the **Contributor License
Agreement** ([`CLA.md`](./CLA.md)), which allows And The Next GmbH to
distribute your contribution under **both** the open-source license
(MPL-2.0) **and** the commercial license (PMEL). You retain copyright to
your contribution.

A CLA bot will ask you to sign on your first pull request.

New source files must carry the standard MPL-2.0 header — copy it verbatim
from the top of any existing `crates/**/*.rs`.

## Building & testing

The toolchain is pinned in `rust-toolchain.toml`. The adapter crates
git-depend the Paged model crates (`paged-model`, `paged-scene`) from the
public `paged-media/core` engine repo, so a build fetches core over https
(no auth — both repos are public).

```bash
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
# the save-back path runs in the editor/SDK wasm, so keep it wasm32-clean:
cargo build -p idml-import -p idml-export --target wasm32-unknown-unknown
```

Do **not** run `cargo fmt --all` to *rewrite* the tree if it drifts
unrelated files — format only the files you touched.

CI runs the same fmt + clippy + native/wasm32 build on every push and PR.

## Scope

This repo is import/export **only** — it maps IDML packages to and from the
Paged native `Document`. The model itself, rendering, and mutation live in
`paged-media/core`; changes there ride the core→plugin-publish rev pin (bump
`core`, then re-pin the `git` rev in this repo's `Cargo.toml`).
