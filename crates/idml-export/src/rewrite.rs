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

//! Attribute-preserving streaming rewrite of Spread / Story XML.
//!
//! Both rewriters share one shape: a `quick_xml::Reader` feeds events,
//! a `quick_xml::Writer` re-emits them. The vast majority of events
//! (processing instructions, comments, `<Properties>`, `<PathGeometry>`,
//! unknown elements, all attributes we don't own) pass through
//! **verbatim** — we hand the original [`Event`] straight to the writer
//! so its bytes are reproduced. Only the start tags of page items
//! (spreads) / style ranges (stories) and `<Content>` text are
//! reconstructed, and even then only the model-owned attributes change;
//! every other attribute keeps its original key, value, and position.
//!
//! # The model→XML mapping is positional within an element family
//!
//! IDML carries no model index on its elements, so we walk the model in
//! the same document order the parser walked it:
//!
//! * Spread page items are matched by their `Self` id (stable, present
//!   on every page item) — robust against reordering.
//! * Story `<ParagraphStyleRange>` / `<CharacterStyleRange>` carry no
//!   `Self` id, so they're matched **positionally** against
//!   `Story::paragraphs[i].runs[j]` in document order. This is the same
//!   order the parser produced them, so an unmutated story round-trips,
//!   and a mutated story (which edits values in place, never inserts /
//!   deletes ranges) stays aligned.
//!
//! # Patch inventory (what is save-able)
//!
//! Spread page items (`TextFrame` / `Rectangle` / `Oval` / `Polygon` /
//! `GraphicLine`), patched on the element start tag:
//!   - `ItemTransform`     (FrameTransform / rotate / scale / flip / move)
//!     — but only when it actually MOVED. IDML spells a transform at full
//!     decimal precision and the model stores `f32`, so re-deriving an
//!     untouched one truncated it (`-1021.8897637779996` →
//!     `-1021.8898`). An untouched transform now passes through verbatim;
//!     see [`TransformPlan`].
//!   - `FillColor`         (FrameFillColor) — on the kinds that MODEL a
//!     fill. `<GraphicLine>` does not (`paged_model::GraphicLine` has no
//!     fill field: a line is a stroked open contour), so its source
//!     `FillColor` / `FillTint` pass through verbatim rather than being
//!     read as cleared. See [`Fill`].
//!   - `FillTint`          (FrameFillTint) — `-1` is IDML's "no tint
//!     override" sentinel, which the parser maps to `None`, so a `None`
//!     model tint keeps a source `-1` instead of deleting it. See
//!     [`preserving_tint_patch`].
//!   - `StrokeColor`       (FrameStrokeColor)
//!   - `StrokeWeight`      (FrameStrokeWeight) — the same rule as the
//!     corner radii: an untouched weight keeps its source spelling,
//!     because IDML writes a hairline as `0.7086614173228347` (0.25 mm in
//!     points) and `format_f32` would re-derive it as `0.7087`. Unlike
//!     `ItemTransform` there is nothing to de-compose — see
//!     [`preserving_f32_patch`].
//!   - `NextTextFrame`     (LinkFrames / UnlinkFrames; TextFrame only)
//!   - `Nonprinting`       (FrameNonprinting) — absence is the implicit
//!     `false`, so turning it off drops the attribute; a source that
//!     already spelled `"false"` keeps its bytes. See
//!     [`preserving_bool_patch`].
//!   - `AppliedObjectStyle` (AppliedObjectStyle) — the reference into
//!     the `<ObjectStyle>` definitions [`crate::resources::patch_styles`]
//!     writes. IDML has no "no object style", so a CLEARED reference
//!     writes the reserved [`NONE_OBJECT_STYLE`] sentinel instead of
//!     dropping the attribute.
//!   - `GeometricBounds`   (FrameBounds) — patched when the source
//!     element carries the attribute. When the frame's geometry instead
//!     lives in `<PathGeometry>`/`<PathPointArray>` (the real-export +
//!     generator shape), the path anchors are rewritten directly: a
//!     `FrameBounds` resize regenerates a rectangle's corners, and
//!     `FramePathPoint` / `FramePath` edits write the moved anchors. See
//!     [`ModelGeometry`].
//!
//! Story ranges:
//!   - `<ParagraphStyleRange AppliedParagraphStyle>` (AppliedParagraphStyle)
//!   - `<CharacterStyleRange AppliedCharacterStyle>` (AppliedCharacterStyle)
//!   - `<CharacterStyleRange PointSize>`   (CharacterFontSize)
//!   - `<CharacterStyleRange FillColor>`   (CharacterFillColor)
//!   - `<CharacterStyleRange Leading / Tracking / BaselineShift /
//!     HorizontalScale / VerticalScale / Skew / FillTint / StrokeWeight>`
//!     (the matching Character* paths)
//!   - `<CharacterStyleRange AppliedFont / FontStyle / Capitalization /
//!     Position / KerningMethod / AppliedLanguage / StrokeColor /
//!     Underline / StrikeThru / Ligatures>` (the matching Character* paths)
//!   - run text — replaced across the run's `<Content>` / `<Br/>` /
//!     `<Tab/>` structure. The parser collapses
//!     `<Content>A</Content><Br/><Content>B</Content>` into one run
//!     string `"A\nB"`; the rewrite splits the model text back the same
//!     way (`\n` → `<Br/>`, `\t` → `<Tab/>`). A run carrying foreign
//!     inline markup (an `<?ACE?>` page-number PI, a
//!     `<TextVariableInstance>`, an anchored frame, an unknown entity)
//!     passes through verbatim — never clobbered (see Known losses).
//!
//! # Structural edits (W1.15 — landed)
//!
//! * **Page-item inserts / removes.** A page item created by an
//!   `InsertNode` op (a frame / rect / oval / polygon since load) is
//!   serialised as a new element at the spread's close, in the canonical
//!   `paged_gen` shape (geometry in `<Properties><PathGeometry>` at the
//!   model bounds, identity `ItemTransform`); an item removed by
//!   `RemoveNode` is dropped from the XML (element + subtree). See
//!   [`write_inserted_items`] / the `remove_depth` skip in
//!   [`rewrite_spread`]. An inserted item carries its full paint —
//!   fill + `FillTint`, stroke + weight, and the
//!   `<TransparencySetting><BlendingSetting>` opacity / blend-mode pair
//!   (C-19; before that the write_new_* lane emitted fill/stroke only,
//!   so a tint or an opacity set on a freshly-created item was lost).
//!   Emission ORDER is the model's own z-table
//!   (`Spread::frames_in_order`) — the order the renderer paints in —
//!   not the per-kind vec order, which `InsertNode`'s `position`
//!   argument can leave reversed relative to creation.
//! * **Group inserts (C-19).** A group the scene created —
//!   `CreateGroup`, e.g. paged.draw's appearance bake — emits as a real
//!   `<Group Self ItemTransform>` with its members NESTED inside it (see
//!   [`write_new_group`]). Members whose elements the source already
//!   carried elsewhere are dropped from their old position and re-emitted
//!   inside the wrapper, so nothing is duplicated; members added to an
//!   EXISTING `<Group>` flush just before that group's close tag, the
//!   same shape as the B-18 container flush. Member transforms are
//!   re-based out of spread space by the group's composed transform.
//! * **Z-ORDER (C-23).** Core's v59 `ReorderNode` — Arrange — permutes
//!   the sibling list a node lives in (`Spread::frames_in_order`, a
//!   `Group::members`, a `Spread::nested_children` entry). IDML spells
//!   that order as ELEMENT order, so the [`crate::reorder`] post-pass
//!   splices each serialised page item into the slot the model asks for,
//!   over this pass's output. Whole elements move, subtree included —
//!   nothing is re-minted, so a moved item keeps the children and
//!   attributes the model never modeled. A no-op (returning the input
//!   buffer) unless the order actually diverged.
//! * **New resources.** Swatches / gradients / paragraph + character
//!   styles created by ops are injected into `Resources/Graphic.xml` /
//!   `Resources/Styles.xml` (see the `resources` module), so a frame
//!   referencing a freshly-minted `Color/u3` resolves on re-open.
//! * **Table-cell text + styles.** A `<Cell Self="...">` is matched to
//!   its model `TableCell`, and its `<ParagraphStyleRange>` /
//!   `<CharacterStyleRange>` patch against the cell paragraphs with
//!   cell-local cursors (text + character-style attrs save).
//! * **Group-member transforms.** The composed group∘member
//!   `item_transform` is de-composed back to the on-disk member
//!   transform by inverting the group-transform accumulation (see
//!   [`recover_member_transform`]). That inversion is only reached for a
//!   member that actually moved: an untouched one is recognised by
//!   replaying the composition FORWARD against the source bytes and keeps
//!   them (see [`TransformPlan`]) — which matters most exactly where the
//!   inversion is least trustworthy, since `f32` composition at
//!   pasteboard magnitudes loses more than the writer's printed precision.
//!
//! # Known losses (documented, not silent)
//!
//! * **Removed PAGES leave an orphaned entry.** A `RemovePage` drops
//!   the `ParsedSpread` from the model, but the writer doesn't delete
//!   the spread's ZIP entry or its `designmap.xml` `<idPkg:Spread>` ref
//!   — the page survives on reopen. (INSERTED pages/spreads — and
//!   stories minted by InsertTextFrame — DO save since C-8: the `emit`
//!   module serialises a full part for any model spread/story with no
//!   source entry and references it from designmap.) Master-spread
//!   inserts and the removal manifest-drop remain deferred — an EDIT to
//!   an existing master does save now (`write_idml` runs
//!   `MasterSpreads/*.xml` through this same rewrite); minting a master
//!   the archive never carried does not.
//! * **Singular group transform.** A group whose `ItemTransform` linear
//!   part is non-invertible can't have its member transforms de-composed;
//!   such a member keeps its `ItemTransform` verbatim (degenerate case;
//!   InDesign never emits one for a translate/rotate/scale group).
//! * **Group-member PATH anchors.** A group member's `<PathPointArray>`
//!   still passes through verbatim. (The parser does NOT compose the
//!   group transform into member anchors — it stores them raw — so a
//!   `FramePathPoint` edit on a grouped item is not yet written; the
//!   transform lane above covers the common move/scale/rotate gesture.)
//! * **A MOVED source item is re-emitted canonically.** When an item the
//!   source XML already carried changes parent (pasted into a container,
//!   or grouped by `CreateGroup`), its original element is dropped and it
//!   is rebuilt by the `write_new_*` emitters at its new home. Those
//!   rebuild the attributes + geometry + `<Label>` the model tracks, so
//!   source-only children (`<Image>`, `<TextWrapPreference>`,
//!   `<ClippingPathSettings>`, on-element corner / effect attrs the model
//!   doesn't own) are lost on the move. This is one behaviour shared by
//!   the B-18 paste-into lane and the C-19 group lane, not two. It is
//!   REPARENTING only: a pure z-order move (C-23) relocates the original
//!   bytes and loses nothing.
//! * **Group DISSOLVE and `SetGroupTransform`.** Two group lanes C-19
//!   deliberately left alone. A `<Group>` whose model entry disappeared
//!   keeps its element, and its members keep their legacy in-place
//!   treatment — including members the model has REMOVED, which is the
//!   pre-existing "inside a group, a structural remove doesn't save"
//!   rule. Concretely: bake → save → reopen → release → save leaves the
//!   old wrapper and its derived layers in the file. Fixing it means
//!   deciding what a dissolve does to z-order, which is a lane of its
//!   own. And a `<Group>`'s own `ItemTransform` is never patched from
//!   the model, so a `SetGroupTransform` does not save back; the member
//!   flush therefore re-bases against the SOURCE group transform, which
//!   is the element the members are actually written inside of.
//! * **Opacity / blend on a SOURCE item.** `<BlendingSetting Opacity /
//!   BlendMode>` is an ELEMENT, not an attribute, so the attribute-patch
//!   lane can't reach it: changing the opacity of an item that already
//!   exists in the XML still does not save (the INSERTED lane above does
//!   emit it). Closing this needs a buffered element-patch pass over
//!   `<TransparencySetting>` in the same style as `<Label>`.
//! * ~~**Inserted-item Z-SLOT.**~~ CLOSED — an insert is written at its
//!   anchor among the source items (C-22), and any residual order
//!   difference is settled by the z-order post-pass (C-23).
//! * **Runs with foreign inline markup.** A run whose text body carries
//!   an `<?ACE?>` page-number marker, a `<TextVariableInstance>`, an
//!   anchored frame, or an unknown entity passes through verbatim (its
//!   attributes still patch). The structured text rewrite only fires on
//!   pure `<Content>` / `<Br/>` / `<Tab/>` runs.
//! * **MoveNode / sections.** Reparenting a node across spreads
//!   (`MoveNode`) and new `<Section>` definitions are not yet reflected.
//! * Anything the parser never modeled (preferences, fonts, tags, the
//!   XML backing store, master-spread item internals beyond the patched
//!   attributes) is carried through verbatim and so is always faithful.

use std::io::Cursor;

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use idml_import::{
    Bounds, CharacterRun, Paragraph, PathAnchor, Spread, Story, TableCell, TextFrame,
};

/// Mirror of `paged_gen::xml::format_f32`: round to 4 decimals, drop
/// trailing zeros + a dangling `.`, normalise `-0` to `0`. Kept as a
/// small local copy rather than depending on `paged-gen` (a dev/CLI
/// crate that pulls clap/anyhow) so this runtime crate stays minimal +
/// wasm-clean. InDesign serialises floats this way, so patched values
/// match the surrounding hand-written / exported numbers.
///
/// # Why the rounding step is `f64`
///
/// `paged-gen`'s original rounds in `f32` — `(v * 10_000.0).round()` —
/// and that multiply is never exact: an `f32` significand is 24 bits and
/// the product needs up to 34, so the value `.round()` sees has already
/// slipped. Below `2^24 / 10_000 = 1677.7216` the slip only ever moves
/// the result across one `.5` boundary, i.e. a wrong LAST decimal
/// (0.06 % of values around 1 pt, rising to 17 % around 300 pt). At
/// and above that magnitude the product passes `2^24`, its own spacing
/// exceeds 1, and `.round()` stops rounding anything at all: the
/// documented "4 decimals" silently becomes coarser, by up to 3
/// ten-thousandths at 2–4 kpt and 79 at 65–131 kpt. Real documents run
/// there — an InDesign pasteboard coordinate is routinely five figures —
/// and this function emits MUTATED values, so it is not only a
/// byte-identity concern.
///
/// Widening to `f64` first makes the multiply exact (34 bits fits
/// comfortably in 53), so the digits printed are the correctly-rounded
/// ones at every magnitude. It buys precision the OUTPUT was throwing
/// away, not precision the `f32` never had: above 1677.7216 pt the
/// input's own ULP is already coarser than the 1e-4 being printed, and
/// below it 4 decimals still discards most of what an `f32` holds. See
/// `tests/format_f32_precision.rs`.
pub(crate) fn format_f32(v: f32) -> String {
    let rounded = (f64::from(v) * 10_000.0).round() / 10_000.0;
    if rounded == 0.0 {
        return "0".to_string();
    }
    let mut s = format!("{rounded:.4}");
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

/// Format a `[a b c d tx ty]` matrix the IDML way (space-separated,
/// fixed precision).
pub(crate) fn format_matrix(m: &[f32; 6]) -> String {
    let parts: Vec<String> = m.iter().map(|v| format_f32(*v)).collect();
    parts.join(" ")
}

/// Parse a `"a b c d tx ty"` IDML matrix. Local copy of the parser's
/// helper (private to `paged-parse`).
fn parse_matrix(s: &str) -> Option<[f32; 6]> {
    let mut it = s.split_whitespace();
    let mut m = [0.0f32; 6];
    for slot in &mut m {
        *slot = it.next()?.parse().ok()?;
    }
    Some(m)
}

/// `a ∘ b` — compose two affine matrices, byte-for-byte matching
/// `idml_import`'s `compose_matrix` (apply `b` first, then `a`). Used to
/// rebuild the group-transform accumulation the parser composes into a
/// group member's `item_transform`, so the writer can invert it back to
/// the on-disk member transform (W1.15 lane 4).
fn compose_matrix(a: &[f32; 6], b: &[f32; 6]) -> [f32; 6] {
    let [a1, b1, c1, d1, tx1, ty1] = *a;
    let [a2, b2, c2, d2, tx2, ty2] = *b;
    [
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * tx2 + c1 * ty2 + tx1,
        b1 * tx2 + d1 * ty2 + ty1,
    ]
}

/// Accumulate a group-transform stack the way the parser does (outer
/// groups apply first). `None` ⇒ no group carries a transform (identity).
fn accumulate_group_xforms(stack: &[Option<[f32; 6]>]) -> Option<[f32; 6]> {
    let mut acc: Option<[f32; 6]> = None;
    for g in stack {
        match (acc, g) {
            (None, Some(m)) => acc = Some(*m),
            (Some(a), Some(m)) => acc = Some(compose_matrix(&a, m)),
            (acc_, None) => acc = acc_,
        }
    }
    acc
}

/// Recover a group member's ON-DISK `ItemTransform` from its composed
/// model `item_transform` and the accumulated group transform `accum`:
/// `member_on_disk = inverse(accum) ∘ composed`. `None` when the group
/// transform is singular (the member then keeps its on-disk transform
/// verbatim — a documented loss for that degenerate case).
fn recover_member_transform(
    accum: Option<[f32; 6]>,
    composed: Option<[f32; 6]>,
) -> Option<Option<[f32; 6]>> {
    match accum {
        // No group transform ⇒ the model value IS the on-disk transform.
        None => Some(composed),
        Some(g) => {
            let inv = invert_matrix(&g)?;
            // A member with no composed transform under a non-identity
            // group is unusual; `None` falls through to verbatim (the
            // outer `None` suppresses the patch at the call site).
            composed.map(|c| Some(compose_matrix(&inv, &c)))
        }
    }
}

/// Invert an affine `[a b c d tx ty]`. `None` when the linear part is
/// singular (a degenerate group transform — the member then can't be
/// de-composed and keeps its on-disk transform verbatim).
fn invert_matrix(m: &[f32; 6]) -> Option<[f32; 6]> {
    let [a, b, c, d, tx, ty] = *m;
    let det = a * d - b * c;
    if det.abs() < 1e-9 {
        return None;
    }
    let inv_det = 1.0 / det;
    let ia = d * inv_det;
    let ib = -b * inv_det;
    let ic = -c * inv_det;
    let id = a * inv_det;
    // Inverse translation: -(inv_linear · t).
    let itx = -(ia * tx + ic * ty);
    let ity = -(ib * tx + id * ty);
    Some([ia, ib, ic, id, itx, ity])
}

/// One attribute patch: the value to write for `key`, or `Remove` to
/// drop the attribute entirely (model value went to `None` on an
/// attribute that was present).
pub(crate) enum Patch {
    Set(String),
    Remove,
    /// B-23 — model-owned, but the model value is byte-equivalent to
    /// what's on disk: emit the ORIGINAL bytes. Distinct from a `None`
    /// lookup (which means "not model-owned at all") so the intent
    /// reads at the call site.
    Keep,
}

/// Rewrite one page-item / range start tag: emit it with the same name,
/// every original attribute in its original order (model-owned keys take
/// their new value; `Remove` keys are dropped), then append any
/// model-owned keys that were newly set (absent from the source).
///
/// `lookup(key, raw_value) -> Option<Patch>`: `None` ⇒ not model-owned
/// (or model-equivalent to what's already on disk), pass the original
/// attribute through byte-for-byte. `Some(Set)` / `Some(Remove)` ⇒
/// patch it. `raw_value` is the ESCAPED on-disk value; B-23's corner
/// attributes use it to answer "would re-emitting this change bytes?"
/// — `format_f32` rounds to 4 decimals, so an untouched
/// `CornerRadius="44.51279527491718"` must pass through rather than be
/// reformatted.
/// `extras`: `(key, value)` pairs to append if the key wasn't already
/// present (newly-set model attributes). Returns the rebuilt
/// `BytesStart` preserving the element name exactly.
pub(crate) fn patch_start<F>(
    src: &BytesStart,
    lookup: F,
    extras: &[(&str, String)],
) -> Result<BytesStart<'static>, quick_xml::Error>
where
    F: Fn(&[u8], &[u8]) -> Option<Patch>,
{
    // Rebuild the start tag's raw inner content (`name attr="v" ...`)
    // by hand so unchanged attributes reproduce their ON-DISK bytes
    // exactly — no decode→re-escape round-trip that could normalise an
    // entity form and break byte-identity. `BytesStart::from_content`
    // takes this raw content and the writer emits it verbatim. IDML +
    // the generator both serialise attributes as ` key="value"` (single
    // space, double quote, no spaces around `=`); we match that so an
    // unmutated frame reproduces the source byte-for-byte.
    let name = src.name().as_ref().to_vec();
    let mut content: Vec<u8> = name.clone();
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for attr in src.attributes() {
        let attr = attr?;
        let key = attr.key.as_ref().to_vec();
        match lookup(&key, attr.value.as_ref()) {
            None | Some(Patch::Keep) => {
                // Not model-owned — copy the raw escaped value bytes.
                content.push(b' ');
                content.extend_from_slice(&key);
                content.extend_from_slice(b"=\"");
                content.extend_from_slice(attr.value.as_ref());
                content.push(b'"');
            }
            Some(Patch::Set(v)) => {
                content.push(b' ');
                content.extend_from_slice(&key);
                content.extend_from_slice(b"=\"");
                content.extend_from_slice(escape_attr(&v).as_bytes());
                content.push(b'"');
            }
            Some(Patch::Remove) => { /* dropped */ }
        }
        seen.push(key);
    }
    for (k, v) in extras {
        if !seen.iter().any(|s| s.as_slice() == k.as_bytes()) {
            content.push(b' ');
            content.extend_from_slice(k.as_bytes());
            content.extend_from_slice(b"=\"");
            content.extend_from_slice(escape_attr(v).as_bytes());
            content.push(b'"');
        }
    }
    let content = String::from_utf8(content).map_err(|e| {
        quick_xml::Error::Io(std::sync::Arc::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e,
        )))
    })?;
    Ok(BytesStart::from_content(content, name.len()).into_owned())
}

/// Escape an attribute value we synthesise.
///
/// The five XML entities, because a patched value is an IDML id / number
/// / colour ref that almost never contains them but a style name could.
///
/// And TAB / LF / CR as CHARACTER REFERENCES, which is not defensive —
/// it is the only spelling that survives a round-trip. XML 1.0 §3.3.3
/// says a parser replaces every literal tab, newline and carriage return
/// inside an attribute value with a SPACE before the value is reported,
/// while a character reference is not touched. So writing the newline
/// literally does not produce the same document: it produces one whose
/// value has spaces where the original had line breaks, silently, on
/// every read after the save.
///
/// A `<Label>`'s `KeyValuePair Value` is where this bites — plugin
/// metadata, JSON and (in `samples/sample-3.idml`) an embedded ecscript
/// document, all of them multi-line, all of them flattened one save at a
/// time. InDesign writes `&#xa;` there for exactly this reason.
pub(crate) fn escape_attr(s: &str) -> std::borrow::Cow<'_, str> {
    if s.bytes()
        .any(|b| matches!(b, b'&' | b'<' | b'>' | b'"' | b'\'' | b'\t' | b'\n' | b'\r'))
    {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                '\t' => out.push_str("&#x9;"),
                '\n' => out.push_str("&#xA;"),
                '\r' => out.push_str("&#xD;"),
                _ => out.push(c),
            }
        }
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

// ---------------------------------------------------------------------
// Path geometry
// ---------------------------------------------------------------------

/// Parse a `"x y"` IDML coordinate pair. Local copy of the parser's
/// helper (it is private to `paged-parse`).
fn parse_xy_pair(s: &str) -> Option<(f32, f32)> {
    let mut it = s.split_whitespace();
    let x: f32 = it.next()?.parse().ok()?;
    let y: f32 = it.next()?.parse().ok()?;
    Some((x, y))
}

/// Format one `(x, y)` pair the IDML way (`"x y"`, fixed precision) for a
/// `PathPointType` `Anchor` / `LeftDirection` / `RightDirection` value.
fn format_xy(p: (f32, f32)) -> String {
    format!("{} {}", format_f32(p.0), format_f32(p.1))
}

/// Stable string key for one anchor, formatted exactly the way the
/// generator / a faithful export serialises it. Comparing keys (rather
/// than raw `f32`s) gives the float-format care the round-trip needs: an
/// unchanged anchor re-formats to the same bytes, so it compares equal
/// and passes through verbatim.
fn anchor_key(a: &PathAnchor) -> (String, String, String) {
    (format_xy(a.anchor), format_xy(a.left), format_xy(a.right))
}

/// AABB of an anchor set, mirroring the parser's `bounds_from_anchors`
/// (anchors only — control handles are ignored). Empty ⇒ a zero box.
fn bounds_of(anchors: &[PathAnchor]) -> Bounds {
    let mut it = anchors.iter();
    let Some(first) = it.next() else {
        return Bounds {
            top: 0.0,
            left: 0.0,
            bottom: 0.0,
            right: 0.0,
        };
    };
    let (mut min_x, mut max_x) = (first.anchor.0, first.anchor.0);
    let (mut min_y, mut max_y) = (first.anchor.1, first.anchor.1);
    for a in it {
        let (x, y) = a.anchor;
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    Bounds {
        top: min_y,
        left: min_x,
        bottom: max_y,
        right: max_x,
    }
}

/// Two bounds equal under `format_f32` (the on-disk precision).
fn bounds_eq_formatted(a: Bounds, b: Bounds) -> bool {
    format_f32(a.top) == format_f32(b.top)
        && format_f32(a.left) == format_f32(b.left)
        && format_f32(a.bottom) == format_f32(b.bottom)
        && format_f32(a.right) == format_f32(b.right)
}

/// Degenerate-handle corner anchor (handles coincide with the anchor —
/// what the generator emits for a plain rectangle corner).
fn corner(x: f32, y: f32) -> PathAnchor {
    PathAnchor {
        anchor: (x, y),
        left: (x, y),
        right: (x, y),
    }
}

/// The four corner anchors of `bounds`, walked in the generator's order
/// (`top-left, bottom-left, bottom-right, top-right`) so a rectangle
/// resized via `FrameBounds` re-emits the same corner sequence InDesign
/// and `paged-gen` use.
fn rect_corners(b: Bounds) -> Vec<PathAnchor> {
    vec![
        corner(b.left, b.top),
        corner(b.left, b.bottom),
        corner(b.right, b.bottom),
        corner(b.right, b.top),
    ]
}

/// The four-arc Bézier circle constant.
const OVAL_KAPPA: f32 = 0.552_284_8;

/// InDesign's own `<Oval>` path (measured 2026-09-06 on a fresh oval
/// exported from InDesign 20.0.1): four anchors at the edge midpoints
/// in the order bottom, right, top, left, each with handles κ·r along
/// its edge. An `<Oval>` is only an ellipse by its path — InDesign drew
/// the four-corner box this used to write as a rectangle (the annual's
/// blend-mode plates).
fn oval_anchors(b: Bounds) -> Vec<PathAnchor> {
    let cx = (b.left + b.right) * 0.5;
    let cy = (b.top + b.bottom) * 0.5;
    let kx = (b.right - b.left) * 0.5 * OVAL_KAPPA;
    let ky = (b.bottom - b.top) * 0.5 * OVAL_KAPPA;
    vec![
        PathAnchor {
            anchor: (cx, b.bottom),
            left: (cx - kx, b.bottom),
            right: (cx + kx, b.bottom),
        },
        PathAnchor {
            anchor: (b.right, cy),
            left: (b.right, cy + ky),
            right: (b.right, cy - ky),
        },
        PathAnchor {
            anchor: (cx, b.top),
            left: (cx + kx, b.top),
            right: (cx - kx, b.top),
        },
        PathAnchor {
            anchor: (b.left, cy),
            left: (b.left, cy - ky),
            right: (b.left, cy + ky),
        },
    ]
}

/// The model's path geometry for one spread page item, plus a hint at
/// how to reconcile a divergence.
struct ModelGeometry {
    /// The item is an `<Oval>`: its geometry is bounds-only in the
    /// model, and its on-disk path is either InDesign's ellipse (kept)
    /// or the four-corner box an older export of ours wrote (re-spelled
    /// as the ellipse — see [`oval_anchors`]).
    oval: bool,
    /// Flat anchor list across all contours (model order).
    anchors: Vec<PathAnchor>,
    /// Per-contour start offsets into `anchors` (see
    /// [`idml_import::Polygon::subpath_starts`]). Empty ⇒ one contour.
    subpath_starts: Vec<usize>,
    /// Model AABB. For a `FrameBounds` edit the anchors stay stale while
    /// this moves, so a divergence here (with unchanged anchors) means
    /// "rectangle resized" — regenerate the corners from these bounds.
    bounds: Bounds,
}

impl ModelGeometry {
    /// The target anchors for the contour starting at `parsed`'s
    /// position. `contour` indexes into `subpath_starts`. `parsed` is
    /// the on-disk anchor set for this `<PathPointArray>`. Returns
    /// `Some(target)` when the contour must be rewritten, or `None` to
    /// pass it through verbatim.
    ///
    /// A contour the MODEL has no entry for passes through — see
    /// [`ModelGeometry::contour_slice`].
    fn target_for_contour(&self, contour: usize, parsed: &[PathAnchor]) -> Option<Vec<PathAnchor>> {
        if self.oval {
            let respell = contour == 0
                && (is_axis_aligned_rect(parsed)
                    || !bounds_eq_formatted(self.bounds, bounds_of(parsed)));
            return respell.then(|| oval_anchors(self.bounds));
        }
        // Bounds-only model (a plain rectangle): the parser keeps no
        // anchors for a 4-corner AABB Rectangle — its geometry lives in
        // `bounds` alone. A `FrameBounds` resize moves `bounds` while the
        // on-disk path stays, so reconcile by regenerating the corners
        // from the model bounds when they diverged (and the on-disk path
        // really is that single 4-corner rectangle).
        if self.anchors.is_empty() {
            if contour == 0
                && is_axis_aligned_rect(parsed)
                && !bounds_eq_formatted(self.bounds, bounds_of(parsed))
            {
                return Some(rect_corners(self.bounds));
            }
            return None;
        }
        let model = self.contour_slice(contour)?;
        // Anchor-edit path (FramePathPoint / FramePath): the model's
        // anchors for this contour diverged from disk → write them.
        if !anchors_eq_formatted(model, parsed) {
            return Some(model.to_vec());
        }
        // Bounds-only edit (FrameBounds): the anchors match disk but the
        // model AABB moved. Only safe to reconstruct for the rectangle
        // case — a single contour of 4 corners that *was* the old AABB.
        // (Non-rectangular bounds-only edits are ambiguous and stay a
        // documented loss.)
        if self.subpath_starts.len() <= 1
            && is_axis_aligned_rect(parsed)
            && !bounds_eq_formatted(self.bounds, bounds_of(parsed))
        {
            return Some(rect_corners(self.bounds));
        }
        None
    }

    /// The model's anchors for contour `contour`, or `None` when the
    /// model carries no such contour.
    ///
    /// # Why `None` rather than an empty slice or a panic
    ///
    /// The model's contour count and the source's `<PathPointArray>`
    /// count can legitimately disagree in ONE direction: the parser
    /// drops a **trailing empty** `<GeometryPathType>` as a "spurious
    /// subpath marker" (a start index that points past the last anchor
    /// — see idml-import's `parse_spread`). Two corpus templates ship
    /// exactly that shape (`the-brochure` `Polygon u1659`: 8 contours,
    /// the last with zero points; `soccer-career-flyer-templates`
    /// `Polygon u687a`: 7, same). Indexing `subpath_starts` for that
    /// last array panicked — and the wasm worker runs `panic = abort`,
    /// so it killed the save outright rather than surfacing an error.
    ///
    /// Returning an empty slice would be worse than the panic: the
    /// caller reads "model says no anchors, disk says some" as a
    /// divergence and REWRITES the contour, so a visible crash becomes
    /// a silent geometry loss. Returning `None` means "the model has
    /// nothing to say about these bytes", and the caller passes the
    /// source contour through untouched — which is exactly right, since
    /// the dropped marker was empty: there is no user edit to lose, and
    /// no way to express one.
    ///
    /// The single-contour canonical form (`subpath_starts == []`) is
    /// held to the same rule: it describes contour 0 and nothing else.
    /// It used to answer for EVERY index, which is how an unmutated
    /// `business-magazine-template` save overwrote a 45-point
    /// `<TextWrapPreference>` contour with the host polygon's 39
    /// anchors. (The companion half of that fix keeps foreign
    /// `<PathPointArray>`s out of the contour count altogether — see
    /// `PathCtx::foreign_depth` in [`rewrite_spread`].)
    fn contour_slice(&self, contour: usize) -> Option<&[PathAnchor]> {
        if self.subpath_starts.is_empty() {
            return (contour == 0).then_some(self.anchors.as_slice());
        }
        let start = *self.subpath_starts.get(contour)?;
        let end = self
            .subpath_starts
            .get(contour + 1)
            .copied()
            .unwrap_or(self.anchors.len());
        Some(self.anchors.get(start..end).unwrap_or(&[]))
    }
}

/// True when a 4-anchor contour is an axis-aligned rectangle: each
/// anchor sits on an AABB corner (degenerate handles) and all four
/// corners are present. This is the only shape a `FrameBounds` resize
/// can faithfully reconstruct from bounds alone — a non-rectangular
/// path needs an explicit `FramePathPoint` / `FramePath` edit, so a
/// bounds-only change there stays a documented loss.
fn is_axis_aligned_rect(anchors: &[PathAnchor]) -> bool {
    if anchors.len() != 4 {
        return false;
    }
    let b = bounds_of(anchors);
    // Each anchor must be one of the four corners (handles degenerate to
    // the anchor), and every corner must be covered exactly once.
    let corners = [
        (b.left, b.top),
        (b.left, b.bottom),
        (b.right, b.bottom),
        (b.right, b.top),
    ];
    let mut covered = [false; 4];
    for a in anchors {
        if format_xy(a.left) != format_xy(a.anchor) || format_xy(a.right) != format_xy(a.anchor) {
            return false; // a real Bezier handle — not a plain corner
        }
        let key = format_xy(a.anchor);
        match corners.iter().position(|c| format_xy(*c) == key) {
            Some(i) if !covered[i] => covered[i] = true,
            _ => return false,
        }
    }
    covered.iter().all(|&c| c)
}

/// Two anchor sets equal under `format_f32` (on-disk precision).
fn anchors_eq_formatted(a: &[PathAnchor], b: &[PathAnchor]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| anchor_key(x) == anchor_key(y))
}

/// The model path geometry for the page item `name`/`self_id` carries,
/// if that kind tracks anchors (TextFrame / Rectangle / Polygon /
/// GraphicLine). Oval geometry is bounds-only in the model (no anchors),
/// so its `<PathPointArray>` always passes through verbatim.
fn model_geometry(
    name: &[u8],
    self_id: &str,
    frames: &std::collections::HashMap<&str, &TextFrame>,
    rectangles: &[idml_import::Rectangle],
    ovals: &[idml_import::Oval],
    polygons: &[idml_import::Polygon],
    graphic_lines: &[idml_import::GraphicLine],
) -> Option<ModelGeometry> {
    match name {
        b"Oval" => ovals
            .iter()
            .find(|o| o.self_id.as_deref() == Some(self_id))
            .map(|o| ModelGeometry {
                oval: true,
                anchors: Vec::new(),
                subpath_starts: Vec::new(),
                bounds: o.bounds,
            }),
        b"TextFrame" => frames.get(self_id).map(|f| ModelGeometry {
            oval: false,
            anchors: f.anchors.clone(),
            subpath_starts: f.subpath_starts.clone(),
            bounds: f.bounds,
        }),
        b"Rectangle" => rectangles
            .iter()
            .find(|r| r.self_id.as_deref() == Some(self_id))
            .map(|r| ModelGeometry {
                oval: false,
                anchors: r.anchors.clone(),
                subpath_starts: r.subpath_starts.clone(),
                bounds: r.bounds,
            }),
        b"Polygon" => polygons
            .iter()
            .find(|r| r.self_id.as_deref() == Some(self_id))
            .map(|r| ModelGeometry {
                oval: false,
                anchors: r.anchors.clone(),
                subpath_starts: r.subpath_starts.clone(),
                bounds: r.bounds,
            }),
        b"GraphicLine" => graphic_lines
            .iter()
            .find(|r| r.self_id.as_deref() == Some(self_id))
            .map(|r| ModelGeometry {
                oval: false,
                anchors: r.anchors.clone(),
                subpath_starts: r.subpath_starts.clone(),
                bounds: r.bounds,
            }),
        _ => None,
    }
}

/// Read one `<PathPointType>` element into a [`PathAnchor`], mirroring
/// the parser: a missing `LeftDirection` / `RightDirection` defaults to
/// the anchor (degenerate handle).
fn path_point_anchor(e: &BytesStart) -> Option<PathAnchor> {
    let a = attr_value(e, b"Anchor").and_then(|s| parse_xy_pair(&s))?;
    let left = attr_value(e, b"LeftDirection")
        .and_then(|s| parse_xy_pair(&s))
        .unwrap_or(a);
    let right = attr_value(e, b"RightDirection")
        .and_then(|s| parse_xy_pair(&s))
        .unwrap_or(a);
    Some(PathAnchor {
        anchor: a,
        left,
        right,
    })
}

/// Emit one `<PathPointType Anchor="x y" LeftDirection="x y"
/// RightDirection="x y"/>` self-closing element, matching the
/// generator's attribute order + `format_f32` precision.
fn write_path_point(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    a: &PathAnchor,
) -> Result<(), quick_xml::Error> {
    let mut e = BytesStart::new("PathPointType");
    e.push_attribute(("Anchor", format_xy(a.anchor).as_str()));
    e.push_attribute(("LeftDirection", format_xy(a.left).as_str()));
    e.push_attribute(("RightDirection", format_xy(a.right).as_str()));
    writer.write_event(Event::Empty(e))?;
    Ok(())
}

// ---------------------------------------------------------------------
// New page-item emission (structural inserts — W1.15)
// ---------------------------------------------------------------------
//
// A page item created by an op since load (`InsertNode`) has a model
// entry but no XML element. We serialise it here in the canonical
// `paged_gen` shape so the writer's own parser round-trips it:
//
//   * geometry lives in `<Properties><PathGeometry>` (inner coords),
//     NOT in a `GeometricBounds` attribute. The parser derives
//     `bounds = bounds_from_anchors(raw anchors)`, so we emit corner
//     anchors directly AT the model's spread-space bounds with an
//     identity `ItemTransform`. (Inserted nodes carry their placement
//     in `bounds`; `item_transform` is `None`/identity — see
//     `paged_mutate::apply::new_rectangle` et al.)
//   * an explicit `StrokeWeight="0"` makes "no stroke" survive
//     InDesign's object-style cascade, matching the generator.

/// `<PathGeometry>` for an axis-aligned box whose corners sit at the
/// given spread-space bounds (top-left, bottom-left, bottom-right,
/// top-right — the generator + `rect_corners` order). The parser reads
/// the anchors back verbatim, so `bounds_from_anchors` reproduces these
/// bounds exactly.
fn write_box_path_geometry(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    b: Bounds,
) -> Result<(), quick_xml::Error> {
    write_closed_path_geometry(writer, &rect_corners(b))
}

/// `<PathGeometry>` for an `<Oval>`: the ellipse inscribed in the
/// spread-space bounds, spelled as InDesign spells it (see
/// [`oval_anchors`]). The anchors sit at the edge midpoints, so the
/// parser's `bounds_from_anchors` reproduces these bounds exactly.
fn write_oval_path_geometry(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    b: Bounds,
) -> Result<(), quick_xml::Error> {
    write_closed_path_geometry(writer, &oval_anchors(b))
}

fn write_closed_path_geometry(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    anchors: &[PathAnchor],
) -> Result<(), quick_xml::Error> {
    writer.write_event(Event::Start(BytesStart::new("PathGeometry")))?;
    let mut gp = BytesStart::new("GeometryPathType");
    gp.push_attribute(("PathOpen", "false"));
    writer.write_event(Event::Start(gp))?;
    writer.write_event(Event::Start(BytesStart::new("PathPointArray")))?;
    for a in anchors {
        write_path_point(writer, a)?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
        "PathPointArray",
    )))?;
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
        "GeometryPathType",
    )))?;
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("PathGeometry")))?;
    Ok(())
}

/// `<PathGeometry>` carrying explicit anchor contours (the Polygon /
/// GraphicLine inserted-node case). `subpath_starts` splits `anchors`
/// into `<GeometryPathType>` contours; `subpath_open` marks the open
/// ones (`PathOpen="true"`). An empty `subpath_starts` is one closed
/// contour over all anchors.
fn write_contour_path_geometry(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    anchors: &[PathAnchor],
    subpath_starts: &[usize],
    subpath_open: &[bool],
) -> Result<(), quick_xml::Error> {
    writer.write_event(Event::Start(BytesStart::new("PathGeometry")))?;
    let starts: Vec<usize> = if subpath_starts.is_empty() {
        vec![0]
    } else {
        subpath_starts.to_vec()
    };
    for (ci, &start) in starts.iter().enumerate() {
        let end = starts.get(ci + 1).copied().unwrap_or(anchors.len());
        let open = subpath_open.get(ci).copied().unwrap_or(false);
        let mut gp = BytesStart::new("GeometryPathType");
        gp.push_attribute(("PathOpen", if open { "true" } else { "false" }));
        writer.write_event(Event::Start(gp))?;
        writer.write_event(Event::Start(BytesStart::new("PathPointArray")))?;
        for a in anchors.get(start..end).unwrap_or(&[]) {
            write_path_point(writer, a)?;
        }
        writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
            "PathPointArray",
        )))?;
        writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
            "GeometryPathType",
        )))?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("PathGeometry")))?;
    Ok(())
}

/// The paint an INSERTED page item carries. Bundled into one struct so
/// the three `write_new_*` emitters don't each grow another six scalars,
/// and so a field added here lands on every kind at once. C-19 added
/// `fill_tint` / `opacity` / `blend_mode`: before that, an item CREATED
/// since load lost its tint and its `<BlendingSetting>` on save (the
/// patch lane only reaches items that already exist in the source XML),
/// which is exactly what paged.draw's per-layer appearance bake needs.
/// The gradient geometry a page item carries beside its swatch
/// references: `GradientFillAngle` / `GradientFillLength` and the stroke
/// pair. Never written before 2026-09-06, so InDesign composed every
/// minted gradient with its defaults (angle 0, length = the box) — the
/// annual's "Ink Dawn" ran diagonally on the canvas and horizontally in
/// InDesign. Both lanes now carry them.
#[derive(Clone, Copy, Default)]
struct GradientGeom {
    fill_angle: Option<f32>,
    fill_length: Option<f32>,
    stroke_angle: Option<f32>,
    stroke_length: Option<f32>,
}

impl GradientGeom {
    fn keys(self) -> [(&'static str, Option<f32>); 4] {
        [
            ("GradientFillAngle", self.fill_angle),
            ("GradientFillLength", self.fill_length),
            ("GradientStrokeAngle", self.stroke_angle),
            ("GradientStrokeLength", self.stroke_length),
        ]
    }

    /// Attributes to append to a minted item (only the ones set).
    fn push_attrs(self, attrs: &mut Vec<(&'static str, String)>) {
        for (k, v) in self.keys() {
            if let Some(v) = v {
                attrs.push((k, format_f32(v)));
            }
        }
    }
}

/// Second patch pass over a rebuilt start tag: the gradient geometry
/// keys patch (or append) from the model; a key the model does not set
/// passes through as the source spelled it.
fn patch_gradient_geometry(
    start: &BytesStart,
    geom: GradientGeom,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let keys = geom.keys();
    let extras: Vec<(&str, String)> = keys
        .iter()
        .filter_map(|(k, v)| v.map(|v| (*k, format_f32(v))))
        .collect();
    patch_start(
        start,
        |k, raw| {
            let (_, v) = keys.iter().find(|(key, _)| key.as_bytes() == k)?;
            v.map(|v| preserving_f32_patch(std::str::from_utf8(raw).ok(), Some(v)))
        },
        &extras,
    )
}

struct NewItemPaint<'a> {
    fill_color: &'a Option<String>,
    /// `FillTint` percent (0..=100). `None` ⇒ no tint override.
    fill_tint: Option<f32>,
    stroke_color: &'a Option<String>,
    stroke_weight: Option<f32>,
    /// `<BlendingSetting Opacity="…">` percent.
    opacity: Option<f32>,
    /// `<BlendingSetting BlendMode="…">`.
    blend_mode: Option<&'a str>,
    nonprinting: bool,
    /// `AppliedObjectStyle`. `None` ⇒ IDML's reserved
    /// [`NONE_OBJECT_STYLE`] sentinel, which is what an item with no
    /// style applied carries.
    applied_object_style: Option<&'a str>,
    /// `ItemLayer` — the layer the item sits on. Engine-minted items
    /// never wrote it: InDesign then stacks the whole document on ONE
    /// layer, by document order alone, and the model's layer order is
    /// lost on open (measured 2026-09-05 on the annual: 0 of 1932 items
    /// carried it, 1519 had a layer in the model).
    item_layer: Option<&'a str>,
    /// The item's drop shadow and effect bag, written with the blending
    /// setting (see [`crate::effects`]).
    drop_shadow: Option<&'a idml_import::DropShadowSetting>,
    effects: Option<&'a idml_import::FrameEffects>,
    /// `GradientFill*` / `GradientStroke*` — see [`GradientGeom`].
    gradient: GradientGeom,
}

/// `Option<String>` has no `const` default that can be borrowed inline,
/// so the "this kind carries no fill" case points at one shared `None`.
static NO_COLOR: Option<String> = None;

/// IDML's reserved "no object style" sentinel. Every page item carries
/// an `AppliedObjectStyle`; this is the value InDesign writes when none
/// is applied, so it is both the inserted-item default and what a
/// CLEARED reference falls back to.
pub(crate) const NONE_OBJECT_STYLE: &str = "ObjectStyle/$ID/[None]";

impl Default for NewItemPaint<'_> {
    fn default() -> Self {
        Self {
            fill_color: &NO_COLOR,
            fill_tint: None,
            stroke_color: &NO_COLOR,
            stroke_weight: None,
            opacity: None,
            blend_mode: None,
            nonprinting: false,
            applied_object_style: None,
            item_layer: None,
            drop_shadow: None,
            effects: None,
            gradient: GradientGeom::default(),
        }
    }
}

/// Common fill/stroke/transform attributes every inserted page item
/// carries, in the generator's order. `kind`-specific attrs (ParentStory
/// etc.) are pushed by the caller before this runs.
fn push_common_item_attrs(
    attrs: &mut Vec<(&'static str, String)>,
    item_transform: Option<[f32; 6]>,
    paint: &NewItemPaint<'_>,
) {
    // The style the item was given, or IDML's reserved "none" sentinel.
    // This used to be hard-coded to the sentinel, so an object style
    // applied to an item CREATED since load saved as unstyled.
    attrs.push((
        "AppliedObjectStyle",
        paint
            .applied_object_style
            .unwrap_or(NONE_OBJECT_STYLE)
            .to_string(),
    ));
    attrs.push((
        "ItemTransform",
        format_matrix(&item_transform.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])),
    ));
    attrs.push((
        "FillColor",
        paint
            .fill_color
            .clone()
            .unwrap_or_else(|| "Swatch/None".to_string()),
    ));
    // `FillTint` is an attribute (unlike opacity/blend, which are a
    // child element) and only emitted when set — absence is IDML's
    // "swatch at full strength".
    if let Some(t) = paint.fill_tint {
        attrs.push(("FillTint", format_f32(t)));
    }
    attrs.push((
        "StrokeColor",
        paint
            .stroke_color
            .clone()
            .unwrap_or_else(|| "Swatch/None".to_string()),
    ));
    // Always emit StrokeWeight so the "no stroke" intent survives the
    // object-style cascade (the generator's rationale).
    attrs.push((
        "StrokeWeight",
        format_f32(paint.stroke_weight.unwrap_or(0.0)),
    ));
    if paint.nonprinting {
        attrs.push(("Nonprinting", "true".to_string()));
    }
    if let Some(l) = paint.item_layer {
        attrs.push(("ItemLayer", l.to_string()));
    }
    paint.gradient.push_attrs(attrs);
}

/// Emit the `<TransparencySetting><BlendingSetting …/></TransparencySetting>`
/// block an inserted item's opacity / blend mode live in. It is a
/// SIBLING of `<Properties>` (see `corpus/generated/transparency.idml`),
/// so callers write it right after the properties block closes.
fn write_transparency_setting(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    paint: &NewItemPaint<'_>,
) -> Result<(), quick_xml::Error> {
    crate::effects::write_transparency(
        writer,
        paint.opacity,
        paint.blend_mode,
        paint.drop_shadow,
        paint.effects,
    )
}

/// Build a start/empty tag's `BytesStart` from `(key, value)` pairs
/// (values escaped). Element name is taken verbatim.
fn tag_with_attrs(
    name: &str,
    attrs: &[(&str, String)],
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let mut content = name.as_bytes().to_vec();
    for (k, v) in attrs {
        content.push(b' ');
        content.extend_from_slice(k.as_bytes());
        content.extend_from_slice(b"=\"");
        content.extend_from_slice(escape_attr(v).as_bytes());
        content.push(b'"');
    }
    let content = String::from_utf8(content).map_err(|e| {
        quick_xml::Error::Io(std::sync::Arc::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e,
        )))
    })?;
    Ok(BytesStart::from_content(content, name.len()).into_owned())
}

/// Emit a start tag from `(key, value)` pairs (values escaped).
pub(crate) fn emit_start_with_attrs(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    name: &str,
    attrs: &[(&str, String)],
) -> Result<(), quick_xml::Error> {
    writer.write_event(Event::Start(tag_with_attrs(name, attrs)?))?;
    Ok(())
}

/// Emit a self-closing tag from `(key, value)` pairs (values escaped).
pub(crate) fn emit_empty_with_attrs(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    name: &str,
    attrs: &[(&str, String)],
) -> Result<(), quick_xml::Error> {
    writer.write_event(Event::Empty(tag_with_attrs(name, attrs)?))?;
    Ok(())
}

/// C-19: emit an item's `Properties/Label` KVPs (IDML's native
/// extension point — the plugin-metadata carrier) inside the
/// `<Properties>` block the `write_new_*` emitters build.
///
/// This matters beyond fresh inserts: a SOURCE item that MOVES (pasted
/// into a container, or grouped by `CreateGroup`) is dropped from its
/// old position and re-emitted through these same emitters, so without
/// this its plugin metadata would vanish on the move. Other source-only
/// children of a moved element (`<Image>`, `<TextWrapPreference>`,
/// on-element corner attrs, …) are still lost — that is the standing
/// characteristic of the move lanes, shared with B-18's paste-into, and
/// is listed under "Known losses".
fn write_item_label(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    spread: &Spread,
    self_id: &str,
) -> Result<(), quick_xml::Error> {
    let Some(entries) = spread.labels.get(self_id).filter(|v| !v.is_empty()) else {
        return Ok(());
    };
    writer.write_event(Event::Start(BytesStart::new("Label")))?;
    for (k, v) in entries {
        // Through [`emit_empty_with_attrs`], hence [`escape_attr`].
        // `BytesStart::push_attribute` escapes the five entities and
        // nothing else, so a multi-line value came back with spaces
        // where its newlines were — and this lane is not only fresh
        // inserts: a SOURCE item that MOVES is re-emitted here.
        emit_empty_with_attrs(
            writer,
            "KeyValuePair",
            &[("Key", k.clone()), ("Value", v.clone())],
        )?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Label")))?;
    Ok(())
}

/// Serialise an inserted `<TextFrame>`. The model classification is
/// authoritative — the element is always emitted as `<TextFrame>` so the
/// re-parse files it back under `Spread::text_frames` (the parser keys
/// on element name, not on `ParentStory`). A frame the model carries
/// without a story still emits `ParentStory="n"` / `ContentType` so it
/// reads back as a (currently empty) text frame.
fn write_new_text_frame(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    spread: &Spread,
    f: &TextFrame,
) -> Result<(), quick_xml::Error> {
    let Some(self_id) = f.self_id.as_deref() else {
        return Ok(());
    };
    let mut attrs: Vec<(&str, String)> = vec![("Self", self_id.to_string())];
    // A wire-minted story id (`Story/u<n>`) is written SANITIZED (`/` →
    // `_`) so the reference matches the id `derive_story_id` re-derives
    // from the emitted `Stories/Story_<sanitized>.xml` entry on reopen
    // (C-8). Parsed story ids carry no slash and pass through unchanged.
    attrs.push((
        "ParentStory",
        f.parent_story
            .as_deref()
            .map(crate::emit::sanitize_id)
            .unwrap_or_else(|| "n".to_string()),
    ));
    attrs.push(("PreviousTextFrame", "n".to_string()));
    attrs.push((
        "NextTextFrame",
        f.next_text_frame.clone().unwrap_or_else(|| "n".to_string()),
    ));
    attrs.push(("ContentType", "TextType".to_string()));
    let paint = NewItemPaint {
        fill_color: &f.fill_color,
        fill_tint: f.fill_tint,
        stroke_color: &f.stroke_color,
        stroke_weight: f.stroke_weight,
        opacity: f.opacity,
        blend_mode: f.blend_mode.as_deref(),
        nonprinting: f.nonprinting,
        applied_object_style: f.applied_object_style.as_deref(),
        item_layer: f.item_layer.as_deref(),
        drop_shadow: f.drop_shadow.as_ref(),
        effects: f.effects.as_ref(),
        gradient: GradientGeom {
            fill_angle: f.gradient_fill_angle,
            fill_length: f.gradient_fill_length,
            stroke_angle: f.gradient_stroke_angle,
            stroke_length: f.gradient_stroke_length,
        },
    };
    push_common_item_attrs(&mut attrs, f.item_transform, &paint);
    emit_start_with_attrs(writer, "TextFrame", &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_item_label(writer, spread, self_id)?;
    write_box_path_geometry(writer, f.bounds)?;
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Properties")))?;
    write_text_frame_preference(writer, f)?;
    write_transparency_setting(writer, &paint)?;
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("TextFrame")))?;
    Ok(())
}

pub(crate) fn auto_sizing_idml(v: idml_import::AutoSizingType) -> &'static str {
    use idml_import::AutoSizingType::*;
    match v {
        Off => "Off",
        HeightOnly => "HeightOnly",
        WidthOnly => "WidthOnly",
        HeightAndWidth => "HeightAndWidth",
        HeightAndWidthProportionally => "HeightAndWidthProportionally",
    }
}

pub(crate) fn auto_sizing_reference_point_idml(
    v: idml_import::AutoSizingReferencePoint,
) -> &'static str {
    use idml_import::AutoSizingReferencePoint::*;
    match v {
        TopLeftPoint => "TopLeftPoint",
        TopCenterPoint => "TopCenterPoint",
        TopRightPoint => "TopRightPoint",
        CenterLeftPoint => "CenterLeftPoint",
        CenterPoint => "CenterPoint",
        CenterRightPoint => "CenterRightPoint",
        BottomLeftPoint => "BottomLeftPoint",
        BottomCenterPoint => "BottomCenterPoint",
        BottomRightPoint => "BottomRightPoint",
    }
}

/// `<TextFramePreference AutoSizingType="…" AutoSizingReferencePoint="…"/>`
/// for an inserted frame that carries auto-sizing. Measured on InDesign
/// 20.0.1: exactly this element, inside the `<TextFrame>`, is honoured
/// (`textFramePreferences.autoSizingType = HEIGHT_ONLY`); the book that
/// wrote no `<TextFramePreference>` at all had every auto-sized frame
/// reported overset. Nothing is written when the model sets no
/// auto-sizing (the parser's `None`), so a plain frame is unchanged.
pub(crate) fn write_text_frame_preference(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    f: &TextFrame,
) -> Result<(), quick_xml::Error> {
    let Some(kind) = f.auto_sizing else {
        return Ok(());
    };
    let mut attrs: Vec<(&str, String)> =
        vec![("AutoSizingType", auto_sizing_idml(kind).to_string())];
    if let Some(p) = f.auto_sizing_reference_point {
        attrs.push((
            "AutoSizingReferencePoint",
            auto_sizing_reference_point_idml(p).to_string(),
        ));
    }
    if let Some(v) = f.minimum_width_for_auto_sizing {
        attrs.push(("MinimumWidthForAutoSizing", format_f32(v)));
    }
    if let Some(v) = f.minimum_height_for_auto_sizing {
        attrs.push(("MinimumHeightForAutoSizing", format_f32(v)));
    }
    if let Some(v) = f.use_minimum_height_for_auto_sizing {
        attrs.push(("UseMinimumHeightForAutoSizing", v.to_string()));
    }
    emit_empty_with_attrs(writer, "TextFramePreference", &attrs)
}

/// Bring a CARRIED-THROUGH story in line with InDesign's navigation
/// spelling (measured on InDesign 20.0.1):
///
/// * every text destination in `anchors` (the model's
///   `HyperlinkDestinationKind::TextAnchor`s naming this story) that the
///   story does not already carry as an inline
///   `<HyperlinkTextDestination/>` marker gets one, in a range of its
///   own at the head of the first paragraph — the designmap spelling the
///   engine's fixtures used binds nothing there;
/// * a `<CrossReferenceSource>` without an `AppliedFormat` gains one
///   naming `xref_format` (InDesign drops a source that points at no
///   `<CrossReferenceFormat>`; `navigation` emits the format).
///
/// Byte-identical when nothing is missing. Runs BEFORE `rewrite_story`,
/// which then derives its provenance from these bytes: the injected
/// range holds no `<Content>`, so the parser drops it and it passes
/// through the rewrite verbatim.
pub(crate) fn inject_story_navigation(
    original: &[u8],
    anchors: &[(String, Option<String>)],
    xref_format: Option<&str>,
) -> Result<Vec<u8>, quick_xml::Error> {
    // Pre-pass: what the story already carries.
    let (present, unformatted_xrefs) = {
        let mut reader = Reader::from_reader(original);
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut present: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut unformatted = 0usize;
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                Event::Start(ref e) | Event::Empty(ref e) => match e.name().as_ref() {
                    b"HyperlinkTextDestination" => {
                        if let Some(id) = attr_value(e, b"Self") {
                            present.insert(id);
                        }
                    }
                    b"CrossReferenceSource" if attr_value(e, b"AppliedFormat").is_none() => {
                        unformatted += 1;
                    }
                    _ => {}
                },
                _ => {}
            }
            buf.clear();
        }
        (present, unformatted)
    };
    let missing: Vec<&(String, Option<String>)> = anchors
        .iter()
        .filter(|(id, _)| !present.contains(id))
        .collect();
    let fix_xrefs = xref_format.is_some() && unformatted_xrefs > 0;
    if missing.is_empty() && !fix_xrefs {
        return Ok(original.to_vec());
    }

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut placed = false;
    let write_markers = |w: &mut Writer<Cursor<Vec<u8>>>| -> Result<(), quick_xml::Error> {
        for (id, name) in &missing {
            emit_start_with_attrs(
                w,
                "CharacterStyleRange",
                &[(
                    "AppliedCharacterStyle",
                    "CharacterStyle/$ID/[No character style]".to_string(),
                )],
            )?;
            emit_empty_with_attrs(
                w,
                "HyperlinkTextDestination",
                &[
                    ("Self", id.clone()),
                    (
                        "Name",
                        name.clone()
                            .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string()),
                    ),
                    ("Hidden", "false".to_string()),
                ],
            )?;
            w.write_event(Event::End(quick_xml::events::BytesEnd::new(
                "CharacterStyleRange",
            )))?;
        }
        Ok(())
    };
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => match e.name().as_ref() {
                b"ParagraphStyleRange" if !placed && !missing.is_empty() => {
                    writer.write_event(Event::Start(e.into_owned()))?;
                    write_markers(&mut writer)?;
                    placed = true;
                }
                b"CrossReferenceSource"
                    if fix_xrefs && attr_value(&e, b"AppliedFormat").is_none() =>
                {
                    let start = patch_start(
                        &e,
                        |_, _| None,
                        &[("AppliedFormat", xref_format.unwrap_or_default().to_string())],
                    )?;
                    writer.write_event(Event::Start(start))?;
                }
                _ => writer.write_event(Event::Start(e.into_owned()))?,
            },
            Event::End(e) if e.name().as_ref() == b"Story" => {
                if !placed && !missing.is_empty() {
                    // A story with no paragraph at all: the markers get one.
                    emit_start_with_attrs(&mut writer, "ParagraphStyleRange", &[])?;
                    write_markers(&mut writer)?;
                    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                        "ParagraphStyleRange",
                    )))?;
                    placed = true;
                }
                writer.write_event(Event::End(e))?;
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

/// Serialise an inserted bounds-only vector frame (`<Rectangle>` /
/// `<Oval>`). Geometry is the four-corner box at the model bounds.
/// B-18: `item_transform` is the value to WRITE (already re-based when
/// the item is emitted nested); when the item is itself a container,
/// its `nested_children` recurse inside the element, re-based against
/// the container's composed MODEL transform.
#[allow(clippy::too_many_arguments)]
fn write_new_box_item(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    kind: &str,
    self_id: &str,
    item_transform: Option<[f32; 6]>,
    paint: &NewItemPaint<'_>,
    bounds: Bounds,
    spread: &Spread,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![("Self", self_id.to_string())];
    push_common_item_attrs(&mut attrs, item_transform, paint);
    emit_start_with_attrs(writer, kind, &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_item_label(writer, spread, self_id)?;
    if kind == "Oval" {
        write_oval_path_geometry(writer, bounds)?;
    } else {
        write_box_path_geometry(writer, bounds)?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Properties")))?;
    write_transparency_setting(writer, paint)?;
    if let Some(children) = spread.nested_children.get(self_id) {
        write_nested_children(
            writer,
            spread,
            model_transform_of(spread, self_id),
            children,
            None,
        )?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(kind)))?;
    Ok(())
}

/// Serialise an inserted path-bearing vector frame (`<Polygon>` /
/// `<GraphicLine>`). Geometry is the explicit anchor contours; when the
/// model has no anchors (rare for these kinds) it falls back to the
/// bounds box so the element still parses.
#[allow(clippy::too_many_arguments)]
fn write_new_path_item(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    kind: &str,
    self_id: &str,
    item_transform: Option<[f32; 6]>,
    paint: &NewItemPaint<'_>,
    bounds: Bounds,
    anchors: &[PathAnchor],
    subpath_starts: &[usize],
    subpath_open: &[bool],
    extra_attrs: &[(&'static str, String)],
    spread: &Spread,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![("Self", self_id.to_string())];
    push_common_item_attrs(&mut attrs, item_transform, paint);
    for (k, v) in extra_attrs {
        attrs.push((k, v.clone()));
    }
    emit_start_with_attrs(writer, kind, &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_item_label(writer, spread, self_id)?;
    if anchors.is_empty() {
        write_box_path_geometry(writer, bounds)?;
    } else {
        write_contour_path_geometry(writer, anchors, subpath_starts, subpath_open)?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Properties")))?;
    write_transparency_setting(writer, paint)?;
    // B-18: a Polygon container's nested children recurse inside the
    // element (GraphicLine ids never key `nested_children`).
    if let Some(children) = spread.nested_children.get(self_id) {
        write_nested_children(
            writer,
            spread,
            model_transform_of(spread, self_id),
            children,
            None,
        )?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(kind)))?;
    Ok(())
}

/// B-18: resolve a `FrameRef` (a `nested_children` entry) to its `Self`
/// id against the spread's backing vecs.
pub(crate) fn nested_ref_self_id(spread: &Spread, r: idml_import::FrameRef) -> Option<&str> {
    use idml_import::FrameRef;
    match r {
        FrameRef::TextFrame(i) => spread.text_frames.get(i)?.self_id.as_deref(),
        FrameRef::Rectangle(i) => spread.rectangles.get(i)?.self_id.as_deref(),
        FrameRef::Oval(i) => spread.ovals.get(i)?.self_id.as_deref(),
        FrameRef::GraphicLine(i) => spread.graphic_lines.get(i)?.self_id.as_deref(),
        FrameRef::Polygon(i) => spread.polygons.get(i)?.self_id.as_deref(),
        FrameRef::Group(i) => spread.groups.get(i)?.self_id.as_deref(),
    }
}

/// The layer a group's members sit on: the first member (depth-first
/// through nested groups) that names one.
fn group_item_layer<'a>(spread: &'a Spread, group: &idml_import::Group) -> Option<&'a str> {
    use idml_import::FrameRef;
    group.members.iter().find_map(|&m| match m {
        FrameRef::TextFrame(i) => spread.text_frames.get(i)?.item_layer.as_deref(),
        FrameRef::Rectangle(i) => spread.rectangles.get(i)?.item_layer.as_deref(),
        FrameRef::Oval(i) => spread.ovals.get(i)?.item_layer.as_deref(),
        FrameRef::GraphicLine(i) => spread.graphic_lines.get(i)?.item_layer.as_deref(),
        FrameRef::Polygon(i) => spread.polygons.get(i)?.item_layer.as_deref(),
        FrameRef::Group(i) => group_item_layer(spread, spread.groups.get(i)?),
    })
}

/// B-18: a container's composed (spread-space) model transform, looked
/// up by `Self` id across the container-capable kinds.
fn model_transform_of(spread: &Spread, id: &str) -> Option<[f32; 6]> {
    if let Some(r) = spread
        .rectangles
        .iter()
        .find(|r| r.self_id.as_deref() == Some(id))
    {
        return r.item_transform;
    }
    if let Some(o) = spread
        .ovals
        .iter()
        .find(|o| o.self_id.as_deref() == Some(id))
    {
        return o.item_transform;
    }
    if let Some(p) = spread
        .polygons
        .iter()
        .find(|p| p.self_id.as_deref() == Some(id))
    {
        return p.item_transform;
    }
    None
}

/// B-18: re-base a child's composed (spread-space) model transform to
/// its container's coordinate space — the form IDML serialises for
/// nested items: `on_disk = inverse(parent) ∘ composed`. A singular
/// parent keeps the composed value (documented loss, mirrors the
/// group lane's degenerate case).
fn relative_to_parent(
    parent: Option<[f32; 6]>,
    child_composed: Option<[f32; 6]>,
) -> Option<[f32; 6]> {
    match parent {
        None => child_composed,
        Some(p) => match invert_matrix(&p) {
            None => child_composed,
            Some(inv) => Some(compose_matrix(
                &inv,
                &child_composed.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            )),
        },
    }
}

/// Compose two optional matrices the same way the parser's
/// `effective_item_transform` accumulates a group stack: `a ∘ b`, with
/// `None` standing for identity.
fn compose_opt(a: Option<[f32; 6]>, b: Option<[f32; 6]>) -> Option<[f32; 6]> {
    match (a, b) {
        (None, x) => x,
        (Some(x), None) => Some(x),
        (Some(x), Some(y)) => Some(compose_matrix(&x, &y)),
    }
}

/// Emit ONE page item the source XML never carried, resolved through its
/// `FrameRef`. `parent_accum` is the COMPOSED (spread-space) transform of
/// everything the item is nested under — a B-18 container, a chain of
/// groups, or `None` at top level. The model stores every leaf's
/// `item_transform` composed into spread space (see
/// `Group::item_transform` / `Spread::nested_children`), so the on-disk
/// value is recovered by `relative_to_parent`. A `Group` ref recurses
/// through [`write_new_group`], which is what makes a scene-created group
/// save (C-19): before, both `write_nested_children` and
/// `write_inserted_items` dropped `FrameRef::Group` on the floor.
fn write_new_item(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    spread: &Spread,
    r: idml_import::FrameRef,
    parent_accum: Option<[f32; 6]>,
) -> Result<(), quick_xml::Error> {
    use idml_import::FrameRef;
    let Some(id) = nested_ref_self_id(spread, r) else {
        return Ok(());
    };
    match r {
        FrameRef::TextFrame(i) => {
            if let Some(f) = spread.text_frames.get(i) {
                let mut f = f.clone();
                f.item_transform = relative_to_parent(parent_accum, f.item_transform);
                write_new_text_frame(writer, spread, &f)?;
            }
        }
        FrameRef::Rectangle(i) => {
            if let Some(rect) = spread.rectangles.get(i) {
                write_new_box_item(
                    writer,
                    "Rectangle",
                    id,
                    relative_to_parent(parent_accum, rect.item_transform),
                    &NewItemPaint {
                        fill_color: &rect.fill_color,
                        fill_tint: rect.fill_tint,
                        stroke_color: &rect.stroke_color,
                        stroke_weight: rect.stroke_weight,
                        opacity: rect.opacity,
                        blend_mode: rect.blend_mode.as_deref(),
                        nonprinting: rect.nonprinting,
                        applied_object_style: rect.applied_object_style.as_deref(),
                        item_layer: rect.item_layer.as_deref(),
                        drop_shadow: rect.drop_shadow.as_ref(),
                        effects: rect.effects.as_ref(),
                        gradient: GradientGeom {
                            fill_angle: rect.gradient_fill_angle,
                            fill_length: rect.gradient_fill_length,
                            stroke_angle: rect.gradient_stroke_angle,
                            stroke_length: rect.gradient_stroke_length,
                        },
                    },
                    rect.bounds,
                    spread,
                )?;
            }
        }
        FrameRef::Oval(i) => {
            if let Some(o) = spread.ovals.get(i) {
                write_new_box_item(
                    writer,
                    "Oval",
                    id,
                    relative_to_parent(parent_accum, o.item_transform),
                    &NewItemPaint {
                        fill_color: &o.fill_color,
                        fill_tint: o.fill_tint,
                        stroke_color: &o.stroke_color,
                        stroke_weight: o.stroke_weight,
                        opacity: o.opacity,
                        blend_mode: o.blend_mode.as_deref(),
                        nonprinting: o.nonprinting,
                        applied_object_style: o.applied_object_style.as_deref(),
                        item_layer: o.item_layer.as_deref(),
                        drop_shadow: None,
                        effects: o.effects.as_ref(),
                        gradient: GradientGeom {
                            fill_angle: o.gradient_fill_angle,
                            fill_length: o.gradient_fill_length,
                            ..GradientGeom::default()
                        },
                    },
                    o.bounds,
                    spread,
                )?;
            }
        }
        FrameRef::Polygon(i) => {
            if let Some(p) = spread.polygons.get(i) {
                write_new_path_item(
                    writer,
                    "Polygon",
                    id,
                    relative_to_parent(parent_accum, p.item_transform),
                    &NewItemPaint {
                        fill_color: &p.fill_color,
                        fill_tint: p.fill_tint,
                        stroke_color: &p.stroke_color,
                        stroke_weight: p.stroke_weight,
                        opacity: p.opacity,
                        blend_mode: p.blend_mode.as_deref(),
                        nonprinting: p.nonprinting,
                        applied_object_style: p.applied_object_style.as_deref(),
                        item_layer: p.item_layer.as_deref(),
                        drop_shadow: None,
                        effects: p.effects.as_ref(),
                        gradient: GradientGeom {
                            fill_angle: p.gradient_fill_angle,
                            fill_length: p.gradient_fill_length,
                            ..GradientGeom::default()
                        },
                    },
                    p.bounds,
                    &p.anchors,
                    &p.subpath_starts,
                    &p.subpath_open,
                    &[],
                    spread,
                )?;
            }
        }
        FrameRef::GraphicLine(i) => {
            if let Some(l) = spread.graphic_lines.get(i) {
                // v43 — an inserted line that was given arrowheads
                // before save keeps them (the patch lane only covers
                // items that exist in the source XML).
                let mut extra: Vec<(&'static str, String)> = Vec::new();
                for (k, t) in [
                    ("LeftLineEnd", l.start_arrow),
                    ("RightLineEnd", l.end_arrow),
                ] {
                    if t.draws() && !t.as_idml().is_empty() {
                        extra.push((k, t.as_idml().to_string()));
                    }
                }
                write_new_path_item(
                    writer,
                    "GraphicLine",
                    id,
                    relative_to_parent(parent_accum, l.item_transform),
                    // `paged_model::GraphicLine` carries no fill, tint,
                    // opacity or blend-mode field at all — a line's
                    // paint is stroke-only, so there is nothing to lose
                    // here (cf. the C-20 arms core deliberately did not
                    // add for this kind).
                    &NewItemPaint {
                        stroke_color: &l.stroke_color,
                        stroke_weight: l.stroke_weight,
                        nonprinting: l.nonprinting,
                        applied_object_style: l.applied_object_style.as_deref(),
                        item_layer: l.item_layer.as_deref(),
                        drop_shadow: None,
                        effects: l.effects.as_ref(),
                        ..Default::default()
                    },
                    l.bounds,
                    &l.anchors,
                    &l.subpath_starts,
                    &l.subpath_open,
                    &extra,
                    spread,
                )?;
            }
        }
        FrameRef::Group(i) => {
            if let Some(g) = spread.groups.get(i) {
                write_new_group(writer, spread, g, parent_accum, None)?;
            }
        }
    }
    Ok(())
}

/// C-19 — serialise a group the source XML never carried as a real IDML
/// `<Group>` with its members NESTED inside it.
///
/// Two conventions make this work without inventing a third code path:
///
/// * `Group::item_transform` is the RAW on-disk matrix (the parser keeps
///   it un-composed with its ancestors), so it is written verbatim.
/// * every MEMBER's `item_transform` is stored COMPOSED into spread
///   space, so it is re-based against the group's composed transform by
///   the same [`relative_to_parent`] the B-18 nested-content lane uses.
///   Member path anchors are stored raw (the parser never composes the
///   group transform into them), so they emit unchanged.
///
/// `present` names the member ids the SOURCE already carries inside this
/// group — used when an EXISTING `<Group>` gained members, so the close
/// flush emits only the missing ones. It is `None` for a wholly new
/// group (nothing of it is in the source yet). Group-level transparency
/// (`<TransparencySetting>` on the `<Group>` itself) is not emitted: no
/// operation authors it, so a scene-created group never carries one.
fn write_new_group(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    spread: &Spread,
    group: &idml_import::Group,
    ancestor_accum: Option<[f32; 6]>,
    present: Option<&std::collections::HashSet<String>>,
) -> Result<(), quick_xml::Error> {
    // A group with no `Self` id can't be matched against the source, so
    // emitting it risks duplicating one that is already there.
    let Some(self_id) = group.self_id.as_deref() else {
        return Ok(());
    };
    let mut attrs = vec![
        ("Self", self_id.to_string()),
        (
            "ItemTransform",
            format_matrix(
                &group
                    .item_transform
                    .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            ),
        ),
    ];
    // InDesign keeps a group's members on the GROUP's layer: a `<Group>`
    // without `ItemLayer` lands on the first layer of the document and
    // takes its members with it, whatever their own `ItemLayer` says
    // (measured 2026-09-06: the annual's 94 traced polygons, all on
    // "Content", sat on "Grid" underneath the page background). The
    // model has no group layer, so the group takes its members' — the
    // layer InDesign itself would have put the group on.
    if let Some(layer) = group_item_layer(spread, group) {
        attrs.push(("ItemLayer", layer.to_string()));
    }
    emit_start_with_attrs(writer, "Group", &attrs)?;
    let accum = compose_opt(ancestor_accum, group.item_transform);
    for &m in &group.members {
        let Some(id) = nested_ref_self_id(spread, m) else {
            continue;
        };
        if present.is_some_and(|p| p.contains(id)) {
            continue;
        }
        write_new_item(writer, spread, m, accum)?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Group")))?;
    Ok(())
}

/// B-18: emit a container's nested children (paste-into content)
/// INSIDE the container element, in model order, skipping the ids the
/// source already carried in place (`present`). Child transforms are
/// re-based to the container's space; a child that is itself a
/// container recurses through the `write_new_*` emitters.
fn write_nested_children(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    spread: &Spread,
    parent_model_transform: Option<[f32; 6]>,
    children: &[idml_import::FrameRef],
    present: Option<&std::collections::HashSet<String>>,
) -> Result<(), quick_xml::Error> {
    for &r in children {
        let Some(id) = nested_ref_self_id(spread, r) else {
            continue;
        };
        if present.is_some_and(|p| p.contains(id)) {
            continue;
        }
        write_new_item(writer, spread, r, parent_model_transform)?;
    }
    Ok(())
}

/// The order inserted top-level items are emitted in: the model's own
/// z-table (`Spread::frames_in_order`), which is exactly the order the
/// RENDERER paints in — so a saved file reopens with the stacking the
/// user saw.
///
/// C-19 sibling fix: this used to be the per-kind vec concatenation, and
/// the two orders are NOT the same. `InsertNode` takes a `position` into
/// the kind vec and a separate `z_slot` into the z-table, so a caller
/// that inserts each new item at `position: 0` (paged.draw's bake does)
/// builds a kind vec in REVERSE creation order while the z-table stays
/// correct — the saved XML came out back-to-front. Driving off the
/// z-table removes the discrepancy by construction.
///
/// The per-kind sweep that follows mirrors the renderer's own legacy
/// fallback (see `paged-renderer`'s `frames_ordered`): text → rect →
/// oval → line → polygon, then groups. It IS the whole order for a
/// spread whose z-table is empty (`register_frame_ref` deliberately
/// no-ops on an empty table, so a document built entirely by
/// `InsertNode` has none), and a safety net otherwise — an item present
/// in its kind vec but missing from the z-table must still be written,
/// or the writer would silently swallow it.
fn insert_emission_order(spread: &Spread) -> Vec<idml_import::FrameRef> {
    use idml_import::FrameRef;
    let mut v: Vec<FrameRef> = spread.frames_in_order.clone();
    // `FrameRef` is `Eq` but not `Hash`, and a spread's item count is
    // small, so a linear membership check is the honest tool here.
    let push_missing = |r: FrameRef, v: &mut Vec<FrameRef>| {
        if !spread.frames_in_order.contains(&r) {
            v.push(r);
        }
    };
    for i in 0..spread.text_frames.len() {
        push_missing(FrameRef::TextFrame(i), &mut v);
    }
    for i in 0..spread.rectangles.len() {
        push_missing(FrameRef::Rectangle(i), &mut v);
    }
    for i in 0..spread.ovals.len() {
        push_missing(FrameRef::Oval(i), &mut v);
    }
    for i in 0..spread.graphic_lines.len() {
        push_missing(FrameRef::GraphicLine(i), &mut v);
    }
    for i in 0..spread.polygons.len() {
        push_missing(FrameRef::Polygon(i), &mut v);
    }
    for i in 0..spread.groups.len() {
        push_missing(FrameRef::Group(i), &mut v);
    }
    v
}

/// Ids that belong INSIDE something else: group members (emitted by the
/// group's own recursion) and B-18 nested children. Everything else in
/// [`insert_emission_order`] is a TOP-LEVEL item.
pub(crate) fn owned_ids(spread: &Spread) -> std::collections::HashSet<&str> {
    let mut owned: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for g in &spread.groups {
        collect_group_member_ids(spread, g, &mut owned);
    }
    for children in spread.nested_children.values() {
        for &r in children {
            if let Some(id) = nested_ref_self_id(spread, r) {
                owned.insert(id);
            }
        }
    }
    owned
}

/// True for the six element names that are page items. `<Group>` counts:
/// it is a page item too, just a container one, and it occupies a z-slot
/// among its siblings exactly like a rectangle does.
pub(crate) fn is_page_item_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"TextFrame" | b"Rectangle" | b"Oval" | b"GraphicLine" | b"Polygon" | b"Group"
    )
}

/// C-22 — what the SOURCE XML already carries, read off in one cheap
/// read-only pre-pass.
///
/// The rewrite itself is a single streaming pass and cannot know, at the
/// moment it reaches a source element, whether a model item that sorts
/// BEFORE it will show up later in the file — which is exactly the
/// question "where does this inserted item go?" needs answered. One
/// pre-pass answers it for the whole spread.
struct SourceItems {
    /// The `Self` ids of every page item at the spread's TOP level, in
    /// document order.
    ///
    /// "Top level" means: not inside a `<Group>` and not inside another
    /// page item (a B-18 paste-into container). Both cases are tracked
    /// by the one depth counter because [`is_page_item_name`] covers
    /// `<Group>` too.
    top_level: Vec<String>,
    /// The `Self` ids of every `<Group>` element, **at any depth**.
    ///
    /// A source `<Group>` is never dropped and never re-minted —
    /// [`rewrite_spread`]'s `Group` arms mark its id seen unconditionally
    /// (unlike the other five kinds, a group takes no part in
    /// [`triage_placement`]). So a group the source carries is already
    /// placed, wherever it sits, and the insert lane must leave it
    /// alone. That matters because a `<Group>` nested inside a page item
    /// is registered by the PARSER into `Spread::frames_in_order` — the
    /// documented B-18 residual "flatten to top level, unclipped", so
    /// the renderer still paints it (see `paged_model::Spread::
    /// skipped_nested_frames`) — and it is therefore a model top-level
    /// id that is absent from `top_level`. Reading that absence as "new"
    /// minted a second copy of the whole subtree.
    groups: std::collections::HashSet<String>,
}

fn scan_source_items(original: &[u8]) -> Result<SourceItems, quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut buf = Vec::new();
    let mut out = SourceItems {
        top_level: Vec::new(),
        groups: std::collections::HashSet::new(),
    };
    let mut item_depth: usize = 0;
    loop {
        let (e, is_start) = match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => (Some(e), true),
            Event::Empty(e) => (Some(e), false),
            Event::End(e) => {
                if is_page_item_name(e.name().as_ref()) {
                    item_depth = item_depth.saturating_sub(1);
                }
                (None, false)
            }
            _ => (None, false),
        };
        if let Some(e) = e {
            let name = e.name().as_ref().to_vec();
            if is_page_item_name(&name) {
                let id = attr_value(&e, b"Self");
                if item_depth == 0 {
                    if let Some(id) = id.clone() {
                        out.top_level.push(id);
                    }
                }
                if name == b"Group" {
                    if let Some(id) = id {
                        out.groups.insert(id);
                    }
                }
                if is_start {
                    item_depth += 1;
                }
            }
        }
        buf.clear();
    }
    Ok(out)
}

/// C-22 — decide WHERE each inserted top-level item is written, instead
/// of dumping all of them at the spread's close.
///
/// The z-order among inserted items was fixed by C-19 (emission walks
/// `frames_in_order`). This is their position relative to the items the
/// SOURCE document already carried: a group the appearance bake dropped
/// into the carrier's z-slot used to reopen ABOVE every source item,
/// because the only emission point was the `</Spread>` flush.
///
/// The rule: an inserted item is written immediately BEFORE the first
/// item that follows it in the model's z-table and is present at the
/// source's top level. That item is its ANCHOR. Anything with no such
/// follower still goes at the close — it belongs on top.
///
/// Why anchor forward rather than reorder: the source's document order
/// IS its z-order, and for an UNMUTATED document it very nearly equals
/// `frames_in_order` element for element (the parser appends to that vec
/// as it walks). So an unmutated spread produces an empty plan, nothing
/// moves, and byte-identity holds.
///
/// "Very nearly", not "exactly": the one place the two lists diverge on
/// an unmutated document is a `<Group>` nested inside a page item, which
/// the parser deliberately flattens into `frames_in_order` (the B-18
/// residual — the alternative is that the pasted-in subtree never
/// paints). Absent from the source's TOP level, present in the model's
/// z-table, and owned by nobody, it read as an insert and was re-minted
/// on top of the copy the source already carried — one 832 KB corpus
/// spread saved back at 1.37 MB. Excluding every source group id (at any
/// depth) restores the invariant the stream pass already keeps: a
/// `<Group>` the source carries is never re-minted. See
/// [`SourceItems::groups`].
///
/// A document whose z was RESHUFFLED is not this lane's business, and it
/// is no longer left alone either: re-ordering elements the SOURCE
/// carried is C-23's [`crate::reorder`] post-pass, which runs over this
/// pass's output and permutes whole serialised elements. This plan stays
/// as it is — it decides where a NEW element is first written, and the
/// two compose (an insert lands at its anchor, then both source and
/// inserted elements are dealt into the model's order).
///
/// Returns `before` — `before[anchor_id]` is the run of refs to write
/// just before that source element, in model z-order. The TAIL is not
/// returned: an insert with no anchor is simply absent from the map, so
/// it is still unseen when [`write_inserted_items`] runs at the close and
/// lands there. That also makes the whole lane self-healing — anything
/// the plan fails to place is written by the close pass rather than
/// dropped.
fn plan_insert_positions(
    spread: &Spread,
    source: &SourceItems,
    owned: &std::collections::HashSet<&str>,
) -> std::collections::HashMap<String, Vec<idml_import::FrameRef>> {
    let mut before: std::collections::HashMap<String, Vec<idml_import::FrameRef>> =
        std::collections::HashMap::new();
    let mut next_anchor: Option<String> = None;
    // Walk the model's top-level z-order BACKWARDS so each insert sees
    // the nearest source item that follows it.
    for r in insert_emission_order(spread).into_iter().rev() {
        let Some(id) = nested_ref_self_id(spread, r) else {
            continue;
        };
        if owned.contains(id) {
            continue;
        }
        if source.top_level.iter().any(|s| s == id) {
            next_anchor = Some(id.to_string());
        } else if source.groups.contains(id) {
            // A `<Group>` the source carries somewhere BELOW the
            // top level. The stream pass keeps it exactly where it is
            // and marks it seen, so it is neither new (nothing to
            // write) nor an anchor (there is no top-level slot to write
            // before). Planning it minted a second copy of the whole
            // subtree — see [`SourceItems::groups`].
            continue;
        } else if let Some(a) = &next_anchor {
            before.entry(a.clone()).or_default().push(r);
        }
    }
    // The reverse walk collected each run back-to-front; flip them so
    // emission is in model z-order.
    for v in before.values_mut() {
        v.reverse();
    }
    before
}

/// C-22 — write the inserted items anchored to the page-item element the
/// reader just opened, if any.
///
/// Fires only for a TOP-LEVEL page item (`group_depth == 0` and no
/// enclosing page item), which is the only place the plan anchors to —
/// `open_items` has not yet been pushed for this element, so
/// `no_open_item` is the enclosing-container answer. Each flushed id
/// joins `seen`, which is what stops the close-of-spread pass writing a
/// second copy.
#[allow(clippy::too_many_arguments)]
fn flush_inserts_before(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    spread: &Spread,
    e: &BytesStart,
    name: &[u8],
    group_depth: usize,
    no_open_item: bool,
    insert_before: &mut std::collections::HashMap<String, Vec<idml_import::FrameRef>>,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), quick_xml::Error> {
    if insert_before.is_empty() || group_depth != 0 || !no_open_item || !is_page_item_name(name) {
        return Ok(());
    }
    let Some(id) = attr_value(e, b"Self") else {
        return Ok(());
    };
    let Some(pending) = insert_before.remove(&id) else {
        return Ok(());
    };
    for r in pending {
        write_new_item(writer, spread, r, None)?;
        if let Some(mid) = nested_ref_self_id(spread, r) {
            seen.insert(mid.to_string());
        }
    }
    Ok(())
}

/// Append every model page item whose `Self` id was NOT seen in the
/// source XML — the inserted nodes — at the spread's close, in
/// [`insert_emission_order`].
///
/// C-22: most inserts are now written earlier, at their real z position
/// among the source items (see [`plan_insert_positions`]); their ids go
/// into `seen` as they are flushed, so this pass skips them. What still
/// lands here is the genuine tail — inserts with no source item above
/// them — plus the safety net for anything the plan could not place.
///
/// A `FrameRef::Group` entry emits the whole group (wrapper + members)
/// via [`write_new_group`]; its members are therefore skipped as
/// top-level items, as are B-18 nested children (paste-into content
/// emits INSIDE its container — at the container's close for source
/// containers, or via the `write_new_*` recursion for inserted ones —
/// never top-level). Keeping the children of a REMOVED container out of
/// the flat lane also matches InDesign's delete semantics: they vanish
/// with it.
pub(crate) fn write_inserted_items(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    spread: &Spread,
    seen: &std::collections::HashSet<String>,
) -> Result<(), quick_xml::Error> {
    let owned = owned_ids(spread);
    for r in insert_emission_order(spread) {
        let Some(id) = nested_ref_self_id(spread, r) else {
            continue;
        };
        if seen.contains(id) || owned.contains(id) {
            continue;
        }
        write_new_item(writer, spread, r, None)?;
    }
    Ok(())
}

/// Where a page-item element sits in the SOURCE XML versus where the
/// MODEL wants it. Filled in by [`rewrite_spread`] for each page-item
/// start tag and resolved by [`triage_placement`].
struct Placement<'a> {
    /// Innermost eligible B-18 container open in the source.
    source_host: Option<&'a str>,
    /// The B-18 container the model nests this item under.
    model_host: Option<&'a str>,
    /// Innermost open `<Group>` in the source whose `Self` id the MODEL
    /// still knows. `None` at top level — or when the enclosing group is
    /// id-less / no longer in the model, in which case
    /// `source_group_opaque` is set and the group lane stands down.
    source_group: Option<&'a str>,
    /// An enclosing source `<Group>` that can't be matched to a model
    /// group (no `Self`, or dissolved away). Its members keep the
    /// legacy in-place treatment — a group DISSOLVE is still a deferred
    /// lane, and guessing here would silently reorder the file.
    source_group_opaque: bool,
    /// The group the model lists this item under.
    model_group: Option<&'a str>,
    /// The model still carries this item somewhere.
    in_model: bool,
    /// The PARSER declined to model this element at all — see
    /// [`idml_import::SpreadProvenance`]. Distinct from `!in_model`,
    /// which is what the model says about an item it DID once hold, and
    /// the distinction is the whole point: an element with no model
    /// counterpart was never the model's to delete.
    unmodelled: bool,
}

/// What to do with a page-item element the reader just opened.
enum ItemVerdict {
    /// Keep the element where it is; mark it seen.
    Keep,
    /// Keep it and record it as present inside the named B-18 container.
    KeepInHost(String),
    /// C-19: keep it and record it as present inside the named group, so
    /// that group's close flush doesn't re-emit it.
    KeepInGroup(String),
    /// Drop the element (and its subtree); it is re-emitted elsewhere —
    /// inside its model container / group, or by the top-level insert
    /// lane. NOT marked seen.
    Drop,
}

/// Resolve a [`Placement`]. The B-18 container lanes are decided first
/// (they own the container relationship); what used to be their
/// catch-all is now the C-19 group lane, with the pre-C-19 behaviour
/// preserved verbatim for the "no container, no group" case.
fn triage_placement(p: &Placement<'_>) -> ItemVerdict {
    // An element the PARSER declined to model is not the model's to
    // move, regroup or delete. Every lane below reads the model's
    // silence about this id as an intention — nested nowhere, grouped
    // nowhere, and (fatally) `in_model: false`, which is a REMOVE — and
    // all of those readings are unfounded, because the model was never
    // shown the element. `brand-guidelines`' `Spread_u1db62` carries two
    // `<Polygon>`s with an empty `<PathPointArray>`; they were deleted
    // out of the user's document on a save that changed nothing.
    //
    // First, and unconditionally: the answer does not depend on where
    // the element sits, so nothing is gained by asking.
    if p.unmodelled {
        return ItemVerdict::Keep;
    }
    match (p.source_host, p.model_host) {
        // Kept in place inside its container.
        (Some(sh), Some(mh)) if sh == mh => return ItemVerdict::KeepInHost(mh.to_string()),
        // PasteInto: top-level in source, nested in the model.
        (None, Some(_)) if p.source_group.is_none() && !p.source_group_opaque => {
            return ItemVerdict::Drop
        }
        // Re-pasted: nested in source under A, under B in the model.
        (Some(_), Some(_)) => return ItemVerdict::Drop,
        // ReleaseFrom: nested in source, top-level in the model.
        (Some(_), None) if p.in_model => return ItemVerdict::Drop,
        _ => {}
    }
    if p.source_group_opaque {
        // Legacy: an item inside an unmatchable group is left alone.
        return ItemVerdict::Keep;
    }
    match (p.source_group, p.model_group) {
        // Grouped in source, same group in the model — stays put.
        (Some(sg), Some(mg)) if sg == mg => ItemVerdict::KeepInGroup(mg.to_string()),
        // Joins a group (CreateGroup over SOURCE items) or is regrouped:
        // drop here, re-emitted inside the model's group.
        (_, Some(_)) => ItemVerdict::Drop,
        // Left its group (dissolve of THIS item's membership) or was
        // removed outright: drop; the insert lane re-emits it if the
        // model still carries it.
        (Some(_), None) => ItemVerdict::Drop,
        // Top level in both: the pre-C-19 rule.
        (None, None) => {
            if p.in_model {
                ItemVerdict::Keep
            } else {
                ItemVerdict::Drop
            }
        }
    }
}

/// Recursively gather the `Self` ids of every page item referenced by a
/// group (and its sub-groups — including the sub-groups' OWN ids) so
/// inserted-item emission skips them: everything in here is emitted by
/// [`write_new_group`]'s recursion instead, nested where it belongs.
fn collect_group_member_ids<'a>(
    spread: &'a Spread,
    group: &'a idml_import::Group,
    out: &mut std::collections::HashSet<&'a str>,
) {
    use idml_import::FrameRef;
    for m in &group.members {
        match *m {
            FrameRef::TextFrame(i) => {
                if let Some(id) = spread.text_frames.get(i).and_then(|f| f.self_id.as_deref()) {
                    out.insert(id);
                }
            }
            FrameRef::Rectangle(i) => {
                if let Some(id) = spread.rectangles.get(i).and_then(|r| r.self_id.as_deref()) {
                    out.insert(id);
                }
            }
            FrameRef::Oval(i) => {
                if let Some(id) = spread.ovals.get(i).and_then(|o| o.self_id.as_deref()) {
                    out.insert(id);
                }
            }
            FrameRef::GraphicLine(i) => {
                if let Some(id) = spread
                    .graphic_lines
                    .get(i)
                    .and_then(|l| l.self_id.as_deref())
                {
                    out.insert(id);
                }
            }
            FrameRef::Polygon(i) => {
                if let Some(id) = spread.polygons.get(i).and_then(|p| p.self_id.as_deref()) {
                    out.insert(id);
                }
            }
            FrameRef::Group(i) => {
                if let Some(sub) = spread.groups.get(i) {
                    if let Some(id) = sub.self_id.as_deref() {
                        out.insert(id);
                    }
                    collect_group_member_ids(spread, sub, out);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// Spread rewrite
// ---------------------------------------------------------------------

/// Rewrite a `Spread_*.xml` body so its page-item start tags reflect the
/// current model. Untouched bytes pass through verbatim; the result is
/// byte-identical to `original` when nothing in `spread` diverged from it.
///
/// # Elements with no model counterpart
///
/// `spread` does NOT hold one item per source page-item element: the
/// parser discards a shape that supplies no geometry at all (see
/// [`idml_import::SpreadProvenance`]). The structural-remove lane below
/// reads "id not in the model" as "the user deleted it", which for those
/// is wrong and destructive, so the parser's own record of what it
/// declined is consulted first and such an element passes through
/// untouched.
///
/// The provenance is derived from `original` here rather than taken as
/// an argument, for the same reason [`rewrite_story`]'s is: it cannot
/// then be computed from different bytes than the ones being streamed.
/// When the parse fails the record is empty, and every element takes the
/// pre-existing path — the conservative answer, and the one that keeps
/// this a pure refinement of the remove lane rather than a new gate in
/// front of it.
pub fn rewrite_spread(original: &[u8], spread: &Spread) -> Result<Vec<u8>, quick_xml::Error> {
    let provenance = idml_import::parse_spread_with_provenance(original)
        .map(|(_, p)| p)
        .unwrap_or_default();

    // Index every page item by its `Self` id so a start tag can find its
    // model counterpart regardless of element ordering.
    let mut frames: std::collections::HashMap<&str, &TextFrame> = std::collections::HashMap::new();
    for f in &spread.text_frames {
        if let Some(id) = f.self_id.as_deref() {
            frames.insert(id, f);
        }
    }

    // W1.15 — structural inserts/removes. `model_ids` is every page-item
    // `Self` the model still carries; `seen_ids` accumulates the ids that
    // appear in the source XML. A top-level XML item whose id left the
    // model is a REMOVE (the element is dropped); a model id never seen
    // in the XML is an INSERT (emitted at the spread's close in model
    // order). Group members are not removed structurally here — a group
    // dissolve / regroup is a separate deferred lane (see Known losses).
    let mut model_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for f in &spread.text_frames {
        if let Some(id) = f.self_id.as_deref() {
            model_ids.insert(id);
        }
    }
    for r in &spread.rectangles {
        if let Some(id) = r.self_id.as_deref() {
            model_ids.insert(id);
        }
    }
    for o in &spread.ovals {
        if let Some(id) = o.self_id.as_deref() {
            model_ids.insert(id);
        }
    }
    for p in &spread.polygons {
        if let Some(id) = p.self_id.as_deref() {
            model_ids.insert(id);
        }
    }
    for l in &spread.graphic_lines {
        if let Some(id) = l.self_id.as_deref() {
            model_ids.insert(id);
        }
    }
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Depth of the open element being dropped as a REMOVE, plus the
    // depth it opened at; while `> 0` every event passes through to the
    // bit-bucket until the matching close. `0` ⇒ not removing.
    let mut remove_depth: usize = 0;

    // ---- B-18 nested content (paste-into) state ----
    // Model-side nesting index: child `Self` id → host container id.
    let mut nested_owner: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for (host, children) in &spread.nested_children {
        for &r in children {
            if let Some(id) = nested_ref_self_id(spread, r) {
                nested_owner.insert(id, host.as_str());
            }
        }
    }
    // Every open page-item element (outermost first) — the source-side
    // nesting truth. `eligible` mirrors the parser's lift rule: only a
    // Rectangle / Oval / Polygon WITH a Self id, and only when no
    // `<Group>` opened in between (`groups_at_open` vs `group_depth`),
    // hosts paste-into children.
    struct OpenItem {
        depth: usize,
        self_id: Option<String>,
        eligible: bool,
        groups_at_open: usize,
    }
    let mut open_items: Vec<OpenItem> = Vec::new();
    // Per host: the child ids the source already carries nested in
    // place, so the container-close flush emits only the missing ones.
    let mut present_in: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();

    // ---- C-19 group-membership state ----
    // Model-side group index: member `Self` id → owning group's `Self`
    // id (sub-groups included — `nested_ref_self_id` resolves a
    // `FrameRef::Group` to the group's own id). Only groups WITH an id
    // participate: an id-less `<Group>` can't be matched against the
    // source, so its members keep the legacy in-place treatment.
    let mut group_owner: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for g in &spread.groups {
        let Some(gid) = g.self_id.as_deref() else {
            continue;
        };
        for &m in &g.members {
            if let Some(id) = nested_ref_self_id(spread, m) {
                group_owner.insert(id, gid);
            }
        }
    }
    // Per source `<Group Self=…>`: the member ids the source already
    // carries inside it, so the `</Group>` flush emits only the ones
    // the model added (a nested `CreateGroup`, or an item moved in).
    let mut present_in_group: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();

    // ---- C-22 inserted-item placement ----
    // Which inserted items go immediately before which SOURCE element,
    // and which are the genuine tail. Entries are drained as their
    // anchor is reached; the flushed ids join `seen_ids` so the
    // close-of-spread pass never writes a second copy.
    let mut insert_before = {
        let owned = owned_ids(spread);
        plan_insert_positions(spread, &scan_source_items(original)?, &owned)
    };

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    // Depth of open `<Group>` elements. Inside a group the parser
    // COMPOSES the group transform into each member's `item_transform`
    // (see `effective_item_transform`), so the model value is the
    // group∘member matrix, NOT the on-disk member transform. W1.15 lane
    // 4 recovers the on-disk transform by inverting the accumulated
    // group stack (`group_xforms`): `member_on_disk = inverse(accum) ∘
    // model.item_transform`. The stack mirrors the parser's: each open
    // `<Group ItemTransform>` pushes its RAW transform (parsed straight
    // off the XML, same source the parser composed from). Fills /
    // strokes / colours are not composed and patch safely at any depth.
    let mut group_depth: usize = 0;
    // RAW `<Group ItemTransform>` per open group, outermost first. `None`
    // for a group with no ItemTransform (identity).
    let mut group_xforms: Vec<Option<[f32; 6]>> = Vec::new();
    // C-19: `Self` per open group, parallel to `group_xforms` (`None`
    // for an id-less group — see `group_owner`). The innermost entry is
    // the SOURCE-side answer to "which group is this item in?".
    let mut group_ids: Vec<Option<String>> = Vec::new();

    // ---- plugin-metadata Label patching state ----
    // Element-name stack (depth tracking) + the innermost open page
    // item that the model labels. The model's `spread.labels` map IS
    // the truth: an item's `<Label>` contents are replaced wholesale
    // with the model entries; a labelled item whose source has no
    // `<Properties>`/`<Label>` gets the block synthesised; an item the
    // model no longer labels has its `<Label>` dropped.
    //
    // "Replaced wholesale" is decided at the `</Label>`, not at the
    // `<Label>`, so an UNCHANGED label can keep its source bytes — the
    // same stance `patch_start` takes attribute by attribute and
    // `<PathPointArray>` takes point by point. Rebuilding one that
    // nobody edited reformats it: the source's indentation goes, and
    // (before [`escape_attr`] learned the character references) its
    // `&#xa;`s came back as literal newlines, which is a different
    // document.
    let mut depth: usize = 0;
    struct LabelCtx {
        /// Depth of the item element itself.
        item_depth: usize,
        /// Model entries; `None` ⇒ the model has no labels for it.
        entries: Option<Vec<(String, String)>>,
        /// A direct `<Properties>` child is currently open.
        in_direct_properties: bool,
        /// We are inside the item's `<Label>` (original KVPs are held
        /// back until the close decides).
        in_label: bool,
        /// Byte offset of the open `<Label>` start tag in `original`.
        label_start: usize,
        /// The `KeyValuePair`s the SOURCE label carries, decoded the way
        /// the parser decodes them, in document order — so they compare
        /// against the model entries like for like.
        source_entries: Vec<(String, String)>,
        /// The model entries have been written.
        handled: bool,
    }
    let mut label_ctx: Vec<LabelCtx> = Vec::new();
    const ITEM_KINDS: [&[u8]; 5] = [
        b"TextFrame",
        b"Rectangle",
        b"Oval",
        b"GraphicLine",
        b"Polygon",
    ];
    fn write_label_entries(
        writer: &mut Writer<Cursor<Vec<u8>>>,
        entries: &[(String, String)],
    ) -> Result<(), quick_xml::Error> {
        writer.write_event(Event::Start(BytesStart::new("Label")))?;
        for (k, v) in entries {
            // Through [`emit_empty_with_attrs`], hence [`escape_attr`] —
            // the same escaper `write_item_label` uses, and the one that
            // knows tab / LF / CR need character references.
            // `BytesStart::push_attribute`, which this used to call,
            // escapes the five entities and nothing else, so a value
            // holding a newline came back with a space in its place.
            emit_empty_with_attrs(
                writer,
                "KeyValuePair",
                &[("Key", k.clone()), ("Value", v.clone())],
            )?;
        }
        writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Label")))?;
        Ok(())
    }

    // ---- PathPointArray rewrite state ----
    // The innermost open page item that tracks anchors (TextFrame /
    // Rectangle / Polygon / GraphicLine). Real InDesign exports (and
    // every generated fixture) carry frame geometry as a
    // `<PathPointArray>` of `<PathPointType>` anchors rather than a
    // `GeometricBounds` attribute, so a `FramePathPoint` / `FramePath`
    // edit — or a `FrameBounds` resize of a rectangular frame — has to
    // rewrite those anchors to save. We buffer each `<PathPointArray>`
    // and, at its close, either re-emit the model anchors (when the
    // contour diverged) or replay the original points verbatim (so an
    // unmutated path stays byte-identical).
    struct PathCtx {
        /// Depth of the page-item element.
        item_depth: usize,
        /// Model geometry, or `None` for a kind that doesn't track
        /// anchors (Oval) / an item with no model match.
        geom: Option<ModelGeometry>,
        /// Index of the next `<GeometryPathType>` contour / its
        /// `<PathPointArray>`.
        contour: usize,
        /// Depth of the open `<PathPointArray>`, or 0 when not in one.
        array_depth: usize,
        /// Depth of the outermost open element whose `<PathGeometry>`
        /// is NOT this frame's outline, or 0 when there is none.
        ///
        /// A page item can host several `<PathPointArray>`s that have
        /// nothing to do with its own contours: a placed picture's box
        /// (`<Image>` / `<EPS>` / `<PDF>` / `<ImportedPage>`), and a
        /// clipping path (`<ClippingPathSettings>`). The PARSER skips
        /// exactly those when it fills `subpath_starts` (its
        /// `in_image_depth` / `in_clipping_path` guards), so counting
        /// them here made the writer's contour index mean something
        /// different from the model's — and the mismatch was not a
        /// harmless off-by-one: an unmutated `business-magazine-template`
        /// save rewrote a 45-point `Image > TextWrapPreference` contour
        /// with the host `Polygon`'s 39 anchors. Foreign arrays now pass
        /// through verbatim and do not advance `contour`, so the two
        /// indices stay in step by construction.
        foreign_depth: usize,
        /// Buffered events inside the open `<PathPointArray>` (point
        /// elements + any whitespace between them).
        buffered: Vec<Event<'static>>,
        /// On-disk anchors parsed from the buffered points.
        parsed: Vec<PathAnchor>,
    }
    let mut path_ctx: Vec<PathCtx> = Vec::new();

    loop {
        // Where this event's markup begins in `original`. Taken BEFORE
        // the read, exactly as `parse_story_with_provenance` takes it,
        // so a `<Label>` that turns out to be unchanged can be copied
        // out of the source rather than re-serialised.
        let event_start = reader.buffer_position() as usize;
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                let name_owned = e.name().as_ref().to_vec();
                // Inside a REMOVE drop everything until the matching
                // close — the element and its whole subtree vanish.
                if remove_depth != 0 {
                    buf.clear();
                    continue;
                }
                // C-22: this element may be the ANCHOR for one or more
                // inserted items — the ones whose model z-slot sits just
                // below it. Flush them BEFORE the triage below, so they
                // still land in the right place when the anchor itself
                // turns out to be dropped (a source item moving into a
                // new group is exactly that case: the appearance bake's
                // carrier).
                flush_inserts_before(
                    &mut writer,
                    spread,
                    &e,
                    &name_owned,
                    group_depth,
                    open_items.is_empty(),
                    &mut insert_before,
                    &mut seen_ids,
                )?;
                // Page-item triage: the legacy structural REMOVE (a
                // top-level item whose `Self` left the model) plus the
                // B-18 paste-into lanes — comparing where the element
                // sits in the SOURCE (top level vs nested under an
                // eligible container) against where the MODEL wants it
                // (`nested_owner`).
                if ITEM_KINDS.contains(&name_owned.as_slice()) {
                    if let Some(id) = attr_value(&e, b"Self") {
                        let source_host: Option<String> = open_items
                            .last()
                            .filter(|it| it.eligible && it.groups_at_open == group_depth)
                            .and_then(|it| it.self_id.clone());
                        let innermost_group = group_ids.last().cloned().flatten();
                        let placement = Placement {
                            source_host: source_host.as_deref(),
                            model_host: nested_owner.get(id.as_str()).copied(),
                            source_group: innermost_group.as_deref(),
                            source_group_opaque: group_depth > 0 && innermost_group.is_none(),
                            model_group: group_owner.get(id.as_str()).copied(),
                            in_model: model_ids.contains(id.as_str()),
                            unmodelled: provenance.is_unmodelled(&id),
                        };
                        match triage_placement(&placement) {
                            ItemVerdict::Keep => {
                                seen_ids.insert(id.clone());
                            }
                            ItemVerdict::KeepInHost(host) => {
                                // Patched below with the host-composed
                                // accum; the container flush won't
                                // re-emit it.
                                seen_ids.insert(id.clone());
                                present_in.entry(host).or_default().insert(id.clone());
                            }
                            ItemVerdict::KeepInGroup(gid) => {
                                seen_ids.insert(id.clone());
                                present_in_group.entry(gid).or_default().insert(id.clone());
                            }
                            ItemVerdict::Drop => {
                                remove_depth = depth;
                                buf.clear();
                                continue;
                            }
                        }
                    }
                }
                // Entering a sub-object that carries its OWN path
                // geometry — the placed picture's box, or a clipping
                // path. Everything inside is foreign to the frame's
                // contours; see `PathCtx::foreign_depth`.
                if matches!(
                    name_owned.as_slice(),
                    b"Image" | b"EPS" | b"PDF" | b"ImportedPage" | b"ClippingPathSettings"
                ) {
                    if let Some(ctx) = path_ctx.last_mut() {
                        if ctx.foreign_depth == 0 {
                            ctx.foreign_depth = depth;
                        }
                    }
                }
                // Buffer a `<PathPointArray>` for the innermost path
                // item so its points can be rewritten at close.
                if name_owned == b"PathPointArray" {
                    if let Some(ctx) = path_ctx.last_mut() {
                        if ctx.array_depth == 0 && ctx.foreign_depth == 0 {
                            ctx.array_depth = depth;
                            ctx.buffered.clear();
                            ctx.parsed.clear();
                            writer.write_event(Event::Start(e.into_owned()))?;
                            buf.clear();
                            continue;
                        }
                    }
                }
                if let Some(ctx) = path_ctx.last_mut() {
                    if ctx.array_depth != 0 {
                        // Nested element inside the array — buffer it.
                        ctx.buffered.push(Event::Start(e.into_owned()));
                        buf.clear();
                        continue;
                    }
                }
                // Label handling for the innermost labelled item.
                if let Some(ctx) = label_ctx.last_mut() {
                    if name_owned == b"Properties" && depth == ctx.item_depth + 1 {
                        ctx.in_direct_properties = true;
                    } else if name_owned == b"Label"
                        && ctx.in_direct_properties
                        && depth == ctx.item_depth + 2
                    {
                        // Hold the Label back. Whether it is replaced,
                        // dropped or kept verbatim is decided at the
                        // `</Label>`, once its source entries are known.
                        ctx.in_label = true;
                        ctx.label_start = event_start;
                        ctx.source_entries.clear();
                        ctx.handled = true;
                        buf.clear();
                        continue; // original <Label> start not written
                    } else if ctx.in_label {
                        // A `<KeyValuePair>` can also arrive as a Start
                        // (with a separate End) rather than an Empty.
                        if name_owned == b"KeyValuePair" {
                            if let Some(kv) = key_value_pair(&e) {
                                ctx.source_entries.push(kv);
                            }
                        }
                        buf.clear();
                        continue;
                    }
                }
                // B-18: a model-nested child re-bases against its
                // HOST's composed model transform (which already folds
                // every group above the host); everything else keeps
                // the legacy group accumulation.
                let nested_accum = attr_value(&e, b"Self")
                    .and_then(|id| nested_owner.get(id.as_str()).copied())
                    .map(|host| model_transform_of(spread, host));
                let group_accum = match nested_accum {
                    Some(tx) => Some(tx),
                    None => {
                        if group_depth > 0 {
                            Some(accumulate_group_xforms(&group_xforms))
                        } else {
                            None
                        }
                    }
                };
                let patched = patch_spread_item(
                    &e,
                    spread,
                    &frames,
                    &spread.rectangles,
                    &spread.ovals,
                    &spread.polygons,
                    &spread.graphic_lines,
                    &spread.groups,
                    group_accum,
                )?;
                match patched {
                    Some(start) => writer.write_event(Event::Start(start))?,
                    None => writer.write_event(Event::Start(e.clone().into_owned()))?,
                }
                if name_owned == b"Group" {
                    group_depth += 1;
                    group_xforms
                        .push(attr_value(&e, b"ItemTransform").and_then(|s| parse_matrix(&s)));
                    // C-19: a group the MODEL still knows takes part in
                    // the membership triage; anything else (no `Self`,
                    // or dissolved out of the model) stays opaque and
                    // its members keep the legacy in-place treatment.
                    let gid = attr_value(&e, b"Self").filter(|id| {
                        spread
                            .groups
                            .iter()
                            .any(|g| g.self_id.as_deref() == Some(id.as_str()))
                    });
                    // Seen either way, so the insert lane never emits a
                    // second copy of a group the source already carries.
                    if let Some(id) = attr_value(&e, b"Self") {
                        seen_ids.insert(id);
                    }
                    group_ids.push(gid);
                }
                if ITEM_KINDS.contains(&name_owned.as_slice()) {
                    let self_id = attr_value(&e, b"Self");
                    let entries = self_id
                        .as_deref()
                        .and_then(|id| spread.labels.get(id).cloned())
                        .filter(|v| !v.is_empty());
                    label_ctx.push(LabelCtx {
                        item_depth: depth,
                        entries,
                        in_direct_properties: false,
                        in_label: false,
                        label_start: 0,
                        source_entries: Vec::new(),
                        handled: false,
                    });
                    // Group-member geometry is composed into the model's
                    // anchors the same way the transform is (see the
                    // group note in `rewrite_spread`), so we don't rewrite
                    // a member's path either — leave `geom: None` inside a
                    // group so its points pass through verbatim. B-18
                    // residual: nested (paste-into) children get the same
                    // conservative treatment — their `<PathPointArray>`
                    // passes through verbatim, so a path-point edit on a
                    // nested child doesn't write back yet.
                    let geom = if group_depth > 0 || !open_items.is_empty() {
                        None
                    } else {
                        self_id.as_deref().and_then(|id| {
                            model_geometry(
                                &name_owned,
                                id,
                                &frames,
                                &spread.rectangles,
                                &spread.ovals,
                                &spread.polygons,
                                &spread.graphic_lines,
                            )
                        })
                    };
                    path_ctx.push(PathCtx {
                        item_depth: depth,
                        geom,
                        contour: 0,
                        array_depth: 0,
                        foreign_depth: 0,
                        buffered: Vec::new(),
                        parsed: Vec::new(),
                    });
                    // B-18: record the open page item for source-side
                    // nesting detection + the container-close flush.
                    open_items.push(OpenItem {
                        depth,
                        self_id: self_id.clone(),
                        eligible: matches!(
                            name_owned.as_slice(),
                            b"Rectangle" | b"Oval" | b"Polygon"
                        ) && self_id.is_some(),
                        groups_at_open: group_depth,
                    });
                }
            }
            Event::Empty(e) => {
                // Inside a REMOVE every empty element vanishes too.
                if remove_depth != 0 {
                    buf.clear();
                    continue;
                }
                // C-22: a self-closing page item anchors inserts exactly
                // like an open one does.
                {
                    let name_owned = e.name().as_ref().to_vec();
                    flush_inserts_before(
                        &mut writer,
                        spread,
                        &e,
                        &name_owned,
                        group_depth,
                        open_items.is_empty(),
                        &mut insert_before,
                        &mut seen_ids,
                    )?;
                }
                // A self-closing page item: the same triage as the
                // Start arm (legacy REMOVE + the B-18 paste-into
                // lanes), except a drop is a plain skip — there is no
                // subtree.
                if ITEM_KINDS.contains(&e.name().as_ref()) {
                    if let Some(id) = attr_value(&e, b"Self") {
                        let source_host: Option<String> = open_items
                            .last()
                            .filter(|it| it.eligible && it.groups_at_open == group_depth)
                            .and_then(|it| it.self_id.clone());
                        let innermost_group = group_ids.last().cloned().flatten();
                        let placement = Placement {
                            source_host: source_host.as_deref(),
                            model_host: nested_owner.get(id.as_str()).copied(),
                            source_group: innermost_group.as_deref(),
                            source_group_opaque: group_depth > 0 && innermost_group.is_none(),
                            model_group: group_owner.get(id.as_str()).copied(),
                            in_model: model_ids.contains(id.as_str()),
                            unmodelled: provenance.is_unmodelled(&id),
                        };
                        match triage_placement(&placement) {
                            ItemVerdict::Keep => {
                                seen_ids.insert(id.clone());
                            }
                            ItemVerdict::KeepInHost(host) => {
                                seen_ids.insert(id.clone());
                                present_in.entry(host).or_default().insert(id.clone());
                            }
                            ItemVerdict::KeepInGroup(gid) => {
                                seen_ids.insert(id.clone());
                                present_in_group.entry(gid).or_default().insert(id.clone());
                            }
                            ItemVerdict::Drop => {
                                buf.clear();
                                continue;
                            }
                        }
                    }
                }
                // C-19: a self-closing `<Group/>` (a group the source
                // carries with no members) is still "seen", so the
                // insert lane never mints a second copy of it.
                if e.name().as_ref() == b"Group" {
                    if let Some(id) = attr_value(&e, b"Self") {
                        seen_ids.insert(id);
                    }
                }
                // Buffer a `<PathPointType>` (or any empty element)
                // inside an open `<PathPointArray>`.
                if let Some(ctx) = path_ctx.last_mut() {
                    if ctx.array_depth != 0 {
                        if e.name().as_ref() == b"PathPointType" {
                            if let Some(a) = path_point_anchor(&e) {
                                ctx.parsed.push(a);
                            }
                        }
                        ctx.buffered.push(Event::Empty(e.into_owned()));
                        buf.clear();
                        continue;
                    }
                }
                // KeyValuePairs inside a held-back Label are recorded,
                // not written — the `</Label>` decides.
                if let Some(ctx) = label_ctx.last_mut() {
                    if ctx.in_label {
                        if e.name().as_ref() == b"KeyValuePair" {
                            if let Some(kv) = key_value_pair(&e) {
                                ctx.source_entries.push(kv);
                            }
                        }
                        buf.clear();
                        continue;
                    }
                }
                let name_is_item = ITEM_KINDS.contains(&e.name().as_ref());
                // B-18: same host-composed accum override as the Start
                // arm.
                let nested_accum = attr_value(&e, b"Self")
                    .and_then(|id| nested_owner.get(id.as_str()).copied())
                    .map(|host| model_transform_of(spread, host));
                let group_accum = match nested_accum {
                    Some(tx) => Some(tx),
                    None => {
                        if group_depth > 0 {
                            Some(accumulate_group_xforms(&group_xforms))
                        } else {
                            None
                        }
                    }
                };
                let patched = patch_spread_item(
                    &e,
                    spread,
                    &frames,
                    &spread.rectangles,
                    &spread.ovals,
                    &spread.polygons,
                    &spread.graphic_lines,
                    &spread.groups,
                    group_accum,
                )?;
                // A labelled item serialised as an EMPTY tag must grow
                // children — expand to Start + Properties/Label + End.
                // Same for a B-18 container the model gave nested
                // children (its paste-into content emits inside).
                let pending_entries = if name_is_item {
                    attr_value(&e, b"Self")
                        .and_then(|id| spread.labels.get(&id).cloned())
                        .filter(|v| !v.is_empty())
                } else {
                    None
                };
                let pending_children: Option<(String, Vec<idml_import::FrameRef>)> = if name_is_item
                {
                    attr_value(&e, b"Self").and_then(|id| {
                        spread
                            .nested_children
                            .get(&id)
                            .filter(|v| !v.is_empty())
                            .map(|v| (id.clone(), v.clone()))
                    })
                } else {
                    None
                };
                if pending_entries.is_some() || pending_children.is_some() {
                    let name_owned = e.name().as_ref().to_vec();
                    match patched {
                        Some(start) => writer.write_event(Event::Start(start))?,
                        None => writer.write_event(Event::Start(e.clone().into_owned()))?,
                    }
                    if let Some(entries) = pending_entries {
                        writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                        write_label_entries(&mut writer, &entries)?;
                        writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                            "Properties",
                        )))?;
                    }
                    if let Some((host_id, children)) = pending_children {
                        write_nested_children(
                            &mut writer,
                            spread,
                            model_transform_of(spread, &host_id),
                            &children,
                            present_in.get(&host_id),
                        )?;
                    }
                    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                        String::from_utf8_lossy(&name_owned).into_owned(),
                    )))?;
                } else {
                    match patched {
                        Some(start) => writer.write_event(Event::Empty(start))?,
                        None => writer.write_event(Event::Empty(e.into_owned()))?,
                    }
                }
            }
            Event::End(e) => {
                let name_owned = e.name().as_ref().to_vec();
                // Closing a REMOVE: when this End matches the removed
                // element's open depth the drop ends; otherwise it is a
                // child of the removed subtree and also vanishes.
                if remove_depth != 0 {
                    if depth == remove_depth {
                        remove_depth = 0;
                    }
                    depth = depth.saturating_sub(1);
                    buf.clear();
                    continue;
                }
                // Closing the `<Spread>` / `<MasterSpread>`: before the
                // tag, flush every model page item the source XML never
                // carried — the structural INSERTs.
                if name_owned == b"Spread" || name_owned == b"MasterSpread" {
                    write_inserted_items(&mut writer, spread, &seen_ids)?;
                    depth = depth.saturating_sub(1);
                    writer.write_event(Event::End(e))?;
                    buf.clear();
                    continue;
                }
                // Close of the buffered `<PathPointArray>`: decide whether
                // this contour diverged and emit the model anchors, or
                // replay the original points verbatim.
                if let Some(ctx) = path_ctx.last_mut() {
                    if ctx.array_depth != 0 {
                        if name_owned == b"PathPointArray" && depth == ctx.array_depth {
                            let contour = ctx.contour;
                            ctx.contour += 1;
                            let target = ctx
                                .geom
                                .as_ref()
                                .and_then(|g| g.target_for_contour(contour, &ctx.parsed));
                            match target {
                                Some(anchors) => {
                                    for a in &anchors {
                                        write_path_point(&mut writer, a)?;
                                    }
                                }
                                None => {
                                    for ev in ctx.buffered.drain(..) {
                                        writer.write_event(ev)?;
                                    }
                                }
                            }
                            ctx.buffered.clear();
                            ctx.parsed.clear();
                            ctx.array_depth = 0;
                            depth = depth.saturating_sub(1);
                            writer.write_event(Event::End(e))?;
                            buf.clear();
                            continue;
                        }
                        // A nested End inside the array — buffer it.
                        ctx.buffered.push(Event::End(e.into_owned()));
                        depth = depth.saturating_sub(1);
                        buf.clear();
                        continue;
                    }
                    // Leaving the picture box / clipping path: the
                    // frame's own contours resume here.
                    if ctx.foreign_depth != 0 && depth == ctx.foreign_depth {
                        ctx.foreign_depth = 0;
                    }
                    if depth == ctx.item_depth && ITEM_KINDS.contains(&name_owned.as_slice()) {
                        path_ctx.pop();
                    }
                }
                if let Some(ctx) = label_ctx.last_mut() {
                    if ctx.in_label && name_owned == b"Label" && depth == ctx.item_depth + 2 {
                        // The whole `<Label>…</Label>` has been read.
                        // Unchanged ⇒ copy the SOURCE bytes; anything
                        // else ⇒ the wholesale replace / drop as before.
                        let unchanged = ctx
                            .entries
                            .as_deref()
                            .is_some_and(|model| model == ctx.source_entries.as_slice());
                        if unchanged {
                            // Through the cursor's `Write`, not its
                            // backing `Vec` — the writer emits at the
                            // cursor's POSITION, so bytes pushed onto
                            // the end of the vec sit past it and the
                            // next `write_event` overwrites them.
                            let end = reader.buffer_position() as usize;
                            std::io::Write::write_all(
                                writer.get_mut(),
                                &original[ctx.label_start..end],
                            )?;
                        } else if let Some(entries) = ctx.entries.as_deref() {
                            write_label_entries(&mut writer, entries)?;
                        }
                        ctx.in_label = false;
                        depth = depth.saturating_sub(1);
                        buf.clear();
                        continue;
                    }
                    if ctx.in_label {
                        // Closing a held-back child inside the Label.
                        depth = depth.saturating_sub(1);
                        buf.clear();
                        continue;
                    }
                    if name_owned == b"Properties" && depth == ctx.item_depth + 1 {
                        // Direct Properties closing without a Label —
                        // synthesise one when the model has entries.
                        if !ctx.handled {
                            if let Some(entries) = ctx.entries.take() {
                                write_label_entries(&mut writer, &entries)?;
                                ctx.handled = true;
                            }
                        }
                        ctx.in_direct_properties = false;
                    }
                    if depth == ctx.item_depth && ITEM_KINDS.contains(&name_owned.as_slice()) {
                        // Item closing without any Properties at all —
                        // synthesise the whole block.
                        if !ctx.handled {
                            if let Some(entries) = ctx.entries.take() {
                                writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                                write_label_entries(&mut writer, &entries)?;
                                writer.write_event(Event::End(
                                    quick_xml::events::BytesEnd::new("Properties"),
                                ))?;
                            }
                        }
                        label_ctx.pop();
                    }
                }
                // B-18: a page item closing — pop it from the open
                // stack and, when the model nests children under it,
                // flush the ones the source didn't already carry in
                // place, just before the close tag (InDesign's element
                // order puts pasted-in content last).
                if ITEM_KINDS.contains(&name_owned.as_slice())
                    && open_items.last().is_some_and(|it| it.depth == depth)
                {
                    let item = open_items.pop().expect("guarded by is_some_and");
                    if let Some(host_id) = item.self_id.as_deref() {
                        if let Some(children) = spread.nested_children.get(host_id) {
                            write_nested_children(
                                &mut writer,
                                spread,
                                model_transform_of(spread, host_id),
                                children,
                                present_in.get(host_id),
                            )?;
                        }
                    }
                }
                if name_owned == b"Group" {
                    group_depth = group_depth.saturating_sub(1);
                    let own_xform = group_xforms.pop().flatten();
                    let gid = group_ids.pop().flatten();
                    // C-19: an EXISTING group that gained members (a
                    // nested `CreateGroup`, or an item moved into it)
                    // flushes the missing ones just before its close —
                    // the same shape as the B-18 container flush above.
                    // An unmutated group flushes nothing, so its bytes
                    // are untouched.
                    if let Some(gid) = gid.as_deref() {
                        if let Some(g) = spread
                            .groups
                            .iter()
                            .find(|g| g.self_id.as_deref() == Some(gid))
                        {
                            // Members re-base against the SOURCE group's
                            // composed transform — that is the element
                            // they are being written inside of. (The
                            // group's own `<Group ItemTransform>` is not
                            // patched from the model; a `SetGroupTransform`
                            // save-back is a separate lane.)
                            let accum =
                                compose_opt(accumulate_group_xforms(&group_xforms), own_xform);
                            let present = present_in_group.get(gid);
                            for &m in &g.members {
                                let Some(mid) = nested_ref_self_id(spread, m) else {
                                    continue;
                                };
                                if present.is_some_and(|p| p.contains(mid))
                                    || seen_ids.contains(mid)
                                {
                                    continue;
                                }
                                write_new_item(&mut writer, spread, m, accum)?;
                            }
                        }
                    }
                }
                depth = depth.saturating_sub(1);
                writer.write_event(Event::End(e))?;
            }
            Event::Text(t) => {
                // Text inside a removed subtree (incl. the indentation
                // around it) vanishes with the element.
                if remove_depth != 0 {
                    buf.clear();
                    continue;
                }
                // Whitespace/indentation inside a buffered
                // `<PathPointArray>` rides with the buffered points so a
                // verbatim replay stays byte-exact.
                if let Some(ctx) = path_ctx.last_mut() {
                    if ctx.array_depth != 0 {
                        ctx.buffered.push(Event::Text(t.into_owned()));
                        buf.clear();
                        continue;
                    }
                }
                // Indentation between KVPs of a replaced Label drops
                // with the rest of the original Label body.
                if label_ctx.last().is_some_and(|c| c.in_label) {
                    buf.clear();
                    continue;
                }
                writer.write_event(Event::Text(t))?;
            }
            other => {
                // PIs / comments inside a removed subtree vanish too.
                if remove_depth != 0 {
                    buf.clear();
                    continue;
                }
                // Any other event inside a buffered array is foreign —
                // keep the original points (drop the rewrite) by leaving
                // the buffer intact and replaying it at array close.
                if let Some(ctx) = path_ctx.last_mut() {
                    if ctx.array_depth != 0 {
                        ctx.buffered.push(other.into_owned());
                        // Mark the parsed set as "do not rewrite" by
                        // poisoning it: a length mismatch vs the model
                        // contour forces verbatim. Simpler: clear geom so
                        // every contour of this item passes through.
                        ctx.geom = None;
                        buf.clear();
                        continue;
                    }
                }
                writer.write_event(other)?;
            }
        }
        buf.clear();
    }

    // C-23 — z-order save-back. The streaming pass above cannot emit an
    // element it has not read yet, so the ORDER question is answered
    // afterwards, by splicing whole serialised elements into the slots
    // the model's z table asks for. A no-op unless the order diverged
    // (see [`crate::reorder`]).
    crate::reorder::apply(spread, writer.into_inner().into_inner())
}

/// The `ItemTransform` decision for one page item.
///
/// Kept as the two INPUTS rather than a pre-derived string, because the
/// question that decides byte-identity is not "what does the model say"
/// but "did the model value come from these very bytes?".
///
/// IDML spells a transform at full decimal precision
/// (`ItemTransform="1 0 0 1 0 -1021.8897637779996"`); the model stores
/// `f32`, and `format_f32` rounds to 4 decimals. So re-emitting an
/// UNTOUCHED transform truncates it — the single largest source of
/// save-back byte gaps in the corpus. [`TransformPlan::is_source`] re-runs
/// the parser's own forward derivation (`compose(accum, on_disk)`, i.e.
/// `idml_import`'s `effective_item_transform`) against the source
/// spelling; when it reproduces the model matrix bit-for-bit the
/// transform is untouched and the source bytes pass through verbatim.
/// The same insight that made the z-order save-back non-lossy: splice the
/// bytes you were given rather than re-deriving them.
///
/// A transform that WAS edited fails that check and is written from the
/// model at `format_f32` precision exactly as before — the derived value
/// is the truth there, and verbatim would be wrong.
#[derive(Clone, Copy)]
struct TransformPlan {
    /// The accumulated group transform this item sits under. `None` at
    /// top level — and also for a group stack that is all-identity, which
    /// behaves identically here and in the parser.
    accum: Option<[f32; 6]>,
    /// The model's matrix: raw for a top-level item, COMPOSED into spread
    /// space for a group member (see `effective_item_transform`).
    model: Option<[f32; 6]>,
    /// `false` ⇒ the group transform is singular, the on-disk value can't
    /// be recovered, and the attribute passes through untouched.
    patch: bool,
}

impl TransformPlan {
    /// Decide the plan. `group_accum` is `None` for a top-level item (the
    /// model transform IS the on-disk transform → patch it). For a group
    /// member it is `Some(accumulated_group_transform)`; the on-disk
    /// transform is recovered by inverting the group accumulation (W1.15
    /// lane 4). When the group transform is singular the recovery fails
    /// and the patch is suppressed (the attribute passes through verbatim
    /// — a documented loss for that degenerate case).
    fn resolve(group_accum: Option<Option<[f32; 6]>>, model: Option<[f32; 6]>) -> Self {
        let accum = group_accum.flatten();
        // Only a genuine group accumulation can be singular; `None`
        // recovers trivially.
        let patch = recover_member_transform(accum, model).is_some();
        Self {
            accum,
            model,
            patch,
        }
    }

    /// The on-disk matrix to WRITE, or `None` to drop the attribute.
    /// Only meaningful when `patch`.
    fn on_disk(&self) -> Option<[f32; 6]> {
        recover_member_transform(self.accum, self.model).unwrap_or(self.model)
    }

    /// Does the SOURCE spelling `raw` derive the model matrix exactly?
    /// `None` ⇒ the element carried no `ItemTransform` at all (identity).
    /// An unparseable spelling is never claimed as the source.
    fn is_source(&self, raw: Option<&[u8]>) -> bool {
        let on_disk = match raw {
            None => None,
            Some(r) => match std::str::from_utf8(r).ok().and_then(parse_matrix) {
                Some(m) => Some(m),
                None => return false,
            },
        };
        compose_opt(self.accum, on_disk) == self.model
    }

    /// The patch for an `ItemTransform` attribute the source carries.
    fn patch_for(&self, raw: &[u8]) -> Option<Patch> {
        if !self.patch {
            return None;
        }
        if self.is_source(Some(raw)) {
            return Some(Patch::Keep);
        }
        Some(match self.on_disk() {
            Some(m) => Patch::Set(format_matrix(&m)),
            None => Patch::Remove,
        })
    }

    /// The value to APPEND when the source element carried no
    /// `ItemTransform`: always one — the identity when that is what the
    /// model derives. InDesign 20.0.1 does NOT read an absent
    /// `ItemTransform` as identity (measured: every one of a book's 1438
    /// transform-less items landed one page width to the left on its
    /// facing spread, whole versos blank; the same file with
    /// `ItemTransform="1 0 0 1 0 0"` added to each placed every item
    /// correctly). InDesign's own packages carry it on every item, so
    /// they never reach this branch and stay byte-identical. `None` only
    /// for the degenerate group case where nothing can be patched.
    fn extra(&self) -> Option<String> {
        if !self.patch {
            return None;
        }
        Some(format_matrix(
            &self.on_disk().unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        ))
    }
}

/// If `e` is a page-item start tag whose `Self` matches a model item,
/// return the patched start tag. `None` ⇒ not a page item we patch
/// (caller emits the original verbatim). `group_accum` carries the
/// accumulated group transform for a member (see [`resolve_item_transform`]).
#[allow(clippy::too_many_arguments)]
fn patch_spread_item(
    e: &BytesStart,
    spread: &Spread,
    frames: &std::collections::HashMap<&str, &TextFrame>,
    rectangles: &[idml_import::Rectangle],
    ovals: &[idml_import::Oval],
    polygons: &[idml_import::Polygon],
    graphic_lines: &[idml_import::GraphicLine],
    groups: &[idml_import::Group],
    group_accum: Option<Option<[f32; 6]>>,
) -> Result<Option<BytesStart<'static>>, quick_xml::Error> {
    let name = e.name();
    let self_id = attr_value(e, b"Self");
    let Some(self_id) = self_id else {
        return Ok(None);
    };

    match name.as_ref() {
        b"TextFrame" => {
            let Some(frame) = frames.get(self_id.as_str()) else {
                return Ok(None);
            };
            let tx = TransformPlan::resolve(group_accum, frame.item_transform);
            let fill = Fill {
                color: frame.fill_color.clone(),
                tint: frame.fill_tint,
            };
            let stroke = frame.stroke_color.clone();
            let stroke_weight = frame.stroke_weight;
            let next = frame.next_text_frame.clone();
            let nonprinting = frame.nonprinting;
            let bounds = frame.bounds;
            let applied_object_style = frame.applied_object_style.clone();
            // C-18: the B-23 residual is closed — a `TextFrame` carries
            // the corner fields and RENDERS them (its body is the same
            // outline a rectangle paints), so they patch back
            // byte-preservingly like a rectangle's.
            let corners =
                corner_attrs_of(frame.corner_radius, &frame.corner_option, &frame.corners);
            let start = patch_start(
                e,
                |k, raw| {
                    frame_attr_patch(
                        k,
                        raw,
                        tx,
                        Some(&fill),
                        &stroke,
                        stroke_weight,
                        Some(&next),
                        nonprinting,
                        bounds,
                        None,
                        None,
                        Some(&corners),
                        &applied_object_style,
                        &frame.item_layer,
                    )
                },
                &frame_attr_extras(
                    tx,
                    Some(&fill),
                    &stroke,
                    stroke_weight,
                    next.as_deref(),
                    nonprinting,
                    None,
                    None,
                    Some(&corners),
                    applied_object_style.as_deref(),
                    frame.item_layer.as_deref(),
                ),
            )?;
            let start = patch_gradient_geometry(
                &start,
                GradientGeom {
                    fill_angle: frame.gradient_fill_angle,
                    fill_length: frame.gradient_fill_length,
                    stroke_angle: frame.gradient_stroke_angle,
                    stroke_length: frame.gradient_stroke_length,
                },
            )?;
            Ok(Some(start.into_owned()))
        }
        b"Rectangle" => {
            let item = rectangles
                .iter()
                .find(|r| r.self_id.as_deref() == Some(self_id.as_str()));
            let tx = TransformPlan::resolve(group_accum, item.and_then(|r| r.item_transform));
            patch_vector_item(
                e,
                tx,
                item.map(|r| VectorItem {
                    fill: Some(Fill {
                        color: r.fill_color.clone(),
                        tint: r.fill_tint,
                    }),
                    stroke_color: r.stroke_color.clone(),
                    stroke_weight: r.stroke_weight,
                    nonprinting: r.nonprinting,
                    bounds: r.bounds,
                    applied_object_style: r.applied_object_style.clone(),
                    item_layer: r.item_layer.clone(),
                    gradient: GradientGeom {
                        fill_angle: r.gradient_fill_angle,
                        fill_length: r.gradient_fill_length,
                        stroke_angle: r.gradient_stroke_angle,
                        stroke_length: r.gradient_stroke_length,
                    },
                    start_arrow: None,
                    end_arrow: None,
                    corners: Some(corner_attrs_of(
                        r.corner_radius,
                        &r.corner_option,
                        &r.corners,
                    )),
                }),
            )
        }
        b"Oval" => {
            let item = ovals
                .iter()
                .find(|r| r.self_id.as_deref() == Some(self_id.as_str()));
            let tx = TransformPlan::resolve(group_accum, item.and_then(|r| r.item_transform));
            patch_vector_item(
                e,
                tx,
                item.map(|r| VectorItem {
                    fill: Some(Fill {
                        color: r.fill_color.clone(),
                        tint: r.fill_tint,
                    }),
                    stroke_color: r.stroke_color.clone(),
                    stroke_weight: r.stroke_weight,
                    nonprinting: r.nonprinting,
                    bounds: r.bounds,
                    applied_object_style: r.applied_object_style.clone(),
                    item_layer: r.item_layer.clone(),
                    gradient: GradientGeom {
                        fill_angle: r.gradient_fill_angle,
                        fill_length: r.gradient_fill_length,
                        ..GradientGeom::default()
                    },
                    start_arrow: None,
                    end_arrow: None,
                    // C-18: the B-23 residual is closed — `Oval` now
                    // carries the corner fields, so they patch back
                    // byte-preservingly like a rectangle's. (The values
                    // never shape an ellipse's geometry; see
                    // `paged_model::Oval::corner_radius`.)
                    corners: Some(corner_attrs_of(
                        r.corner_radius,
                        &r.corner_option,
                        &r.corners,
                    )),
                }),
            )
        }
        b"Polygon" => {
            let item = polygons
                .iter()
                .find(|r| r.self_id.as_deref() == Some(self_id.as_str()));
            let tx = TransformPlan::resolve(group_accum, item.and_then(|r| r.item_transform));
            patch_vector_item(
                e,
                tx,
                item.map(|r| VectorItem {
                    fill: Some(Fill {
                        color: r.fill_color.clone(),
                        tint: r.fill_tint,
                    }),
                    stroke_color: r.stroke_color.clone(),
                    stroke_weight: r.stroke_weight,
                    nonprinting: r.nonprinting,
                    bounds: r.bounds,
                    applied_object_style: r.applied_object_style.clone(),
                    item_layer: r.item_layer.clone(),
                    gradient: GradientGeom {
                        fill_angle: r.gradient_fill_angle,
                        fill_length: r.gradient_fill_length,
                        ..GradientGeom::default()
                    },
                    start_arrow: None,
                    end_arrow: None,
                    corners: Some(corner_attrs_of(
                        r.corner_radius,
                        &r.corner_option,
                        &r.corners,
                    )),
                }),
            )
        }
        b"GraphicLine" => {
            let item = graphic_lines
                .iter()
                .find(|r| r.self_id.as_deref() == Some(self_id.as_str()));
            let tx = TransformPlan::resolve(group_accum, item.and_then(|r| r.item_transform));
            patch_vector_item(
                e,
                tx,
                item.map(|r| VectorItem {
                    // `paged_model::GraphicLine` has no fill field at
                    // all; see [`Fill`].
                    fill: None,
                    stroke_color: r.stroke_color.clone(),
                    stroke_weight: r.stroke_weight,
                    nonprinting: r.nonprinting,
                    bounds: r.bounds,
                    applied_object_style: r.applied_object_style.clone(),
                    item_layer: r.item_layer.clone(),
                    gradient: GradientGeom::default(),
                    start_arrow: Some(r.start_arrow),
                    end_arrow: Some(r.end_arrow),
                    // C-18: the B-23 residual is closed — `GraphicLine`
                    // now carries the corner fields (the corpus's 21
                    // lines all have real radii), so they patch back
                    // byte-preservingly. See
                    // `paged_model::GraphicLine::corner_radius`.
                    corners: Some(corner_attrs_of(
                        r.corner_radius,
                        &r.corner_option,
                        &r.corners,
                    )),
                }),
            )
        }
        // C-18 — a `<Group>` gets a corner-ONLY patch lane. It is not a
        // `VectorItem`: it has no bounds, fill, stroke or arrowheads, and
        // its own `ItemTransform` is deliberately NOT patched from the
        // model (a `SetGroupTransform` save-back is a separate lane —
        // see the known-losses list). So the lookup answers only for the
        // ten corner keys; everything else passes through verbatim.
        b"Group" => {
            let Some(g) = groups
                .iter()
                .find(|g| g.self_id.as_deref() == Some(self_id.as_str()))
            else {
                return Ok(None);
            };
            let corners = corner_attrs_of(g.corner_radius, &g.corner_option, &g.corners);
            let mut extras = corner_attr_extras(&corners);
            // A group written without an `ItemTransform` (an older
            // export of ours) gets the identity spelled: InDesign does
            // not read an absent one as identity — see
            // `TransformPlan::extra`. InDesign's own groups carry it.
            if attr_value(e, b"ItemTransform").is_none() {
                extras.push(("ItemTransform", "1 0 0 1 0 0".to_string()));
            }
            // A group without `ItemLayer` lands on the document's first
            // layer in InDesign and drags its members there (see
            // `write_new_group`); spell its members' layer when the
            // source never did. One the source names is kept as is.
            if attr_value(e, b"ItemLayer").is_none() {
                if let Some(layer) = group_item_layer(spread, g) {
                    extras.push(("ItemLayer", layer.to_string()));
                }
            }
            let start = patch_start(e, |k, raw| corner_attr_patch(k, raw, &corners), &extras)?;
            Ok(Some(start.into_owned()))
        }
        _ => Ok(None),
    }
}

/// The frame attributes shared by every page-item kind, lifted into one
/// shape so a single patch routine covers Rectangle / Oval / Polygon /
/// GraphicLine.
struct VectorItem {
    /// `FillColor` + `FillTint`, or `None` for a kind that models NO
    /// fill — see [`Fill`].
    fill: Option<Fill>,
    stroke_color: Option<String>,
    stroke_weight: Option<f32>,
    nonprinting: bool,
    bounds: idml_import::Bounds,
    /// `AppliedObjectStyle` — the reference into the `<ObjectStyle>`
    /// definitions `resources::patch_styles` writes. Model-owned
    /// (`SetProperty(AppliedObjectStyle)` rewrites it), so it has to
    /// patch back or the applied style is lost on save.
    applied_object_style: Option<String>,
    /// `ItemLayer` — patched back when the model names a layer; never
    /// removed (see [`item_layer_patch`]).
    item_layer: Option<String>,
    /// Gradient geometry — see [`GradientGeom`].
    gradient: GradientGeom,
    /// v43 — `LeftLineEnd` / `RightLineEnd`. `None` for the kinds that
    /// don't carry the fields (Rectangle / Oval / Polygon), so their
    /// source attributes pass through verbatim.
    start_arrow: Option<idml_import::ArrowheadType>,
    end_arrow: Option<idml_import::ArrowheadType>,
    /// B-23 — `CornerOption` / `CornerRadius` + the four per-corner
    /// pairs. `Some` for the kinds whose model parses them (Rectangle,
    /// Polygon); `None` for Oval / GraphicLine / TextFrame, whose
    /// on-disk corner attributes pass through verbatim because there is
    /// no model field that could have changed them.
    corners: Option<CornerAttrs>,
}

/// The fill a page-item kind MODELS: its `FillColor` swatch reference
/// and its `FillTint` percent.
///
/// Wrapped in an `Option` at every use site, because the distinction
/// that matters is one level up from either field: `Some(Fill { color:
/// None, .. })` is "the model carries a fill and it is unset", and
/// `None` is "this kind has no fill in the model at all".
///
/// `<GraphicLine>` is the `None` case. `paged_model::GraphicLine` has no
/// fill field — a line is a stroked open contour, and the model says so
/// in as many words ("Lines carry no fill"). The writer used to pass
/// `fill_color: None` for it, which reads identically to a cleared fill
/// and deleted the attribute: 191 `FillColor`s across 48 corpus spreads,
/// including `FillColor="Color/c25m15y77k0"` on lines InDesign itself
/// wrote. Nothing in the model can ever have changed them, so there is
/// nothing for the writer to say and it now says nothing — the same
/// stance `next` takes for the kinds with no `NextTextFrame` field, and
/// `corners` for the kinds with no corner fields.
struct Fill {
    color: Option<String>,
    tint: Option<f32>,
}

/// B-23 — the corner vocabulary IDML writes on a page item, lifted out
/// of the model so one patch routine covers Rectangle and Polygon.
/// `corners` is `[top_left, top_right, bottom_right, bottom_left]`.
struct CornerAttrs {
    corner_radius: Option<f32>,
    corner_option: Option<String>,
    corners: [idml_import::CornerSpec; 4],
}

fn corner_attrs_of(
    corner_radius: Option<f32>,
    corner_option: &Option<String>,
    corners: &[idml_import::CornerSpec; 4],
) -> CornerAttrs {
    CornerAttrs {
        corner_radius,
        corner_option: corner_option.clone(),
        corners: *corners,
    }
}

/// The IDML token InDesign writes for a `CornerOption` value. Note
/// `BevelCorner` (not `BeveledCorner`) — that's the spelling measured in
/// the real-export corpus, and the parser accepts both.
fn corner_option_idml(v: idml_import::CornerOption) -> &'static str {
    use idml_import::CornerOption as C;
    match v {
        C::None => "None",
        C::Rounded => "RoundedCorner",
        C::Inverse => "InverseRoundedCorner",
        C::Inset => "InsetCorner",
        C::Bevel => "BevelCorner",
        C::Fancy => "FancyCorner",
    }
}

/// `[top_left, top_right, bottom_right, bottom_left]` attribute names,
/// index-parallel to `CornerAttrs::corners`.
const PER_CORNER_KEYS: [(&str, &str); 4] = [
    ("TopLeftCornerOption", "TopLeftCornerRadius"),
    ("TopRightCornerOption", "TopRightCornerRadius"),
    ("BottomRightCornerOption", "BottomRightCornerRadius"),
    ("BottomLeftCornerOption", "BottomLeftCornerRadius"),
];

/// Patch decision for one corner attribute. `None` ⇒ the key isn't a
/// corner attribute, or the model value is byte-equivalent to what's
/// already on disk (pass the original bytes through — `format_f32`
/// rounds to 4 decimals and the option enum loses the exact source
/// spelling, so re-emitting an UNMUTATED value would corrupt an
/// otherwise byte-identical round-trip).
fn corner_attr_patch(key: &[u8], raw: &[u8], c: &CornerAttrs) -> Option<Patch> {
    let raw = std::str::from_utf8(raw).ok();
    if key == b"CornerRadius" {
        return Some(preserving_f32_patch(raw, c.corner_radius));
    }
    if key == b"CornerOption" {
        // Parsed verbatim as a String, so the model value already IS
        // the on-disk spelling — a plain string patch round-trips.
        return Some(opt_string_patch(&c.corner_option));
    }
    for (i, (okey, rkey)) in PER_CORNER_KEYS.iter().enumerate() {
        if key == okey.as_bytes() {
            return Some(preserving_option_patch(raw, c.corners[i].option));
        }
        if key == rkey.as_bytes() {
            return Some(preserving_f32_patch(raw, c.corners[i].radius));
        }
    }
    None
}

/// Corner attributes to append when the model carries a value the source
/// element didn't have (a corner written onto a frame that never had the
/// attribute). Unmutated frames append nothing.
fn corner_attr_extras(c: &CornerAttrs) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(s) = &c.corner_option {
        out.push(("CornerOption", s.clone()));
    }
    if let Some(r) = c.corner_radius {
        out.push(("CornerRadius", format_f32(r)));
    }
    for (i, (okey, rkey)) in PER_CORNER_KEYS.iter().enumerate() {
        if let Some(o) = c.corners[i].option {
            out.push((okey, corner_option_idml(o).to_string()));
        }
        if let Some(r) = c.corners[i].radius {
            out.push((rkey, format_f32(r)));
        }
    }
    out
}

/// `Set` only when the model number differs from the on-disk spelling's
/// own parse; otherwise `None` keeps the source bytes.
pub(crate) fn preserving_f32_patch(raw: Option<&str>, v: Option<f32>) -> Patch {
    match v {
        Some(n) => {
            if raw.and_then(|s| s.trim().parse::<f32>().ok()) == Some(n) {
                Patch::Keep
            } else {
                Patch::Set(format_f32(n))
            }
        }
        None => Patch::Remove,
    }
}

/// Same rule for an IDML tint percentage, whose parse is lossy in a way
/// `f32` alone is not: `FillTint="-1"` is IDML's sentinel for *no tint
/// override*, so [`idml_import::parse_tint`] maps it to `None` — the
/// same `None` an absent attribute gives, because they are the same
/// document.
///
/// [`preserving_f32_patch`] answers `Remove` for `None`, which is
/// correct only when the source spelled a REAL tint that the model has
/// since cleared. Against a `-1` it deletes an attribute nobody touched:
/// 156 corpus stories and 6 spreads, 288 attributes, on a save that
/// changed nothing. So ask the source spelling what IT means first, and
/// keep the bytes when the two agree.
///
/// The rule is not restated here — [`idml_import::parse_tint`] is the
/// one the parser reads through, called directly.
pub(crate) fn preserving_tint_patch(raw: Option<&str>, v: Option<f32>) -> Patch {
    if raw.and_then(idml_import::parse_tint) == v {
        return Patch::Keep;
    }
    match v {
        Some(n) => Patch::Set(format_f32(n)),
        None => Patch::Remove,
    }
}

/// The same shape once more for a flag with an IMPLICIT default:
/// `Nonprinting`, which the parser reads as
/// `attr.and_then(parse::<bool>).unwrap_or(false)`.
///
/// The writer used to spell `false` as `Remove` unconditionally — sound
/// reasoning (absence restores the default) with a byte cost, because a
/// source that spells `Nonprinting="false"` explicitly means exactly
/// what the model holds and loses the attribute anyway. `Remove` is
/// right only for a source that spelled the NON-default and has since
/// been changed back.
fn preserving_bool_patch(raw: Option<&str>, v: bool, default: bool) -> Patch {
    if raw.and_then(|s| s.parse::<bool>().ok()).unwrap_or(default) == v {
        Patch::Keep
    } else if v == default {
        Patch::Remove
    } else {
        Patch::Set(v.to_string())
    }
}

/// Same rule for a `CornerOption` enum: the parse is lossy across
/// spellings (`BevelCorner` / `BeveledCorner` both mean `Bevel`), so an
/// unmutated value must keep its source token.
fn preserving_option_patch(raw: Option<&str>, v: Option<idml_import::CornerOption>) -> Patch {
    match v {
        Some(o) => {
            if raw.and_then(idml_import::CornerOption::from_idml) == Some(o) {
                Patch::Keep
            } else {
                Patch::Set(corner_option_idml(o).to_string())
            }
        }
        None => Patch::Remove,
    }
}

fn patch_vector_item(
    e: &BytesStart,
    tx: TransformPlan,
    item: Option<VectorItem>,
) -> Result<Option<BytesStart<'static>>, quick_xml::Error> {
    let Some(item) = item else {
        return Ok(None);
    };
    let start = patch_start(
        e,
        |k, raw| {
            frame_attr_patch(
                k,
                raw,
                tx,
                item.fill.as_ref(),
                &item.stroke_color,
                item.stroke_weight,
                None,
                item.nonprinting,
                item.bounds,
                item.start_arrow,
                item.end_arrow,
                item.corners.as_ref(),
                &item.applied_object_style,
                &item.item_layer,
            )
        },
        &frame_attr_extras(
            tx,
            item.fill.as_ref(),
            &item.stroke_color,
            item.stroke_weight,
            None,
            item.nonprinting,
            item.start_arrow,
            item.end_arrow,
            item.corners.as_ref(),
            item.applied_object_style.as_deref(),
            item.item_layer.as_deref(),
        ),
    )?;
    let start = patch_gradient_geometry(&start, item.gradient)?;
    Ok(Some(start.into_owned()))
}

/// Patch decision for one frame attribute key. `next` is `Some` only for
/// TextFrame (`NextTextFrame` lives there); `None` skips that key for
/// other kinds. Bounds patch only fires for a `GeometricBounds`
/// attribute that the source element already carries. `tx` decides
/// `ItemTransform` — including passing it through verbatim, which is both
/// the degenerate-group case and (far more commonly) an untouched
/// high-precision transform; see [`TransformPlan`].
#[allow(clippy::too_many_arguments)]
fn frame_attr_patch(
    key: &[u8],
    raw: &[u8],
    tx: TransformPlan,
    fill: Option<&Fill>,
    stroke: &Option<String>,
    stroke_weight: Option<f32>,
    next: Option<&Option<String>>,
    nonprinting: bool,
    bounds: idml_import::Bounds,
    start_arrow: Option<idml_import::ArrowheadType>,
    end_arrow: Option<idml_import::ArrowheadType>,
    corners: Option<&CornerAttrs>,
    applied_object_style: &Option<String>,
    item_layer: &Option<String>,
) -> Option<Patch> {
    // B-23 — corner vocabulary first; `None` falls through to the rest.
    if let Some(c) = corners {
        if let Some(p) = corner_attr_patch(key, raw, c) {
            return Some(p);
        }
    }
    match key {
        b"AppliedObjectStyle" => Some(applied_object_style_patch(raw, applied_object_style)),
        b"ItemLayer" => Some(item_layer_patch(raw, item_layer)),
        b"ItemTransform" => tx.patch_for(raw),
        // `fill: None` ⇒ this KIND models no fill (see [`Fill`]); the
        // source attribute is nobody's to rewrite and passes through,
        // the same way `next` does for the kinds with no
        // `NextTextFrame` field.
        b"FillColor" => fill.map(|f| opt_string_patch(&f.color)),
        b"FillTint" => fill.map(|f| preserving_tint_patch(std::str::from_utf8(raw).ok(), f.tint)),
        b"StrokeColor" => Some(opt_string_patch(stroke)),
        // Preserved, not re-derived. The parser stores this attribute as
        // a plain `"…".parse::<f32>()` — no composition, no unit
        // conversion, and object styles are resolved by consumers out of
        // their own registry rather than folded back into the item — so
        // replaying that derivation against the source spelling is just
        // parsing it, and `preserving_f32_patch` is the whole check.
        // Without it the hairlines InDesign writes at full precision
        // (`0.7086614173228347`) came back as `0.7087` on a save that
        // changed nothing.
        b"StrokeWeight" => Some(preserving_f32_patch(
            std::str::from_utf8(raw).ok(),
            stroke_weight,
        )),
        // The parser defaults absent → false. Dropping the attribute
        // restores that default — but only when the source spelled
        // something else; a source `Nonprinting="false"` already says
        // what the model holds. See [`preserving_bool_patch`].
        b"Nonprinting" => Some(preserving_bool_patch(
            std::str::from_utf8(raw).ok(),
            nonprinting,
            false,
        )),
        b"NextTextFrame" => next.map(opt_string_patch),
        b"LeftLineEnd" => arrow_patch(start_arrow),
        b"RightLineEnd" => arrow_patch(end_arrow),
        b"GeometricBounds" => Some(Patch::Set(format!(
            "{} {} {} {}",
            format_f32(bounds.top),
            format_f32(bounds.left),
            format_f32(bounds.bottom),
            format_f32(bounds.right),
        ))),
        _ => None,
    }
}

/// Extras to append when a model attribute is set but the source element
/// didn't carry the key. Only emitted for genuinely-set values (so an
/// unmutated frame appends nothing and round-trips byte-identically).
/// `GeometricBounds` is intentionally NOT an extra: a path-geometry
/// frame's bounds are saved by rewriting its `<PathPointArray>` anchors
/// (see [`ModelGeometry`]), not by inventing a `GeometricBounds`
/// attribute the source never had.
#[allow(clippy::too_many_arguments)]
fn frame_attr_extras(
    tx: TransformPlan,
    fill: Option<&Fill>,
    stroke: &Option<String>,
    stroke_weight: Option<f32>,
    next: Option<&str>,
    nonprinting: bool,
    start_arrow: Option<idml_import::ArrowheadType>,
    end_arrow: Option<idml_import::ArrowheadType>,
    corners: Option<&CornerAttrs>,
    applied_object_style: Option<&str>,
    item_layer: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    // An object style APPLIED to an item whose source element never
    // carried the attribute (a leaner generator's output) — without
    // this the application is dropped, the same way C-19's `FillTint`
    // used to fall off here.
    if let Some(s) = applied_object_style {
        out.push(("AppliedObjectStyle", s.to_string()));
    }
    // The layer an item sits on, when the source element never said —
    // every engine-minted item, until 2026-09-05. Without it InDesign
    // opens the whole document on one layer.
    if let Some(l) = item_layer {
        out.push(("ItemLayer", l.to_string()));
    }
    if let Some(m) = tx.extra() {
        out.push(("ItemTransform", m));
    }
    // A kind that models no fill (see [`Fill`]) appends neither key —
    // it has nothing to say about them.
    if let Some(f) = fill {
        if let Some(c) = &f.color {
            out.push(("FillColor", c.clone()));
        }
        // C-19: a tint SET on an item whose source element never carried
        // a `FillTint` attribute used to fall off here (the patch lane
        // only rewrites keys the source already has).
        if let Some(t) = f.tint {
            out.push(("FillTint", format_f32(t)));
        }
    }
    if let Some(c) = stroke {
        out.push(("StrokeColor", c.clone()));
    }
    if let Some(w) = stroke_weight {
        out.push(("StrokeWeight", format_f32(w)));
    }
    if let Some(n) = next {
        out.push(("NextTextFrame", n.to_string()));
    }
    if nonprinting {
        out.push(("Nonprinting", "true".to_string()));
    }
    for (key, arrow) in [("LeftLineEnd", start_arrow), ("RightLineEnd", end_arrow)] {
        // `None` (the variant) is IDML's implicit default — absence of
        // the attribute restores it, so only drawable, representable
        // ends are appended.
        if let Some(t) = arrow {
            if t.draws() && !t.as_idml().is_empty() {
                out.push((key, t.as_idml().to_string()));
            }
        }
    }
    if let Some(c) = corners {
        out.extend(corner_attr_extras(c));
    }
    out
}

/// Patch decision for a `LeftLineEnd` / `RightLineEnd` attribute. The
/// kinds that don't carry the model fields pass `None` — their source
/// attribute survives verbatim. So does `Other` (an out-of-vocabulary
/// source token the parse layer couldn't keep): patching it would
/// clobber a spelling we can't reproduce.
fn arrow_patch(v: Option<idml_import::ArrowheadType>) -> Option<Patch> {
    use idml_import::ArrowheadType as A;
    match v {
        None | Some(A::Other) => None,
        Some(A::None) => Some(Patch::Remove),
        Some(t) => Some(Patch::Set(t.as_idml().to_string())),
    }
}

pub(crate) fn opt_string_patch(v: &Option<String>) -> Patch {
    match v {
        Some(s) => Patch::Set(s.clone()),
        None => Patch::Remove,
    }
}

/// `ItemLayer`: the model's layer when it names one (kept when the
/// source already spells it), and KEPT — never removed — when the model
/// names none: an item's layer is never something the engine unsets, so
/// an absent model value means "the source's word stands".
fn item_layer_patch(raw: &[u8], model: &Option<String>) -> Patch {
    match model {
        Some(l) if raw == l.as_bytes() => Patch::Keep,
        Some(l) => Patch::Set(l.clone()),
        None => Patch::Keep,
    }
}

/// `AppliedObjectStyle` is not an ordinary optional attribute: IDML has
/// no "no object style", it has the reserved
/// [`NONE_OBJECT_STYLE`] sentinel, which InDesign writes on every page
/// item. So a CLEARED reference restores that sentinel rather than
/// dropping the attribute.
///
/// The parser reads this value RAW (`util::attr`, no entity decoding),
/// so an untouched reference IS the on-disk bytes — `Keep` them instead
/// of round-tripping them through `escape_attr`, which would turn a
/// source `&amp;` into `&amp;amp;` and break byte-identity.
fn applied_object_style_patch(raw: &[u8], v: &Option<String>) -> Patch {
    match v {
        Some(s) if s.as_bytes() == raw => Patch::Keep,
        Some(s) => Patch::Set(s.clone()),
        None => Patch::Set(NONE_OBJECT_STYLE.to_string()),
    }
}

// ---------------------------------------------------------------------
// Story rewrite
// ---------------------------------------------------------------------

/// Index every `<Table>` cell in the story by its `Self` id so a `<Cell
/// Self="...">` start tag in the XML can find its model counterpart
/// (W1.15 lane 3). Cells hang off `Paragraph::table.cells`. IDML DOES
/// allow a table nested inside a cell's paragraph, so this recurses into
/// every cell's nested table — otherwise the inner cells aren't matched
/// and their `AppliedParagraphStyle`/`AppliedCharacterStyle` drop on a
/// rewrite. A cell with no `Self` id (rare) is skipped — its content
/// keeps passing through verbatim.
fn collect_story_cells(story: &Story) -> std::collections::HashMap<&str, &TableCell> {
    let mut out: std::collections::HashMap<&str, &TableCell> = std::collections::HashMap::new();
    for p in &story.paragraphs {
        if let Some(table) = &p.table {
            collect_table_cells(table, &mut out);
        }
    }
    out
}

/// Collect a table's cells (by `Self`) and recurse into any table nested
/// in a cell's paragraph.
fn collect_table_cells<'a>(
    table: &'a idml_import::Table,
    out: &mut std::collections::HashMap<&'a str, &'a TableCell>,
) {
    for cell in &table.cells {
        if let Some(id) = cell.self_id.as_deref() {
            out.insert(id, cell);
        }
        for cp in &cell.paragraphs {
            if let Some(inner) = &cp.table {
                collect_table_cells(inner, out);
            }
        }
    }
}

/// Rewrite a `Story_*.xml` body so its `<ParagraphStyleRange>` /
/// `<CharacterStyleRange>` attributes + single-Content text reflect the
/// current model.
///
/// # Matching ranges to model items
///
/// IDML carries no id on a style range, so the range→model link has to
/// be derived. It used to be derived by COUNTING: the nth
/// `<CharacterStyleRange>` element patched against the nth
/// `CharacterRun`. That is only correct if the parser keeps exactly one
/// model item per source element, and it does not — it drops a range
/// whose text came out empty (one holding only a `<Table>`, a
/// self-closing `<CharacterStyleRange/>`), drops a paragraph range left
/// with neither a run nor a table, and splits a range containing a
/// `<TextVariableInstance>` into several runs. Every element after such
/// a range was then patched against the WRONG model item, so an
/// unmutated save moved `PointSize="10"` to `8`, dropped a
/// `FontStyle="Bold"`, rewrote `AppliedCharacterStyle`s, and deleted
/// `AppliedParagraphStyle` from 99 corpus stories outright.
///
/// The link now comes from [`idml_import::StoryProvenance`] — the map
/// the PARSER emits saying which model item each source element became.
/// The rule that drops and splits lives in one place, on the side that
/// owns it; the writer looks up rather than guesses. An element with no
/// entry has no model counterpart and passes through VERBATIM, which is
/// also the right answer for markup the parser suppresses wholesale
/// (`<HiddenText>`, `<Note>`).
///
/// The provenance is derived from `original` here rather than taken as
/// an argument so it cannot be computed from different bytes than the
/// ones being streamed. That costs one extra parse of the entry; a
/// provenance recomputed by re-deriving the drop rule on this side would
/// cost the invariant instead. When the parse fails the map is empty and
/// every range passes through verbatim — the same conservative answer.
pub fn rewrite_story(original: &[u8], story: &Story) -> Result<Vec<u8>, quick_xml::Error> {
    rewrite_story_in_frame(original, story, None)
}

/// [`rewrite_story`] with the inner width of the text column the story
/// flows in, when the caller knows it — the column fallback for a table
/// the model never sized (see `emit::write_table`).
pub fn rewrite_story_in_frame(
    original: &[u8],
    story: &Story,
    host_width: Option<f32>,
) -> Result<Vec<u8>, quick_xml::Error> {
    // Tab stops and bullet characters are `<Properties>` children, not
    // range attributes; they are spelled first, and the provenance below
    // is derived from the spelled bytes. The pass adds, replaces and
    // drops those children only, so the parser maps the same ranges and
    // runs either way.
    let spelled = crate::paragraph_props::spell(original, story)?;
    let original: &[u8] = &spelled;
    let (provenance, provenance_ok) = match idml_import::parse_story_with_provenance(original) {
        Ok((_, p)) => (p, true),
        Err(_) => (Default::default(), false),
    };
    // How many story-level `<ParagraphStyleRange>`s the source carries —
    // so the LAST one is recognisable when the model appends paragraphs
    // after it (see the `</ParagraphStyleRange>` arm).
    let mut mapped_top = 0usize;
    // Story-level ranges with nothing in them at all — `<ParagraphStyleRange/>`
    // or a start tag followed by its end tag. Not a paragraph to InDesign,
    // not one to the parser: dropped rather than passed through. An older
    // emitter left one where a table failed to serialise, and the trailing
    // mark then landed inside it as an empty line above every captioned
    // table in the annual (2026-09-06).
    let mut childless: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let source_psr_total = {
        let mut r = Reader::from_reader(original);
        r.config_mut().trim_text(false);
        let mut b = Vec::new();
        let mut depth_t = 0usize;
        let mut n = 0usize;
        let mut open_psr: Option<u64> = None;
        loop {
            let pos = r.buffer_position();
            match r.read_event_into(&mut b)? {
                Event::Eof => break,
                Event::Start(ref e) if e.name().as_ref() == b"Table" => {
                    depth_t += 1;
                    open_psr = None;
                }
                Event::End(ref e) if e.name().as_ref() == b"Table" => {
                    depth_t = depth_t.saturating_sub(1);
                    open_psr = None;
                }
                Event::Start(ref e)
                    if depth_t == 0 && e.name().as_ref() == b"ParagraphStyleRange" =>
                {
                    if provenance.paragraph_at(pos).is_some() {
                        mapped_top += 1;
                    }
                    n += 1;
                    open_psr = Some(pos);
                }
                Event::Empty(ref e)
                    if depth_t == 0 && e.name().as_ref() == b"ParagraphStyleRange" =>
                {
                    if provenance.paragraph_at(pos).is_some() {
                        mapped_top += 1;
                    }
                    n += 1;
                    childless.insert(pos);
                    open_psr = None;
                }
                Event::End(ref e)
                    if depth_t == 0 && e.name().as_ref() == b"ParagraphStyleRange" =>
                {
                    if let Some(start) = open_psr.take() {
                        childless.insert(start);
                    }
                }
                Event::Text(ref t) if t.iter().all(|c| c.is_ascii_whitespace()) => {}
                _ => open_psr = None,
            }
            b.clear();
        }
        n
    };
    // The model has FEWER story-level paragraphs than the part maps: a
    // paragraph was deleted since the part was written, or the part
    // carries empty ranges an older parser dropped and the model never
    // held (the annual's captioned tables sat a blank line lower in
    // InDesign than on the canvas). Positional patching cannot say which
    // range went; the model is the truth, so the part is written fresh.
    // (A minted story does not serialise footnotes yet, so a story that
    // carries some keeps the positional lane rather than lose them.)
    let has_footnotes = story.paragraphs.iter().any(|p| !p.footnotes.is_empty());
    if provenance_ok && mapped_top > story.paragraphs.len() && !has_footnotes {
        return fresh_story_part(original, story, host_width);
    }

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    // The model paragraph the open `<ParagraphStyleRange>` maps to, one
    // slot per scope (the story's own stream, and a table cell's). A
    // range's runs are looked up inside it.
    let mut story_para: Option<&Paragraph> = None;
    let mut cell_para: Option<&Paragraph> = None;
    // The run currently open (for Content text + attribute patching).
    let mut current_run: Option<&CharacterRun> = None;
    // True when the open range produced MORE than one model run — a
    // `<TextVariableInstance>` split. Its attributes are still patched
    // (the split runs are clones of one style), but its text must never
    // be re-serialised, because no single run holds all of it.
    let mut current_run_split = false;
    // Buffered inline body of the open `<CharacterStyleRange>`. The
    // parser collapses a run's `<Content>A</Content><Br/><Content>B
    // </Content>` (and `<Tab/>` between segments) into one run string
    // with `\n` / `\t` separators, so a faithful save has to split the
    // model text back across that Content/Br/Tab structure — not just
    // patch a single Content. We buffer the whole contiguous inline
    // region (it is always the LAST thing in a run; `<Properties>` and
    // anchored frames come first and stream out immediately) so the
    // replace-or-passthrough decision can be made once at the run's
    // close, when the full reconstructed text is known. See
    // [`RunBody`].
    let mut body = RunBody::default();
    // Whether the previous run in this paragraph ended with a `<Br/>`:
    // the parser then starts THIS run's text with the `\n` (see
    // `flush_run_body`). Reset at every paragraph range.
    let mut prev_ended_with_br = false;
    // Model runs the MODEL split off source ranges earlier in this
    // paragraph (see the `</CharacterStyleRange>` arm): every later range's
    // provenance index shifts by this much. Reset at every paragraph range.
    let mut run_offset: usize = 0;
    // The model index of the open range's (first) run, offset applied.
    let mut current_first: Option<usize> = None;
    // Wrapping-form hyperlink / cross-reference sources currently open
    // OUTSIDE a range (the engine's older spelling): a run replaced inside
    // one must not gain a second, inner wrapper.
    let mut open_sources: usize = 0;
    let mut csr_open = false;
    // ---- what the SOURCE story lacks and the model has ----
    // The highest model paragraph index a story-level range resolved to:
    // model paragraphs beyond it have no source element (a table
    // `InsertTable` appended after a checkpoint, text appended at the
    // end) and are written whole at `</Story>`.
    let mut max_story_para: Option<usize> = None;
    let mut source_psr_seen = 0usize;
    // Inside a childless story-level range being dropped (see `childless`).
    let mut skip_psr = false;
    // Whether the open story-level range carried a `<Table>` — a model
    // paragraph with a table the source range lacks gets it at the
    // range's close.
    let mut table_in_para = false;
    // Depth of open `<Table>` elements. Inside a table the
    // `<ParagraphStyleRange>` / `<CharacterStyleRange>` belong to CELL
    // paragraphs, which the parser stores on `paragraph.table.cells[]`,
    // NOT on the story's top-level `paragraphs`. Patching them against
    // `story.paragraphs` would misalign, so the story-level cursors do
    // NOT advance inside a table.
    let mut table_depth: usize = 0;

    // W1.8 — depth of open `<Footnote>` elements. A footnote is a
    // self-contained paragraph stream anchored mid-run; the parser keeps
    // its body on `paragraph.footnotes[].paragraphs`, NOT on the story's
    // top-level `paragraphs` (see `idml_import::story`'s footnote stack).
    // So the story-level positional cursors must NOT advance inside a
    // footnote, and the footnote's own `<ParagraphStyleRange>` /
    // `<CharacterStyleRange>` / `<Content>` must NOT patch against the
    // host story. While `footnote_depth > 0` the entire subtree is
    // treated as opaque inline markup of the *host* run: it buffers into
    // the open `RunBody` as foreign (so the host run replays verbatim and
    // never rewrites over the anchor) and the matching `</Footnote>`
    // restores normal flow. Without this guard the footnote's inner
    // ranges escaped the buffer, advanced the cursors, and left the host
    // run's `<Content>` + `<Footnote>` open tag dropped — yielding a
    // mismatched `</Footnote>` and a re-parse failure (zero pages).
    let mut footnote_depth: usize = 0;

    // W1.15 lane 3 — table-cell text write-back. Inside a `<Cell
    // Self="...">` the `<ParagraphStyleRange>` / `<CharacterStyleRange>`
    // patch against the matched model `TableCell.paragraphs[]`, resolved
    // through the same provenance map (its indices are scope-local, so a
    // cell range resolves inside the cell). When a cell has no model
    // match — or the cell text is unchanged — its content passes through
    // verbatim, exactly as before.
    let cells = collect_story_cells(story);
    // Depth of the open `<Cell>`, or 0.
    let mut current_cell: Option<&TableCell> = None;
    let mut cell_depth: usize = 0;
    // Nested tables (a table in a cell's paragraph) nest `<Cell>`s, so
    // the cell-local state is a stack: each `<Cell>` open parks the
    // enclosing cell's state, each `</Cell>` restores it.
    type CellFrame<'a> = (Option<&'a TableCell>, usize, Option<&'a Paragraph>);
    let mut cell_stack: Vec<CellFrame> = Vec::new();

    loop {
        // The offset at which this event's markup begins — the key the
        // parser built its provenance map on (see the doc comment).
        let event_pos = reader.buffer_position();
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                // True while patching cell content: inside a `<Cell>`
                // that matched a model cell. The ranges then resolve
                // against the cell's paragraphs with cell-local cursors.
                let in_cell = table_depth > 0 && current_cell.is_some();
                match e.name().as_ref() {
                    b"Footnote" => {
                        // Enter a footnote: its body is a separate stream
                        // the host story doesn't model. Buffer the whole
                        // subtree into the open host run as foreign inline
                        // markup so it replays verbatim and the host run's
                        // text is never rewritten over the anchor. Activate
                        // the body if the footnote leads the run (no prior
                        // `<Content>`), so the buffer captures it.
                        footnote_depth += 1;
                        body.active = true;
                        body.foreign = true;
                        body.events.push(Event::Start(e.into_owned()));
                    }
                    // Inside a footnote, every element is opaque host-run
                    // markup — buffer it, don't patch it against the story.
                    _ if footnote_depth > 0 => {
                        body.events.push(Event::Start(e.into_owned()));
                    }
                    b"Table" => {
                        table_depth += 1;
                        if table_depth == 1 {
                            table_in_para = true;
                        }
                        writer.write_event(Event::Start(e.into_owned()))?;
                    }
                    b"Cell" if table_depth > 0 => {
                        // Enter a cell — park the enclosing cell's cursors
                        // (nested tables nest cells) then bind this cell's
                        // model counterpart (by `Self`) + reset the
                        // cell-local cursors. The start tag passes through
                        // verbatim (cell-level attributes patched elsewhere).
                        cell_stack.push((current_cell, cell_depth, cell_para));
                        cell_depth = table_depth;
                        cell_para = None;
                        current_cell =
                            attr_value(&e, b"Self").and_then(|id| cells.get(id.as_str()).copied());
                        writer.write_event(Event::Start(e.into_owned()))?;
                    }
                    b"ParagraphStyleRange" if table_depth == 0 => {
                        run_offset = 0;
                        table_in_para = false;
                        source_psr_seen += 1;
                        if let Some(i) = provenance
                            .paragraph_at(event_pos)
                            .and_then(|i| model_paragraph_index(&story.paragraphs, i))
                        {
                            max_story_para = Some(max_story_para.map_or(i, |m| m.max(i)));
                        }
                        story_para = resolve_paragraph(&provenance, event_pos, &story.paragraphs);
                        let start = patch_paragraph_range(&e, story_para)?;
                        if childless.contains(&event_pos) {
                            // Dropped; the paragraph before it keeps its
                            // mark state for the trailing-content rule.
                            skip_psr = true;
                        } else {
                            prev_ended_with_br = false;
                            writer.write_event(Event::Start(start))?;
                        }
                    }
                    b"ParagraphStyleRange" if in_cell => {
                        prev_ended_with_br = false;
                        run_offset = 0;
                        cell_para = current_cell
                            .and_then(|c| resolve_paragraph(&provenance, event_pos, &c.paragraphs));
                        let start = patch_paragraph_range(&e, cell_para)?;
                        writer.write_event(Event::Start(start))?;
                    }
                    b"CharacterStyleRange" if table_depth == 0 => {
                        (current_run, current_run_split, current_first) =
                            resolve_run(&provenance, event_pos, story_para, run_offset);
                        csr_open = true;
                        body = RunBody::default();
                        let start = patch_character_range(&e, current_run)?;
                        writer.write_event(Event::Start(start))?;
                    }
                    b"CharacterStyleRange" if in_cell => {
                        (current_run, current_run_split, current_first) =
                            resolve_run(&provenance, event_pos, cell_para, run_offset);
                        csr_open = true;
                        body = RunBody::default();
                        let start = patch_character_range(&e, current_run)?;
                        writer.write_event(Event::Start(start))?;
                    }
                    b"Content" if table_depth == 0 || in_cell => {
                        // A `<Content>` opens the inline body region (or
                        // continues it). Buffer the start; the text /
                        // entities inside accumulate into the body, and
                        // the matching End is buffered too. Once any
                        // inline leaf appears, every later event in the
                        // run buffers (foreign markup flips the guard).
                        body.active = true;
                        body.in_content = true;
                        body.ends_with_br = false;
                        body.events.push(Event::Start(e.into_owned()));
                    }
                    _ => {
                        if body.active {
                            // A non-inline element opened inside the
                            // buffered region (e.g. an unexpected child
                            // of `<Content>`). Never rewrite over it.
                            body.foreign = true;
                            body.events.push(Event::Start(e.into_owned()));
                        } else {
                            if !csr_open
                                && matches!(
                                    e.name().as_ref(),
                                    b"HyperlinkTextSource" | b"CrossReferenceSource"
                                )
                            {
                                open_sources += 1;
                            }
                            writer.write_event(Event::Start(e.into_owned()))?;
                        }
                    }
                }
            }
            Event::Empty(e) => {
                let in_cell = table_depth > 0 && current_cell.is_some();
                // Inside a footnote every empty element is opaque host-run
                // markup (a footnote anchor's own `<Br/>` etc.) — buffer it
                // so it replays verbatim and never advances the story
                // cursors. A self-closing `<Footnote/>` (no body) opens and
                // closes in one event, so it never changes `footnote_depth`.
                if footnote_depth > 0 {
                    body.events.push(Event::Empty(e.into_owned()));
                    buf.clear();
                    continue;
                }
                // A self-closing CharacterStyleRange / ParagraphStyleRange
                // still advances the positional cursor + patches attrs.
                match e.name().as_ref() {
                    b"ParagraphStyleRange" if table_depth == 0 => {
                        if !childless.contains(&event_pos) {
                            prev_ended_with_br = false;
                        }
                        run_offset = 0;
                        source_psr_seen += 1;
                        if let Some(i) = provenance
                            .paragraph_at(event_pos)
                            .and_then(|i| model_paragraph_index(&story.paragraphs, i))
                        {
                            max_story_para = Some(max_story_para.map_or(i, |m| m.max(i)));
                        }
                        // A self-closing paragraph range has no runs, so
                        // the parser dropped it and the map has no entry
                        // — it passes through verbatim. Still recorded
                        // as the open paragraph so a following range
                        // resolves against the right scope.
                        story_para = resolve_paragraph(&provenance, event_pos, &story.paragraphs);
                        let start = patch_paragraph_range(&e, story_para)?;
                        if !childless.contains(&event_pos) {
                            writer.write_event(Event::Empty(start))?;
                        }
                    }
                    b"ParagraphStyleRange" if in_cell => {
                        prev_ended_with_br = false;
                        run_offset = 0;
                        cell_para = current_cell
                            .and_then(|c| resolve_paragraph(&provenance, event_pos, &c.paragraphs));
                        let start = patch_paragraph_range(&e, cell_para)?;
                        writer.write_event(Event::Empty(start))?;
                    }
                    b"CharacterStyleRange" if table_depth == 0 => {
                        current_run = None;
                        current_run_split = false;
                        current_first = None;
                        csr_open = false;
                        body = RunBody::default();
                        let (run, _, _) =
                            resolve_run(&provenance, event_pos, story_para, run_offset);
                        let start = patch_character_range(&e, run)?;
                        writer.write_event(Event::Empty(start))?;
                    }
                    b"CharacterStyleRange" if in_cell => {
                        current_run = None;
                        current_run_split = false;
                        current_first = None;
                        csr_open = false;
                        body = RunBody::default();
                        let (run, _, _) =
                            resolve_run(&provenance, event_pos, cell_para, run_offset);
                        let start = patch_character_range(&e, run)?;
                        writer.write_event(Event::Empty(start))?;
                    }
                    b"Br" if (table_depth == 0 || in_cell) && !body.in_content => {
                        // `<Br/>` is an inline leaf → `\n` in the parser's
                        // run text. It opens (or continues) the body
                        // region — a run can start with `\n` (a leading
                        // `<Br/>` before the first `<Content>`). Mirror
                        // the newline so the split survives a rewrite.
                        body.active = true;
                        body.text.push('\n');
                        body.ends_with_br = true;
                        body.events.push(Event::Empty(e.into_owned()));
                    }
                    b"Tab" if (table_depth == 0 || in_cell) && !body.in_content => {
                        // `<Tab/>` is an inline leaf → `\t`. Opens or
                        // continues the body region (see `<Br/>`).
                        body.active = true;
                        body.text.push('\t');
                        body.ends_with_br = false;
                        body.events.push(Event::Empty(e.into_owned()));
                    }
                    _ => {
                        if body.active {
                            // An empty element inside the span (PI-like
                            // marker, anchored frame, unknown) — never
                            // rewrite over it.
                            body.foreign = true;
                            body.events.push(Event::Empty(e.into_owned()));
                        } else {
                            writer.write_event(Event::Empty(e.into_owned()))?;
                        }
                    }
                }
            }
            Event::Text(t) => {
                if skip_psr {
                    // Whitespace inside a dropped childless range.
                    buf.clear();
                    continue;
                }
                if body.active && body.in_content {
                    // Buffer — the replace decision happens at the run
                    // close once the whole (possibly entity-split) span
                    // is known. Reconstruct the text with the PARSER's
                    // rule, not a near-miss of it (see
                    // [`push_run_text`]).
                    let decoded = t
                        .xml_content(quick_xml::XmlVersion::Implicit1_0)
                        .unwrap_or_default();
                    push_run_text(&mut body.text, &decoded);
                    body.events.push(Event::Text(t.into_owned()));
                } else if body.active {
                    // Indentation/whitespace between inline leaves —
                    // buffer it so a verbatim replay stays byte-exact.
                    body.events.push(Event::Text(t.into_owned()));
                } else {
                    writer.write_event(Event::Text(t))?;
                }
            }
            Event::GeneralRef(r) => {
                if body.active && body.in_content {
                    // Resolve the reference (predefined five + numeric)
                    // so the comparison sees the parsed run's chars.
                    let name = String::from_utf8_lossy(r.as_ref()).into_owned();
                    let resolved = quick_xml::escape::unescape(&format!("&{name};"))
                        .map(|c| c.into_owned())
                        .unwrap_or_default();
                    if resolved.is_empty() {
                        // Unknown entity — never rewrite over it.
                        body.foreign = true;
                    }
                    push_run_text(&mut body.text, &resolved);
                    body.events.push(Event::GeneralRef(r.into_owned()));
                } else if body.active {
                    body.foreign = true;
                    body.events.push(Event::GeneralRef(r.into_owned()));
                } else {
                    writer.write_event(Event::GeneralRef(r))?;
                }
            }
            Event::End(e) => {
                let mut drop_this_end = false;
                // Inside a footnote every End buffers into the host run
                // (foreign) so the subtree replays verbatim; the matching
                // `</Footnote>` (when depth returns to 0) restores normal
                // flow. The inner `</CharacterStyleRange>` must NOT trigger
                // a host-run flush, and the inner `</ParagraphStyleRange>`
                // must NOT touch the story cursors.
                if e.name().as_ref() == b"Footnote" {
                    footnote_depth = footnote_depth.saturating_sub(1);
                    body.events.push(Event::End(e.into_owned()));
                    buf.clear();
                    continue;
                }
                if footnote_depth > 0 {
                    body.events.push(Event::End(e.into_owned()));
                    buf.clear();
                    continue;
                }
                match e.name().as_ref() {
                    b"Table" => table_depth = table_depth.saturating_sub(1),
                    b"Cell" if cell_depth != 0 && table_depth == cell_depth => {
                        // Leave the cell — restore the enclosing cell's
                        // state (a nested table's cell pops back to its
                        // host cell; a top-level cell pops back to None) so
                        // siblings + post-table markup patch correctly.
                        let (cc, cd, cp) = cell_stack.pop().unwrap_or((None, 0, None));
                        current_cell = cc;
                        cell_depth = cd;
                        cell_para = cp;
                    }
                    b"Content" if body.active => {
                        body.in_content = false;
                        body.events.push(Event::End(e.into_owned()));
                        continue; // already buffered + advanced
                    }
                    b"CharacterStyleRange" => {
                        // A range the PARSER split (`TextVariableInstance`)
                        // spreads its text over several runs; re-serialising
                        // it from any one of them would delete the rest.
                        // Flush it as unmatched (verbatim replay) — the
                        // attributes were still patched off the first run
                        // at the open tag.
                        let text_run = if current_run_split { None } else { current_run };
                        // A range the MODEL split (see `split_tail`): the
                        // first piece replaces the body, the other pieces
                        // follow as ranges of their own, the paragraph
                        // mark moving to the last of them.
                        let scope = if table_depth > 0 && current_cell.is_some() {
                            cell_para
                        } else {
                            story_para
                        };
                        let source_text = if body.ends_with_br {
                            body.text
                                .strip_suffix('\n')
                                .unwrap_or(&body.text)
                                .to_string()
                        } else {
                            body.text.clone()
                        };
                        let extras = if body.active && !body.foreign && !current_run_split {
                            split_tail(scope, current_first, &source_text, prev_ended_with_br)
                        } else {
                            Vec::new()
                        };
                        let mark = body.ends_with_br;
                        let wrap = open_sources == 0;
                        flush_run_body(
                            &mut writer,
                            &mut body,
                            text_run,
                            &mut prev_ended_with_br,
                            !extras.is_empty(),
                            wrap,
                        )?;
                        writer.write_event(Event::End(e))?;
                        if let Some(para) = scope {
                            let last = extras.len().saturating_sub(1);
                            for (n, i) in extras.iter().enumerate() {
                                write_split_run(
                                    &mut writer,
                                    &para.runs[*i],
                                    wrap,
                                    mark && n == last,
                                )?;
                            }
                        }
                        run_offset += extras.len();
                        current_run = None;
                        current_run_split = false;
                        current_first = None;
                        csr_open = false;
                        buf.clear();
                        continue;
                    }
                    // Any other End that arrives while the inline body is
                    // buffering has to buffer TOO, or the run's markup
                    // comes back scrambled.
                    //
                    // The buffer holds Starts, Texts and Emptys and
                    // replays them at `</CharacterStyleRange>`; an End
                    // written straight to the writer therefore JUMPS
                    // AHEAD of everything already buffered. A run that
                    // carries an anchored page item after its content —
                    // `<Content>…</Content><Rectangle>…</Rectangle>`, the
                    // shape InDesign uses for an anchored object, and
                    // `<HyperlinkTextSource>…<Content>…</Content></…>` for
                    // a hyperlink — then came out with the whole subtree's
                    // closing tags stacked in front of its opening ones.
                    // In the corpus that produced NOT-WELL-FORMED XML: a
                    // killed save, invisible to a byte-count and invisible
                    // to a sweep that only looked at Spreads.
                    //
                    // `Table` / `Cell` / `Content` / `CharacterStyleRange`
                    // keep their own arms above: they carry writer-side
                    // cursor state (table depth, the cell stack, the run
                    // flush) that buffering would strand.
                    _ if body.active => {
                        body.foreign = true;
                        body.events.push(Event::End(e.into_owned()));
                        buf.clear();
                        continue;
                    }
                    b"HyperlinkTextSource" | b"CrossReferenceSource" if !csr_open => {
                        open_sources = open_sources.saturating_sub(1);
                    }
                    b"ParagraphStyleRange" if table_depth == 0 && skip_psr => {
                        skip_psr = false;
                        drop_this_end = true;
                        let trailing_from = max_story_para.map_or(0, |m| m + 1);
                        let trailing_content = story
                            .paragraphs
                            .get(trailing_from..)
                            .is_some_and(|t| t.iter().any(|p| !paragraph_is_empty(p)));
                        if provenance_ok
                            && source_psr_seen == source_psr_total
                            && trailing_content
                            && !prev_ended_with_br
                        {
                            // The paragraph before the dropped range never
                            // ended, and text follows: it gets its mark in
                            // a range of its own (an empty line, the price
                            // of a source that lost one).
                            emit_start_with_attrs(&mut writer, "ParagraphStyleRange", &[])?;
                            emit_start_with_attrs(&mut writer, "CharacterStyleRange", &[])?;
                            writer.write_event(Event::Empty(BytesStart::new("Br")))?;
                            writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                                "CharacterStyleRange",
                            )))?;
                            writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                                "ParagraphStyleRange",
                            )))?;
                            prev_ended_with_br = true;
                        }
                        table_in_para = false;
                    }
                    b"ParagraphStyleRange" if table_depth == 0 => {
                        // A table the model holds on this paragraph and the
                        // source range does not carry: written here, in the
                        // minted-story vocabulary. (Every `InsertTable` lands
                        // on a fresh paragraph, so this is the checkpoint
                        // case — a `.paged` saved by an exporter without a
                        // table lane, reloaded, exported again.)
                        if !table_in_para {
                            if let Some(t) = story_para.and_then(|p| p.table.as_ref()) {
                                emit_start_with_attrs(
                                    &mut writer,
                                    "CharacterStyleRange",
                                    &[(
                                        "AppliedCharacterStyle",
                                        crate::emit::NO_CHARACTER_STYLE.to_string(),
                                    )],
                                )?;
                                crate::emit::write_table(&mut writer, t, host_width)?;
                                writer.write_event(Event::End(
                                    quick_xml::events::BytesEnd::new("CharacterStyleRange"),
                                ))?;
                            }
                        }
                        // The LAST source paragraph, with model paragraphs
                        // still to come after it: it needs a paragraph mark
                        // or InDesign reads the appended text as part of it.
                        let trailing_from = max_story_para.map_or(0, |m| m + 1);
                        let trailing_content = story
                            .paragraphs
                            .get(trailing_from..)
                            .is_some_and(|t| t.iter().any(|p| !paragraph_is_empty(p)));
                        if provenance_ok
                            && source_psr_seen == source_psr_total
                            && trailing_content
                            && !prev_ended_with_br
                        {
                            emit_start_with_attrs(&mut writer, "CharacterStyleRange", &[])?;
                            writer.write_event(Event::Empty(BytesStart::new("Br")))?;
                            writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                                "CharacterStyleRange",
                            )))?;
                        }
                        table_in_para = false;
                    }
                    b"Story" => {
                        // Model paragraphs beyond the last source range —
                        // appended after the source part was written.
                        let trailing_from = max_story_para.map_or(0, |m| m + 1);
                        let trailing_content = story
                            .paragraphs
                            .get(trailing_from..)
                            .is_some_and(|t| t.iter().any(|p| !paragraph_is_empty(p)));
                        if provenance_ok && trailing_content {
                            crate::emit::write_paragraphs(
                                &mut writer,
                                &story.paragraphs[trailing_from..],
                                &[],
                                host_width,
                            )?;
                        }
                    }
                    _ => {}
                }
                if !drop_this_end {
                    writer.write_event(Event::End(e))?;
                }
            }
            other => {
                if body.active {
                    // PI (e.g. InDesign's <?ACE 18?> marker) or other
                    // markup inside the span — buffer in order and
                    // never rewrite over it.
                    body.foreign = true;
                    body.events.push(other.into_owned());
                } else {
                    writer.write_event(other)?;
                }
            }
        }
        buf.clear();
    }

    Ok(writer.into_inner().into_inner())
}

/// Buffered inline body (`<Content>` / `<Br/>` / `<Tab/>` leaves) of one
/// open `<CharacterStyleRange>`. The decision to rewrite the run's text
/// — possibly across several `<Content>` segments — can only be made at
/// the run's close, once the whole reconstructed string is known. Until
/// then every inline event is buffered here in document order so an
/// unchanged run (or one with foreign markup) can be replayed
/// byte-for-byte.
#[derive(Default)]
struct RunBody {
    /// True once the first inline leaf has been seen — from that point
    /// every event in the run buffers rather than streaming out.
    active: bool,
    /// True while inside a `<Content>` element (its text accumulates).
    in_content: bool,
    /// Reconstructed run text: Content text verbatim, `\n` per `<Br/>`,
    /// `\t` per `<Tab/>` — exactly how the parser collapses the run.
    text: String,
    /// Any markup the rewrite must not clobber appeared in the body (a
    /// PI / ACE page-number marker, an anchored frame, a TextVariable
    /// instance, an unknown entity, …). When set, the body replays
    /// verbatim regardless of the model text.
    foreign: bool,
    /// Buffered events, in document order.
    events: Vec<Event<'static>>,
    /// The last inline leaf buffered was a `<Br/>` — a PARAGRAPH MARK when
    /// the run is the paragraph's last, a break that the parser hands to
    /// the NEXT run's text otherwise. Either way it is not in this run's
    /// text, and the comparison in [`flush_run_body`] must not read it
    /// as a difference.
    ends_with_br: bool,
}

/// Append one decoded `<Content>` fragment to a run's reconstructed
/// text, applying **exactly** the normalisation
/// `idml_import::parse_story` applies when it builds
/// `CharacterRun::text`: the Unicode line/paragraph separators
/// U+2028 / U+2029 (InDesign's "forced line break", Shift+Enter)
/// collapse to `\n`.
///
/// This is a comparison contract, not a preference. [`flush_run_body`]
/// decides whether to REPLACE a run's body by asking whether the model
/// text still equals the reconstructed source text — so any rule the
/// parser applies and the reconstruction doesn't makes every such run
/// look mutated. That is precisely what happened: 283 corpus stories
/// came back with different bytes and **no attribute difference at
/// all**, every one of them because it contained a U+2028. The rewrite
/// re-serialised each of those runs from the model, turning a forced
/// LINE break into `<Br/>` — an IDML PARAGRAPH break — and flattening
/// the source's `<Content>` / `<Br />` layout on the way. A save that
/// nobody asked for was silently changing the text's break semantics.
///
/// The caller decodes with `xml_content(Implicit1_0)` (the parser's
/// decoder) rather than `decode()`, which additionally gets the XML 1.0
/// end-of-line rule — `\r\n` and lone `\r` both normalise to `\n` — so a
/// CRLF-serialised story compares equal too.
fn push_run_text(out: &mut String, decoded: &str) {
    for ch in decoded.chars() {
        if matches!(ch, '\u{2028}' | '\u{2029}') {
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
}

/// Emit the buffered inline body of a closing run. When the model text
/// diverged from the reconstructed source AND the body is pure
/// Content/Br/Tab (no foreign markup to preserve), re-serialise the
/// model text across the Content/Br/Tab structure (mirroring
/// `paged_gen`'s `write_run_content`: `\n` → `<Br/>`, `\t` → `<Tab/>`,
/// runs of plain text → `<Content>…</Content>`). Otherwise replay the
/// original events so an unchanged run — or one carrying markers — stays
/// byte-identical.
///
/// # The paragraph mark is not text
///
/// The parser (`idml_import::story`) treats a `<Br/>` that closes a
/// run as PENDING: content following it in the same paragraph turns it
/// into a `\n` at the START of that following run's text; the
/// paragraph's end discards it — it was the terminator, not a
/// character. So a run's reconstructed source text can end with a `\n`
/// that is in no model run, and the NEXT model run can start with a `\n`
/// that is in no source range of its own. Comparing the raw strings read
/// both as mutations: every InDesign paragraph (which ends its last run
/// with `<Br />`) was re-serialised on an unmutated save, and the
/// re-serialisation DROPPED the mark — so a saved story reopened in
/// InDesign as one merged paragraph. `prev_br` carries the previous
/// run's trailing mark into this comparison; a replaced run re-emits the
/// mark it had.
///
/// `hold_br`: the paragraph mark belongs to a later split-off range —
/// do not write it here. `wrap_source`: a replaced run tagged with a
/// hyperlink source may take InDesign's inner `<HyperlinkTextSource>`
/// (false while a wrapping-form source is open around the range).
fn flush_run_body(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    body: &mut RunBody,
    run: Option<&CharacterRun>,
    prev_br: &mut bool,
    hold_br: bool,
    wrap_source: bool,
) -> Result<(), quick_xml::Error> {
    if !body.active {
        return Ok(());
    }
    let ends_with_br = body.ends_with_br;
    let model_text = |r: &CharacterRun| -> String {
        let t = r.text.as_str();
        let t = if *prev_br {
            t.strip_prefix('\n').unwrap_or(t)
        } else {
            t
        };
        t.to_string()
    };
    let source_text = if ends_with_br {
        body.text.strip_suffix('\n').unwrap_or(&body.text)
    } else {
        body.text.as_str()
    };
    // A run tagged with a hyperlink source whose source range carries no
    // wrapper (the range was tagged after the part was written — a
    // checkpoint reload) is re-serialised too, wrapped: an existing
    // wrapper inside the range makes the body foreign, an existing
    // wrapping-form source around it clears `wrap_source`, so neither is
    // wrapped twice.
    let needs_wrap = wrap_source && run.is_some_and(|r| r.hyperlink_source.is_some());
    let replace = match run {
        Some(r) => (model_text(r) != source_text || needs_wrap) && !body.foreign,
        None => false,
    };
    if replace {
        let r = run.expect("checked above");
        let text = model_text(r);
        match (&r.hyperlink_source, wrap_source) {
            (Some(src), true) => {
                emit_start_with_attrs(
                    writer,
                    "HyperlinkTextSource",
                    &[
                        ("Self", src.clone()),
                        ("Name", src.rsplit('/').next().unwrap_or(src).to_string()),
                        ("Hidden", "false".to_string()),
                    ],
                )?;
                write_run_content(writer, &text)?;
                writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                    "HyperlinkTextSource",
                )))?;
            }
            _ => write_run_content(writer, &text)?,
        }
        if ends_with_br && !hold_br {
            writer.write_event(Event::Empty(BytesStart::new("Br")))?;
        }
    } else {
        for ev in body.events.drain(..) {
            writer.write_event(ev)?;
        }
    }
    *prev_br = ends_with_br;
    body.active = false;
    body.in_content = false;
    body.ends_with_br = false;
    body.events.clear();
    Ok(())
}

/// Serialise a run's text body back into IDML `<Content>` / `<Br/>`
/// structure (a tab stays a literal U+0009 inside `<Content>`, as
/// InDesign writes it), byte-for-byte matching `paged_gen`'s emitter so
/// a saved edit re-parses to the same model. Empty text emits an empty
/// `<Content></Content>` (the IDML form for a zero-length run).
pub(crate) fn write_run_content(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    text: &str,
) -> Result<(), quick_xml::Error> {
    fn flush(
        writer: &mut Writer<Cursor<Vec<u8>>>,
        buf: &mut String,
    ) -> Result<(), quick_xml::Error> {
        if !buf.is_empty() {
            writer.write_event(Event::Start(BytesStart::new("Content")))?;
            writer.write_event(Event::Text(BytesText::new(buf)))?;
            writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Content")))?;
            buf.clear();
        }
        Ok(())
    }
    if text.is_empty() {
        writer.write_event(Event::Start(BytesStart::new("Content")))?;
        writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Content")))?;
        return Ok(());
    }
    let mut buf = String::new();
    // A tab is a literal U+0009 inside `<Content>` — that is how InDesign
    // writes one (measured 2026-09-06: `<Content>Largest single run
    // (Q4)\t2,390 copies</Content>`). The `<Tab/>` element this used to
    // emit was a private spelling InDesign ignores, so every tab in the
    // annual's manuscript composed as nothing and its columns glued
    // together ("Largest single run (Q4)2,390 copies"). The reader keeps
    // accepting `<Tab/>` for parts written before this.
    for ch in text.chars() {
        match ch {
            '\n' => {
                flush(writer, &mut buf)?;
                writer.write_event(Event::Empty(BytesStart::new("Br")))?;
            }
            _ => buf.push(ch),
        }
    }
    flush(writer, &mut buf)?;
    Ok(())
}

/// The model paragraph the `<ParagraphStyleRange>` at `pos` produced,
/// within `scope` (the story's paragraph list, or a cell's).
/// The story part written from the model alone, keeping the source's
/// `Self`, `DOMVersion` and inline text-destination markers.
fn fresh_story_part(
    original: &[u8],
    story: &Story,
    host_width: Option<f32>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let mut r = Reader::from_reader(original);
    r.config_mut().trim_text(false);
    let mut b = Vec::new();
    let mut self_id: Option<String> = None;
    let mut dom_version: Option<String> = None;
    loop {
        match r.read_event_into(&mut b)? {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => match e.name().as_ref() {
                b"idPkg:Story" => dom_version = attr_value(e, b"DOMVersion"),
                b"Story" => {
                    self_id = attr_value(e, b"Self");
                    break;
                }
                _ => {}
            },
            _ => {}
        }
        b.clear();
    }
    let anchors = idml_import::story_text_anchors(original).unwrap_or_default();
    crate::emit::story_part(
        self_id.as_deref().unwrap_or("Story_u0"),
        story,
        dom_version.as_deref().unwrap_or("20.0"),
        &anchors,
        host_width,
    )
}

pub(crate) fn resolve_paragraph<'a>(
    provenance: &idml_import::StoryProvenance,
    pos: u64,
    scope: &'a [Paragraph],
) -> Option<&'a Paragraph> {
    provenance
        .paragraph_at(pos)
        .and_then(|i| model_paragraph_index(scope, i))
        .and_then(|i| scope.get(i))
}

/// A paragraph the parser DROPS on read (neither a run nor a table).
fn paragraph_is_empty(p: &Paragraph) -> bool {
    p.runs.is_empty() && p.table.is_none()
}

/// The MODEL index of the parser's `parse_index`-th paragraph.
///
/// The provenance map counts in PARSE space: the parser drops a
/// paragraph with neither a run nor a table, so its indices skip them.
/// A model that came from the native part (`document.pgm`) still holds
/// such paragraphs — the annual's DOCX-lowered story kept an empty one
/// in the middle, and every source range after it was patched against
/// the paragraph BEFORE the one it came from: the whole tail of the
/// story shifted by one on save, and the two hyperlink sources in it
/// never met their runs. Counting only the paragraphs the parser would
/// have kept puts the two spaces back in step.
fn model_paragraph_index(scope: &[Paragraph], parse_index: usize) -> Option<usize> {
    // The parser keeps every range that had a child (an empty line is a
    // paragraph), so parse space IS model space.
    (parse_index < scope.len()).then_some(parse_index)
}

/// The model run the `<CharacterStyleRange>` at `pos` produced inside
/// `para`, plus whether the parser SPLIT that one element across several
/// runs (see [`idml_import::RunSlot`]).
fn resolve_run<'a>(
    provenance: &idml_import::StoryProvenance,
    pos: u64,
    para: Option<&'a Paragraph>,
    offset: usize,
) -> (Option<&'a CharacterRun>, bool, Option<usize>) {
    let Some(slot) = provenance.run_at(pos) else {
        return (None, false, None);
    };
    let first = slot.first + offset;
    let run = para.and_then(|p| p.runs.get(first));
    (run, slot.count > 1, Some(first))
}

/// The model runs a source range was SPLIT into after parse — a
/// hyperlink, a style or a placeholder applied to PART of a run
/// (`paged_mutate::split_run_at`) leaves the model with several runs
/// where the source has one element. The provenance index still names
/// the first piece; the pieces after it have no source element of their
/// own, so without this they were simply never written: an
/// `InsertHyperlink` over "Hello" in "Hello world" saved "Hello".
///
/// Detected by TEXT: the source range's text must equal the first piece
/// followed by the next `k` runs exactly. Returns those `k` extra run
/// indices (empty when the range is not a split, or the pieces were also
/// edited — then the old single-run comparison applies).
fn split_tail(
    para: Option<&Paragraph>,
    first: Option<usize>,
    source_text: &str,
    prev_br: bool,
) -> Vec<usize> {
    let (Some(para), Some(first)) = (para, first) else {
        return Vec::new();
    };
    let Some(head) = para.runs.get(first) else {
        return Vec::new();
    };
    let head_text = if prev_br {
        head.text.strip_prefix('\n').unwrap_or(&head.text)
    } else {
        head.text.as_str()
    };
    if head_text == source_text || !source_text.starts_with(head_text) {
        return Vec::new();
    }
    let mut acc = head_text.to_string();
    let mut extras = Vec::new();
    for (i, r) in para.runs.iter().enumerate().skip(first + 1) {
        acc.push_str(&r.text);
        extras.push(i);
        if acc == source_text {
            return extras;
        }
        if !source_text.starts_with(&acc) {
            break;
        }
    }
    Vec::new()
}

/// One `<CharacterStyleRange>` for a split-off model run (see
/// [`split_tail`]), in the minted-story vocabulary.
fn write_split_run(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    r: &CharacterRun,
    wrap_source: bool,
    mark: bool,
) -> Result<(), quick_xml::Error> {
    emit_start_with_attrs(
        writer,
        "CharacterStyleRange",
        &crate::emit::character_run_attrs(r),
    )?;
    crate::emit::emit_applied_font(writer, &r.font)?;
    match (&r.hyperlink_source, wrap_source) {
        (Some(src), true) => {
            emit_start_with_attrs(
                writer,
                "HyperlinkTextSource",
                &[
                    ("Self", src.clone()),
                    ("Name", src.rsplit('/').next().unwrap_or(src).to_string()),
                    ("Hidden", "false".to_string()),
                ],
            )?;
            write_run_content(writer, &r.text)?;
            writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                "HyperlinkTextSource",
            )))?;
        }
        _ => write_run_content(writer, &r.text)?,
    }
    if mark {
        writer.write_event(Event::Empty(BytesStart::new("Br")))?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
        "CharacterStyleRange",
    )))?;
    Ok(())
}

fn patch_paragraph_range(
    e: &BytesStart,
    para: Option<&idml_import::Paragraph>,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let Some(para) = para else {
        return Ok(e.clone().into_owned());
    };
    let extras = crate::emit::paragraph_attrs(para);
    let start = patch_start(e, |k, raw| paragraph_attr_patch(k, raw, para), &extras)?;
    Ok(start.into_owned())
}

/// An integer attribute whose absence means `0` (`DropCapCharacters`,
/// `DropCapLines`, `DropCapDetail`): a source spelling of the model's
/// value keeps its bytes, a zero drops the attribute, anything else is
/// rewritten.
pub(crate) fn preserving_int_patch(raw: Option<&str>, v: i64) -> Patch {
    if raw.and_then(|s| s.trim().parse::<i64>().ok()) == Some(v) {
        return Patch::Keep;
    }
    if v == 0 {
        Patch::Remove
    } else {
        Patch::Set(v.to_string())
    }
}

pub(crate) fn opt_u32_patch(raw: Option<&str>, v: Option<u32>) -> Patch {
    match v {
        Some(n) => {
            if raw.and_then(|s| s.trim().parse::<u32>().ok()) == Some(n) {
                Patch::Keep
            } else {
                Patch::Set(n.to_string())
            }
        }
        None => Patch::Remove,
    }
}

/// `AppliedNumberingList`: the parser reads InDesign's several "no
/// list" spellings (`n`, `NumberingList/n`, `…[No numbering list]`) as
/// `None`, so a `None` model keeps such a source spelling rather than
/// deleting it, and only a real list name that the model dropped is
/// rewritten to `n`.
pub(crate) fn numbering_list_patch(raw: Option<&str>, v: &Option<String>) -> Patch {
    match v {
        Some(s) => Patch::Set(s.clone()),
        None => match raw {
            Some("n") | Some("NumberingList/n") | Some("") => Patch::Keep,
            Some(r) if r.ends_with("[No numbering list]") => Patch::Keep,
            Some(_) => Patch::Set("n".to_string()),
            None => Patch::Remove,
        },
    }
}

pub(crate) fn paragraph_rule_patch(
    key: &[u8],
    raw: Option<&str>,
    keys: &crate::emit::RuleKeys,
    rule: &idml_import::ParagraphRule,
) -> Option<Patch> {
    let k = std::str::from_utf8(key).ok()?;
    if k == keys.on {
        Some(opt_bool_patch(rule.on))
    } else if k == keys.color {
        Some(opt_string_patch(&rule.color))
    } else if k == keys.tint {
        Some(preserving_f32_patch(raw, rule.tint))
    } else if k == keys.weight {
        Some(preserving_f32_patch(raw, rule.weight))
    } else if k == keys.offset {
        Some(preserving_f32_patch(raw, rule.offset))
    } else if k == keys.left_indent {
        Some(preserving_f32_patch(raw, rule.left_indent))
    } else if k == keys.right_indent {
        Some(preserving_f32_patch(raw, rule.right_indent))
    } else if k == keys.width {
        Some(opt_string_patch(&rule.width))
    } else {
        None
    }
}

/// The `<ParagraphStyleRange>` attributes the model owns — every
/// paragraph override InDesign reads from the range (the key set
/// `emit::paragraph_attrs` writes). A key the model does not own passes
/// through verbatim (`DropcapDetail` in InDesign's own lowercase-c
/// spelling, …).
pub(crate) fn paragraph_attr_patch(
    key: &[u8],
    raw: &[u8],
    p: &idml_import::Paragraph,
) -> Option<Patch> {
    let raw = std::str::from_utf8(raw).ok();
    match key {
        b"AppliedParagraphStyle" => Some(opt_string_patch(&p.paragraph_style)),
        b"Justification" => Some(match p.justification {
            Some(j) if raw == Some(j.as_idml()) => Patch::Keep,
            Some(j) => Patch::Set(j.as_idml().to_string()),
            None => Patch::Remove,
        }),
        b"FirstLineIndent" => Some(preserving_f32_patch(raw, p.first_line_indent)),
        b"LeftIndent" => Some(preserving_f32_patch(raw, p.left_indent)),
        b"RightIndent" => Some(preserving_f32_patch(raw, p.right_indent)),
        b"SpaceBefore" => Some(preserving_f32_patch(raw, p.space_before)),
        b"SpaceAfter" => Some(preserving_f32_patch(raw, p.space_after)),
        b"DropCapCharacters" => Some(preserving_int_patch(raw, p.drop_cap_characters as i64)),
        b"DropCapLines" => Some(preserving_int_patch(raw, p.drop_cap_lines as i64)),
        b"DropCapDetail" => Some(preserving_int_patch(raw, p.drop_cap_detail as i64)),
        b"Hyphenation" => Some(opt_bool_patch(p.hyphenation)),
        // The rest of InDesign's hyphenation panel. These used to fall
        // through to the pass-through arm; now that the importer reads
        // them, the model owns them and a mutation has to be able to
        // rewrite them.
        b"HyphenationZone" => Some(preserving_f32_patch(raw, p.hyphenation_zone)),
        b"HyphenateCapitalizedWords" => Some(opt_bool_patch(p.hyphenate_capitalized_words)),
        b"HyphenateLastWord" => Some(opt_bool_patch(p.hyphenate_last_word)),
        b"HyphenateAcrossColumns" => Some(opt_bool_patch(p.hyphenate_across_columns)),
        b"HyphenateAfterFirst" => Some(opt_u32_patch(raw, p.hyphenate_after_first)),
        b"HyphenateBeforeLast" => Some(opt_u32_patch(raw, p.hyphenate_before_last)),
        b"HyphenateWordsLongerThan" => Some(opt_u32_patch(raw, p.hyphenate_words_longer_than)),
        b"HyphenateLadderLimit" => Some(opt_u32_patch(raw, p.hyphenate_ladder_limit)),
        b"HyphenWeight" => Some(opt_u32_patch(raw, p.hyphen_weight)),
        b"KeepLinesTogether" => Some(opt_bool_patch(p.keep_lines_together)),
        b"KeepWithNext" => Some(opt_u32_patch(raw, p.keep_with_next)),
        b"BulletsAndNumberingListType" => Some(opt_string_patch(&p.bullets_list_type)),
        b"NumberingFormat" => Some(opt_string_patch(&p.numbering_format)),
        b"AppliedNumberingList" => Some(numbering_list_patch(raw, &p.applied_numbering_list)),
        b"KinsokuSet" => Some(opt_string_patch(&p.kinsoku_set)),
        _ => paragraph_rule_patch(key, raw, &crate::emit::RULE_ABOVE, &p.rule_above)
            .or_else(|| paragraph_rule_patch(key, raw, &crate::emit::RULE_BELOW, &p.rule_below)),
    }
}

fn patch_character_range(
    e: &BytesStart,
    run: Option<&CharacterRun>,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let Some(run) = run else {
        // No model run aligns with this range — pass through verbatim.
        return Ok(e.clone().into_owned());
    };
    let r = run.clone();
    let extras = character_extras(&r);
    let start = patch_start(e, |k, raw| character_attr_patch(k, raw, &r), &extras)?;
    Ok(start.into_owned())
}

/// Patch decision for one `<CharacterStyleRange>` attribute. Covers the
/// character paths the mutation surface writes.
///
/// # Why the numbers use the PRESERVING patch
///
/// Every numeric here is read straight off THIS element's attribute with
/// a plain `parse::<f32>()` (see `idml_import::story`): no composition,
/// no inheritance from the applied paragraph / character style, no unit
/// conversion. So the on-disk spelling is the ONLY source of the model
/// number, and re-emitting it through `format_f32` (4 decimals) is a
/// pure loss whenever the source carried more — which InDesign's
/// `StrokeWeight="0.9921259842519686"` and
/// `BaselineShift="4.097337047350078"` routinely do.
///
/// That makes this the SAME case as `StrokeWeight` on a page item —
/// the simple [`preserving_f32_patch`] — and NOT the `ItemTransform`
/// case, which needed a forward-replay predicate because a group
/// member's `item_transform` is stored COMPOSED with its ancestors' and
/// has to be de-composed before the on-disk spelling can be compared.
/// Nothing on a `CharacterStyleRange` is stored derived, so there is
/// nothing to replay: comparing the model number against the source
/// spelling's own parse is exact.
///
/// `Leading` has one wrinkle — it can also arrive as a
/// `<Properties><Leading>` child, which overrides the attribute. The
/// preserving rule handles that correctly by construction: when the two
/// agree it keeps the source bytes, when they disagree the model value
/// (the Properties one) is written, exactly as before.
fn character_attr_patch(key: &[u8], raw: &[u8], r: &CharacterRun) -> Option<Patch> {
    let raw = std::str::from_utf8(raw).ok();
    match key {
        b"AppliedCharacterStyle" => Some(opt_string_patch(&r.character_style)),
        b"AppliedFont" => Some(opt_string_patch(&r.font)),
        b"FontStyle" => Some(opt_string_patch(&r.font_style)),
        b"PointSize" => Some(preserving_f32_patch(raw, r.point_size)),
        b"FillColor" => Some(opt_string_patch(&r.fill_color)),
        // NOT the plain preserving-f32 rule: `FillTint="-1"` parses to
        // `None` (IDML's "no tint override" sentinel,
        // `idml_import::parse_tint`), and spelling that `None` as
        // "delete the attribute" rewrote 156 corpus stories — 288
        // attributes — on a save that changed nothing. See
        // [`preserving_tint_patch`].
        b"FillTint" => Some(preserving_tint_patch(raw, r.fill_tint)),
        b"StrokeColor" => Some(opt_string_patch(&r.stroke_color)),
        b"StrokeWeight" => Some(preserving_f32_patch(raw, r.stroke_weight)),
        b"Leading" => Some(preserving_f32_patch(raw, r.leading)),
        b"Tracking" => Some(preserving_f32_patch(raw, r.tracking)),
        b"BaselineShift" => Some(preserving_f32_patch(raw, r.baseline_shift)),
        b"HorizontalScale" => Some(preserving_f32_patch(raw, r.horizontal_scale)),
        b"VerticalScale" => Some(preserving_f32_patch(raw, r.vertical_scale)),
        b"Skew" => Some(preserving_f32_patch(raw, r.skew)),
        b"Capitalization" => Some(opt_string_patch(&r.capitalization)),
        b"Position" => Some(opt_string_patch(&r.position)),
        b"KerningMethod" => Some(opt_string_patch(&r.kerning_method)),
        b"AppliedLanguage" => Some(opt_string_patch(&r.applied_language)),
        b"Underline" => Some(opt_bool_patch(r.underline)),
        b"StrikeThru" => Some(opt_bool_patch(r.strikethru)),
        b"Ligatures" => Some(opt_bool_patch(r.ligatures_on)),
        _ => None,
    }
}

/// Newly-set character attributes to append when absent from the source.
/// Only the high-frequency authoring fields are appended; the rest patch
/// in place when present but aren't invented (keeps unmutated round-trips
/// byte-identical and avoids spraying defaults).
fn character_extras(r: &CharacterRun) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(s) = &r.fill_color {
        out.push(("FillColor", s.clone()));
    }
    if let Some(sz) = r.point_size {
        out.push(("PointSize", format_f32(sz)));
    }
    if let Some(s) = &r.character_style {
        out.push(("AppliedCharacterStyle", s.clone()));
    }
    out
}

pub(crate) fn opt_bool_patch(v: Option<bool>) -> Patch {
    match v {
        Some(b) => Patch::Set(b.to_string()),
        None => Patch::Remove,
    }
}

/// A `<KeyValuePair Key=… Value=…>`'s pair, decoded the way the PARSER
/// decodes it (`util::attr_unescaped`: XML attribute-value
/// normalization, so `&quot;` → `"` and `&#xa;` → a newline). Read
/// through the same normalization on both sides, a source label and a
/// model label compare like for like — which is what lets an unedited
/// `<Label>` keep its source bytes.
///
/// `None` when either attribute is missing or undecodable; such a pair
/// can't be compared, so the label is not treated as unchanged.
fn key_value_pair(e: &BytesStart) -> Option<(String, String)> {
    fn normalized(e: &BytesStart, key: &[u8]) -> Option<String> {
        e.attributes()
            .flatten()
            .find(|a| a.key.as_ref() == key)
            .and_then(|a| {
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
    }
    Some((normalized(e, b"Key")?, normalized(e, b"Value")?))
}

/// Read an attribute's decoded value off a start tag.
pub(crate) fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|Attribute { value, .. }| std::str::from_utf8(&value).ok().map(|s| s.to_string()))
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

/// How many `<CrossReferenceSource>` elements in a story part name no
/// `AppliedFormat` — the ones InDesign drops (see
/// [`inject_story_navigation`]).
pub(crate) fn unformatted_xref_sources(story: &[u8]) -> Result<usize, quick_xml::Error> {
    let mut reader = Reader::from_reader(story);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut n = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e)
                if e.name().as_ref() == b"CrossReferenceSource"
                    && attr_value(e, b"AppliedFormat").is_none() =>
            {
                n += 1;
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_childless_range_is_dropped_and_the_appended_table_follows_the_caption() {
        // An older emitter wrote the table paragraph as an empty range;
        // the appended table used to land after a mark injected into
        // that range — an empty line above every captioned table.
        let src = br#"<idPkg:Story xmlns:idPkg="x"><Story Self="s"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Caption"><CharacterStyleRange><Content>Caption</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;
        let mut story = idml_import::parse_story(src).unwrap();
        assert_eq!(story.paragraphs.len(), 1);
        story.paragraphs.push(Paragraph {
            table: Some(idml_import::Table {
                self_id: Some("t".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        let out = String::from_utf8(rewrite_story(src, &story).unwrap()).unwrap();
        assert!(
            out.contains(r#"<Content>Caption</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Table Self="t""#),
            "{out}"
        );
        assert!(
            !out.contains("<ParagraphStyleRange></ParagraphStyleRange>"),
            "{out}"
        );
    }

    #[test]
    fn a_story_with_fewer_paragraphs_than_its_part_is_written_fresh() {
        // The part carries an empty range the model never held (an
        // older parser dropped it): InDesign showed a blank line the
        // canvas did not. The model is the truth; the part is rewritten
        // from it, Self and DOMVersion kept.
        let src = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><Story Self="Story_u9"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Caption"><CharacterStyleRange><Content>Caption</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange><Content>Body</Content></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;
        let mut story = idml_import::parse_story(src).unwrap();
        assert_eq!(story.paragraphs.len(), 3, "the empty range is a paragraph");
        assert_eq!(
            rewrite_story(src, &story).unwrap(),
            src.to_vec(),
            "unchanged: byte-identical"
        );
        story.paragraphs.remove(1);
        let out = String::from_utf8(rewrite_story(src, &story).unwrap()).unwrap();
        assert!(
            out.contains(r#"<Story Self="Story_u9"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Caption"><CharacterStyleRange><Content>Caption</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange><Content>Body</Content></CharacterStyleRange></ParagraphStyleRange></Story>"#),
            "{out}"
        );
        assert!(out.contains(r#"DOMVersion="20.0""#), "{out}");
    }

    #[test]
    fn paragraph_overrides_are_patched_onto_the_range() {
        // InDesign reads a paragraph's local overrides from these range
        // attributes; the patch lane used to own only the style
        // reference, so an override authored by mutation was saved
        // nowhere (the annual's spacing battery, 2026-09-06).
        let src = br#"<idPkg:Story xmlns:idPkg="x"><Story Self="s"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body" Justification="CenterAlign" SpaceAfter="6" HyphenationZone="36"><CharacterStyleRange><Content>a</Content></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;
        let mut story = idml_import::parse_story(src).unwrap();
        assert_eq!(
            std::str::from_utf8(&rewrite_story(src, &story).unwrap()).unwrap(),
            std::str::from_utf8(src).unwrap(),
            "unmutated: byte-identical"
        );
        let p = &mut story.paragraphs[0];
        p.justification = Some(idml_import::Justification::LeftJustified);
        p.space_after = None;
        p.space_before = Some(13.0);
        p.first_line_indent = Some(26.0);
        p.left_indent = Some(18.0);
        p.right_indent = Some(18.0);
        p.keep_with_next = Some(1);
        p.keep_lines_together = Some(true);
        p.hyphenation = Some(false);
        p.drop_cap_characters = 1;
        p.drop_cap_lines = 3;
        p.rule_above.on = Some(true);
        p.rule_above.weight = Some(1.5);
        p.rule_above.color = Some("Color/Black".into());
        let out = String::from_utf8(rewrite_story(src, &story).unwrap()).unwrap();
        for needle in [
            r#"Justification="LeftJustified""#,
            r#"SpaceBefore="13""#,
            r#"FirstLineIndent="26""#,
            r#"LeftIndent="18""#,
            r#"RightIndent="18""#,
            r#"KeepWithNext="1""#,
            r#"KeepLinesTogether="true""#,
            r#"Hyphenation="false""#,
            r#"DropCapCharacters="1""#,
            r#"DropCapLines="3""#,
            r#"RuleAbove="true""#,
            r#"RuleAboveColor="Color/Black""#,
            r#"RuleAboveLineWeight="1.5""#,
            r#"HyphenationZone="36""#,
        ] {
            assert!(out.contains(needle), "{needle} missing: {out}");
        }
        assert!(
            !out.contains("SpaceAfter"),
            "a cleared override is dropped: {out}"
        );
    }

    /// The `f32` you get from parsing a corpus spelling.
    ///
    /// The corner radii InDesign writes carry f64-grade precision
    /// (`99.21259842519686`), which an `f32` cannot hold — so comparing
    /// against the literal trips clippy's `excessive_precision`, and
    /// hand-truncating it to `99.212_6` would hide WHICH corpus value is
    /// under test. Parsing the real spelling keeps the evidence visible
    /// and the comparison exact. (That the SOURCE STRING survives
    /// unrounded is a separate guarantee, pinned by the byte-identity
    /// test.)
    fn f(spelling: &str) -> Option<f32> {
        Some(spelling.parse::<f32>().expect("corpus float"))
    }

    /// A spread carrying one polygon with the corner vocabulary spelled
    /// the way InDesign writes it: long floats and the `BevelCorner`
    /// token (which the model's enum normalises to `Bevel`).
    const POLY_SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<Spread Self="s"><Polygon Self="p" GeometricBounds="0 0 100 200" ItemTransform="1 0 0 1 0 0" CornerOption="BevelCorner" CornerRadius="44.51279527491718" TopLeftCornerOption="BevelCorner" TopLeftCornerRadius="44.51279527491718" TopRightCornerRadius="44.51279527491718" FillColor="Color/Black"/></Spread>
</idPkg:Spread>"#;

    fn parsed() -> idml_import::Spread {
        idml_import::parse_spread(POLY_SPREAD).expect("parse")
    }

    /// B-23 — an UNMUTATED polygon round-trips byte-identically even
    /// though every corner value now flows through the patch path.
    /// `format_f32` rounds to 4 decimals and the option enum loses the
    /// source spelling, so the preserving rule is load-bearing: without
    /// it, `44.51279527491718` would come back as `44.5128` and
    /// `BevelCorner` as `BeveledCorner`.
    #[test]
    fn b23_unmutated_polygon_corner_attrs_round_trip_byte_identically() {
        let out = rewrite_spread(POLY_SPREAD, &parsed()).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(POLY_SPREAD),
            "unmutated corner attributes must reproduce their on-disk bytes"
        );
    }

    /// A mutated corner value patches IN PLACE — same attribute
    /// position, only the value changes; every other attribute is
    /// untouched.
    #[test]
    fn b23_mutated_polygon_corner_attrs_patch_in_place() {
        let mut spread = parsed();
        spread.polygons[0].corners[0].radius = Some(12.5);
        spread.polygons[0].corners[0].option = Some(idml_import::CornerOption::Rounded);
        let out = rewrite_spread(POLY_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(r#"TopLeftCornerOption="RoundedCorner""#), "{s}");
        assert!(s.contains(r#"TopLeftCornerRadius="12.5""#), "{s}");
        // Untouched neighbours keep their exact source spelling.
        assert!(
            s.contains(r#"TopRightCornerRadius="44.51279527491718""#),
            "{s}"
        );
        assert!(s.contains(r#"CornerOption="BevelCorner""#), "{s}");
        assert!(s.contains(r#"FillColor="Color/Black""#), "{s}");
    }

    /// Clearing a corner value drops the attribute (restoring the IDML
    /// implicit default) rather than writing an empty string.
    #[test]
    fn b23_cleared_polygon_corner_attr_is_removed() {
        let mut spread = parsed();
        spread.polygons[0].corners[0].option = None;
        spread.polygons[0].corner_radius = None;
        let out = rewrite_spread(POLY_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("TopLeftCornerOption="), "{s}");
        assert!(!s.contains(r#" CornerRadius="#), "{s}");
        assert!(
            s.contains(r#"TopLeftCornerRadius="44.51279527491718""#),
            "{s}"
        );
    }

    /// A corner written onto a frame whose source element never had the
    /// attribute is APPENDED (the `extras` lane), not silently dropped.
    #[test]
    fn b23_newly_set_polygon_corner_attr_is_appended() {
        let mut spread = parsed();
        spread.polygons[0].corners[3].option = Some(idml_import::CornerOption::Inverse);
        spread.polygons[0].corners[3].radius = Some(9.0);
        let out = rewrite_spread(POLY_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"BottomLeftCornerOption="InverseRoundedCorner""#),
            "{s}"
        );
        assert!(s.contains(r#"BottomLeftCornerRadius="9""#), "{s}");
    }

    // -----------------------------------------------------------------
    // C-19 — scene-created groups + inserted-item z-order
    // -----------------------------------------------------------------

    /// A top-level rectangle, a top-level polygon, and a `<Group>` (with
    /// its own `ItemTransform`) wrapping one rectangle. Enough shape to
    /// exercise: the byte-identity invariant, source items joining a new
    /// group, a source group gaining a member, and the transform re-base.
    const GROUP_SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<Spread Self="s"><Rectangle Self="r1" ItemTransform="1 0 0 1 10 10" GeometricBounds="0 0 50 50" FillColor="Color/Black"/><Polygon Self="p1" ItemTransform="1 0 0 1 20 20" GeometricBounds="0 0 30 30" FillColor="Color/Paper"/><Group Self="g1" ItemTransform="1 0 0 1 100 0"><Rectangle Self="r2" ItemTransform="1 0 0 1 5 5" GeometricBounds="0 0 20 20" FillColor="Color/Paper"/></Group></Spread>
</idPkg:Spread>"#;

    fn grouped() -> idml_import::Spread {
        idml_import::parse_spread(GROUP_SPREAD).expect("parse")
    }

    /// Clone the fixture polygon into a model-only ("inserted") one.
    fn inserted_polygon(
        spread: &idml_import::Spread,
        self_id: &str,
        item_transform: Option<[f32; 6]>,
    ) -> idml_import::Polygon {
        let mut p = spread.polygons[0].clone();
        p.self_id = Some(self_id.to_string());
        p.item_transform = item_transform;
        p
    }

    fn new_group(
        self_id: &str,
        members: Vec<idml_import::FrameRef>,
        item_transform: Option<[f32; 6]>,
    ) -> idml_import::Group {
        idml_import::Group {
            self_id: Some(self_id.to_string()),
            members,
            transparency: Default::default(),
            item_transform,
            corner_radius: None,
            corner_option: None,
            corners: Default::default(),
        }
    }

    /// Count non-overlapping occurrences of `needle` in `hay`.
    fn count(hay: &str, needle: &str) -> usize {
        hay.matches(needle).count()
    }

    /// InDesign keeps a group's members on the GROUP's layer, and a
    /// `<Group>` without `ItemLayer` lands on the document's first layer
    /// (measured 2026-09-06: the annual's traced polygons, every one on
    /// "Content", sat on "Grid" underneath the page background). A
    /// minted group therefore names its members' layer.
    #[test]
    fn a_minted_group_sits_on_its_members_layer() {
        let mut spread = grouped();
        let base = spread.polygons[0].clone();
        for id in ["u1", "u2"] {
            let mut p = base.clone();
            p.self_id = Some(id.to_string());
            p.item_layer = Some("uContent".to_string());
            spread.polygons.push(p);
        }
        let members = vec![
            idml_import::FrameRef::Polygon(1),
            idml_import::FrameRef::Polygon(2),
        ];
        spread.groups.push(new_group("gtrace", members, None));
        let gref = idml_import::FrameRef::Group(spread.groups.len() - 1);
        spread.frames_in_order.push(gref);

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<Group Self="gtrace" ItemTransform="1 0 0 1 0 0" ItemLayer="uContent">"#),
            "{s}"
        );
        // Members without a layer leave the group without one too.
        let mut bare = grouped();
        let mut p = bare.polygons[0].clone();
        p.self_id = Some("u3".to_string());
        p.item_layer = None;
        bare.polygons.push(p);
        bare.groups.push(new_group(
            "gbare",
            vec![idml_import::FrameRef::Polygon(1)],
            None,
        ));
        let gref = idml_import::FrameRef::Group(bare.groups.len() - 1);
        bare.frames_in_order.push(gref);
        let out = rewrite_spread(GROUP_SPREAD, &bare).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<Group Self="gbare" ItemTransform="1 0 0 1 0 0">"#),
            "{s}"
        );
    }

    /// A tab is a literal U+0009 inside `<Content>` — InDesign's own
    /// spelling (measured 2026-09-06). The `<Tab/>` element we used to
    /// write was a private form InDesign ignores, gluing "Largest single
    /// run (Q4)" to "2,390 copies" on the annual's page 117.
    #[test]
    fn a_tab_is_a_literal_character_inside_content() {
        let mut w = Writer::new(Cursor::new(Vec::new()));
        write_run_content(&mut w, "Largest single run (Q4)\t2,390 copies\nSpoilage")
            .expect("write");
        let s = String::from_utf8(w.into_inner().into_inner()).unwrap();
        assert_eq!(
            s,
            "<Content>Largest single run (Q4)\t2,390 copies</Content><Br/><Content>Spoilage</Content>"
        );
    }

    /// An EXISTING `<Group>` the source wrote without `ItemLayer` (our
    /// own earlier exports) gains its members' layer on the way out;
    /// one the source names keeps its bytes.
    #[test]
    fn an_existing_group_without_a_layer_takes_its_members_layer() {
        let mut spread = grouped();
        // The fixture's g1 wraps r2; give r2 a layer in the model.
        spread.rectangles[1].item_layer = Some("uContent".to_string());
        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<Group Self="g1" ItemTransform="1 0 0 1 100 0" ItemLayer="uContent">"#),
            "{s}"
        );
        let named = String::from_utf8_lossy(GROUP_SPREAD).replace(
            r#"<Group Self="g1" ItemTransform="1 0 0 1 100 0">"#,
            r#"<Group Self="g1" ItemTransform="1 0 0 1 100 0" ItemLayer="uGrid">"#,
        );
        let out = rewrite_spread(named.as_bytes(), &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<Group Self="g1" ItemTransform="1 0 0 1 100 0" ItemLayer="uGrid">"#),
            "{s}"
        );
    }

    /// InDesign draws an `<Oval>` by its PATH. Ours used to spell the
    /// four-corner box (a rectangle to InDesign — the annual's blend
    /// plates); now an oval carries InDesign's own ellipse: midpoint
    /// anchors bottom, right, top, left with κ·r handles (measured
    /// 2026-09-06). An existing box-spelled oval is re-spelled; one
    /// carrying InDesign's ellipse at the same bounds keeps its bytes.
    #[test]
    fn an_oval_is_spelled_as_indesign_spells_it() {
        const BOX_OVAL: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<Spread Self="s"><Oval Self="o1" ItemTransform="1 0 0 1 0 0" FillColor="Color/Black"><Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="100 200" LeftDirection="100 200" RightDirection="100 200"/><PathPointType Anchor="100 400" LeftDirection="100 400" RightDirection="100 400"/><PathPointType Anchor="300 400" LeftDirection="300 400" RightDirection="300 400"/><PathPointType Anchor="300 200" LeftDirection="300 200" RightDirection="300 200"/></PathPointArray></GeometryPathType></PathGeometry></Properties></Oval></Spread>
</idPkg:Spread>"#;
        let spread = idml_import::parse_spread(BOX_OVAL).expect("parse");
        let out = rewrite_spread(BOX_OVAL, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        // 200 × 200 at (100, 200): centre (200, 300), κ·r = 55.2285 (four decimals on disk).
        assert!(
            s.contains(r#"<PathPointType Anchor="200 400" LeftDirection="144.7715 400" RightDirection="255.2285 400"/>"#),
            "{s}"
        );
        assert!(
            s.contains(r#"<PathPointType Anchor="300 300" LeftDirection="300 355.2285" RightDirection="300 244.7715"/>"#),
            "{s}"
        );
        assert!(
            s.contains(r#"Anchor="200 200""#) && s.contains(r#"Anchor="100 300""#),
            "{s}"
        );
        assert_eq!(s.matches("<PathPointType ").count(), 4);

        // InDesign's own spelling round-trips byte-identically.
        let ellipse = String::from_utf8_lossy(BOX_OVAL).replace(
            r#"<PathPointType Anchor="100 200" LeftDirection="100 200" RightDirection="100 200"/><PathPointType Anchor="100 400" LeftDirection="100 400" RightDirection="100 400"/><PathPointType Anchor="300 400" LeftDirection="300 400" RightDirection="300 400"/><PathPointType Anchor="300 200" LeftDirection="300 200" RightDirection="300 200"/>"#,
            r#"<PathPointType Anchor="200 400" LeftDirection="144.7715 400" RightDirection="255.2285 400"/><PathPointType Anchor="300 300" LeftDirection="300 355.2285" RightDirection="300 244.7715"/><PathPointType Anchor="200 200" LeftDirection="255.2285 200" RightDirection="144.7715 200"/><PathPointType Anchor="100 300" LeftDirection="100 244.7715" RightDirection="100 355.2285"/>"#,
        );
        let spread = idml_import::parse_spread(ellipse.as_bytes()).expect("parse");
        let out = rewrite_spread(ellipse.as_bytes(), &spread).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            ellipse,
            "an InDesign ellipse keeps its bytes"
        );
    }

    /// `GradientFillAngle` / `GradientFillLength` (and the stroke pair)
    /// travel with the item in both lanes; a model that sets none leaves
    /// the source bytes alone.
    #[test]
    fn gradient_geometry_is_written_and_patched() {
        let mut spread = grouped();
        spread.rectangles[0].gradient_fill_angle = Some(15.0);
        spread.rectangles[0].gradient_fill_length = Some(432.0);
        spread.rectangles[0].gradient_stroke_angle = Some(90.0);
        let mut minted = spread.rectangles[0].clone();
        minted.self_id = Some("rg".to_string());
        spread.rectangles.push(minted);
        spread
            .frames_in_order
            .push(idml_import::FrameRef::Rectangle(2));
        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<Rectangle Self="r1" ItemTransform="1 0 0 1 10 10" GeometricBounds="0 0 50 50" FillColor="Color/Black" GradientFillAngle="15" GradientFillLength="432" GradientStrokeAngle="90"/>"#),
            "existing item: {s}"
        );
        assert!(
            s.contains(
                r#"GradientFillAngle="15" GradientFillLength="432" GradientStrokeAngle="90">"#
            ) && s.contains(r#"<Rectangle Self="rg""#),
            "minted item: {s}"
        );
        let out = rewrite_spread(GROUP_SPREAD, &grouped()).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(GROUP_SPREAD)
        );
    }

    /// THE PRIME INVARIANT. Every C-19 lane (group triage, the
    /// `</Group>` member flush, the z-table-driven insert order) runs on
    /// this document, and an unmutated model must still reproduce the
    /// source bytes exactly.
    #[test]
    fn c19_unmutated_group_spread_round_trips_byte_identically() {
        let out = rewrite_spread(GROUP_SPREAD, &grouped()).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(GROUP_SPREAD),
            "an unmutated document must stay byte-identical"
        );
    }

    /// A group the scene created over items the scene ALSO created (the
    /// paged.draw appearance bake) emits as a real `<Group>` with every
    /// member nested inside it. Before C-19 the wrapper AND all its
    /// members were dropped.
    #[test]
    fn c19_inserted_group_over_inserted_items_emits_a_real_group() {
        let mut spread = grouped();
        let base = spread.polygons[0].clone();
        for (i, id) in ["u1", "u2", "u3"].iter().enumerate() {
            let mut p = base.clone();
            p.self_id = Some((*id).to_string());
            p.item_transform = Some([1.0, 0.0, 0.0, 1.0, i as f32, 0.0]);
            spread.polygons.push(p);
        }
        let members = vec![
            idml_import::FrameRef::Polygon(1),
            idml_import::FrameRef::Polygon(2),
            idml_import::FrameRef::Polygon(3),
        ];
        spread.groups.push(new_group("gbake", members, None));
        let gref = idml_import::FrameRef::Group(spread.groups.len() - 1);
        spread.frames_in_order.push(gref);

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(r#"<Group Self="gbake""#), "{s}");
        // Every member is present exactly once, and INSIDE the wrapper.
        let open = s.find(r#"<Group Self="gbake""#).unwrap();
        let close = s[open..].find("</Group>").unwrap() + open;
        for id in ["u1", "u2", "u3"] {
            let needle = format!(r#"Self="{id}""#);
            assert_eq!(count(&s, &needle), 1, "{id} emitted once: {s}");
            let at = s.find(&needle).unwrap();
            assert!(at > open && at < close, "{id} must sit inside gbake: {s}");
        }
        // Members keep their creation order inside the wrapper.
        assert!(s.find(r#"Self="u1""#) < s.find(r#"Self="u2""#));
        assert!(s.find(r#"Self="u2""#) < s.find(r#"Self="u3""#));
    }

    /// A group created over items the SOURCE already carries: the
    /// members leave their original top-level slots and re-emit inside
    /// the new wrapper — each exactly once. Before C-19 the members
    /// stayed where they were and the wrapper vanished.
    #[test]
    fn c19_group_over_source_items_moves_them_inside_the_wrapper() {
        let mut spread = grouped();
        spread.groups.push(new_group(
            "gnew",
            vec![
                idml_import::FrameRef::Rectangle(0),
                idml_import::FrameRef::Polygon(0),
            ],
            None,
        ));
        let gref = idml_import::FrameRef::Group(spread.groups.len() - 1);
        // The z-table swaps the two members for the wrapper at the
        // earliest member's slot (what `CreateGroup` does).
        spread.frames_in_order = vec![gref, idml_import::FrameRef::Group(0)];

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(r#"<Group Self="gnew""#), "{s}");
        for id in ["r1", "p1"] {
            let needle = format!(r#"Self="{id}""#);
            assert_eq!(
                count(&s, &needle),
                1,
                "{id} must not be duplicated by the move: {s}"
            );
        }
        let open = s.find(r#"<Group Self="gnew""#).unwrap();
        assert!(s.find(r#"Self="r1""#).unwrap() > open, "{s}");
        assert!(s.find(r#"Self="p1""#).unwrap() > open, "{s}");
        // The untouched source group is still there, once.
        assert_eq!(count(&s, r#"Self="g1""#), 1, "{s}");
        assert_eq!(count(&s, r#"Self="r2""#), 1, "{s}");
    }

    /// A member's `item_transform` is stored COMPOSED into spread space,
    /// so emitting it inside a group with its own `ItemTransform` must
    /// re-base it: `on_disk = inverse(group) ∘ composed`.
    #[test]
    fn c19_group_members_rebase_against_the_group_transform() {
        let mut spread = grouped();
        // Composed = group(1 0 0 1 100 0) ∘ member(1 0 0 1 30 20).
        spread.polygons.push(inserted_polygon(
            &spread,
            "u9",
            Some([1.0, 0.0, 0.0, 1.0, 130.0, 20.0]),
        ));
        spread.groups.push(new_group(
            "gtx",
            vec![idml_import::FrameRef::Polygon(1)],
            Some([1.0, 0.0, 0.0, 1.0, 100.0, 0.0]),
        ));
        let gref = idml_import::FrameRef::Group(spread.groups.len() - 1);
        spread.frames_in_order.push(gref);

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<Group Self="gtx" ItemTransform="1 0 0 1 100 0">"#),
            "the group writes its own raw transform: {s}"
        );
        let member = &s[s.find(r#"Self="u9""#).unwrap()..];
        assert!(
            member.starts_with(r#"Self="u9" AppliedObjectStyle="ObjectStyle/$ID/[None]" ItemTransform="1 0 0 1 30 20""#),
            "member transform must be re-based into group space: {member}"
        );
    }

    /// An EXISTING source group that gained a member flushes it just
    /// before its own close tag, re-based against the group's transform.
    #[test]
    fn c19_source_group_that_gains_a_member_flushes_it_at_the_close() {
        let mut spread = grouped();
        // Composed = g1(1 0 0 1 100 0) ∘ member(1 0 0 1 7 3).
        spread.polygons.push(inserted_polygon(
            &spread,
            "u7",
            Some([1.0, 0.0, 0.0, 1.0, 107.0, 3.0]),
        ));
        spread.groups[0]
            .members
            .push(idml_import::FrameRef::Polygon(1));

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert_eq!(count(&s, r#"Self="u7""#), 1, "{s}");
        let member = s.find(r#"Self="u7""#).unwrap();
        let group_close = s.find("</Group>").unwrap();
        let r2 = s.find(r#"Self="r2""#).unwrap();
        assert!(
            r2 < member && member < group_close,
            "the new member lands after the existing one, inside g1: {s}"
        );
        assert!(
            s[member..].starts_with(r#"Self="u7" AppliedObjectStyle="ObjectStyle/$ID/[None]" ItemTransform="1 0 0 1 7 3""#),
            "{s}"
        );
    }

    /// C-19 sibling — inserted items emit in the model's Z-TABLE order,
    /// not its per-kind vec order. `InsertNode` takes a `position` into
    /// the kind vec independently of the z-slot, so a caller that
    /// inserts each new item at `position: 0` builds a REVERSED kind vec
    /// while `frames_in_order` stays right; the writer used to serialise
    /// that reversal into the file.
    #[test]
    fn c19_inserted_items_emit_in_z_table_order_not_kind_vec_order() {
        let mut spread = grouped();
        let base = spread.polygons[0].clone();
        // Kind vec ends up [p1, u3, u2, u1] — creation order reversed,
        // exactly what repeated `position: 0` inserts produce.
        for id in ["u3", "u2", "u1"] {
            let mut p = base.clone();
            p.self_id = Some(id.to_string());
            spread.polygons.push(p);
        }
        // The z-table carries the truth: u1 bottom-most of the three.
        spread
            .frames_in_order
            .extend([3, 2, 1].map(idml_import::FrameRef::Polygon));

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        let at = |id: &str| s.find(&format!(r#"Self="{id}""#)).expect("emitted");
        assert!(at("u1") < at("u2"), "u1 must paint before u2: {s}");
        assert!(at("u2") < at("u3"), "u2 must paint before u3: {s}");
    }

    /// The `write_new_*` lane used to emit only fill/stroke/weight, so a
    /// tint, an opacity, or a blend mode set on an item CREATED since
    /// load was silently lost on save (the patch lane only reaches items
    /// that exist in the source XML). All three now ride along — the
    /// per-layer paint a paged.draw appearance bake needs.
    #[test]
    fn c19_inserted_item_carries_tint_opacity_and_blend_mode() {
        let mut spread = grouped();
        let mut p = inserted_polygon(&spread, "u5", None);
        p.fill_tint = Some(40.0);
        p.opacity = Some(60.0);
        p.blend_mode = Some("Multiply".to_string());
        spread.polygons.push(p);
        spread
            .frames_in_order
            .push(idml_import::FrameRef::Polygon(1));

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out.clone()).unwrap();
        assert!(
            s.contains(r#"FillColor="Color/Paper" FillTint="40""#),
            "{s}"
        );
        assert!(
            s.contains(r#"<TransparencySetting><BlendingSetting Opacity="60" BlendMode="Multiply"/></TransparencySetting>"#),
            "{s}"
        );
        // And it re-parses: opacity / blend land back on the model.
        let reparsed = idml_import::parse_spread(&out).expect("re-parse");
        let back = reparsed
            .polygons
            .iter()
            .find(|p| p.self_id.as_deref() == Some("u5"))
            .expect("inserted polygon survives the round trip");
        assert_eq!(back.fill_tint, Some(40.0));
        assert_eq!(back.opacity, Some(60.0));
        assert_eq!(back.blend_mode.as_deref(), Some("Multiply"));
    }

    /// THE BAKE SHAPE: a group over one SOURCE carrier (paint cleared,
    /// plugin metadata intact) plus N inserted derived paths. The
    /// carrier moves into the wrapper and keeps its `<Label>` — the
    /// metadata is what lets the editor re-open the editable stack, so
    /// losing it on the move would defeat the bake as thoroughly as
    /// losing the group did.
    #[test]
    fn c19_mixed_group_moves_a_labelled_source_carrier_and_keeps_its_metadata() {
        let mut spread = grouped();
        spread.labels.insert(
            "r1".to_string(),
            vec![("paged.draw".to_string(), r#"{"fills":[]}"#.to_string())],
        );
        let base = spread.polygons[0].clone();
        for id in ["ufill", "ustroke"] {
            let mut p = base.clone();
            p.self_id = Some(id.to_string());
            spread.polygons.push(p);
        }
        spread.groups.push(new_group(
            "gbake",
            vec![
                idml_import::FrameRef::Rectangle(0), // the source carrier
                idml_import::FrameRef::Polygon(1),
                idml_import::FrameRef::Polygon(2),
            ],
            None,
        ));
        let gref = idml_import::FrameRef::Group(spread.groups.len() - 1);
        spread.frames_in_order = vec![
            gref,
            idml_import::FrameRef::Polygon(0),
            idml_import::FrameRef::Group(0),
        ];

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out.clone()).unwrap();
        assert_eq!(count(&s, r#"Self="r1""#), 1, "carrier not duplicated: {s}");
        assert!(
            s.contains(r#"<KeyValuePair Key="paged.draw" Value="{&quot;fills&quot;:[]}"/>"#),
            "the carrier's plugin metadata rides the move: {s}"
        );

        let reparsed = idml_import::parse_spread(&out).expect("re-parse");
        let g = reparsed
            .groups
            .iter()
            .find(|g| g.self_id.as_deref() == Some("gbake"))
            .expect("wrapper survives");
        assert_eq!(g.members.len(), 3, "carrier + both derived layers");
        assert_eq!(
            reparsed.labels.get("r1").map(|v| v.len()),
            Some(1),
            "and the metadata re-parses off the moved carrier"
        );
    }

    /// The whole point, end to end: a baked group survives a save and a
    /// re-parse with its wrapper, its members, and their per-layer paint
    /// intact.
    #[test]
    fn c19_baked_group_survives_a_reparse() {
        let mut spread = grouped();
        let base = spread.polygons[0].clone();
        for (i, id) in ["ufill", "ustroke"].iter().enumerate() {
            let mut p = base.clone();
            p.self_id = Some((*id).to_string());
            p.opacity = Some(50.0 + i as f32 * 10.0);
            spread.polygons.push(p);
        }
        spread.groups.push(new_group(
            "gbake",
            vec![
                idml_import::FrameRef::Polygon(1),
                idml_import::FrameRef::Polygon(2),
            ],
            None,
        ));
        let gref = idml_import::FrameRef::Group(spread.groups.len() - 1);
        spread.frames_in_order.push(gref);

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let reparsed = idml_import::parse_spread(&out).expect("re-parse");
        let g = reparsed
            .groups
            .iter()
            .find(|g| g.self_id.as_deref() == Some("gbake"))
            .expect("the baked group is a real <Group> on reopen");
        assert_eq!(g.members.len(), 2, "both derived layers are members");
        let ids: Vec<&str> = g
            .members
            .iter()
            .filter_map(|&m| match m {
                idml_import::FrameRef::Polygon(i) => reparsed.polygons[i].self_id.as_deref(),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec!["ufill", "ustroke"]);
        let opacities: Vec<Option<f32>> = g
            .members
            .iter()
            .filter_map(|&m| match m {
                idml_import::FrameRef::Polygon(i) => Some(reparsed.polygons[i].opacity),
                _ => None,
            })
            .collect();
        assert_eq!(opacities, vec![Some(50.0), Some(60.0)]);
    }

    // -----------------------------------------------------------------
    // C-18 — the corner vocabulary on the remaining page-item kinds
    // -----------------------------------------------------------------

    /// One of every kind that C-18 added, each carrying the corner
    /// vocabulary spelled the way InDesign writes it: long floats, the
    /// `BevelCorner` token (which the model's enum normalises to
    /// `Bevel`, losing the source spelling), and — on the GraphicLine —
    /// radii with NO option, which is the only shape the real corpus
    /// ever puts on a line.
    ///
    /// Every item carries an explicit `ItemTransform`, as every
    /// InDesign package does, so this fixture isolates the CORNER lane:
    /// an item without one has the identity appended (InDesign does not
    /// read an absent transform as identity — see
    /// `TransformPlan::extra`), which would otherwise show up as a byte
    /// diff that has nothing to do with C-18.
    const C18_SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<Spread Self="s"><TextFrame Self="tf1" GeometricBounds="0 0 100 200" ItemTransform="1 0 0 1 0 0" CornerOption="BevelCorner" CornerRadius="14.740157480314963" TopLeftCornerOption="BevelCorner" TopLeftCornerRadius="14.740157480314963"/><Oval Self="ov1" GeometricBounds="0 0 80 80" ItemTransform="1 0 0 1 0 0" CornerOption="RoundedCorner" CornerRadius="42.51968503937008" TopLeftCornerRadius="42.51968503937008"/><GraphicLine Self="gl1" GeometricBounds="0 0 10 10" ItemTransform="1 0 0 1 0 0" CornerRadius="99.21259842519686" TopLeftCornerRadius="99.21259842519686"/><Group Self="g9" ItemTransform="1 0 0 1 0 0" CornerOption="RoundedCorner" CornerRadius="70.86614173228347" TopLeftCornerOption="RoundedCorner"><Rectangle Self="rg" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 5 5"/></Group></Spread>
</idPkg:Spread>"#;

    fn c18_parsed() -> idml_import::Spread {
        idml_import::parse_spread(C18_SPREAD).expect("parse")
    }

    /// C-18 — the parse half. Every kind reads its own corner
    /// vocabulary; before this only Rectangle and Polygon did.
    #[test]
    fn c18_every_kind_parses_its_corner_attributes() {
        let s = c18_parsed();
        assert_eq!(s.text_frames[0].corner_radius, f("14.740157480314963"));
        assert_eq!(
            s.text_frames[0].corners[0].option,
            Some(idml_import::CornerOption::Bevel)
        );
        assert_eq!(s.ovals[0].corner_radius, f("42.51968503937008"));
        assert_eq!(s.ovals[0].corners[0].radius, f("42.51968503937008"));
        // A line carries radii and no option — the corpus shape.
        assert_eq!(s.graphic_lines[0].corner_radius, f("99.21259842519686"));
        assert_eq!(s.graphic_lines[0].corner_option, None);
        assert_eq!(s.groups[0].corner_radius, f("70.86614173228347"));
        assert_eq!(
            s.groups[0].corners[0].option,
            Some(idml_import::CornerOption::Rounded)
        );
    }

    /// C-18 — the byte-identity invariant, the same one B-23 pinned for
    /// polygons, now for the four kinds it left behind.
    ///
    /// This is the load-bearing test for the whole write half: every
    /// corner value on every kind now flows through the patch path, and
    /// `format_f32` rounds to 4 decimals while the option enum loses the
    /// source spelling. Without `preserving_f32_patch` /
    /// `preserving_option_patch`, `14.740157480314963` would come back
    /// `14.7402` and `BevelCorner` would become `BeveledCorner`.
    #[test]
    fn c18_unmutated_corner_attrs_round_trip_byte_identically_on_every_kind() {
        let out = rewrite_spread(C18_SPREAD, &c18_parsed()).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(C18_SPREAD),
            "unmutated corner attributes must reproduce their on-disk bytes \
             on TextFrame / Oval / GraphicLine / Group too"
        );
    }

    /// C-18 — and a MUTATED corner patches in place on each kind, with
    /// its neighbours' exact source spelling untouched.
    #[test]
    fn c18_mutated_corner_attrs_patch_in_place_on_every_kind() {
        let mut s = c18_parsed();
        s.text_frames[0].corners[0].radius = Some(12.5);
        s.ovals[0].corner_option = Some("InverseRoundedCorner".to_string());
        s.graphic_lines[0].corners[0].radius = Some(3.0);
        s.groups[0].corner_radius = Some(8.25);
        let out = rewrite_spread(C18_SPREAD, &s).expect("rewrite");
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains(r#"TopLeftCornerRadius="12.5""#), "{out}");
        assert!(
            out.contains(r#"CornerOption="InverseRoundedCorner""#),
            "{out}"
        );
        assert!(
            out.contains(r#"TopLeftCornerRadius="3""#),
            "line radius: {out}"
        );
        assert!(out.contains(r#"CornerRadius="8.25""#), "group: {out}");
        // The text frame's UNTOUCHED global radius keeps its full
        // precision, proving the patch is per-attribute rather than a
        // whole-element re-serialisation.
        assert!(
            out.contains(r#"CornerRadius="14.740157480314963""#),
            "{out}"
        );
    }

    /// C-18 — a `<Group>`'s corner-only patch lane must not start
    /// patching anything else. In particular its own `ItemTransform` is
    /// a documented known loss (a `SetGroupTransform` save-back is a
    /// separate lane), so a model transform that differs from the source
    /// must NOT be written.
    #[test]
    fn c18_group_patch_lane_touches_corners_only() {
        let mut s = c18_parsed();
        s.groups[0].item_transform = Some([2.0, 0.0, 0.0, 2.0, 99.0, 99.0]);
        s.groups[0].corner_radius = Some(1.5);
        let out = rewrite_spread(C18_SPREAD, &s).expect("rewrite");
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains(r#"CornerRadius="1.5""#), "corner: {out}");
        assert!(
            out.contains(r#"<Group Self="g9" ItemTransform="1 0 0 1 0 0""#),
            "the group's own transform must pass through verbatim: {out}"
        );
    }

    // -----------------------------------------------------------------
    // C-22 — an inserted item lands at its real z position
    // -----------------------------------------------------------------

    /// C-22 — the headline case, and the exact shape paged.draw's
    /// appearance bake produces: a new `<Group>` takes the z-slot of a
    /// SOURCE rectangle that moves inside it. Before this fix the group
    /// emitted at `</Spread>`, so it reopened ABOVE the polygon the
    /// source already carried; its file z-slot contradicted its canvas
    /// one.
    #[test]
    fn c22_inserted_group_lands_in_the_carriers_z_slot_not_at_the_close() {
        let mut spread = grouped();
        // A new group wrapping the SOURCE rectangle r1 (which currently
        // sits first in the file, below p1).
        spread.groups.push(new_group(
            "gbake",
            vec![idml_import::FrameRef::Rectangle(0)],
            None,
        ));
        let gref = idml_import::FrameRef::Group(spread.groups.len() - 1);
        // Model z-order: the bake group takes r1's slot — bottom-most.
        spread
            .frames_in_order
            .retain(|r| *r != idml_import::FrameRef::Rectangle(0));
        spread.frames_in_order.insert(0, gref);

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        let at = |id: &str| s.find(&format!(r#"Self="{id}""#)).expect("emitted");
        assert!(
            at("gbake") < at("p1"),
            "the inserted group must emit BELOW the source polygon it \
             sits under on canvas, not at the spread's close: {s}"
        );
        // …and the carrier really did move inside it.
        assert!(at("gbake") < at("r1"), "r1 must be inside gbake: {s}");
    }

    /// C-22 — the byte-identity invariant for the placement lane: a
    /// document with NOTHING inserted must be untouched. The plan is
    /// empty because the source's document order equals
    /// `frames_in_order` element for element, so no anchor ever fires.
    #[test]
    fn c22_unmutated_spread_is_untouched_by_the_placement_lane() {
        let out = rewrite_spread(GROUP_SPREAD, &grouped()).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(GROUP_SPREAD),
            "the C-22 placement pass must not move a byte of an \
             unmutated spread"
        );
    }

    /// C-22 — an insert whose z-slot sits BETWEEN two source items lands
    /// between them, not after both.
    #[test]
    fn c22_insert_between_two_source_items_lands_between_them() {
        let mut spread = grouped();
        spread
            .polygons
            .push(inserted_polygon(&spread, "umid", None));
        let pref = idml_import::FrameRef::Polygon(spread.polygons.len() - 1);
        // Source order is r1, p1, g1. Put the new polygon between r1 and
        // p1 in the z-table.
        let at = spread
            .frames_in_order
            .iter()
            .position(|r| *r == idml_import::FrameRef::Polygon(0))
            .expect("p1 in z-table");
        spread.frames_in_order.insert(at, pref);

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        let at = |id: &str| s.find(&format!(r#"Self="{id}""#)).expect("emitted");
        assert!(at("r1") < at("umid"), "umid must follow r1: {s}");
        assert!(at("umid") < at("p1"), "umid must precede p1: {s}");
    }

    /// C-22 — an insert with NO source item above it still goes at the
    /// close. That is not a fallback, it is the correct answer: it
    /// belongs on top of everything.
    #[test]
    fn c22_topmost_insert_still_emits_at_the_close() {
        let mut spread = grouped();
        spread
            .polygons
            .push(inserted_polygon(&spread, "utop", None));
        spread
            .frames_in_order
            .push(idml_import::FrameRef::Polygon(spread.polygons.len() - 1));

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        let at = |id: &str| s.find(&format!(r#"Self="{id}""#)).expect("emitted");
        assert!(
            at("g1") < at("utop"),
            "utop must follow every source item: {s}"
        );
    }

    /// C-22 × C-19 — the order AMONG several inserts sharing one anchor
    /// stays the z-table order C-19 established.
    #[test]
    fn c22_multiple_inserts_sharing_an_anchor_keep_z_order() {
        let mut spread = grouped();
        for id in ["ua", "ub", "uc"] {
            let p = inserted_polygon(&spread, id, None);
            spread.polygons.push(p);
        }
        let n = spread.polygons.len();
        // All three sit below p1, in creation order ua, ub, uc.
        let at = spread
            .frames_in_order
            .iter()
            .position(|r| *r == idml_import::FrameRef::Polygon(0))
            .expect("p1 in z-table");
        for (k, idx) in (n - 3..n).enumerate() {
            spread
                .frames_in_order
                .insert(at + k, idml_import::FrameRef::Polygon(idx));
        }

        let out = rewrite_spread(GROUP_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        let at = |id: &str| s.find(&format!(r#"Self="{id}""#)).expect("emitted");
        assert!(at("r1") < at("ua"), "{s}");
        assert!(at("ua") < at("ub"), "{s}");
        assert!(at("ub") < at("uc"), "{s}");
        assert!(at("uc") < at("p1"), "{s}");
    }

    /// C-22 — the placement pass must not confuse a NESTED source item
    /// (inside a group) for a top-level anchor. `r2` lives inside `g1`;
    /// an insert can never anchor to it, or it would be written inside
    /// the group.
    #[test]
    fn c22_nested_source_item_is_not_a_top_level_anchor() {
        let source = scan_source_items(GROUP_SPREAD).expect("prescan");
        assert_eq!(
            source.top_level,
            vec!["r1".to_string(), "p1".to_string(), "g1".to_string()],
            "the pre-scan must see only the spread's top-level items"
        );
    }

    // -----------------------------------------------------------------
    // Object-style REFERENCES on page items (the other half of the
    // object-style save-back: `resources::patch_styles` writes the
    // definition, this lane writes the `AppliedObjectStyle` pointing at
    // it).
    // -----------------------------------------------------------------

    /// InDesign writes `AppliedObjectStyle` on every page item — the
    /// reserved `ObjectStyle/$ID/[None]` when nothing is applied. The
    /// polygon here deliberately carries NO such attribute (the shape a
    /// leaner generator writes), so the "newly set on an element that
    /// never had the key" lane is covered too.
    const OBJSTYLE_SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<Spread Self="s"><Rectangle Self="r1" AppliedObjectStyle="ObjectStyle/$ID/[None]" ItemTransform="1 0 0 1 10 10" GeometricBounds="0 0 50 50" FillColor="Color/Black"/><TextFrame Self="tf1" AppliedObjectStyle="ObjectStyle/Callout" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 40 90"/><Polygon Self="p1" ItemTransform="1 0 0 1 20 20" GeometricBounds="0 0 30 30" FillColor="Color/Paper"/></Spread>
</idPkg:Spread>"#;

    fn objstyled() -> idml_import::Spread {
        idml_import::parse_spread(OBJSTYLE_SPREAD).expect("parse")
    }

    /// Byte-identity still holds with the reference lane live: an
    /// unmutated `AppliedObjectStyle` reproduces its source bytes (both
    /// the reserved `[None]` spelling and a real applied style).
    #[test]
    fn unmutated_applied_object_style_round_trips_byte_identically() {
        let out = rewrite_spread(OBJSTYLE_SPREAD, &objstyled()).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(OBJSTYLE_SPREAD),
            "an unmutated object-style reference must stay byte-identical"
        );
    }

    /// Applying a style to a SOURCE item patches the attribute in place
    /// — the definition `patch_styles` writes is useless if the item
    /// that should point at it still says `$ID/[None]`.
    #[test]
    fn applied_object_style_set_on_a_source_item_is_saved() {
        let mut spread = objstyled();
        spread.rectangles[0].applied_object_style = Some("ObjectStyle/u0".to_string());
        let out = rewrite_spread(OBJSTYLE_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<Rectangle Self="r1" AppliedObjectStyle="ObjectStyle/u0""#),
            "{s}"
        );
        // The neighbours are untouched.
        assert!(
            s.contains(r#"<TextFrame Self="tf1" AppliedObjectStyle="ObjectStyle/Callout""#),
            "{s}"
        );
    }

    /// Applying a style to an item whose source element never carried
    /// the attribute APPENDS it (the extras lane), rather than dropping
    /// the application on the floor.
    #[test]
    fn applied_object_style_is_appended_when_the_source_lacked_the_key() {
        let mut spread = objstyled();
        spread.polygons[0].applied_object_style = Some("ObjectStyle/u0".to_string());
        let out = rewrite_spread(OBJSTYLE_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(r#"Self="p1""#), "{s}");
        assert!(s.contains(r#"AppliedObjectStyle="ObjectStyle/u0""#), "{s}");
    }

    /// Clearing an applied style restores IDML's reserved `[None]`
    /// rather than dropping the attribute — an item with no
    /// `AppliedObjectStyle` at all is legal, but InDesign always writes
    /// the reserved value, and dropping it would read as "removed by
    /// the writer" on a diff.
    #[test]
    fn cleared_applied_object_style_falls_back_to_the_reserved_none() {
        let mut spread = objstyled();
        spread.text_frames[0].applied_object_style = None;
        let out = rewrite_spread(OBJSTYLE_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<TextFrame Self="tf1" AppliedObjectStyle="ObjectStyle/$ID/[None]""#),
            "{s}"
        );
    }

    /// An item CREATED since load carries the style it was given — the
    /// inserted-item lane used to hard-code `ObjectStyle/$ID/[None]`,
    /// so a styled new frame saved as unstyled.
    #[test]
    fn inserted_item_carries_its_applied_object_style() {
        let mut spread = objstyled();
        let mut p = spread.polygons[0].clone();
        p.self_id = Some("unew".to_string());
        p.applied_object_style = Some("ObjectStyle/u0".to_string());
        spread.polygons.push(p);
        spread
            .frames_in_order
            .push(idml_import::FrameRef::Polygon(spread.polygons.len() - 1));

        let out = rewrite_spread(OBJSTYLE_SPREAD, &spread).expect("rewrite");
        let s = String::from_utf8(out.clone()).unwrap();
        let at = s.find(r#"Self="unew""#).expect("inserted item emitted");
        assert!(
            s[at..].starts_with(r#"Self="unew" AppliedObjectStyle="ObjectStyle/u0""#),
            "{s}"
        );
        // And it re-parses onto the model.
        let reparsed = idml_import::parse_spread(&out).expect("re-parse");
        let back = reparsed
            .polygons
            .iter()
            .find(|p| p.self_id.as_deref() == Some("unew"))
            .expect("survives");
        assert_eq!(back.applied_object_style.as_deref(), Some("ObjectStyle/u0"));
    }
}
