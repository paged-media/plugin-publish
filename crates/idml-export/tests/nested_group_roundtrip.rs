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

//! A `<Group>` nested inside a page item must not be DUPLICATED on save.
//!
//! # The defect
//!
//! InDesign's paste-into puts a whole `<Group>` inside a `<Rectangle>`.
//! The parser deliberately registers such a group in
//! `Spread::frames_in_order` anyway — the documented B-18 residual
//! "flatten to top level, unclipped" (see
//! `paged_model::Spread::skipped_nested_frames`); the alternative is
//! that the pasted-in subtree is unreachable and never paints.
//!
//! C-22's insert-placement pre-pass read the model's top-level z-table
//! against the ids the source carries **at its top level** and called
//! anything missing "new". A nested group is missing from that list, is
//! owned by no other item, and so was re-minted on top of the copy the
//! source already had: `envato/packs/catalog/template.idml`'s
//! `Spreads/Spread_udc.xml` saved back at 1.37 MB from 832 KB, with a
//! duplicated group subtree. Six spreads of that one template accounted
//! for 7.4 MB of duplicated bytes.
//!
//! # Why the fix is on the WRITER side
//!
//! The stream pass already holds the invariant — every source `<Group>`
//! id is marked seen unconditionally (a group takes no part in
//! `triage_placement`), which is exactly why the close-of-spread insert
//! flush never duplicated it. The C-22 pre-pass introduced a second,
//! depth-blind newness test that contradicted the first. Teaching the
//! pre-pass about groups at any depth restores one rule.
//!
//! The parser side is NOT the place: dropping the group from
//! `frames_in_order` would trade a save-side duplication for the pasted
//! subtree vanishing from the canvas, and lifting it into
//! `Spread::nested_children` (the real B-18 completion — clipping plus
//! container-transform composition) is a render-semantics change this
//! repo cannot validate.

use idml_export::rewrite::rewrite_spread;

/// The `catalog` shape, minimised: a `<Group>` pasted INSIDE a
/// `<Rectangle>`, with a plain sibling above and below it so an insert
/// has real anchors to choose between.
const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Rectangle Self="r0" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 20 20" FillColor="Color/Black"/>
<Rectangle Self="host" ItemTransform="1 0 0 1 10 10" GeometricBounds="0 0 200 200" FillColor="Color/Paper">
<TextWrapPreference Inverse="false" TextWrapMode="None"/>
<Group Self="g1" ItemTransform="1 0 0 1 5 5">
<Rectangle Self="m1" ItemTransform="1 0 0 1 1 1" GeometricBounds="0 0 10 10" FillColor="Color/Black"/>
<TextFrame Self="m2" ItemTransform="1 0 0 1 2 2" GeometricBounds="0 0 30 40" ParentStory="st1"/>
</Group>
</Rectangle>
<Oval Self="o1" ItemTransform="1 0 0 1 30 30" GeometricBounds="0 0 70 70" FillColor="Color/Black"/>
</Spread>
</idPkg:Spread>"#;

fn count(haystack: &str, id: &str) -> usize {
    haystack.matches(&format!(r#"Self="{id}""#)).count()
}

/// The parse-side premise this whole test file rests on, asserted rather
/// than assumed: the nested `<Group>` DOES land in the spread's z-table,
/// and its members land on the group. If that ever changes, this file
/// should fail loudly rather than keep passing for the wrong reason.
#[test]
fn parser_flattens_a_pasted_in_group_into_the_z_table() {
    let spread = idml_import::parse_spread(SPREAD).expect("parse");
    assert_eq!(spread.groups.len(), 1);
    assert_eq!(spread.groups[0].self_id.as_deref(), Some("g1"));
    assert_eq!(
        spread.groups[0].members.len(),
        2,
        "the group kept both members"
    );
    assert!(
        spread
            .frames_in_order
            .contains(&idml_import::FrameRef::Group(0)),
        "B-18 residual: a pasted-in group still surfaces in the z-table \
         so the renderer paints it — {:?}",
        spread.frames_in_order
    );
    assert!(
        !spread.nested_children.contains_key("host"),
        "a pasted-in GROUP is not lifted into nested_children (only plain \
         page items are) — that is what makes it look top-level"
    );
}

/// THE DEFECT, closed. An unmutated spread carrying a pasted-in group
/// round-trips byte-identically; before the fix the group's whole
/// subtree was minted a second time.
#[test]
fn nested_group_is_not_duplicated_on_an_unmutated_save() {
    let spread = idml_import::parse_spread(SPREAD).expect("parse");
    let out = rewrite_spread(SPREAD, &spread).expect("rewrite");
    let xml = String::from_utf8(out.clone()).expect("utf8");
    for id in ["r0", "host", "g1", "m1", "m2", "o1"] {
        assert_eq!(count(&xml, id), 1, "{id} must appear exactly once:\n{xml}");
    }
    assert_eq!(
        out, SPREAD,
        "an unmutated spread must round-trip byte-identically:\n{xml}"
    );
}

/// The fix must not disable C-22. A genuinely NEW top-level item still
/// lands at its z position among the source items — immediately before
/// the source item that follows it in the model's z-table — rather than
/// being dumped at the spread's close.
#[test]
fn a_real_insert_still_lands_at_its_anchor() {
    let mut spread = idml_import::parse_spread(SPREAD).expect("parse");
    // `InsertNode` — a rectangle created since load (modelled the way
    // the unit tests do: clone a source item, give it a fresh id),
    // slotted between the host rectangle and the oval.
    let idx = spread.rectangles.len();
    let mut fresh = spread.rectangles[0].clone();
    fresh.self_id = Some("new1".to_string());
    spread.rectangles.push(fresh);
    let at = spread
        .frames_in_order
        .iter()
        .position(|r| *r == idml_import::FrameRef::Oval(0))
        .expect("the oval is in the z-table");
    spread
        .frames_in_order
        .insert(at, idml_import::FrameRef::Rectangle(idx));

    let out = rewrite_spread(SPREAD, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    let pos = |id: &str| xml.find(&format!(r#"Self="{id}""#)).expect("emitted");
    assert!(pos("host") < pos("new1"), "{xml}");
    assert!(
        pos("new1") < pos("o1"),
        "the insert anchors to the oval:\n{xml}"
    );
    // ...and the nested group is still written exactly once.
    for id in ["g1", "m1", "m2"] {
        assert_eq!(count(&xml, id), 1, "{id} must appear exactly once:\n{xml}");
    }
}

/// The corpus template the defect was measured on. Opt-in: the corpus is
/// private and gitignored, so this no-ops cleanly wherever it is absent
/// (mirrors plugin-image's `PAGED_PSD_ORACLE` lane).
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test nested_group_roundtrip \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn corpus_catalog_spread_does_not_inflate() {
    let Some(root) = corpus::root() else { return };
    let package = root.join("idml/packs/catalog/template.idml");
    if !package.exists() {
        eprintln!("SKIP: {} not found", package.display());
        return;
    }
    // Every spread of the template; `Spread_udc.xml` is the 832 KB →
    // 1.37 MB one, but all six carried the same defect.
    let (mut checked, mut grew) = (0usize, 0usize);
    for (name, body) in corpus::spreads(&package) {
        let spread = idml_import::parse_spread(&body).expect("parse");
        let out = rewrite_spread(&body, &spread).expect("rewrite");
        checked += 1;
        if out.len() > body.len() {
            grew += 1;
            eprintln!("GREW {name}: {} -> {}", body.len(), out.len());
        }
    }
    assert!(checked > 0, "the template has spreads to check");
    assert_eq!(
        grew, 0,
        "an unmutated save must never make a spread bigger — that is \
         duplicated content"
    );
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
