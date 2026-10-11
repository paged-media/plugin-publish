# idml-coverage

A static inventory of the IDML that this repository's codecs understand. The
tool walks `crates/idml-import/src` and `crates/idml-export/src` with `syn`
and writes `docs/reference/idml-coverage.json`: one row per element, attribute
and kind, saying whether the importer reads it, whether the exporter writes it,
and where.

The docs site uses the file to build its per-element reference pages.

## Regenerate

```sh
cargo run -p idml-coverage > docs/reference/idml-coverage.json
```

The `idml_coverage_json_is_current` test (`cargo test -p idml-coverage`) fails
when the committed file no longer matches the code. After any change to the
importer, the exporter or `manual.toml`, regenerate the file and commit it with
the change. The output has no timestamps or revisions, so it is byte-stable.

The tool reads the `paged-model` sources (a pinned git dependency) through
`cargo metadata --offline`, so dependencies must already be fetched
(`cargo fetch`). It uses them to type untyped `.parse()` reads from the struct
field they fill.

## Output

```json
{
  "generatedFrom": { "importer": "...", "exporter": "...", "manual": "...", ... },
  "counts": { "rows": 0, "elements": 0, "unresolved": 0, ... },
  "rows": [
    {
      "element": "ParagraphStyleRange",
      "attribute": "Justification",
      "kind": "attr",
      "importerField": "Paragraph.justification",
      "coarseType": "enum",
      "imported": true,
      "exported": true,
      "source": { "file": "crates/idml-import/src/story.rs", "line": 610 },
      "exportSource": { "file": "crates/idml-export/src/emit.rs", "line": 642 }
    }
  ],
  "unresolved": []
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
- `unresolved` lists, with `file:line`, what the tool saw but could not
  attribute: a read with a computed key, a read or write with no element
  context, or a byte-string attribute name that is not a read. Resolve each
  entry in the code or in `manual.toml`.

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

## Debugging

`IDML_COVERAGE_DEBUG=1 cargo run -p idml-coverage > /dev/null` prints, on
stderr, each function's read and write counts, its inferred elements, and every
write that fell back to its function's elements.

## Limits

- Attribution through a shared helper is per function, not per argument. A
  helper that patches `EndCap` for every page item counts as writing it for
  each kind that calls it, even if one caller passes `None`. `manual.toml`
  corrects the cases that matter.
- Pass-through is not counted. Attributes the exporter keeps byte-for-byte
  from the source are not marked exported unless the code names them.
