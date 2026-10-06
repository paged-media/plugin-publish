#!/usr/bin/env bash
# Ask InDesign 2025 whether the object-model export holds (ADR 131/132
# defects: Visible/Locked, authored text variables, the TextPath end
# bracket, page-item names). Takes the InDesign GUI for a few seconds.
#
#   bash crates/idml-export/tests/indesign/probe.sh <scratch dir>
#
# 1. base.jsx makes base.idml in InDesign;
# 2. the ignored `indesign_object_model` test mutates it through the model
#    and writes out.idml with this crate's writer;
# 3. verify.jsx opens out.idml, saves it as .indd, reopens, and records
#    what InDesign sees in verify.json (+ out-resaved.idml).
# Copy verify.json over tests/fixtures/indesign-2025-object-model/ to
# re-record.
set -euo pipefail
OUT="$(mkdir -p "$1" && cd "$1" && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="${INDESIGN_APP:-Adobe InDesign 2025}"
run_jsx () {
    local shim
    shim="$(mktemp -t paged-om-probe).jsx"
    printf 'var PAGED_OUT_DIR = "%s";\n$.evalFile(File("%s"));\n' "$OUT" "$1" > "$shim"
    # `do script` by its raw event code (compiles without InDesign's
    # dynamic terminology; see core tools/indesign-export/run-export.sh).
    osascript <<OSA
with timeout of 300 seconds
    tell application "$APP"
        «event K2  dosc» (POSIX file "$shim") given «class doLg»:«constant ****JSLg»
    end tell
end timeout
OSA
    rm -f "$shim"
}
run_jsx "$HERE/base.jsx"
PAGED_INDESIGN_PROBE="$OUT" cargo test -p idml-export --test indesign_object_model -- --ignored --nocapture
run_jsx "$HERE/verify.jsx"
echo "==> $OUT/verify.json"
