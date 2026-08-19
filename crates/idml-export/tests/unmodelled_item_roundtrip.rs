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

//! An element the PARSER declined to model is not a DELETED item.
//!
//! # The defect
//!
//! `finalize_page_item` discards a page item that supplies no geometry
//! at all — no `GeometricBounds` attribute *and* no path anchors — so
//! downstream code never sees a zero-rect ghost. An empty
//! `<PathPointArray>` produces exactly that, and
//! `envato/packs/brand-guidelines`' `Spread_u1db62` carries two
//! `<Polygon>`s in that shape.
//!
//! The writer's structural-remove lane asks "is this element's `Self`
//! still in the model?" and reads *no* as "the user deleted it". So
//! `u20048` and `u2004c` were in the source package and absent from the
//! saved one — 4,532 bytes of the user's document, on a save that
//! changed nothing, with no error and a still-valid package. Same
//! severity as the dropped master spread; a different cause.
//!
//! This is the THIRD defect the empty-`<PathPointArray>` shape has
//! produced. It killed a save outright (a panic, fixed in `88b5ac8`) and
//! it silently overwrote text-wrap geometry before that. The shape keeps
//! finding new lanes because the two sides disagree about what it means,
//! so the fix states the meaning once, on the side that owns it: the
//! parser records what it declined, and the writer looks up.
//!
//! # What this file pins
//!
//! The PREMISE (the parser really does decline this shape — otherwise
//! the rest of the file proves nothing), the defect (the element and its
//! whole subtree survive a save), and the invariant that must not
//! regress on the way: a genuine removal still removes.

use idml_export::rewrite::rewrite_spread;

/// The `brand-guidelines` shape, minimised. `p_ghost` carries an EMPTY
/// `<PathPointArray>` and no `GeometricBounds`; `r1` / `p_real` are
/// ordinary neighbours, above and below it, so a mis-triage of the ghost
/// would be visible as a reordering too.
const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Rectangle Self="r1" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 20 20" FillColor="Color/Black"/>
<Polygon Self="p_ghost" ContentType="Unassigned" FillColor="Color/c25m15y77k0" StrokeWeight="1" ItemTransform="1 0 0 1 0 0">
<Properties>
<PathGeometry>
<GeometryPathType PathOpen="false">
<PathPointArray>
</PathPointArray>
</GeometryPathType>
</PathGeometry>
</Properties>
<TextWrapPreference Inverse="false" TextWrapMode="None">
<Properties>
<TextWrapOffset Top="0" Left="0" Bottom="0" Right="0"/>
</Properties>
</TextWrapPreference>
<InCopyExportOption IncludeGraphicProxies="true" IncludeAllResources="false"/>
</Polygon>
<Polygon Self="p_real" ItemTransform="1 0 0 1 40 40" GeometricBounds="0 0 30 30" FillColor="Color/Paper"/>
</Spread>
</idPkg:Spread>"#;

fn parsed() -> idml_import::Spread {
    idml_import::parse_spread(SPREAD).expect("parse")
}

fn rewritten(spread: &idml_import::Spread) -> String {
    String::from_utf8(rewrite_spread(SPREAD, spread).expect("rewrite")).expect("utf-8")
}

// -------------------------------------------------------------------
// Premises — the facts the defect rests on. These pass at BOTH the
// parent commit and this one; if one ever fails, the assertions below
// are proving something other than what they claim.
// -------------------------------------------------------------------

/// The parser really does decline the shape. Without this the "removal"
/// below would be an ordinary, correct removal and the fix would be
/// papering over a parse bug instead.
#[test]
fn premise_the_parser_models_no_item_for_an_empty_path_point_array() {
    let s = parsed();
    let ids: Vec<&str> = s
        .polygons
        .iter()
        .filter_map(|p| p.self_id.as_deref())
        .collect();
    assert!(
        !ids.contains(&"p_ghost"),
        "premise: an empty <PathPointArray> with no GeometricBounds must \
         produce no model item; got {ids:?}"
    );
    assert!(
        ids.contains(&"p_real"),
        "premise: the ordinary polygon IS modelled; got {ids:?}"
    );
}

/// And it says so, keyed by the `Self` the writer joins on.
#[test]
fn premise_the_parser_records_what_it_declined() {
    let (_, prov) = idml_import::parse_spread_with_provenance(SPREAD).expect("parse");
    assert!(
        prov.is_unmodelled("p_ghost"),
        "premise: the provenance names the declined element"
    );
    assert!(
        !prov.is_unmodelled("p_real"),
        "premise: a modelled element is not named"
    );
    assert!(
        !prov.is_unmodelled("r1"),
        "premise: nor is an ordinary rectangle"
    );
}

// -------------------------------------------------------------------
// The defect.
// -------------------------------------------------------------------

/// The element survives — start tag, subtree and close.
#[test]
fn an_unmodelled_page_item_is_not_deleted_on_save() {
    let xml = rewritten(&parsed());
    assert!(
        xml.contains(r#"<Polygon Self="p_ghost""#),
        "the source's polygon must still be there:\n{xml}"
    );
    assert!(
        xml.contains("</Polygon>"),
        "…and so must its close tag:\n{xml}"
    );
    // Its subtree, which the removal lane swallowed wholesale.
    assert!(
        xml.contains(r#"<TextWrapPreference Inverse="false" TextWrapMode="None">"#),
        "the whole subtree rides along:\n{xml}"
    );
    assert!(
        xml.contains(r#"<InCopyExportOption IncludeGraphicProxies="true""#),
        "…including the trailing children:\n{xml}"
    );
}

/// Nothing about it is rewritten either. An element the model never saw
/// is not the model's to reformat any more than it is the model's to
/// delete — and the whole spread is byte-identical because of it.
#[test]
fn an_unmutated_spread_with_an_unmodelled_item_round_trips_byte_identically() {
    let out = rewrite_spread(SPREAD, &parsed()).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(SPREAD),
        "an unmutated save must not change a byte"
    );
}

// -------------------------------------------------------------------
// The invariant the fix must not cost.
// -------------------------------------------------------------------

/// A REAL removal still removes. The fix narrows the remove lane to
/// items the model actually once held; it must not disable it.
#[test]
fn a_genuinely_removed_item_is_still_removed() {
    let mut s = parsed();
    s.polygons
        .retain(|p| p.self_id.as_deref() != Some("p_real"));
    s.frames_in_order.clear();
    let xml = rewritten(&s);
    assert!(
        !xml.contains(r#"Self="p_real""#),
        "an item the model dropped must go:\n{xml}"
    );
    // …and the declined one is still untouched by that lane.
    assert!(
        xml.contains(r#"Self="p_ghost""#),
        "the declined element is not collateral:\n{xml}"
    );
}

/// The corpus entry the defect was measured on: two polygons, −4,532
/// bytes. Opt-in — the corpus is private and gitignored, so this no-ops
/// cleanly wherever it is absent.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test unmodelled_item_roundtrip \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn corpus_brand_guidelines_keeps_its_two_anchorless_polygons() {
    let Some(root) = corpus::root() else { return };
    let package = root.join("vendor/envato/packs/brand-guidelines/template.idml");
    if !package.exists() {
        eprintln!("SKIP: {} not found", package.display());
        return;
    }
    let mut checked = 0usize;
    for (name, body) in corpus::spreads(&package) {
        if !name.ends_with("Spread_u1db62.xml") {
            continue;
        }
        checked += 1;
        let spread = idml_import::parse_spread(&body).expect("parse");
        let out = rewrite_spread(&body, &spread).expect("rewrite");
        let text = String::from_utf8_lossy(&out);
        for id in ["u20048", "u2004c"] {
            let needle = format!("Self=\"{id}\"");
            assert!(
                String::from_utf8_lossy(&body).contains(&needle),
                "premise: {id} is in the source entry {name}"
            );
            assert!(
                text.contains(&needle),
                "{id} must survive the save of {name}"
            );
        }
        assert_eq!(
            out.len(),
            body.len(),
            "{name}: an unmutated save must not change its size"
        );
    }
    assert_eq!(checked, 1, "the measured spread entry was found");
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
