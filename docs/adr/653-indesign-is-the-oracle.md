# ADR 653 — InDesign's own reading is the conformance oracle

- **Status:** Accepted. Recorded retroactively on 2026-10-02 from the code at `6994ad1`.
- **Scope:** `crates/idml-export` (what it writes), `crates/idml-import` (what it accepts),
  and their tests

## Context

The importer and the exporter in this repository read each other's output, so a round
trip through both can pass while another reader of the file sees something else. Commit
`18857d9` (2026-08-31) records the case: a 134-page document was exported, opened in
InDesign and exported again. Paragraphs came back concatenated and named tints were
discarded, and the commit names the cause: "both were private conventions our reader and
writer share with nobody else — which is exactly why our own round trip could never see
them."

Measurements on 2026-09-05 found the same for sections, hyperlinks, bookmarks and
conditional text: InDesign 20.0.1 dropped them from files the engine had written
(`crates/idml-export/src/navigation.rs:15-23`, commit `7f2f5ab`).

## Decision

A rule in the IDML writer is correct when InDesign reads the result as intended. It is
not enough that `idml-import` reads it back.

- **Rules are measurements.** Spelling rules are recorded beside the code that implements
  them, as measurements against a named InDesign version. The header of `navigation.rs`
  lists those for `designmap.xml` and says "each is a measured fact, not a guess".
- **The writer emits InDesign's spelling.** Content carried through from a source in the
  engine's older spelling "is REWRITTEN on every export", so that an existing document
  exports correctly without being authored again.
- **The reader accepts both.** The importer keeps reading the older spellings next to
  InDesign's. For conditions, InDesign's location wins when an id appears in both.
- **Byte identity yields to InDesign in two places.** An unmutated round trip of a source
  already in InDesign's spelling is byte-identical
  ([ADR 652](652-idml-save-back-patches.md)) except that a story no frame references is
  dropped, because InDesign discards it on open, and that `Resources/Fonts.xml` gains
  every applied face it does not declare, because InDesign reports an undeclared face as
  not available.
- **One InDesign-authored package is a fixture.** `indesign-20.0.1-navigation.idml` is
  InDesign's own export. The importer must read it, and the exporter must leave every
  entry of it byte-identical when nothing changed.

## Evidence

- `crates/idml-export/src/navigation.rs:23-58` — the measured rules for `designmap.xml`
- `crates/idml-export/src/navigation.rs:60-66` — byte identity for InDesign's spelling;
  the older spelling is rewritten on every export
- `crates/idml-export/src/lib.rs:189-198`, `:573-580` — the two exceptions to byte identity
- `crates/idml-export/tests/indesign_canonical.rs:15-21`, `:113-122` — the fixture and the
  byte-identity test over it
- `crates/idml-import/src/lib.rs:210-216`, `:256-259`;
  `crates/idml-import/src/designmap.rs:229-232`, `:352-353` — both spellings are read
- `crates/idml-export/tests/support/corpus.rs:15-29`, `:38-59` — the opt-in corpus lanes
- `.github/workflows/ci.yml:93-97`, `.config/nextest.toml:8-12` — what CI runs

## Alternatives considered

The project's own round trip as the gate is what the commits above replaced. Reading the
rule off the IDML specification is set aside in commit `7f2f5ab`: "every claim below was
probed, not read off the spec".

## Consequences

InDesign is not part of this repository's checks. No script here drives InDesign; the
tooling that opens generated packages in InDesign lives in the engine repository
(`core: tools/indesign-export/`). The experiment notes that
`crates/idml-export/src/navigation.rs:25` refers to are not in this repository either.
What a contributor can read here is the result of each experiment, written as a comment
or a commit message.

What CI gates: `cargo nextest run --profile ci` runs the tests that are not ignored,
which include the two tests over the InDesign-authored fixture and the round-trip tests
that pin individual spellings. CI therefore catches a regression against a spelling that
has a test. It cannot tell whether InDesign reads a spelling nobody has measured.

What CI does not gate: ten tests, one in each of ten files under
`crates/idml-export/tests/`, run the writer over real packages from a corpus kept outside
this repository. Each is marked `#[ignore]` and returns early unless `PAGED_IDML_CORPUS`
is set; CI has no copy of the corpus. The byte-identity sweep
`crates/idml-export/examples/corpus_sweep.rs` needs the same corpus and is run by hand.

The importer keeps the older read paths. Commit `18857d9` gives the reason for one of
them: "documents already written must keep opening".

## Related

- [ADR 652](652-idml-save-back-patches.md) — the patching writer whose byte identity this qualifies
- [ADR 120](https://github.com/paged-media/core/blob/main/docs/adr/120-indesign-is-the-oracle.md) — the same oracle in the engine repository: a generator authors, InDesign answers, a diff gates
- [ADR 007](https://github.com/paged-media/core/blob/main/docs/adr/007-carry-through-rendering-honesty.md) — carry-through and reporting what was lost
