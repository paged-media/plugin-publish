# idml-coverage

A static inventory of the IDML that this repository's codecs understand. The
tool walks `crates/idml-import/src` and `crates/idml-export/src` with `syn`
and writes `docs/reference/idml-coverage.json`: one row per element, attribute
and kind, saying whether the importer reads it, whether the exporter writes it,
and where.

The docs site uses the file to build its per-element reference pages.

Each read and write also carries the package part it works on, and the rows
are joined with a census of real InDesign documents, so the tool can say how
much of what documents actually contain the importer reads, from the part
where InDesign writes it. A committed baseline keeps those shares from
falling (see [Parts, census and ratchet](#parts-census-and-ratchet)).

## Regenerate

```sh
cargo run -p idml-coverage > docs/reference/idml-coverage.json
```

The `idml_coverage_json_is_current` test (`cargo test -p idml-coverage`) fails
when the committed file no longer matches the code. After any change to the
importer, the exporter or `manual.toml`, regenerate the file and commit it with
the change. The output has no timestamps or revisions, so it is byte-stable.
The `idml_coverage_ratchet` test then asks for the baseline too:

```sh
cargo run -p idml-coverage -- --ratchet       # record a gain
cargo run -p idml-coverage -- --queue 25      # the uncovered rows that matter most
```

The tool reads the `paged-model` sources (a pinned git dependency) through
`cargo metadata --offline`, so dependencies must already be fetched
(`cargo fetch`). It uses them to type untyped `.parse()` reads from the struct
field they fill.

## Output

```json
{
  "generatedFrom": { "importer": "...", "exporter": "...", "manual": "...", ... },
  "counts": { "rows": 0, "elements": 0, "unresolved": 0, ... },
  "census": { "file": "tools/idml-coverage/census.json", "sha256": "...", ... },
  "counts": { "rows": 0, "elements": 0, "unresolved": 0, ... },
  "metrics": { "weightedImport": { "covered": 0, "total": 0, "value": 0.0 }, ... },
  "rows": [
    {
      "element": "ParagraphStyleRange",
      "attribute": "Justification",
      "kind": "attr",
      "importerField": "Paragraph.justification",
      "coarseType": "enum",
      "imported": true,
      "exported": true,
      "importParts": ["Stories"],
      "exportParts": ["Stories"],
      "source": { "file": "crates/idml-import/src/story.rs", "line": 610 },
      "exportSource": { "file": "crates/idml-export/src/emit.rs", "line": 642 }
    }
  ],
  "misplaced": [],
  "unseen": [],
  "unresolved": [],
  "partRows": [
    {"part":"Stories","element":"ParagraphStyleRange","attribute":"Justification","kind":"attr","docCount":0,"nonModalDocs":0,"imported":true,"exported":true,"readElsewhere":false}
  ]
}
```

- `kind` is `attr` for an XML attribute, or `properties-child` for a typed
  child of the element's `<Properties>` (`<AppliedFont type="string">`).
- `importerField` is the model field the read fills, as `Struct.field` when the
  read sits in a struct literal, or `field` when it is an assignment. It is
  `null` when the value goes through a local first.
- `coarseType` is read off the conversion the importer applies: `number`,
  `bool`, `tint`, `bounds`, `matrix`, `list`, `enum` or `string`. It is
  `parsed` for an untyped `.parse()` whose target the tool cannot see, and
  `null` for a row the importer does not read.
- `source` is the first importer read, or the first exporter write for an
  export-only row. `exportSource` is the first exporter write.
- Rows are sorted by element, attribute and kind. A read or write that applies
  to several elements produces one row for each.
- `importParts` and `exportParts` are the package parts the reads and writes
  happen in (see below).
- `unresolved` lists, with `file:line`, what the tool saw but could not
  attribute: a read with a computed key, a read or write with no element
  context or no package part, or a byte-string attribute name that is not a
  read. Resolve each entry in the code or in `manual.toml`.
- `partRows`, `misplaced`, `unseen` and `metrics` come from the census join
  below.

## How elements are found

- **Importer.** A read is any call to `attr`, `attr_unescaped`, `parse_f`,
  `parse_tint_attr` or `label_value`. Functions and closures that pass a key
  parameter on to one of these count as readers too. The element comes from
  the nearest enclosing `match` arm on a byte-string element name
  (`b"TextFrame" => …`), or from an `if` or arm guard comparing the name
  (`e.name().as_ref() == b"Footnote"`, `matches!(name, b"A" | b"B")`).
  Otherwise it comes from the enclosing function. A function's elements are
  inferred from what its call sites sit inside. `(N, b"Child")` arms are
  Properties children.
- **Exporter.** A write is a `("Name", value)` tuple, a `push_opt_*`-style
  helper that takes the key as a parameter, or a `match k { b"Name" => … }` /
  `k == b"Name"` in a patch closure. The element comes from the local the
  attributes go into, once that local is emitted
  (`emit_start_with_attrs(w, "TextFrame", &attrs)` or
  `BytesStart::new("Story")`). Failing that it comes from the enclosing arm,
  and then from the function. A function emitted by literal
  (`&paragraph_attrs(p)` beside `"ParagraphStyleRange"`) is a strong binding.
  A strong binding beats one inherited from an enclosing container arm.

## manual.toml

This file holds what the walk cannot see. Every entry is checked against the
code: each literal it names must still appear in the item it names, and each
fn must still exist. Otherwise the tool fails and names the stale entry.

| Table | Use |
|---|---|
| `[[fn]]` | A function's elements, replacing call-site inference. |
| `[[properties_kind]]` | The element behind `properties_kind = N` in `(N, b"Child")` arms, with optional child renames. |
| `[[properties_child]]` | Properties children read as text by a pending-property arm. |
| `[[read]]` | Reads through a computed key: arrays of names, keys assembled from a prefix. `verify` lists the literals to check when they are only parts of the names. `elements` defaults to the item's inferred elements. |
| `[[export]]` | Writes the tuple walk cannot see: struct-literal key tables, typed Properties children. `prefix` is prepended to each attribute. |
| `[[ignore]]` | Literals whose walk attribution is dropped, because they are values rather than names, or because a `[[read]]` or `[[export]]` attributes them more precisely. |
| `[[part_entry]]` | The package part(s) a function is handed (`parse_graphic` → `Resources/Graphic`). Callees inherit them. |
| `[[unseen]]` | Why the importer reads an (element, attribute) no census document carries: `keys` (`Element@Attribute`) or whole `elements`, and a `reason`. A name that is no longer an unseen read fails the tool. |

## Parts, census and ratchet

**Parts.** InDesign writes each element to one part of the package:
`<ColorGroup>` to `designmap.xml`, `<FootnoteOption>` to
`Resources/Preferences.xml`. A reader pointed at another part reads nothing in
InDesign's files, however correct its attribute names. So every function that
is handed a part's bytes has a `[[part_entry]]`, and the parts flow down the
call graph to a fixpoint, the same way elements do. Seeds add to what callers
pass down, because one function can serve several parts (`parse_conditions`
reads the designmap and, for `parse_stylesheet`, Styles.xml). File names are
not used: `parse_chapter_number_preference` lives in `designmap.rs` and reads
Preferences.xml. A read or write in a function no seed reaches is unresolved.

**Census.** `census.json` is the corpus census (`paged.idml-census/2`,
written by the corpus repo's `harness/attr-census.mjs`, published as the docs
repo's `data/idml-census.json`). For each element and attribute it gives the
documents carrying it per part, and `nonModalDocs`: the documents where a
value differs from the corpus's most common one. Only aggregate counts are in
it. To refresh it, copy the docs repo's file over `census.json`, regenerate
the coverage file, and re-record the ratchet with `--allow-regression` (the
shares are measured against a new population). The output names the census
by date and SHA-256.

**Part rows.** Every census (part, element, attribute, kind) becomes a
`partRows` entry: `docCount`, `nonModalDocs` (in that part), whether the
importer reads it in that part (`imported`), whether the exporter writes it
there (`exported`), and `readElsewhere` when it is read only from other parts.

**Metrics**, over all census rows:

| Metric | Meaning |
|---|---|
| `weightedImport` | Σ `docCount` over rows read in their part / Σ `docCount` over all rows. |
| `impactImport` | The same, weighted by `nonModalDocs`: the documents whose rendering depends on the read rather than on a default that happens to match. |
| `weightedExport` | As `weightedImport`, for the exporter's writes. |
| `misplacedReads` | (element, attribute, kind) read, but from none of the parts the census finds it in (listed in `misplaced`). |
| `unseenReads` | (element, attribute, kind) read that no census document carries (listed in `unseen`, each with its `[[unseen]]` reason). `unjustifiedUnseenReads` counts those without one. |

**Ratchet.** `docs/reference/idml-coverage-ratchet.json` holds the census
identity, the metrics, the unresolved count and every covered
`part|Element@Attribute|kind` key on both sides. The `idml_coverage_ratchet`
test fails on a regression (a covered key dropped, a share fell, unresolved,
misplaced or unjustified unseen reads rose, the census changed) and on a gain the baseline has not recorded. `--ratchet`
rewrites the baseline and refuses a regression unless `--allow-regression` is
given.

**Queue.** `--queue N` prints the N uncovered part rows with the highest
impact, `nonModalDocs × docCount`. The first factor is what reading the
attribute changes when the engine's default is the corpus mode; the second
hedges for a default that is not, and favours what most documents carry. Ties
go to the more common row. An element that occurs several times in a document
with different values (fonts, colours, styles) counts as non-modal in that
document, so names and ids rank high beside the elements they belong to.

## Debugging

`IDML_COVERAGE_DEBUG=1 cargo run -p idml-coverage > /dev/null` prints, on
stderr, each function's read and write counts, its inferred elements and
package parts, and every write that fell back to its function's elements.

## Limits

- Attribution through a shared helper is per function, not per argument. A
  helper that patches `EndCap` for every page item counts as writing it for
  each kind that calls it, even if one caller passes `None`. `manual.toml`
  corrects the cases that matter.
- Pass-through is not counted. Attributes the exporter keeps byte-for-byte
  from the source are not marked exported unless the code names them.
- Parts are per function, not per call site. `parse_stylesheet` is handed
  Graphic.xml for its stroke styles, so everything it reads counts as read in
  Graphic.xml too. Rows the census does not have in that part are unaffected.
