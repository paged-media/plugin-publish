/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * This file is part of paged (https://paged.media) and is additionally
 * available under the Paged Media Enterprise License (PMEL). Full
 * copyright and license information is available in LICENSE.md which is
 * distributed with this source code.
 *
 *  @copyright  Copyright (c) And The Next GmbH
 *  @license    MPL-2.0 OR Paged Media Enterprise License (PMEL)
 */

//! An UNTOUCHED `StrokeWeight` must save back as the bytes it arrived as.
//!
//! # The defect
//!
//! IDML stores a stroke weight in POINTS, but InDesign's stroke palette
//! is authored in millimetres — so a 0.25 mm hairline is spelled
//! `StrokeWeight="0.7086614173228347"`, a 0.5 mm rule
//! `1.4173228346456694`, a 2 mm bar `5.669291338582678`. The model stores
//! `f32` and the writer re-emitted the value through `format_f32`, which
//! rounds to 4 decimals, so an untouched hairline came back `0.7087` on a
//! save that changed nothing.
//!
//! Measured over the same 99-package corpus the `ItemTransform` work used
//! (`examples/corpus_sweep.rs`): 70 attributes, spread across 31 of the 87
//! spread entries that still differed after that fix — 61 of the 70 on
//! `<GraphicLine>`, which is the element a rule, a divider or an
//! underline is. It is the largest single cause left, and unlike the
//! others it is not a dropped attribute or a normalised default: the
//! number itself changes.
//!
//! # Why the SIMPLE check is the right one here
//!
//! `ItemTransform` needed its verbatim check run FORWARD — replaying
//! `compose(group_accum, on_disk)` — because a group member's transform
//! is stored COMPOSED into spread space, and recovering the on-disk value
//! by inverting that composition is exactly the step whose `f32` round-off
//! the defect lived in. Comparing a recovered value against the source
//! would have false-negatived precisely where it mattered.
//!
//! A stroke weight has no such derivation. `idml-import` reads it as
//! `attr(e, b"StrokeWeight").and_then(|s| s.parse().ok())` — one step, no
//! composition, no unit conversion, no inheritance: an `<ObjectStyle>`'s
//! own `StrokeWeight` is parsed into the style registry and resolved by
//! consumers, never folded back into the page item's field. So replaying
//! the parser's derivation against the source spelling IS parsing the
//! spelling, which is what `preserving_f32_patch` (in the file since B-23,
//! for corner radii) already does. Reaching for the `TransformPlan`
//! machinery here would be cargo-culting in the other direction.
//!
//! # What still re-derives
//!
//! A weight that WAS changed fails the check and is written from the model
//! at the writer's precision, exactly as before; a cleared one still drops
//! the attribute, and one set on an element that never carried it is still
//! appended. `a_stroke_weight_edit_still_saves` and friends pin that.

use idml_export::rewrite::rewrite_spread;

/// Real spellings and a real element shape from
/// `envato/packs/furniture-product-catalog`, minimised: the
/// `<GraphicLine>` is `Spread_u1b7.xml`'s `u1103` with its attribute
/// order and its 0.25 mm hairline intact. The other three weights are
/// corpus values too — 2 mm, 0.5 mm and the odd one out
/// (`0.2990447182611103`) that a scaled group leaves behind.
///
/// Deliberately no `FillColor` on the `<GraphicLine>`: dropping that is a
/// separate, still-open defect (191 attributes in the same sweep), and a
/// fixture carrying it would fail this file's byte-identity assertions for
/// a reason that has nothing to do with stroke weights.
const STROKED: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<GraphicLine Self="u1103" ContentType="Unassigned" Visible="true" Name="$ID/" StrokeWeight="0.7086614173228347" StrokeColor="Color/Paper" ItemLayer="u102" Locked="false"/>
<Rectangle Self="r1" GeometricBounds="0 0 50 50" StrokeWeight="5.669291338582678" StrokeColor="Color/Black" FillColor="Color/Black"/>
<Oval Self="o1" GeometricBounds="0 0 20 20" StrokeWeight="1.4173228346456694" StrokeColor="Color/Black" FillColor="Color/Black"/>
<Polygon Self="p1" GeometricBounds="0 0 30 30" StrokeWeight="0.2990447182611103" StrokeColor="Color/Black" FillColor="Color/Black"/>
</Spread>
</idPkg:Spread>"#;

/// The writer's output precision (`rewrite::format_f32`, private): round
/// to 4 decimals, drop trailing zeros and a dangling `.`. Re-stated here
/// so the tests can assert what a RE-DERIVED value would look like.
fn fmt4(v: f32) -> String {
    let r = (f64::from(v) * 10_000.0).round() / 10_000.0;
    if r == 0.0 {
        return "0".to_string();
    }
    let mut s = format!("{r:.4}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Every `StrokeWeight="…"` value in document order.
fn weights_in(xml: &str) -> Vec<&str> {
    xml.match_indices(r#"StrokeWeight=""#)
        .map(|(i, _)| {
            let rest = &xml[i + 14..];
            &rest[..rest.find('"').expect("closing quote")]
        })
        .collect()
}

fn parsed() -> idml_import::Spread {
    idml_import::parse_spread(STROKED).expect("parse")
}

/// The premise, asserted rather than assumed: the model genuinely cannot
/// spell these values back, so nothing below can pass for the wrong
/// reason. If the stored precision or the output format is ever widened,
/// this fails FIRST and says so.
#[test]
fn the_model_cannot_reproduce_the_source_spelling() {
    let spread = parsed();
    for (got, want, source) in [
        (
            spread.graphic_lines[0].stroke_weight,
            "0.7087",
            "0.7086614173228347",
        ),
        (
            spread.rectangles[0].stroke_weight,
            "5.6693",
            "5.669291338582678",
        ),
        (
            spread.ovals[0].stroke_weight,
            "1.4173",
            "1.4173228346456694",
        ),
        (
            spread.polygons[0].stroke_weight,
            "0.299",
            "0.2990447182611103",
        ),
    ] {
        let v = got.expect("the parser read a stroke weight");
        assert_eq!(
            fmt4(v),
            want,
            "a re-derived weight truncates {source} to {want}"
        );
        assert_ne!(fmt4(v), source, "…and cannot spell the source back");
        // The f32 itself still holds the value to its own precision — the
        // loss is the OUTPUT format's, which is why preservation (not a
        // wider field) is the fix.
        assert_eq!(
            v,
            source.parse::<f32>().expect("corpus float"),
            "the model value IS the source spelling's own parse"
        );
    }
}

/// THE DEFECT, closed. Nothing was mutated, so nothing may be rewritten.
#[test]
fn an_unmutated_stroked_spread_round_trips_byte_identically() {
    let out = rewrite_spread(STROKED, &parsed()).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(STROKED),
        "an unmutated spread must round-trip byte-identically"
    );
}

/// Redundant with the byte comparison above, deliberately: this one names
/// WHICH bytes were at risk, on all four stroked page-item kinds.
#[test]
fn every_stroke_weight_spelling_survives_verbatim() {
    let out = rewrite_spread(STROKED, &parsed()).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        weights_in(&xml),
        vec![
            "0.7086614173228347",
            "5.669291338582678",
            "1.4173228346456694",
            "0.2990447182611103",
        ],
        "no stroke weight may be re-derived on an unmutated save"
    );
}

/// The fix must not disable the save-back. A real edit still writes, at
/// the writer's own precision, and leaves every other weight alone.
#[test]
fn a_stroke_weight_edit_still_saves() {
    let mut spread = parsed();
    spread.graphic_lines[0].stroke_weight = Some(2.25);
    let out = rewrite_spread(STROKED, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        weights_in(&xml),
        vec![
            "2.25",
            "5.669291338582678",
            "1.4173228346456694",
            "0.2990447182611103",
        ],
        "the changed weight is rewritten; its neighbours keep their bytes:\n{xml}"
    );
}

/// A weight nudged by LESS than the writer's 4-decimal output precision is
/// still an edit, and still saves — the preserving check compares the
/// model number against the source spelling's own parse, not against the
/// rounded output, so it cannot swallow a sub-ten-thousandth change into
/// the verbatim lane.
#[test]
fn a_sub_precision_stroke_weight_edit_still_saves() {
    let mut spread = parsed();
    let original = spread.rectangles[0].stroke_weight.expect("weight");
    let nudged = f32::from_bits(original.to_bits() + 1);
    assert_eq!(
        fmt4(nudged),
        fmt4(original),
        "premise: the nudge is invisible at the writer's output precision"
    );
    spread.rectangles[0].stroke_weight = Some(nudged);

    let out = rewrite_spread(STROKED, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        weights_in(&xml)[1],
        fmt4(nudged),
        "an edit the output format cannot show is still an edit:\n{xml}"
    );
    assert_ne!(
        weights_in(&xml)[1],
        "5.669291338582678",
        "the source spelling must not survive an edit"
    );
}

/// A weight CLEARED in the model still drops the attribute — the `Remove`
/// arm is reachable, not shadowed by the verbatim check.
#[test]
fn a_cleared_stroke_weight_still_drops_the_attribute() {
    let mut spread = parsed();
    spread.rectangles[0].stroke_weight = None;
    let out = rewrite_spread(STROKED, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert!(
        !xml.contains("5.669291338582678"),
        "the cleared weight must not survive:\n{xml}"
    );
    assert_eq!(
        weights_in(&xml).len(),
        3,
        "exactly one attribute was dropped:\n{xml}"
    );
}

/// A weight SET on an item whose source element carried none is still
/// appended — the extras lane is untouched by this change.
#[test]
fn a_new_stroke_weight_is_still_appended_to_an_item_that_had_none() {
    const UNSTROKED: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Rectangle Self="r1" GeometricBounds="0 0 50 50" FillColor="Color/Black"/>
</Spread>
</idPkg:Spread>"#;

    let spread = idml_import::parse_spread(UNSTROKED).expect("parse");
    assert!(
        spread.rectangles[0].stroke_weight.is_none(),
        "premise: the parser leaves an absent weight absent"
    );
    let untouched = rewrite_spread(UNSTROKED, &spread).expect("rewrite");
    assert_eq!(
        untouched, UNSTROKED,
        "an item that never had the attribute must not grow one"
    );

    let mut spread = spread;
    spread.rectangles[0].stroke_weight = Some(3.0);
    let out = rewrite_spread(UNSTROKED, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        weights_in(&xml),
        vec!["3"],
        "the new weight must be appended:\n{xml}"
    );
}

/// The package the defect was measured on. 4 of its 7 spreads differed on
/// an unmutated save, ONLY on `StrokeWeight`; all 7 are byte-identical
/// now. Opt-in: the corpus is private and gitignored, so this no-ops
/// cleanly wherever it is absent.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test stroke_weight_precision \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn a_corpus_template_saves_back_byte_identically() {
    let Some(root) = corpus::root() else { return };
    let package = root.join("envato/packs/furniture-product-catalog/template.idml");
    if !package.exists() {
        eprintln!("SKIP: {} not found", package.display());
        return;
    }
    let mut checked = 0usize;
    let mut hairlines = 0usize;
    for (name, body) in corpus::spreads(&package) {
        let spread = idml_import::parse_spread(&body).expect("parse");
        let out = rewrite_spread(&body, &spread).expect("rewrite");
        checked += 1;
        hairlines += weights_in(&String::from_utf8_lossy(&body))
            .iter()
            .filter(|w| w.len() > 8)
            .count();
        assert_eq!(
            weights_in(&String::from_utf8_lossy(&out)),
            weights_in(&String::from_utf8_lossy(&body)),
            "{name}: every stroke weight must survive an unmutated save"
        );
        assert!(
            out == body,
            "{name}: an unmutated save must be byte-identical \
             ({} bytes in, {} out)",
            body.len(),
            out.len()
        );
    }
    assert!(checked > 0, "the template had reachable spreads");
    assert!(
        hairlines > 0,
        "premise: the template really does carry full-precision weights"
    );
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
