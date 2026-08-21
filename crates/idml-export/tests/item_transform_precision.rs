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

//! An UNTOUCHED `ItemTransform` must save back as the bytes it arrived as.
//!
//! # The defect
//!
//! IDML spells a transform at full decimal precision — real corpus
//! values, all used verbatim below:
//!
//! ```text
//! ItemTransform="1 0 0 1 0 -1021.8897637779996"
//! ItemTransform="-0.9659258262890683 0.25881904510252074 …"
//! ```
//!
//! The model stores `[f32; 6]` and the writer re-emitted it through
//! `format_f32`, which rounds to 4 decimals — so
//! `-1021.8897637779996` came back as `-1021.8898` on a save that
//! changed nothing. Measured on the 99-package corpus that was **32 630**
//! rewritten attributes across **596** differing spread entries, **509**
//! of which differed by NOTHING ELSE. It kept the save-back's
//! byte-identity gap pinned near 600 no matter what else was fixed, which
//! is the permanently-red-gate failure mode: the number stopped being a
//! signal.
//!
//! Two thirds of those were pure rounding; the other **8 839** were worse
//! than rounding — the re-emitted number had MOVED, past the last decimal
//! it did print. A group MEMBER is stored composed into spread space, and
//! `f32` carries ~7 significant digits, so at the magnitudes InDesign
//! actually writes the composition is lossy: a member at
//! `3457.1334792175285` inside a group at `9360.511811009803` composes to
//! `12817.6455078125`, where one ULP is already `9.8e-4`. Subtracting the
//! group back out yields `3457.1337890625` — three ten-thousandths of a
//! point away, i.e. wrong in the last digit the writer prints.
//! `DEEP_TRANSLATION` is that exact corpus pairing.
//!
//! # The fix
//!
//! Preserve the source bytes, the same way the z-order save-back is
//! non-lossy by splicing rather than re-deriving. The writer re-runs the
//! PARSER's own forward derivation (`compose(group_accum, on_disk)` —
//! `idml_import`'s `effective_item_transform`) against the source
//! spelling; if that reproduces the model matrix bit-for-bit, nothing
//! touched the transform and the original attribute passes through
//! untouched.
//!
//! Checking FORWARD is what makes this work for a group member. The
//! writer's existing recovery goes backward — `inverse(group) ∘ composed`
//! — and the round-off above means the recovered matrix is generally NOT
//! the one on disk. Forward, the check is an exact replay of the
//! computation that produced the model value, so it is bit-exact by
//! construction.
//!
//! Widening the stored precision was the alternative, and it is a poorer
//! fit. It would not fix this on its own — `format_f32` rounds the OUTPUT
//! to 4 decimals whatever the field holds — so it is really two changes,
//! the second of which (a shortest-round-trip formatter) would rewrite
//! the spelling of every value the writer legitimately authors. And it
//! still would not guarantee reproducing the digits a third-party writer
//! chose. Preservation is a different property from precision, and
//! preservation is the one a round-trip claim needs.
//!
//! # What still re-derives
//!
//! A transform that WAS edited fails the check and is written from the
//! model exactly as before — there the derived value is the truth and
//! verbatim would be wrong. `transform_edits_still_save*` pin that.

use idml_export::rewrite::rewrite_spread;

/// A real `Spreads/Spread_uf6.xml` shape from
/// `envato/packs/ancient-building-magazine`, minimised: one top-level
/// frame, and a group whose member carries a rotated, far-translated
/// transform. Every `ItemTransform` spelling here is copied from that
/// package.
const HIGH_PRECISION: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Rectangle Self="r1" ItemTransform="1 0 0 1 0 -1021.8897637779996" GeometricBounds="0 0 50 50" FillColor="Color/Black"/>
<TextFrame Self="tf1" ItemTransform="1 0 0 1 -534.476377953026 294.4259134145127" GeometricBounds="0 0 40 90" ParentStory="st1"/>
<Group Self="g1" ItemTransform="1 0 0 1 -294.4043156793173 -304.19133213498253">
<Polygon Self="p1" ItemTransform="-0.9659258262890683 0.25881904510252074 0.25881904510252074 0.9659258262890683 7445.486330843208 -736.7717825638288" FillColor="Color/Black">
	<Properties>
		<PathGeometry>
			<GeometryPathType PathOpen="false">
				<PathPointArray>
					<PathPointType Anchor="0 0" LeftDirection="0 0" RightDirection="0 0" />
					<PathPointType Anchor="0 50" LeftDirection="0 50" RightDirection="0 50" />
					<PathPointType Anchor="50 50" LeftDirection="50 50" RightDirection="50 50" />
				</PathPointArray>
			</GeometryPathType>
		</PathGeometry>
	</Properties>
</Polygon>
<TextFrame Self="tf2" ItemTransform="1 0 0 1 195.48031539614882 396.0750603756012" GeometricBounds="0 0 30 60" ParentStory="st2"/>
</Group>
</Spread>
</idPkg:Spread>"#;

/// A real `Spreads/Spread_u197.xml` pairing from
/// `envato/packs/annual-report-template-8b5d40`: a group 9360pt down the
/// pasteboard holding a member 3457pt further down again. The composed
/// spread-space coordinate lands where `f32`'s ULP is bigger than the
/// last decimal the writer prints, so de-composing the member back can
/// only ever produce a WRONG number — this is the case verbatim
/// preservation exists for.
const DEEP_TRANSLATION: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Group Self="g1" ItemTransform="1 0 0 1 -21.281346544262625 9360.511811009803">
<TextFrame Self="tf1" ItemTransform="1 0 0 1 -665.8060438305663 3457.1334792175285" GeometricBounds="0 0 40 90" ParentStory="st1"/>
</Group>
</Spread>
</idPkg:Spread>"#;

/// The writer's output precision (`rewrite::format_f32`, private): round
/// to 4 decimals, drop trailing zeros and a dangling `.`. Re-stated here
/// so the tests can assert what a RE-DERIVED value would look like.
fn fmt4(v: f32) -> String {
    let r = (v * 10_000.0).round() / 10_000.0;
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

fn fmt_matrix(m: &[f32; 6]) -> String {
    m.iter().map(|v| fmt4(*v)).collect::<Vec<_>>().join(" ")
}

/// Every `ItemTransform="…"` value in document order.
fn transforms_in(xml: &str) -> Vec<&str> {
    xml.match_indices(r#"ItemTransform=""#)
        .map(|(i, _)| {
            let rest = &xml[i + 15..];
            &rest[..rest.find('"').expect("closing quote")]
        })
        .collect()
}

/// The premise, asserted rather than assumed: the model really does lose
/// these digits, so the tests below cannot pass for the wrong reason. If
/// the stored precision is ever widened this fails FIRST and says so.
#[test]
fn the_model_cannot_reproduce_the_source_spelling() {
    let spread = idml_import::parse_spread(HIGH_PRECISION).expect("parse");
    let rect = spread.rectangles[0].item_transform.expect("transform");
    assert_eq!(
        fmt_matrix(&rect),
        "1 0 0 1 0 -1021.8898",
        "a re-derived top-level transform truncates 12 digits away"
    );

    // A group member is stored COMPOSED into spread space, so its on-disk
    // spelling has to be recovered by inverting the group. At the ordinary
    // magnitudes above that only rounds…
    let composed = spread.polygons[0].item_transform.expect("transform");
    let group = spread.groups[0].item_transform.expect("group transform");
    // member = inverse(group) ∘ composed; both groups here are pure
    // translations, so the inverse is a subtraction.
    assert_eq!(
        fmt4(composed[4] - group[4]),
        "7445.4863",
        "a re-derived member transform truncates too"
    );

    // …but far enough down the pasteboard the recovery is not merely
    // rounded, it has MOVED: the composition through `f32` costs more
    // than the last decimal the writer prints. No output format recovers
    // that, which is why the fix has to be preservation.
    const DEEP_MEMBER_TY: f64 = 3_457.133_479_217_528_5;
    let deep = idml_import::parse_spread(DEEP_TRANSLATION).expect("parse");
    let composed = deep.text_frames[0].item_transform.expect("transform");
    let group = deep.groups[0].item_transform.expect("group transform");
    let recovered = f64::from(composed[5]) - f64::from(group[5]);
    assert!(
        (recovered - DEEP_MEMBER_TY).abs() > 1e-4,
        "de-composing the member drifts past the printed precision \
         (recovered {recovered}, on disk {DEEP_MEMBER_TY})"
    );
    assert_ne!(
        fmt4(composed[5] - group[5]),
        "3457.1334792175285",
        "and the writer's own formatter cannot spell it back either"
    );
}

/// The worse-than-rounding case, closed: the pasteboard-deep group member
/// keeps its bytes even though nothing could have re-derived them.
#[test]
fn a_pasteboard_deep_group_member_round_trips_byte_identically() {
    let spread = idml_import::parse_spread(DEEP_TRANSLATION).expect("parse");
    let out = rewrite_spread(DEEP_TRANSLATION, &spread).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(DEEP_TRANSLATION),
        "an unmutated spread must round-trip byte-identically"
    );
}

/// THE DEFECT, closed. Nothing was mutated, so nothing may be rewritten.
#[test]
fn an_unmutated_high_precision_spread_round_trips_byte_identically() {
    let spread = idml_import::parse_spread(HIGH_PRECISION).expect("parse");
    let out = rewrite_spread(HIGH_PRECISION, &spread).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(HIGH_PRECISION),
        "an unmutated spread must round-trip byte-identically"
    );
}

/// Stated as the thing a reader will look for: every spelling survives,
/// top-level and group member alike. Redundant with the byte comparison
/// above, and deliberately so — this one names WHICH bytes were at risk.
#[test]
fn every_transform_spelling_survives_verbatim() {
    let spread = idml_import::parse_spread(HIGH_PRECISION).expect("parse");
    let out = rewrite_spread(HIGH_PRECISION, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        transforms_in(&xml),
        transforms_in(&String::from_utf8_lossy(HIGH_PRECISION)),
        "no transform may be re-derived on an unmutated save"
    );
}

/// The fix must not disable the save-back. A real edit to a TOP-LEVEL
/// transform still writes, at the writer's own precision, and leaves
/// every other transform alone.
#[test]
fn transform_edits_still_save_at_top_level() {
    let mut spread = idml_import::parse_spread(HIGH_PRECISION).expect("parse");
    spread.rectangles[0].item_transform = Some([1.0, 0.0, 0.0, 1.0, 12.5, -30.25]);

    let out = rewrite_spread(HIGH_PRECISION, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        transforms_in(&xml),
        vec![
            "1 0 0 1 12.5 -30.25",
            "1 0 0 1 -534.476377953026 294.4259134145127",
            "1 0 0 1 -294.4043156793173 -304.19133213498253",
            "-0.9659258262890683 0.25881904510252074 0.25881904510252074 0.9659258262890683 7445.486330843208 -736.7717825638288",
            "1 0 0 1 195.48031539614882 396.0750603756012",
        ],
        "the moved item is rewritten; its neighbours keep their bytes:\n{xml}"
    );
}

/// The same for a GROUP MEMBER, which is the case the forward check had
/// to be designed around: the model value is composed into spread space,
/// so a moved member is still de-composed back through the group.
#[test]
fn transform_edits_still_save_for_a_group_member() {
    let mut spread = idml_import::parse_spread(HIGH_PRECISION).expect("parse");
    let group = spread.groups[0].item_transform.expect("group transform");
    // Nudge the member 10pt right IN SPREAD SPACE, the way a drag does.
    let composed = spread.polygons[0].item_transform.expect("transform");
    let mut moved = composed;
    moved[4] += 10.0;
    spread.polygons[0].item_transform = Some(moved);

    let out = rewrite_spread(HIGH_PRECISION, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    let written = transforms_in(&xml)[3];
    // The group is a pure translation, so the expected on-disk value is
    // the composed one minus the group's own.
    let expected = fmt4(moved[4] - group[4]);
    assert!(
        written.contains(&expected),
        "a moved group member must save its de-composed transform \
         (wanted tx {expected}, got {written})"
    );
    assert!(
        !written.contains("7445.486330843208"),
        "the edit must not be swallowed by the verbatim lane: {written}"
    );
    assert_eq!(
        transforms_in(&xml)[4],
        "1 0 0 1 195.48031539614882 396.0750603756012",
        "the member that did NOT move keeps its bytes:\n{xml}"
    );
}

/// A transform CLEARED in the model still drops the attribute — the
/// `Remove` arm is reachable, not shadowed by the verbatim check.
#[test]
fn a_cleared_transform_still_drops_the_attribute() {
    let mut spread = idml_import::parse_spread(HIGH_PRECISION).expect("parse");
    spread.rectangles[0].item_transform = None;

    let out = rewrite_spread(HIGH_PRECISION, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert!(
        !xml.contains("-1021.8897637779996"),
        "the cleared transform must not survive:\n{xml}"
    );
    assert_eq!(
        transforms_in(&xml).len(),
        4,
        "exactly one attribute was dropped:\n{xml}"
    );
}

/// A transform SET on an item whose source element carried none is still
/// appended — the extras lane is only suppressed when the model matrix is
/// what an ABSENT attribute already derives.
#[test]
fn a_new_transform_is_still_appended_to_an_item_that_had_none() {
    const NO_TRANSFORM: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Rectangle Self="r1" GeometricBounds="0 0 50 50" FillColor="Color/Black"/>
</Spread>
</idPkg:Spread>"#;

    let spread = idml_import::parse_spread(NO_TRANSFORM).expect("parse");
    assert!(
        spread.rectangles[0].item_transform.is_none(),
        "premise: the parser leaves an absent transform absent"
    );
    let untouched = rewrite_spread(NO_TRANSFORM, &spread).expect("rewrite");
    assert_eq!(
        untouched, NO_TRANSFORM,
        "an item that never had the attribute must not grow one"
    );

    let mut spread = spread;
    spread.rectangles[0].item_transform = Some([1.0, 0.0, 0.0, 1.0, 7.0, 3.0]);
    let out = rewrite_spread(NO_TRANSFORM, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        transforms_in(&xml),
        vec!["1 0 0 1 7 3"],
        "the new transform must be appended:\n{xml}"
    );
}

/// The same rule on the ABSENT side. A group member with no
/// `ItemTransform` of its own still has one in the model — the group's,
/// composed in — so the writer used to de-compose it back to identity and
/// APPEND `ItemTransform="1 0 0 1 0 0"` to an element that never carried
/// the attribute. Absence is the source spelling of identity, and it has
/// to survive as absence.
#[test]
fn a_group_member_with_no_transform_does_not_grow_one() {
    const BARE_MEMBER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Group Self="g1" ItemTransform="1 0 0 1 -294.4043156793173 -304.19133213498253">
<Rectangle Self="r1" GeometricBounds="0 0 50 50" FillColor="Color/Black"/>
</Group>
</Spread>
</idPkg:Spread>"#;

    let spread = idml_import::parse_spread(BARE_MEMBER).expect("parse");
    assert_eq!(
        spread.rectangles[0].item_transform, spread.groups[0].item_transform,
        "premise: the member's composed transform IS the group's"
    );
    let out = rewrite_spread(BARE_MEMBER, &spread).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(BARE_MEMBER),
        "an unmutated spread must round-trip byte-identically"
    );
}

/// The package the defect was measured on. All 7 of its spreads differed
/// on an unmutated save, ONLY on `ItemTransform`; all 7 are byte-identical
/// now. Opt-in: the corpus is private and gitignored, so this no-ops
/// cleanly wherever it is absent.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test item_transform_precision \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn a_corpus_template_saves_back_byte_identically() {
    let Some(root) = corpus::root() else { return };
    let package = corpus::package(&root, "idml/packs/ancient-building-magazine/template.idml");
    let mut checked = 0usize;
    for (name, body) in corpus::spreads(&package) {
        let spread = idml_import::parse_spread(&body).expect("parse");
        let out = rewrite_spread(&body, &spread).expect("rewrite");
        checked += 1;
        assert_eq!(
            transforms_in(&String::from_utf8_lossy(&out)),
            transforms_in(&String::from_utf8_lossy(&body)),
            "{name}: every transform spelling must survive an unmutated save"
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
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
