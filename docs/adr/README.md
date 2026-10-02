# Architecture decision records

An ADR records one load-bearing decision that has already been made: what was decided, what
in the code shows it, and what it obliges other code to do. It is a record, not a proposal.
When the code stops matching a record, the body is left as it is and a dated amendment is
added at the end.

ADR numbers are unique across the paged-media repositories, so a number names the same
record wherever it is cited. New records in this repository use 650–699. Record 022 predates
that scheme and keeps its number; it amends ADR 021, which lives in the engine repository
([core ADR 021](https://github.com/paged-media/core/blob/main/docs/adr/021-paged-native-document-model-idml-as-format.md)).
Records 650–657 were written on 2026-10-02 from the code as it stood, for decisions made
earlier; their status says so.

| ADR | Title | Status |
|---|---|---|
| [022](022-idml-relocates-to-plugin-publish.md) | IDML relocates to plugin-publish; the model self-owns natively (amends ADR-021) | Accepted (amended 2026-10-02) |
| [650](650-mutual-git-revision-pins.md) | The IDML adapter and the engine pin each other by git revision | Accepted, recorded retroactively 2026-10-02 |
| [651](651-idml-compiled-into-engine-wasm.md) | IDML is compiled into the engine wasm; the bundle is a registration shim | Accepted, recorded retroactively 2026-10-02 |
| [652](652-idml-save-back-patches.md) | IDML save-back patches the source package | Accepted, recorded retroactively 2026-10-02 |
| [653](653-indesign-is-the-oracle.md) | InDesign's own reading is the conformance oracle | Accepted, recorded retroactively 2026-10-02 |
| [654](654-pdf-ir-and-mapper.md) | PDF understanding lives in TypeScript; a PDF-blind mapper builds the model | Accepted, recorded retroactively 2026-10-02 |
| [655](655-pdfium-reader.md) | PDFium is the PDF reader | Accepted, recorded retroactively 2026-10-02 |
| [656](656-pdf-opens-as-native-document.md) | A PDF opens as a native document: editable first, page images as the fallback | Accepted, recorded retroactively 2026-10-02 |
| [657](657-one-bundle-per-format.md) | One repo for foreign formats, one bundle per format | Accepted, recorded retroactively 2026-10-02 |

Decisions made in other repositories that this repository's code rests on are listed in
[`../README.md`](../README.md).
