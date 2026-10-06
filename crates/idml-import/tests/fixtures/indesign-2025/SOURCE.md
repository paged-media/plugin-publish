# InDesign 2025 recordings (object-model round-trip survey, 2026-10-06)

Excerpts, byte-exact, of `recorded/labels-fresh.indd.idml` (InDesign 2025
opened our `fixtures/labels-fresh.idml` and saved it) from the thoughts
register, `docs/paged/object-model/indesign-roundtrip/cases-2/`:

- `aid-label-keyvaluepair.xml` — the `x-paged:aid` `KeyValuePair` on the
  text frame `u100` of `Spreads/Spread_ue7.xml`: InDesign re-spelled every
  non-BMP character of the value as `<?AID xxxx?>` per UTF-16 code unit.
- `textframe-start-tag.xml` — that text frame's start tag (`Name="card-text"`).
- `label-value.json` — the value we injected (`assets/label-value.json`);
  the decoded label must equal it exactly.
