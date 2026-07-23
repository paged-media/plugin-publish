#!/usr/bin/env bash
# build-wasm.sh — build the pdf-import mapper crate to the pdf bundle's wasm
# artifact.
#
# pdf-import is the PDF-BLIND Rust mapper: a reading-ordered Document IR (JSON,
# emitted by the pdf-bundle's pdf.js reconstruction) → paged_scene::Document →
# native `.paged` OCF bytes. This script compiles it to wasm32 and runs
# wasm-bindgen `--target web`, producing the glue + module the bundle's
# `engine-loader.ts` imports.
#
# Output: packages/pdf-bundle/bin/pdf_import.js + pdf_import_bg.wasm
# (manifest `capabilities.wasm`, gitignored generated output).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="$HERE/crates/pdf-import"
OUT="$HERE/packages/pdf-bundle/bin"
TARGET="wasm32-unknown-unknown"

echo "build-wasm: crate=$CRATE"

if ! rustup target list --installed 2>/dev/null | grep -q "$TARGET"; then
  echo "build-wasm: installing $TARGET target" >&2
  rustup target add "$TARGET"
fi

# Build from the workspace root so the shared git-dep lock is reused; -p
# selects just the mapper (idml-import/-export are not needed for the bundle).
( cd "$HERE" && cargo build --release --target "$TARGET" -p pdf-import )

WASM_IN="$HERE/target/$TARGET/release/pdf_import.wasm"
if [[ ! -f "$WASM_IN" ]]; then
  echo "build-wasm: expected $WASM_IN — build produced no cdylib" >&2
  exit 1
fi

mkdir -p "$OUT"
if command -v wasm-bindgen >/dev/null 2>&1; then
  wasm-bindgen "$WASM_IN" --target web --out-dir "$OUT" --out-name pdf_import
else
  echo "build-wasm: wasm-bindgen not found — install with: cargo install wasm-bindgen-cli" >&2
  exit 1
fi

# Optional size pass.
if command -v wasm-opt >/dev/null 2>&1 && [[ -f "$OUT/pdf_import_bg.wasm" ]]; then
  wasm-opt -Oz "$OUT/pdf_import_bg.wasm" -o "$OUT/pdf_import_bg.wasm"
fi

echo "build-wasm: wrote artifact(s) to $OUT"
ls -la "$OUT"

# The mapper is tiny (serde + zip + the paged model, no PDF stack); a size
# regression here would signal an accidental heavy dep.
BG="$OUT/pdf_import_bg.wasm"
if [[ -f "$BG" ]]; then
  RAW_BYTES=$(wc -c < "$BG" | tr -d ' ')
  printf 'build-wasm: artifact %s = %d bytes (%.2f MiB)\n' \
    "$(basename "$BG")" "$RAW_BYTES" "$(echo "$RAW_BYTES" | awk '{print $1/1048576}')"
fi
