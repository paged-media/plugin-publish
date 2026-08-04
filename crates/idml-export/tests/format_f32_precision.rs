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

//! The writer's "4 decimals" is a real contract at every magnitude — it
//! used to stop being one at 1677.7216 pt.
//!
//! # The defect
//!
//! `rewrite::format_f32` rounded by computing `(v * 10_000.0).round()` in
//! `f32`. That multiply is never exact — an `f32` significand is 24 bits
//! and the product needs up to 34 — so the value `.round()` sees has
//! already slipped. Below `2^24 / 10_000 = 1677.7216` the slip can only
//! carry the product across one `.5` boundary, so the damage is bounded at
//! a single wrong ten-thousandth. At and above that magnitude the product
//! passes `2^24`, where the spacing between representable `f32`s is
//! greater than 1: `.round()` stops rounding anything at all, and the
//! error grows with the number.
//!
//! Measured exhaustively over every `f32` in the range (not sampled):
//!
//! ```text
//! band                worst error   first witness
//! [1677.7216, 2048)      2 units    1677.7225 -> printed 1677.7227
//! [2048, 4096)           3 units    3355.4434 -> printed 3355.4431
//! [8192, 16384)         10 units    13421.775 -> printed 13421.7764
//! [65536, 131072)       79 units    107374.29 -> printed 107374.2812
//! ```
//!
//! (units of the 1e-4 the function claims to round to; 14 % of all `f32`
//! values above the wall are affected.) The very first value above the
//! wall is already damaged, just not yet by more than a unit: `1677.7217`
//! and `1677.7216` both used to print `1677.7216`.
//!
//! # Why this is not only a byte-identity concern
//!
//! `format_f32` is what the writer emits on a MUTATED save — a dragged
//! item's `ItemTransform`, a resized frame's `GeometricBounds`. An
//! InDesign pasteboard runs to five figures of points, so this is
//! reachable by editing, not only by round-tripping, and nothing
//! announced it.
//!
//! # What the fix does, and what it does NOT
//!
//! The rounding now happens in `f64`, where the multiply IS exact (34 bits
//! fits in 53), so the digits printed are the correctly-rounded ones at
//! every magnitude. Above the wall that is exactly lossless: 1e-4 is finer
//! than the input's own ULP there, so the emitted spelling re-parses to
//! the very same `f32` (`the_emitted_spelling_reparses_*`).
//!
//! It does not buy precision the `f32` never had. Above the wall the
//! input's ULP is *coarser* than the ten-thousandth being printed — at
//! 13 kpt, adjacent representable values are ten units apart — so most
//! 4-decimal spellings are simply unreachable, and the number was already
//! that coarse before it reached the formatter
//! (`the_fix_does_not_add_precision_the_f32_never_had`). Below the wall,
//! 4 decimals still discards most of what an `f32` holds, exactly as
//! before (`below_the_wall_four_decimals_is_still_lossy`) — that is the
//! output format InDesign uses, and preserving a source spelling rather
//! than re-deriving it is a different lane (see `TransformPlan` and
//! `preserving_f32_patch`), which only untouched values get.

use idml_export::rewrite::rewrite_spread;

/// One rectangle, no `<PathGeometry>`, so both the `ItemTransform` and the
/// `GeometricBounds` attribute lanes are reachable by mutating the model.
const ONE_RECT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Rectangle Self="r1" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 50 50" FillColor="Color/Black"/>
</Spread>
</idPkg:Spread>"#;

/// The magnitude past which an `f32` can no longer hold the integer count
/// of ten-thousandths the old rounding step needed: `2^24 / 10_000`.
const WALL: f32 = 1677.7216;

/// Drag the rectangle to `ty` and read back the `ItemTransform` the writer
/// emitted for it.
fn saved_transform_ty(ty: f32) -> String {
    let mut spread = idml_import::parse_spread(ONE_RECT).expect("parse");
    spread.rectangles[0].item_transform = Some([1.0, 0.0, 0.0, 1.0, 0.0, ty]);
    let out = rewrite_spread(ONE_RECT, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    let i = xml.find(r#"ItemTransform=""#).expect("attribute");
    let rest = &xml[i + 15..];
    let value = &rest[..rest.find('"').expect("closing quote")];
    value
        .rsplit(' ')
        .next()
        .expect("six components")
        .to_string()
}

/// The correctly-rounded 4-decimal spelling of an `f32`, computed the slow
/// honest way: exact `f64` arithmetic on the exact value, printed from the
/// integer count of ten-thousandths so no float formatting is trusted.
fn correctly_rounded(v: f32) -> String {
    let quanta = (f64::from(v) * 10_000.0).round() as i64;
    if quanta == 0 {
        return "0".to_string();
    }
    let q = quanta.unsigned_abs();
    let mut s = format!("{}.{:04}", q / 10_000, q % 10_000);
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if quanta < 0 {
        format!("-{s}")
    } else {
        s
    }
}

/// The distance to the next representable `f32`.
fn ulp(v: f32) -> f64 {
    f64::from(f32::from_bits(v.to_bits() + 1)) - f64::from(v)
}

/// The premise, asserted rather than assumed: this is a property of `f32`,
/// not of our code. Above `2^24 / 10_000` the product `v * 10_000` lands
/// where representable values are more than 1 apart, so `.round()` — the
/// entire point of the step — cannot do anything.
#[test]
fn the_f32_multiply_gives_out_at_the_2_24_wall() {
    assert_eq!(
        WALL,
        (1u32 << 24) as f32 / 10_000.0,
        "the wall is 2^24 ten-thousandths"
    );

    // Just below: the product is still in the exactly-representable
    // integer range, so rounding is meaningful.
    let below = f32::from_bits(WALL.to_bits() - 1);
    assert!(
        (below * 10_000.0) < (1u32 << 24) as f32,
        "premise: below the wall the product is under 2^24"
    );
    assert_eq!(
        ulp(below * 10_000.0),
        1.0,
        "…where consecutive f32 are still 1 apart"
    );

    // Just above: consecutive representable products are 2 apart, so
    // `.round()` is a no-op on a value that is already wrong.
    let above = f32::from_bits(WALL.to_bits() + 1);
    assert_eq!(
        ulp(above * 10_000.0),
        2.0,
        "above the wall the product's own spacing exceeds 1"
    );

    // …so the very first value above the wall already collides with the
    // wall itself: two distinct `f32` used to print the same spelling.
    assert_eq!(
        format!("{:.4}", (above * 10_000.0).round() / 10_000.0),
        "1677.7216",
        "the shipped f32 rounding cannot tell {above} from {WALL}"
    );
    assert_eq!(
        correctly_rounded(above),
        "1677.7217",
        "…though a ten-thousandth resolves them perfectly well"
    );

    // The first value the old step got wrong by MORE than one
    // ten-thousandth, found by walking every f32 upward from the wall.
    let witness = 1677.7225f32;
    let f32_rounded = (witness * 10_000.0).round() / 10_000.0;
    let f64_rounded = (f64::from(witness) * 10_000.0).round() / 10_000.0;
    assert!(
        (f64::from(f32_rounded) - f64_rounded).abs() > 1.5e-4,
        "the shipped f32 rounding lands >1 ten-thousandth away at {witness}"
    );
}

/// THE DEFECT, closed, on the path a user reaches by dragging: a moved
/// item's coordinate is emitted correctly rounded. The old code printed
/// `13421.7725` here — nine ten-thousandths off, and not even a value the
/// `f32` can represent.
#[test]
fn a_dragged_item_saves_its_pasteboard_coordinate_correctly() {
    let ty = 13_421.773f32;
    assert_eq!(
        saved_transform_ty(ty),
        "13421.7734",
        "a pasteboard-scale drag must save its correctly-rounded coordinate"
    );
    assert_eq!(saved_transform_ty(ty), correctly_rounded(ty));
    assert_ne!(
        saved_transform_ty(ty),
        "13421.7725",
        "the f32-rounded spelling the old writer emitted"
    );
}

/// The same on the `GeometricBounds` lane, which a resize writes.
#[test]
fn a_resized_frame_saves_its_pasteboard_bounds_correctly() {
    let bottom = 3355.4434f32;
    let mut spread = idml_import::parse_spread(ONE_RECT).expect("parse");
    spread.rectangles[0].bounds.bottom = bottom;
    let out = rewrite_spread(ONE_RECT, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert!(
        xml.contains(r#"GeometricBounds="0 0 3355.4434 50""#),
        "a resized frame must save its correctly-rounded bounds:\n{xml}"
    );
    assert_eq!(correctly_rounded(bottom), "3355.4434");
    assert!(
        !xml.contains("3355.4431"),
        "the f32-rounded spelling the old writer emitted:\n{xml}"
    );
}

/// ABOVE the wall the fix is exactly lossless: a ten-thousandth is finer
/// than the input's own ULP there, so the spelling the writer emits
/// re-parses to the very same `f32`. Swept across every octave from the
/// wall to 33 kpt — an InDesign pasteboard does not reach further.
#[test]
fn the_emitted_spelling_reparses_to_the_same_f32_above_the_wall() {
    let mut checked = 0usize;
    let mut v = WALL;
    // Every 8191st representable value: coprime with the stride between
    // octaves, so the samples do not line up with any binade boundary.
    while v < 33_554.43 {
        let saved = saved_transform_ty(v);
        assert_eq!(
            saved,
            correctly_rounded(v),
            "the emitted spelling must be the correctly-rounded one"
        );
        assert_eq!(
            saved.parse::<f32>().expect("parses back"),
            v,
            "above the wall the emitted spelling must recover the exact f32"
        );
        checked += 1;
        v = f32::from_bits(v.to_bits() + 8191);
    }
    assert!(checked > 500, "the sweep covered the range ({checked})");
}

/// The limit of the claim, stated so nobody reads more into the fix than
/// it does. BELOW the wall an `f32` resolves far finer than a
/// ten-thousandth, so rounding to 4 decimals throws information away — it
/// did before this change and it still does. That is InDesign's output
/// format, not a defect; the answer to it is preserving a source spelling
/// rather than re-deriving it, which is a different lane.
#[test]
fn below_the_wall_four_decimals_is_still_lossy() {
    let v = 0.708_661_4f32; // a 0.25 mm hairline, in points
    assert!(
        ulp(v) < 1e-4,
        "premise: down here the f32 resolves finer than the printed quantum"
    );
    let saved = saved_transform_ty(v);
    assert_eq!(saved, "0.7087", "still rounded to 4 decimals");
    assert_ne!(
        saved.parse::<f32>().expect("parses back"),
        v,
        "and still not recoverable from those 4 decimals"
    );
}

/// The other half of the honesty: above the wall the printed digits are
/// exact FOR THE F32 YOU HAVE, but the `f32` is coarser than they suggest.
/// Widening the arithmetic does not change that, and no output format
/// could.
#[test]
fn the_fix_does_not_add_precision_the_f32_never_had() {
    for v in [1677.7225f32, 3355.4434, 13_421.773, 107_374.29] {
        assert!(
            ulp(v) > 1e-4,
            "at {v} the input's own spacing ({:.3e}) already exceeds the \
             ten-thousandth being printed",
            ulp(v)
        );
    }
    // Concretely: at 13 kpt the representable values are ten quanta apart,
    // so nine out of every ten 4-decimal spellings cannot come out of this
    // writer at all.
    let v = 13_421.773f32;
    let next = f32::from_bits(v.to_bits() + 1);
    assert_eq!(correctly_rounded(v), "13421.7734");
    assert_eq!(correctly_rounded(next), "13421.7744");
}
