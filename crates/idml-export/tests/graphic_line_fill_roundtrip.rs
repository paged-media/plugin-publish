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

//! An attribute the MODEL does not carry is not an attribute the model
//! cleared.
//!
//! # The defect
//!
//! `paged_model::GraphicLine` has no fill field at all — a line is a
//! stroked open contour, and the model says as much in its own comments
//! ("Lines carry no fill"). The writer's vector-patch lane, which is
//! shared with Rectangle / Oval / Polygon, therefore passed
//! `fill_color: None` for a line, `None` meant `Patch::Remove`, and 191
//! `FillColor`s across 48 corpus spreads were deleted on saves that
//! changed nothing — including `FillColor="Color/c25m15y77k0"` on lines
//! InDesign itself wrote. Five more spreads lost a `FillTint` the same
//! way.
//!
//! # Why this is NOT the tint fix in a different costume
//!
//! `FillTint="-1"` needed the source SPELLING replayed against the model
//! value (`tint_sentinel_roundtrip.rs`); the two are comparable and the
//! predicate compares them. Here there is nothing to compare: no field
//! exists, no mutation can ever have touched the attribute, and the raw
//! bytes are irrelevant. The fact needed is a static property of the
//! model type, so the fix is that the kind declares it has no fill and
//! the patch lane returns *no decision* — the same stance `next` already
//! takes for the kinds with no `NextTextFrame` field, and `corners` for
//! the kinds with no corner fields. Sharing one predicate across the two
//! would have bought a name and cost the meaning.
//!
//! Growing the model a fill field instead would be a change to
//! `paged-media/core`, and the wrong one: it would model something IDML
//! does not render.
//!
//! # What this file pins
//!
//! The premise (proved empirically, not by inspection: the model cannot
//! represent a line's fill), the defect, and the invariant that the
//! kinds which DO model a fill still patch it.

use idml_export::rewrite::rewrite_spread;

const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<GraphicLine Self="gl1" ItemTransform="1 0 0 1 0 0" GeometricBounds="10 10 110 210" FillColor="Color/c25m15y77k0" FillTint="60" StrokeColor="Color/Black" StrokeWeight="1"/>
<Rectangle Self="r1" ItemTransform="1 0 0 1 40 40" GeometricBounds="0 0 30 30" FillColor="Color/Paper" FillTint="30"/>
</Spread>
</idPkg:Spread>"#;

/// The same spread with the LINE's fill attributes changed and nothing
/// else. If the model could represent them, the two parses would differ.
const SPREAD_OTHER_FILL: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<GraphicLine Self="gl1" ItemTransform="1 0 0 1 0 0" GeometricBounds="10 10 110 210" FillColor="Color/Black" FillTint="5" StrokeColor="Color/Black" StrokeWeight="1"/>
<Rectangle Self="r1" ItemTransform="1 0 0 1 40 40" GeometricBounds="0 0 30 30" FillColor="Color/Paper" FillTint="30"/>
</Spread>
</idPkg:Spread>"#;

fn spread() -> idml_import::Spread {
    idml_import::parse_spread(SPREAD).expect("parse")
}

// -------------------------------------------------------------------
// Premise.
// -------------------------------------------------------------------

/// The model cannot represent a line's fill — demonstrated, not
/// asserted. Two sources that differ ONLY in the line's `FillColor` and
/// `FillTint` parse to identical `GraphicLine`s, which is precisely what
/// makes the writer's `None` mean "not modelled" rather than "cleared".
#[test]
fn premise_the_model_cannot_represent_a_lines_fill() {
    let a = idml_import::parse_spread(SPREAD).expect("parse a");
    let b = idml_import::parse_spread(SPREAD_OTHER_FILL).expect("parse b");
    let line = |s: &idml_import::Spread| format!("{:?}", s.graphic_lines);
    assert_eq!(
        line(&a),
        line(&b),
        "premise: changing a line's fill changes nothing in the model"
    );
    // The control: the same experiment on a kind that DOES model a fill
    // has to come out different, or the comparison above proves nothing.
    let rect = |s: &idml_import::Spread| {
        s.rectangles
            .first()
            .and_then(|r| r.fill_color.clone())
            .unwrap_or_default()
    };
    assert_eq!(rect(&a), "Color/Paper");
    assert_ne!(
        format!("{:?}", a.graphic_lines),
        format!("{:?}", a.rectangles),
        "premise: the two kinds are distinct shapes"
    );
    let mut c = idml_import::parse_spread(SPREAD).expect("parse c");
    c.rectangles[0].fill_color = Some("Color/Black".to_string());
    assert_ne!(
        rect(&a),
        c.rectangles
            .first()
            .and_then(|r| r.fill_color.clone())
            .unwrap_or_default(),
        "premise: a rectangle's fill IS model-owned"
    );
}

// -------------------------------------------------------------------
// The defect.
// -------------------------------------------------------------------

#[test]
fn a_graphic_lines_fill_attributes_survive_a_save() {
    let xml =
        String::from_utf8(rewrite_spread(SPREAD, &spread()).expect("rewrite")).expect("utf-8");
    assert!(
        xml.contains(r#"FillColor="Color/c25m15y77k0""#),
        "the line's FillColor must not be deleted:\n{xml}"
    );
    assert!(xml.contains(r#"FillTint="60""#), "nor its FillTint:\n{xml}");
}

#[test]
fn an_unmutated_spread_with_a_filled_line_round_trips_byte_identically() {
    let out = rewrite_spread(SPREAD, &spread()).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(SPREAD),
        "an unmutated save must not change a byte"
    );
}

/// The writer must not INVENT the attributes either — a line whose
/// source never carried a fill does not acquire one.
#[test]
fn a_graphic_line_without_a_fill_does_not_gain_one() {
    const BARE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<GraphicLine Self="gl1" ItemTransform="1 0 0 1 0 0" GeometricBounds="10 10 110 210" StrokeColor="Color/Black"/>
</Spread>
</idPkg:Spread>"#;
    let s = idml_import::parse_spread(BARE).expect("parse");
    let out = rewrite_spread(BARE, &s).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(BARE),
        "an unmutated save must not change a byte"
    );
}

// -------------------------------------------------------------------
// The invariant the fix must not cost.
// -------------------------------------------------------------------

/// The kinds that DO model a fill still patch it — set, changed and
/// cleared. Standing down for `<GraphicLine>` must not stand down for
/// everyone.
#[test]
fn a_modelled_fill_still_patches() {
    let mut s = spread();
    s.rectangles[0].fill_color = Some("Color/Black".to_string());
    s.rectangles[0].fill_tint = Some(90.0);
    let xml = String::from_utf8(rewrite_spread(SPREAD, &s).expect("rewrite")).expect("utf-8");
    assert!(
        xml.contains(r#"FillColor="Color/Black""#),
        "a rectangle's changed fill must be written:\n{xml}"
    );
    assert!(
        xml.contains(r#"FillTint="90""#),
        "…and its changed tint:\n{xml}"
    );

    let mut s = spread();
    s.rectangles[0].fill_color = None;
    s.rectangles[0].fill_tint = None;
    let xml = String::from_utf8(rewrite_spread(SPREAD, &s).expect("rewrite")).expect("utf-8");
    assert!(
        !xml.contains(r#"FillColor="Color/Paper""#),
        "a rectangle's CLEARED fill must go:\n{xml}"
    );
    assert!(
        !xml.contains(r#"FillTint="30""#),
        "…and its cleared tint:\n{xml}"
    );
    // The line's, meanwhile, is still nobody's to touch.
    assert!(
        xml.contains(r#"FillColor="Color/c25m15y77k0""#),
        "the line is not collateral:\n{xml}"
    );
}

/// The corpus template the defect was measured on (11 of the 48 spreads
/// with a deleted line fill). Opt-in.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test graphic_line_fill_roundtrip \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn corpus_graphic_line_fills_survive_an_unmutated_save() {
    let Some(root) = corpus::root() else { return };
    let package = corpus::package(&root, "idml/packs/catalog-brochure-template/template.idml");
    let mut with_line_fills = 0usize;
    for (name, body) in corpus::spreads(&package) {
        let text = String::from_utf8_lossy(&body).into_owned();
        if !text.contains("<GraphicLine") {
            continue;
        }
        with_line_fills += 1;
        let spread = idml_import::parse_spread(&body).expect("parse");
        let out = rewrite_spread(&body, &spread).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            text,
            "{name}: an unmutated save must not change a byte"
        );
    }
    assert!(
        with_line_fills > 0,
        "premise: the template really does carry <GraphicLine>s"
    );
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
