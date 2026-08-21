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

//! An attribute whose SPELLING already says what the model says is not a
//! cleared attribute.
//!
//! # The defect
//!
//! `FillTint="-1"` is IDML's sentinel for *no tint override* — the same
//! document as writing no `FillTint` at all — so `parse_tint` maps it to
//! `None`. The writer spelled `None` as `Patch::Remove`, i.e. "the user
//! cleared this", and deleted 288 attributes across 156 corpus stories
//! and 6 spreads on saves that changed nothing.
//!
//! `Nonprinting="false"` is the same mistake in a second vocabulary: the
//! parser defaults an absent attribute to `false`, and the writer dropped
//! the attribute to restore that default — including from documents that
//! had spelled the default out. One corpus spread carries both on the
//! same elements, which is why fixing only the tint would have left it a
//! gap.
//!
//! Both are the same FACT — "does the source spelling denote what the
//! model holds?" — answerable from the raw bytes and the model value
//! alone, so they share one predicate shape
//! (`preserving_tint_patch` / `preserving_bool_patch`). That is NOT the
//! fact the `<GraphicLine>` fill defect needs (a static property of the
//! model type) or the one the unmodelled-item defect needs (the parser's
//! own record), so those do not share the code — see
//! `graphic_line_fill_roundtrip.rs` and `unmodelled_item_roundtrip.rs`.
//!
//! # What this file pins
//!
//! The premises (the sentinel really does parse to `None`, on both a
//! page item and a character run), the defect (the spelling survives a
//! save), and the invariants: a genuinely CLEARED tint still clears, and
//! a MUTATED one still writes.

use idml_export::rewrite::{rewrite_spread, rewrite_story};

const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Rectangle Self="r_sentinel" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 20 20" FillColor="Color/Black" FillTint="-1" Nonprinting="false"/>
<Polygon Self="p_real" ItemTransform="1 0 0 1 40 40" GeometricBounds="0 0 30 30" FillColor="Color/Paper" FillTint="40"/>
</Spread>
</idPkg:Spread>"#;

const STORY: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Story Self="st1">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/[No paragraph style]">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" FillColor="Color/Black" FillTint="-1">
<Content>hello</Content>
</CharacterStyleRange>
</ParagraphStyleRange>
</Story>
</idPkg:Story>"#;

fn spread() -> idml_import::Spread {
    idml_import::parse_spread(SPREAD).expect("parse spread")
}

fn story() -> idml_import::Story {
    idml_import::parse_story(STORY).expect("parse story")
}

// -------------------------------------------------------------------
// Premises.
// -------------------------------------------------------------------

/// The rule itself, in the one place it lives.
#[test]
fn premise_the_sentinel_parses_to_no_override() {
    assert_eq!(
        idml_import::parse_tint("-1"),
        None,
        "premise: -1 is the sentinel"
    );
    assert_eq!(idml_import::parse_tint("40"), Some(40.0));
    assert_eq!(idml_import::parse_tint("0"), Some(0.0));
    assert_eq!(idml_import::parse_tint("100"), Some(100.0));
    assert_eq!(
        idml_import::parse_tint("101"),
        None,
        "premise: out of range is also 'no override'"
    );
}

/// And the parser really applies it — on a page item…
#[test]
fn premise_a_spread_items_sentinel_tint_is_none_in_the_model() {
    let s = spread();
    let r = s
        .rectangles
        .iter()
        .find(|r| r.self_id.as_deref() == Some("r_sentinel"))
        .expect("premise: the rectangle is modelled");
    assert_eq!(
        r.fill_tint, None,
        "premise: FillTint=\"-1\" reaches the model as None"
    );
    assert!(
        !r.nonprinting,
        "premise: Nonprinting=\"false\" reaches the model as false"
    );
    let p = s
        .polygons
        .iter()
        .find(|p| p.self_id.as_deref() == Some("p_real"))
        .expect("premise: the polygon is modelled");
    assert_eq!(
        p.fill_tint,
        Some(40.0),
        "premise: a REAL tint is distinguishable from the sentinel"
    );
}

/// …and on a character run, which is where 156 of the 162 entries were.
#[test]
fn premise_a_runs_sentinel_tint_is_none_in_the_model() {
    let st = story();
    let run = st
        .paragraphs
        .first()
        .and_then(|p| p.runs.first())
        .expect("premise: the run is modelled");
    assert_eq!(
        run.fill_tint, None,
        "premise: FillTint=\"-1\" reaches the model as None"
    );
    assert_eq!(
        run.fill_color.as_deref(),
        Some("Color/Black"),
        "premise: the run IS the one carrying the attribute"
    );
}

// -------------------------------------------------------------------
// The defect.
// -------------------------------------------------------------------

#[test]
fn a_spread_items_sentinel_tint_survives_a_save() {
    let out = rewrite_spread(SPREAD, &spread()).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf-8");
    assert!(
        xml.contains(r#"FillTint="-1""#),
        "the sentinel must not be deleted:\n{xml}"
    );
}

#[test]
fn an_explicit_nonprinting_false_survives_a_save() {
    let out = rewrite_spread(SPREAD, &spread()).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf-8");
    assert!(
        xml.contains(r#"Nonprinting="false""#),
        "an explicitly-spelled default must not be deleted:\n{xml}"
    );
}

#[test]
fn a_runs_sentinel_tint_survives_a_save() {
    let out = rewrite_story(STORY, &story()).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf-8");
    assert!(
        xml.contains(r#"FillTint="-1""#),
        "the sentinel must not be deleted:\n{xml}"
    );
}

#[test]
fn the_unmutated_entries_round_trip_byte_identically() {
    let out = rewrite_spread(SPREAD, &spread()).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(SPREAD),
        "spread: an unmutated save must not change a byte"
    );
    let out = rewrite_story(STORY, &story()).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(STORY),
        "story: an unmutated save must not change a byte"
    );
}

// -------------------------------------------------------------------
// The invariants the fix must not cost.
// -------------------------------------------------------------------

/// A tint the user really CLEARED still clears. The source spelled a
/// real 40; the model now says none; that is a mutation and the
/// attribute has to go.
#[test]
fn a_cleared_tint_is_still_removed() {
    let mut s = spread();
    for p in &mut s.polygons {
        p.fill_tint = None;
    }
    let xml = String::from_utf8(rewrite_spread(SPREAD, &s).expect("rewrite")).expect("utf-8");
    assert!(
        !xml.contains(r#"FillTint="40""#),
        "the cleared tint must go:\n{xml}"
    );
    // …while the sentinel elsewhere is untouched.
    assert!(
        xml.contains(r#"FillTint="-1""#),
        "the sentinel is not collateral:\n{xml}"
    );
}

/// A CHANGED tint still writes.
#[test]
fn a_mutated_tint_is_still_written() {
    let mut s = spread();
    for p in &mut s.polygons {
        p.fill_tint = Some(75.0);
    }
    let xml = String::from_utf8(rewrite_spread(SPREAD, &s).expect("rewrite")).expect("utf-8");
    assert!(
        xml.contains(r#"FillTint="75""#),
        "the new tint must be written:\n{xml}"
    );
}

/// A tint SET where the source spelled the sentinel still writes — the
/// preserving rule keeps bytes only while the two agree.
#[test]
fn a_tint_set_over_the_sentinel_is_still_written() {
    let mut s = spread();
    for r in &mut s.rectangles {
        r.fill_tint = Some(20.0);
    }
    let xml = String::from_utf8(rewrite_spread(SPREAD, &s).expect("rewrite")).expect("utf-8");
    assert!(
        xml.contains(r#"FillTint="20""#),
        "the new tint must replace the sentinel:\n{xml}"
    );
    assert!(
        !xml.contains(r#"FillTint="-1""#),
        "…and the sentinel must not survive alongside it:\n{xml}"
    );
}

/// `Nonprinting` still round-trips a real change in both directions.
#[test]
fn a_mutated_nonprinting_is_still_written() {
    let mut s = spread();
    for r in &mut s.rectangles {
        r.nonprinting = true;
    }
    let xml = String::from_utf8(rewrite_spread(SPREAD, &s).expect("rewrite")).expect("utf-8");
    assert!(
        xml.contains(r#"Nonprinting="true""#),
        "turning it on must be written:\n{xml}"
    );

    // And a source that spelled the NON-default, turned back off, loses
    // the attribute — the implicit default is restored, as before.
    const ON: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Rectangle Self="r1" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 20 20" Nonprinting="true"/>
</Spread>
</idPkg:Spread>"#;
    let mut s = idml_import::parse_spread(ON).expect("parse");
    assert!(
        s.rectangles[0].nonprinting,
        "premise: the source spelled the non-default"
    );
    s.rectangles[0].nonprinting = false;
    let xml = String::from_utf8(rewrite_spread(ON, &s).expect("rewrite")).expect("utf-8");
    assert!(
        !xml.contains("Nonprinting"),
        "turning it off restores the implicit default:\n{xml}"
    );
}

/// The corpus entries the defect was measured on. Opt-in.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test tint_sentinel_roundtrip \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn corpus_sentinel_tints_survive_an_unmutated_save() {
    let Some(root) = corpus::root() else { return };
    // One package per lane: the stories carrying `FillTint="-1"`, and
    // the spread that carries it alongside `Nonprinting="false"`.
    let cases: [(&str, bool); 2] = [
        (
            "idml/packs/company-profile-canva-docx-id-psd/template.idml",
            true,
        ),
        ("idml/packs/brand-guideline-template/template.idml", false),
    ];
    let mut seen_sentinels = 0usize;
    for (rel, do_stories) in cases {
        let package = corpus::package(&root, rel);
        let entries = if do_stories {
            corpus::stories(&package)
        } else {
            corpus::spreads(&package)
        };
        for (name, body) in entries {
            if !String::from_utf8_lossy(&body).contains(r#"FillTint="-1""#) {
                continue;
            }
            seen_sentinels += 1;
            let out = if do_stories {
                let st = idml_import::parse_story(&body).expect("parse story");
                rewrite_story(&body, &st).expect("rewrite story")
            } else {
                let sp = idml_import::parse_spread(&body).expect("parse spread");
                rewrite_spread(&body, &sp).expect("rewrite spread")
            };
            assert_eq!(
                String::from_utf8_lossy(&out),
                String::from_utf8_lossy(&body),
                "{name}: an unmutated save must not change a byte"
            );
        }
    }
    assert!(
        seen_sentinels > 0,
        "premise: the corpus really does carry FillTint=\"-1\" entries"
    );
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
