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
//!   - `FillColor`         (FrameFillColor)
//!   - `FillTint`          (FrameFillTint)
//!   - `StrokeColor`       (FrameStrokeColor)
//!   - `StrokeWeight`      (FrameStrokeWeight)
//!   - `NextTextFrame`     (LinkFrames / UnlinkFrames; TextFrame only)
//!   - `Nonprinting`       (FrameNonprinting)
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
//!   [`recover_member_transform`]).
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
//!   inserts and the removal manifest-drop remain deferred.
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
//!   the B-18 paste-into lane and the C-19 group lane, not two.
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
//! * **Inserted-item Z-SLOT.** Inserted items are appended before
//!   `</Spread>`, in z-table order relative to each other, but always
//!   ABOVE everything the source already carried. An insert whose
//!   `z_slot` puts it below or between existing items comes back on top.
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

use idml_import::{Bounds, CharacterRun, PathAnchor, Spread, Story, TableCell, TextFrame};

/// Mirror of `paged_gen::xml::format_f32`: round to 4 decimals, drop
/// trailing zeros + a dangling `.`, normalise `-0` to `0`. Kept as a
/// small local copy rather than depending on `paged-gen` (a dev/CLI
/// crate that pulls clap/anyhow) so this runtime crate stays minimal +
/// wasm-clean. InDesign serialises floats this way, so patched values
/// match the surrounding hand-written / exported numbers.
pub(crate) fn format_f32(v: f32) -> String {
    let rounded = (v * 10_000.0).round() / 10_000.0;
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
enum Patch {
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
fn patch_start<F>(
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

/// Escape the five XML entities for an attribute value we synthesise.
/// Patched values are IDML ids / numbers / colour refs that almost never
/// contain these, but a style name could — so escape defensively to keep
/// the output well-formed.
pub(crate) fn escape_attr(s: &str) -> std::borrow::Cow<'_, str> {
    if s.bytes()
        .any(|b| matches!(b, b'&' | b'<' | b'>' | b'"' | b'\''))
    {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
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

/// The model's path geometry for one spread page item, plus a hint at
/// how to reconcile a divergence.
struct ModelGeometry {
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
    fn target_for_contour(&self, contour: usize, parsed: &[PathAnchor]) -> Option<Vec<PathAnchor>> {
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
        let model = self.contour_slice(contour);
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

    fn contour_slice(&self, contour: usize) -> &[PathAnchor] {
        if self.subpath_starts.is_empty() {
            return &self.anchors;
        }
        let start = self.subpath_starts[contour];
        let end = self
            .subpath_starts
            .get(contour + 1)
            .copied()
            .unwrap_or(self.anchors.len());
        self.anchors.get(start..end).unwrap_or(&[])
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
    polygons: &[idml_import::Polygon],
    graphic_lines: &[idml_import::GraphicLine],
) -> Option<ModelGeometry> {
    match name {
        b"TextFrame" => frames.get(self_id).map(|f| ModelGeometry {
            anchors: f.anchors.clone(),
            subpath_starts: f.subpath_starts.clone(),
            bounds: f.bounds,
        }),
        b"Rectangle" => rectangles
            .iter()
            .find(|r| r.self_id.as_deref() == Some(self_id))
            .map(|r| ModelGeometry {
                anchors: r.anchors.clone(),
                subpath_starts: r.subpath_starts.clone(),
                bounds: r.bounds,
            }),
        b"Polygon" => polygons
            .iter()
            .find(|r| r.self_id.as_deref() == Some(self_id))
            .map(|r| ModelGeometry {
                anchors: r.anchors.clone(),
                subpath_starts: r.subpath_starts.clone(),
                bounds: r.bounds,
            }),
        b"GraphicLine" => graphic_lines
            .iter()
            .find(|r| r.self_id.as_deref() == Some(self_id))
            .map(|r| ModelGeometry {
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
    writer.write_event(Event::Start(BytesStart::new("PathGeometry")))?;
    let mut gp = BytesStart::new("GeometryPathType");
    gp.push_attribute(("PathOpen", "false"));
    writer.write_event(Event::Start(gp))?;
    writer.write_event(Event::Start(BytesStart::new("PathPointArray")))?;
    for a in rect_corners(b) {
        write_path_point(writer, &a)?;
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
}

/// `Option<String>` has no `const` default that can be borrowed inline,
/// so the "this kind carries no fill" case points at one shared `None`.
static NO_COLOR: Option<String> = None;

impl NewItemPaint<'_> {
    /// True when the item needs a `<TransparencySetting>` sibling after
    /// its `<Properties>` block.
    fn has_transparency(&self) -> bool {
        self.opacity.is_some() || self.blend_mode.is_some()
    }
}

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
    attrs.push(("AppliedObjectStyle", "ObjectStyle/$ID/[None]".to_string()));
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
}

/// Emit the `<TransparencySetting><BlendingSetting …/></TransparencySetting>`
/// block an inserted item's opacity / blend mode live in. It is a
/// SIBLING of `<Properties>` (see `corpus/generated/transparency.idml`),
/// so callers write it right after the properties block closes.
fn write_transparency_setting(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    paint: &NewItemPaint<'_>,
) -> Result<(), quick_xml::Error> {
    if !paint.has_transparency() {
        return Ok(());
    }
    writer.write_event(Event::Start(BytesStart::new("TransparencySetting")))?;
    let mut attrs: Vec<(&str, String)> = Vec::new();
    if let Some(o) = paint.opacity {
        attrs.push(("Opacity", format_f32(o)));
    }
    if let Some(m) = paint.blend_mode {
        attrs.push(("BlendMode", m.to_string()));
    }
    emit_empty_with_attrs(writer, "BlendingSetting", &attrs)?;
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
        "TransparencySetting",
    )))?;
    Ok(())
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
        let mut kvp = BytesStart::new("KeyValuePair");
        kvp.push_attribute(("Key", k.as_str()));
        kvp.push_attribute(("Value", v.as_str()));
        writer.write_event(Event::Empty(kvp))?;
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
    };
    push_common_item_attrs(&mut attrs, f.item_transform, &paint);
    emit_start_with_attrs(writer, "TextFrame", &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_item_label(writer, spread, self_id)?;
    write_box_path_geometry(writer, f.bounds)?;
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Properties")))?;
    write_transparency_setting(writer, &paint)?;
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("TextFrame")))?;
    Ok(())
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
    write_box_path_geometry(writer, bounds)?;
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
fn nested_ref_self_id(spread: &Spread, r: idml_import::FrameRef) -> Option<&str> {
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
    emit_start_with_attrs(
        writer,
        "Group",
        &[
            ("Self", self_id.to_string()),
            (
                "ItemTransform",
                format_matrix(
                    &group
                        .item_transform
                        .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                ),
            ),
        ],
    )?;
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

/// Append every model page item whose `Self` id was NOT seen in the
/// source XML — the inserted nodes — at the spread's close, in
/// [`insert_emission_order`].
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
    // Ids that belong INSIDE something else: group members (emitted by
    // the group's own recursion) and B-18 nested children.
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
pub fn rewrite_spread(original: &[u8], spread: &Spread) -> Result<Vec<u8>, quick_xml::Error> {
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
    let mut depth: usize = 0;
    struct LabelCtx {
        /// Depth of the item element itself.
        item_depth: usize,
        /// Model entries; `None` ⇒ the model has no labels for it.
        entries: Option<Vec<(String, String)>>,
        /// A direct `<Properties>` child is currently open.
        in_direct_properties: bool,
        /// We are inside the item's `<Label>` (original KVPs drop).
        in_label: bool,
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
            let mut kvp = BytesStart::new("KeyValuePair");
            kvp.push_attribute(("Key", k.as_str()));
            kvp.push_attribute(("Value", v.as_str()));
            writer.write_event(Event::Empty(kvp))?;
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
        /// Buffered events inside the open `<PathPointArray>` (point
        /// elements + any whitespace between them).
        buffered: Vec<Event<'static>>,
        /// On-disk anchors parsed from the buffered points.
        parsed: Vec<PathAnchor>,
    }
    let mut path_ctx: Vec<PathCtx> = Vec::new();

    loop {
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
                // Buffer a `<PathPointArray>` for the innermost path
                // item so its points can be rewritten at close.
                if name_owned == b"PathPointArray" {
                    if let Some(ctx) = path_ctx.last_mut() {
                        if ctx.array_depth == 0 {
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
                        // Replace (or drop) the Label wholesale.
                        ctx.in_label = true;
                        if let Some(entries) = ctx.entries.as_deref() {
                            write_label_entries(&mut writer, entries)?;
                        }
                        ctx.handled = true;
                        buf.clear();
                        continue; // original <Label> start not written
                    } else if ctx.in_label {
                        // Unexpected child inside a replaced Label —
                        // drop it with the rest of the Label body.
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
                    &frames,
                    &spread.rectangles,
                    &spread.ovals,
                    &spread.polygons,
                    &spread.graphic_lines,
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
                // KeyValuePairs inside a replaced Label drop (the
                // model entries were already written).
                if let Some(ctx) = label_ctx.last() {
                    if ctx.in_label {
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
                    &frames,
                    &spread.rectangles,
                    &spread.ovals,
                    &spread.polygons,
                    &spread.graphic_lines,
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
                    if depth == ctx.item_depth && ITEM_KINDS.contains(&name_owned.as_slice()) {
                        path_ctx.pop();
                    }
                }
                if let Some(ctx) = label_ctx.last_mut() {
                    if ctx.in_label && name_owned == b"Label" && depth == ctx.item_depth + 2 {
                        // Closing the replaced Label — the new entries
                        // (with their own End) were already written.
                        ctx.in_label = false;
                        depth = depth.saturating_sub(1);
                        buf.clear();
                        continue;
                    }
                    if ctx.in_label {
                        // Closing a dropped child inside the Label.
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

    Ok(writer.into_inner().into_inner())
}

/// Decide whether to patch a page item's `ItemTransform`, and with what
/// value. `group_accum` is `None` for a top-level item (the model
/// transform IS the on-disk transform → patch it). For a group member it
/// is `Some(accumulated_group_transform)`; the on-disk transform is
/// recovered by inverting the group accumulation (W1.15 lane 4). When the
/// group transform is singular the recovery fails and the patch is
/// suppressed (the attribute passes through verbatim — a documented loss
/// for that degenerate case).
fn resolve_item_transform(
    group_accum: Option<Option<[f32; 6]>>,
    model_transform: Option<[f32; 6]>,
) -> (bool, Option<[f32; 6]>) {
    match group_accum {
        None => (true, model_transform),
        Some(accum) => match recover_member_transform(accum, model_transform) {
            Some(on_disk) => (true, on_disk),
            None => (false, model_transform),
        },
    }
}

/// If `e` is a page-item start tag whose `Self` matches a model item,
/// return the patched start tag. `None` ⇒ not a page item we patch
/// (caller emits the original verbatim). `group_accum` carries the
/// accumulated group transform for a member (see [`resolve_item_transform`]).
#[allow(clippy::too_many_arguments)]
fn patch_spread_item(
    e: &BytesStart,
    frames: &std::collections::HashMap<&str, &TextFrame>,
    rectangles: &[idml_import::Rectangle],
    ovals: &[idml_import::Oval],
    polygons: &[idml_import::Polygon],
    graphic_lines: &[idml_import::GraphicLine],
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
            let (patch_tx, item_transform) =
                resolve_item_transform(group_accum, frame.item_transform);
            let fill = frame.fill_color.clone();
            let fill_tint = frame.fill_tint;
            let stroke = frame.stroke_color.clone();
            let stroke_weight = frame.stroke_weight;
            let next = frame.next_text_frame.clone();
            let nonprinting = frame.nonprinting;
            let bounds = frame.bounds;
            let start = patch_start(
                e,
                |k, raw| {
                    frame_attr_patch(
                        k,
                        raw,
                        patch_tx,
                        item_transform,
                        &fill,
                        fill_tint,
                        &stroke,
                        stroke_weight,
                        Some(&next),
                        nonprinting,
                        bounds,
                        None,
                        None,
                        // TextFrame carries the corner attrs on disk but
                        // has no model fields (B-23 residual) — `None`
                        // passes every corner attribute through verbatim.
                        None,
                    )
                },
                &frame_attr_extras(
                    patch_tx,
                    item_transform,
                    &fill,
                    fill_tint,
                    &stroke,
                    stroke_weight,
                    next.as_deref(),
                    nonprinting,
                    None,
                    None,
                    None,
                ),
            )?;
            Ok(Some(start.into_owned()))
        }
        b"Rectangle" => {
            let item = rectangles
                .iter()
                .find(|r| r.self_id.as_deref() == Some(self_id.as_str()));
            let (patch_tx, tx) =
                resolve_item_transform(group_accum, item.and_then(|r| r.item_transform));
            patch_vector_item(
                e,
                patch_tx,
                item.map(|r| VectorItem {
                    item_transform: tx,
                    fill_color: r.fill_color.clone(),
                    fill_tint: r.fill_tint,
                    stroke_color: r.stroke_color.clone(),
                    stroke_weight: r.stroke_weight,
                    nonprinting: r.nonprinting,
                    bounds: r.bounds,
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
            let (patch_tx, tx) =
                resolve_item_transform(group_accum, item.and_then(|r| r.item_transform));
            patch_vector_item(
                e,
                patch_tx,
                item.map(|r| VectorItem {
                    item_transform: tx,
                    fill_color: r.fill_color.clone(),
                    fill_tint: r.fill_tint,
                    stroke_color: r.stroke_color.clone(),
                    stroke_weight: r.stroke_weight,
                    nonprinting: r.nonprinting,
                    bounds: r.bounds,
                    start_arrow: None,
                    end_arrow: None,
                    // B-23 residual: Oval carries the corner attrs on
                    // disk but has no model fields — pass verbatim.
                    corners: None,
                }),
            )
        }
        b"Polygon" => {
            let item = polygons
                .iter()
                .find(|r| r.self_id.as_deref() == Some(self_id.as_str()));
            let (patch_tx, tx) =
                resolve_item_transform(group_accum, item.and_then(|r| r.item_transform));
            patch_vector_item(
                e,
                patch_tx,
                item.map(|r| VectorItem {
                    item_transform: tx,
                    fill_color: r.fill_color.clone(),
                    fill_tint: r.fill_tint,
                    stroke_color: r.stroke_color.clone(),
                    stroke_weight: r.stroke_weight,
                    nonprinting: r.nonprinting,
                    bounds: r.bounds,
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
            let (patch_tx, tx) =
                resolve_item_transform(group_accum, item.and_then(|r| r.item_transform));
            patch_vector_item(
                e,
                patch_tx,
                item.map(|r| VectorItem {
                    item_transform: tx,
                    fill_color: None,
                    fill_tint: None,
                    stroke_color: r.stroke_color.clone(),
                    stroke_weight: r.stroke_weight,
                    nonprinting: r.nonprinting,
                    bounds: r.bounds,
                    start_arrow: Some(r.start_arrow),
                    end_arrow: Some(r.end_arrow),
                    // B-23 residual: GraphicLine carries the corner
                    // attrs on disk but has no model fields.
                    corners: None,
                }),
            )
        }
        _ => Ok(None),
    }
}

/// The frame attributes shared by every page-item kind, lifted into one
/// shape so a single patch routine covers Rectangle / Oval / Polygon /
/// GraphicLine.
struct VectorItem {
    item_transform: Option<[f32; 6]>,
    fill_color: Option<String>,
    fill_tint: Option<f32>,
    stroke_color: Option<String>,
    stroke_weight: Option<f32>,
    nonprinting: bool,
    bounds: idml_import::Bounds,
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
fn preserving_f32_patch(raw: Option<&str>, v: Option<f32>) -> Patch {
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
    patch_tx: bool,
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
                patch_tx,
                item.item_transform,
                &item.fill_color,
                item.fill_tint,
                &item.stroke_color,
                item.stroke_weight,
                None,
                item.nonprinting,
                item.bounds,
                item.start_arrow,
                item.end_arrow,
                item.corners.as_ref(),
            )
        },
        &frame_attr_extras(
            patch_tx,
            item.item_transform,
            &item.fill_color,
            item.fill_tint,
            &item.stroke_color,
            item.stroke_weight,
            None,
            item.nonprinting,
            item.start_arrow,
            item.end_arrow,
            item.corners.as_ref(),
        ),
    )?;
    Ok(Some(start.into_owned()))
}

/// Patch decision for one frame attribute key. `next` is `Some` only for
/// TextFrame (`NextTextFrame` lives there); `None` skips that key for
/// other kinds. Bounds patch only fires for a `GeometricBounds`
/// attribute that the source element already carries. `patch_tx` false
/// passes `ItemTransform` through verbatim (group member — see
/// [`rewrite_spread`]).
#[allow(clippy::too_many_arguments)]
fn frame_attr_patch(
    key: &[u8],
    raw: &[u8],
    patch_tx: bool,
    item_transform: Option<[f32; 6]>,
    fill: &Option<String>,
    fill_tint: Option<f32>,
    stroke: &Option<String>,
    stroke_weight: Option<f32>,
    next: Option<&Option<String>>,
    nonprinting: bool,
    bounds: idml_import::Bounds,
    start_arrow: Option<idml_import::ArrowheadType>,
    end_arrow: Option<idml_import::ArrowheadType>,
    corners: Option<&CornerAttrs>,
) -> Option<Patch> {
    // B-23 — corner vocabulary first; `None` falls through to the rest.
    if let Some(c) = corners {
        if let Some(p) = corner_attr_patch(key, raw, c) {
            return Some(p);
        }
    }
    match key {
        b"ItemTransform" if !patch_tx => None,
        b"ItemTransform" => Some(match item_transform {
            Some(m) => Patch::Set(format_matrix(&m)),
            None => Patch::Remove,
        }),
        b"FillColor" => Some(opt_string_patch(fill)),
        b"FillTint" => Some(opt_f32_patch(fill_tint)),
        b"StrokeColor" => Some(opt_string_patch(stroke)),
        b"StrokeWeight" => Some(opt_f32_patch(stroke_weight)),
        b"Nonprinting" => Some(if nonprinting {
            Patch::Set("true".to_string())
        } else {
            // The parser defaults absent → false; drop the attribute to
            // restore the implicit default rather than write "false".
            Patch::Remove
        }),
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
    patch_tx: bool,
    item_transform: Option<[f32; 6]>,
    fill: &Option<String>,
    fill_tint: Option<f32>,
    stroke: &Option<String>,
    stroke_weight: Option<f32>,
    next: Option<&str>,
    nonprinting: bool,
    start_arrow: Option<idml_import::ArrowheadType>,
    end_arrow: Option<idml_import::ArrowheadType>,
    corners: Option<&CornerAttrs>,
) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if patch_tx {
        if let Some(m) = item_transform {
            out.push(("ItemTransform", format_matrix(&m)));
        }
    }
    if let Some(c) = fill {
        out.push(("FillColor", c.clone()));
    }
    // C-19: a tint SET on an item whose source element never carried a
    // `FillTint` attribute used to fall off here (the patch lane only
    // rewrites keys the source already has).
    if let Some(t) = fill_tint {
        out.push(("FillTint", format_f32(t)));
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

fn opt_string_patch(v: &Option<String>) -> Patch {
    match v {
        Some(s) => Patch::Set(s.clone()),
        None => Patch::Remove,
    }
}

fn opt_f32_patch(v: Option<f32>) -> Patch {
    match v {
        Some(n) => Patch::Set(format_f32(n)),
        None => Patch::Remove,
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
/// current model. Ranges are matched positionally (IDML carries no id on
/// them); the parser produced them in this same order.
pub fn rewrite_story(original: &[u8], story: &Story) -> Result<Vec<u8>, quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    // Positional cursors into the model.
    let mut para_idx: isize = -1;
    let mut run_idx: isize = -1;
    // The run currently open (for Content text + attribute patching).
    let mut current_run: Option<&CharacterRun> = None;
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
    // patch against the matched model `TableCell.paragraphs[]` with
    // cell-local positional cursors (reset on each `<Cell>` open). When
    // a cell has no model match — or the cell text is unchanged — its
    // content passes through verbatim, exactly as before.
    let cells = collect_story_cells(story);
    let mut current_cell: Option<&TableCell> = None;
    let mut cell_depth: usize = 0; // depth of the open `<Cell>`, or 0
    let mut cell_para_idx: isize = -1;
    let mut cell_run_idx: isize = -1;
    // Nested tables (a table in a cell's paragraph) nest `<Cell>`s, so the
    // cell-local cursor state is a stack: each `<Cell>` open parks the
    // enclosing cell's state, each `</Cell>` restores it.
    type CellFrame<'a> = (Option<&'a TableCell>, usize, isize, isize);
    let mut cell_stack: Vec<CellFrame> = Vec::new();

    loop {
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
                        writer.write_event(Event::Start(e.into_owned()))?;
                    }
                    b"Cell" if table_depth > 0 => {
                        // Enter a cell — park the enclosing cell's cursors
                        // (nested tables nest cells) then bind this cell's
                        // model counterpart (by `Self`) + reset the
                        // cell-local cursors. The start tag passes through
                        // verbatim (cell-level attributes patched elsewhere).
                        cell_stack.push((current_cell, cell_depth, cell_para_idx, cell_run_idx));
                        cell_depth = table_depth;
                        cell_para_idx = -1;
                        cell_run_idx = -1;
                        current_cell =
                            attr_value(&e, b"Self").and_then(|id| cells.get(id.as_str()).copied());
                        writer.write_event(Event::Start(e.into_owned()))?;
                    }
                    b"ParagraphStyleRange" if table_depth == 0 => {
                        para_idx += 1;
                        run_idx = -1;
                        let para = story.paragraphs.get(para_idx as usize);
                        let start = patch_paragraph_range(&e, para)?;
                        writer.write_event(Event::Start(start))?;
                    }
                    b"ParagraphStyleRange" if in_cell => {
                        cell_para_idx += 1;
                        cell_run_idx = -1;
                        let para =
                            current_cell.and_then(|c| c.paragraphs.get(cell_para_idx as usize));
                        let start = patch_paragraph_range(&e, para)?;
                        writer.write_event(Event::Start(start))?;
                    }
                    b"CharacterStyleRange" if table_depth == 0 => {
                        run_idx += 1;
                        current_run = story
                            .paragraphs
                            .get(para_idx as usize)
                            .and_then(|p| p.runs.get(run_idx as usize));
                        body = RunBody::default();
                        let start = patch_character_range(&e, current_run)?;
                        writer.write_event(Event::Start(start))?;
                    }
                    b"CharacterStyleRange" if in_cell => {
                        cell_run_idx += 1;
                        current_run = current_cell
                            .and_then(|c| c.paragraphs.get(cell_para_idx as usize))
                            .and_then(|p| p.runs.get(cell_run_idx as usize));
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
                        para_idx += 1;
                        run_idx = -1;
                        let para = story.paragraphs.get(para_idx as usize);
                        let start = patch_paragraph_range(&e, para)?;
                        writer.write_event(Event::Empty(start))?;
                    }
                    b"ParagraphStyleRange" if in_cell => {
                        cell_para_idx += 1;
                        cell_run_idx = -1;
                        let para =
                            current_cell.and_then(|c| c.paragraphs.get(cell_para_idx as usize));
                        let start = patch_paragraph_range(&e, para)?;
                        writer.write_event(Event::Empty(start))?;
                    }
                    b"CharacterStyleRange" if table_depth == 0 => {
                        run_idx += 1;
                        current_run = None;
                        body = RunBody::default();
                        let run = story
                            .paragraphs
                            .get(para_idx as usize)
                            .and_then(|p| p.runs.get(run_idx as usize));
                        let start = patch_character_range(&e, run)?;
                        writer.write_event(Event::Empty(start))?;
                    }
                    b"CharacterStyleRange" if in_cell => {
                        cell_run_idx += 1;
                        current_run = None;
                        body = RunBody::default();
                        let run = current_cell
                            .and_then(|c| c.paragraphs.get(cell_para_idx as usize))
                            .and_then(|p| p.runs.get(cell_run_idx as usize));
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
                        body.events.push(Event::Empty(e.into_owned()));
                    }
                    b"Tab" if (table_depth == 0 || in_cell) && !body.in_content => {
                        // `<Tab/>` is an inline leaf → `\t`. Opens or
                        // continues the body region (see `<Br/>`).
                        body.active = true;
                        body.text.push('\t');
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
                if body.active && body.in_content {
                    // Buffer — the replace decision happens at the run
                    // close once the whole (possibly entity-split) span
                    // is known.
                    let decoded = t.decode().unwrap_or_default();
                    let orig = quick_xml::escape::unescape(&decoded)
                        .map(|c| c.into_owned())
                        .unwrap_or_else(|_| decoded.into_owned());
                    body.text.push_str(&orig);
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
                    body.text.push_str(&resolved);
                    body.events.push(Event::GeneralRef(r.into_owned()));
                } else if body.active {
                    body.foreign = true;
                    body.events.push(Event::GeneralRef(r.into_owned()));
                } else {
                    writer.write_event(Event::GeneralRef(r))?;
                }
            }
            Event::End(e) => {
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
                        // cursors (a nested table's cell pops back to its
                        // host cell; a top-level cell pops back to None) so
                        // siblings + post-table markup patch correctly.
                        let (cc, cd, cp, cr) = cell_stack.pop().unwrap_or((None, 0, -1, -1));
                        current_cell = cc;
                        cell_depth = cd;
                        cell_para_idx = cp;
                        cell_run_idx = cr;
                    }
                    b"Content" if body.active => {
                        body.in_content = false;
                        body.events.push(Event::End(e.into_owned()));
                        continue; // already buffered + advanced
                    }
                    b"CharacterStyleRange" => {
                        flush_run_body(&mut writer, &mut body, current_run)?;
                        current_run = None;
                    }
                    _ => {}
                }
                writer.write_event(Event::End(e))?;
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
}

/// Emit the buffered inline body of a closing run. When the model text
/// diverged from the reconstructed source AND the body is pure
/// Content/Br/Tab (no foreign markup to preserve), re-serialise the
/// model text across the Content/Br/Tab structure (mirroring
/// `paged_gen`'s `write_run_content`: `\n` → `<Br/>`, `\t` → `<Tab/>`,
/// runs of plain text → `<Content>…</Content>`). Otherwise replay the
/// original events so an unchanged run — or one carrying markers — stays
/// byte-identical.
fn flush_run_body(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    body: &mut RunBody,
    run: Option<&CharacterRun>,
) -> Result<(), quick_xml::Error> {
    if !body.active {
        return Ok(());
    }
    let replace = match run {
        Some(r) => r.text != body.text && !body.foreign,
        None => false,
    };
    if replace {
        write_run_content(writer, &run.expect("checked above").text)?;
    } else {
        for ev in body.events.drain(..) {
            writer.write_event(ev)?;
        }
    }
    body.active = false;
    body.in_content = false;
    body.events.clear();
    Ok(())
}

/// Serialise a run's text body back into IDML `<Content>` / `<Br/>` /
/// `<Tab/>` structure, byte-for-byte matching `paged_gen`'s emitter so
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
    for ch in text.chars() {
        match ch {
            '\t' => {
                flush(writer, &mut buf)?;
                writer.write_event(Event::Empty(BytesStart::new("Tab")))?;
            }
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

fn patch_paragraph_range(
    e: &BytesStart,
    para: Option<&idml_import::Paragraph>,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    let style = para.and_then(|p| p.paragraph_style.clone());
    let extras: Vec<(&str, String)> = match &style {
        Some(s) => vec![("AppliedParagraphStyle", s.clone())],
        None => Vec::new(),
    };
    let start = patch_start(
        e,
        |k, _| match k {
            b"AppliedParagraphStyle" => Some(opt_string_patch(&style)),
            _ => None,
        },
        &extras,
    )?;
    Ok(start.into_owned())
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
    let start = patch_start(e, |k, _| character_attr_patch(k, &r), &extras)?;
    Ok(start.into_owned())
}

/// Patch decision for one `<CharacterStyleRange>` attribute. Covers the
/// character paths the mutation surface writes.
fn character_attr_patch(key: &[u8], r: &CharacterRun) -> Option<Patch> {
    match key {
        b"AppliedCharacterStyle" => Some(opt_string_patch(&r.character_style)),
        b"AppliedFont" => Some(opt_string_patch(&r.font)),
        b"FontStyle" => Some(opt_string_patch(&r.font_style)),
        b"PointSize" => Some(opt_f32_patch(r.point_size)),
        b"FillColor" => Some(opt_string_patch(&r.fill_color)),
        b"FillTint" => Some(opt_f32_patch(r.fill_tint)),
        b"StrokeColor" => Some(opt_string_patch(&r.stroke_color)),
        b"StrokeWeight" => Some(opt_f32_patch(r.stroke_weight)),
        b"Leading" => Some(opt_f32_patch(r.leading)),
        b"Tracking" => Some(opt_f32_patch(r.tracking)),
        b"BaselineShift" => Some(opt_f32_patch(r.baseline_shift)),
        b"HorizontalScale" => Some(opt_f32_patch(r.horizontal_scale)),
        b"VerticalScale" => Some(opt_f32_patch(r.vertical_scale)),
        b"Skew" => Some(opt_f32_patch(r.skew)),
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

fn opt_bool_patch(v: Option<bool>) -> Patch {
    match v {
        Some(b) => Patch::Set(b.to_string()),
        None => Patch::Remove,
    }
}

/// Read an attribute's decoded value off a start tag.
fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|Attribute { value, .. }| std::str::from_utf8(&value).ok().map(|s| s.to_string()))
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A spread carrying one polygon with the corner vocabulary spelled
    /// the way InDesign writes it: long floats and the `BevelCorner`
    /// token (which the model's enum normalises to `Bevel`).
    const POLY_SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<Spread Self="s"><Polygon Self="p" GeometricBounds="0 0 100 200" CornerOption="BevelCorner" CornerRadius="44.51279527491718" TopLeftCornerOption="BevelCorner" TopLeftCornerRadius="44.51279527491718" TopRightCornerRadius="44.51279527491718" FillColor="Color/Black"/></Spread>
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
        }
    }

    /// Count non-overlapping occurrences of `needle` in `hay`.
    fn count(hay: &str, needle: &str) -> usize {
        hay.matches(needle).count()
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
}
