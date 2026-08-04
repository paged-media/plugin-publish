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

//! Z-ORDER save-back — the lane C-22 deferred (C-23).
//!
//! IDML has no z attribute: the stacking order of a spread's page items
//! IS their document order inside `<Spread>` (and, one level down, the
//! order of a `<Group>`'s members / a B-18 container's nested children).
//! So core's v59 `ReorderNode` — Arrange: bring-to-front, send-to-back,
//! forward, backward, `{ index }` — is a save-back question about
//! ELEMENT ORDER, and nothing else. No attribute changes.
//!
//! # Why a post-pass, and not a change to the streaming writer
//!
//! [`crate::rewrite::rewrite_spread`] is a single forward pass: it reads
//! the source events and writes them out, deciding each element's fate
//! as it reaches it. It cannot emit an element it has not read yet, so a
//! "write these items in a different order" instruction is exactly the
//! shape that pass cannot take — which is why C-22 could only ANCHOR new
//! items among the source ones and had to leave a reshuffled z alone.
//!
//! The alternative — dropping a source element and re-minting it at its
//! new slot through the `write_new_*` emitters — is the lane the writer
//! already uses for a MOVED item, and it is lossy by construction: those
//! emitters rebuild only what the model tracks, so `<Image>`,
//! `<TextWrapPreference>`, `<ClippingPathSettings>` and every unparsed
//! attribute would evaporate the moment a user pressed "bring to front".
//! An Arrange that silently strips a placed image is no better than an
//! Arrange that silently reverts.
//!
//! So this runs AFTER the rewrite, over its output, and moves BYTES:
//! each page item's serialised element (start tag through matching end
//! tag, subtree included) is spliced into the slot the model's z-order
//! asks for. Whatever the rewrite decided about that element — patched
//! attributes, rewritten anchors, flushed nested children — travels with
//! it verbatim. Nothing is re-minted, so nothing is lost.
//!
//! # The three sibling lists, one rule
//!
//! `ReorderNode` permutes the list the node already lives in — the
//! spread's `frames_in_order`, a `Group::members`, or a
//! `Spread::nested_children` entry (its containment guarantee). This
//! pass mirrors that exactly: it walks the emitted XML into a tree of
//! page-item elements and, at each level, reorders the children to match
//! whichever of those three lists owns them. A spread-level reshuffle
//! therefore cannot disturb a group's internal order, or a container's
//! pasted-in content: they are different runs of the recursion.
//!
//! # Byte-identity
//!
//! Two properties keep the repo's lazy-verbatim posture intact:
//!
//! * **The pass is a no-op unless the order actually differs.** Every
//!   level reports "unchanged" up the recursion, and when the whole
//!   document reports unchanged the INPUT buffer is returned — not a
//!   re-serialisation of it. The parser appends to `frames_in_order` in
//!   document order, so an unmutated spread reports unchanged at every
//!   level by construction. (`write_idml` then takes the verbatim
//!   ZIP-copy path for the entry, so the package stays bit-for-bit.)
//! * **Only known items move.** An element whose `Self` id the model's
//!   sibling list doesn't mention — an id-less item, a C-28 opacity-mask
//!   artwork (painted from no list), anything a future parse lifts out
//!   of the z table — is PINNED to its slot, and the known items permute
//!   around it. The pass never invents an order for something it cannot
//!   account for.
//!
//! A spread whose z table is EMPTY is skipped at the top level
//! entirely: on such a document (built wholly by `InsertNode`, where
//! `register_frame_ref` no-ops) the table is not a z-order at all, and
//! `paged_mutate`'s `ensure_frames_in_order` materialises a kind-vec
//! order that would reshuffle the file for no reason.

use std::collections::HashSet;

use quick_xml::events::Event;
use quick_xml::Reader;

use idml_import::Spread;

use crate::rewrite::{attr_value, is_page_item_name, nested_ref_self_id, owned_ids};

/// One page-item element in the emitted XML, with the byte span it
/// occupies and the page items nested directly inside it.
struct Node {
    /// `Self` id, when the element carries one.
    id: Option<String>,
    /// `<Group>` (members) vs any other page item (B-18 children).
    is_group: bool,
    /// Byte offset of the element's `<` in the emitted buffer.
    start: usize,
    /// Byte offset just past the element's closing `>`.
    end: usize,
    /// Start offset of the ENCLOSING element, or `usize::MAX` at the
    /// document root. Only read for top-level nodes, where it keeps a
    /// permutation from ever crossing a parent boundary.
    parent: usize,
    /// Page items nested directly inside this one (group members, or
    /// B-18 paste-into content).
    children: Vec<Node>,
}

/// Re-splice `xml` so every page item sits in the slot the model's
/// z-order asks for. Returns `xml` untouched when it already does.
pub(crate) fn apply(spread: &Spread, xml: Vec<u8>) -> Result<Vec<u8>, quick_xml::Error> {
    let roots = scan(&xml)?;
    if roots.is_empty() {
        return Ok(xml);
    }
    let desired = top_level_order(spread);
    let mut out: Vec<u8> = Vec::new();
    let mut cursor = 0usize;
    let mut changed = false;
    // Top-level items sharing a parent element form one contiguous run —
    // XML nesting makes interleaving impossible — so grouping by parent
    // both preserves containment and needs no sorting.
    let mut i = 0usize;
    while i < roots.len() {
        let mut j = i + 1;
        while j < roots.len() && roots[j].parent == roots[i].parent {
            j += 1;
        }
        let run = &roots[i..j];
        if let Some(bytes) = render_run(&xml, run, desired.as_deref(), spread) {
            out.extend_from_slice(&xml[cursor..run[0].start]);
            out.extend_from_slice(&bytes);
            cursor = run[run.len() - 1].end;
            changed = true;
        }
        i = j;
    }
    if !changed {
        return Ok(xml);
    }
    out.extend_from_slice(&xml[cursor..]);
    Ok(out)
}

/// The model's TOP-LEVEL stacking order, as `Self` ids.
///
/// `None` when the spread's z table is empty — see the module note: an
/// empty table is not an order, and reading one out of the kind vecs
/// would reshuffle a file nobody reordered.
fn top_level_order(spread: &Spread) -> Option<Vec<&str>> {
    if spread.frames_in_order.is_empty() {
        return None;
    }
    let owned = owned_ids(spread);
    Some(
        spread
            .frames_in_order
            .iter()
            .filter_map(|&r| nested_ref_self_id(spread, r))
            .filter(|id| !owned.contains(id))
            .collect(),
    )
}

/// The model's order for the page items nested inside `node`: a
/// `<Group>`'s members, or a B-18 container's `nested_children`. `None`
/// when the model has no list for it (an id-less element, or a group the
/// model no longer carries — the same "stand down rather than guess"
/// rule the writer's group triage uses).
fn child_order<'a>(spread: &'a Spread, node: &Node) -> Option<Vec<&'a str>> {
    let id = node.id.as_deref()?;
    if node.is_group {
        let group = spread
            .groups
            .iter()
            .find(|g| g.self_id.as_deref() == Some(id))?;
        return Some(
            group
                .members
                .iter()
                .filter_map(|&m| nested_ref_self_id(spread, m))
                .collect(),
        );
    }
    spread.nested_children.get(id).map(|children| {
        children
            .iter()
            .filter_map(|&r| nested_ref_self_id(spread, r))
            .collect()
    })
}

/// Rebuild one page item, with the page items inside it reordered.
/// `None` ⇒ nothing in this subtree moved (the caller keeps the source
/// bytes).
fn render_node(xml: &[u8], node: &Node, spread: &Spread) -> Option<Vec<u8>> {
    let order = child_order(spread, node);
    let inner = render_run(xml, &node.children, order.as_deref(), spread)?;
    // `render_run` only returns `Some` for a non-empty run, so the run's
    // bounds exist.
    let run_start = node.children[0].start;
    let run_end = node.children[node.children.len() - 1].end;
    let mut out = Vec::with_capacity(node.end - node.start);
    out.extend_from_slice(&xml[node.start..run_start]);
    out.extend_from_slice(&inner);
    out.extend_from_slice(&xml[run_end..node.end]);
    Some(out)
}

/// Rebuild a contiguous run of sibling page items in `order`.
///
/// The bytes BETWEEN the items (indentation, and any non-page-item
/// sibling that happens to sit between them) stay at their original
/// offsets — only the item spans are permuted through the slots — so a
/// reorder cannot move an element out of, or into, the run.
///
/// `None` ⇒ the run is already in order and no descendant moved.
fn render_run(
    xml: &[u8],
    items: &[Node],
    order: Option<&[&str]>,
    spread: &Spread,
) -> Option<Vec<u8>> {
    if items.is_empty() {
        return None;
    }
    let rebuilt: Vec<Option<Vec<u8>>> = items
        .iter()
        .map(|child| render_node(xml, child, spread))
        .collect();
    let perm = plan_permutation(items, order);
    if perm.is_none() && rebuilt.iter().all(Option::is_none) {
        return None;
    }
    let mut out = Vec::new();
    let mut cursor = items[0].start;
    for slot in 0..items.len() {
        let source = perm.as_ref().map_or(slot, |p| p[slot]);
        out.extend_from_slice(&xml[cursor..items[slot].start]);
        match &rebuilt[source] {
            Some(bytes) => out.extend_from_slice(bytes),
            None => out.extend_from_slice(&xml[items[source].start..items[source].end]),
        }
        cursor = items[slot].end;
    }
    Some(out)
}

/// Which source item fills each slot. `None` ⇒ the identity — every item
/// is already where the model wants it (or the model has no opinion).
///
/// Items the model's list doesn't name keep their slot; the named ones
/// are dealt into the slots they occupy, in model order. That is what
/// makes an unaccounted-for element (no `Self`, a mask artwork, a future
/// kind) harmless rather than a reshuffle hazard.
fn plan_permutation(items: &[Node], order: Option<&[&str]>) -> Option<Vec<usize>> {
    let order = order?;
    let mut queue: Vec<usize> = Vec::new();
    let mut named: HashSet<&str> = HashSet::new();
    for want in order {
        if !named.insert(*want) {
            continue;
        }
        if let Some(at) = items.iter().position(|n| n.id.as_deref() == Some(*want)) {
            queue.push(at);
        }
    }
    let movable: HashSet<usize> = queue.iter().copied().collect();
    let mut next = queue.into_iter();
    let mut perm = Vec::with_capacity(items.len());
    let mut moved = false;
    for slot in 0..items.len() {
        let source = if movable.contains(&slot) {
            next.next().unwrap_or(slot)
        } else {
            slot
        };
        moved |= source != slot;
        perm.push(source);
    }
    moved.then_some(perm)
}

/// Read the emitted XML into a tree of page-item elements with their
/// byte spans.
///
/// Events tile the input exactly (`trim_text(false)`, empty elements not
/// expanded), so `buffer_position` before and after an event bracket its
/// bytes — which is what lets the splice above move a subtree without
/// re-serialising a single tag.
fn scan(xml: &[u8]) -> Result<Vec<Node>, quick_xml::Error> {
    struct Frame {
        node: Node,
        depth: usize,
    }
    fn attach(open: &mut [Frame], roots: &mut Vec<Node>, node: Node) {
        match open.last_mut() {
            Some(parent) => parent.node.children.push(node),
            None => roots.push(node),
        }
    }

    let mut reader = Reader::from_reader(xml);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut buf = Vec::new();
    let mut roots: Vec<Node> = Vec::new();
    let mut open: Vec<Frame> = Vec::new();
    // Start offsets of every open element, page item or not — the last
    // entry is the enclosing element of whatever we are looking at.
    let mut enclosing: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let mut pos = 0usize;
    loop {
        let event = reader.read_event_into(&mut buf)?;
        let after = reader.buffer_position() as usize;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                if is_page_item_name(e.name().as_ref()) {
                    open.push(Frame {
                        node: Node {
                            id: attr_value(&e, b"Self"),
                            is_group: e.name().as_ref() == b"Group",
                            start: pos,
                            end: 0,
                            parent: enclosing.last().copied().unwrap_or(usize::MAX),
                            children: Vec::new(),
                        },
                        depth,
                    });
                }
                enclosing.push(pos);
            }
            Event::End(_) => {
                enclosing.pop();
                if open.last().is_some_and(|f| f.depth == depth) {
                    let mut frame = open.pop().expect("guarded by is_some_and");
                    frame.node.end = after;
                    attach(&mut open, &mut roots, frame.node);
                }
                depth = depth.saturating_sub(1);
            }
            Event::Empty(e) => {
                if is_page_item_name(e.name().as_ref()) {
                    let node = Node {
                        id: attr_value(&e, b"Self"),
                        is_group: e.name().as_ref() == b"Group",
                        start: pos,
                        end: after,
                        parent: enclosing.last().copied().unwrap_or(usize::MAX),
                        children: Vec::new(),
                    };
                    attach(&mut open, &mut roots, node);
                }
            }
            _ => {}
        }
        pos = after;
        buf.clear();
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four rectangles and a group of two, the shape core's v59 wire
    /// test builds (`a_reorder_survives_paged_and_is_lost_on_idml_export`).
    const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Spread Self="sp1" PageCount="1">
    <Page Self="p1" GeometricBounds="0 0 600 600"/>
    <Rectangle Self="a" GeometricBounds="0 0 10 10" FillColor="Color/Black"/>
    <Rectangle Self="b" GeometricBounds="0 0 20 20" FillColor="Color/Black"/>
    <Rectangle Self="c" GeometricBounds="0 0 30 30" FillColor="Color/Black"/>
    <Group Self="grp" ItemTransform="1 0 0 1 0 0">
      <Rectangle Self="g1" GeometricBounds="0 0 40 40" FillColor="Color/Black"/>
      <Rectangle Self="g2" GeometricBounds="0 0 50 50" FillColor="Color/Black"/>
    </Group>
  </Spread>
</idPkg:Spread>"#;

    fn parsed() -> Spread {
        idml_import::parse_spread(SPREAD).expect("parse")
    }

    /// The `Self` ids of the emitted page items, in document order —
    /// nesting flattened, which is enough to read a stacking order off.
    ///
    /// Read with an INDEPENDENT reader rather than [`scan`], so a bug in
    /// the span walk can't hide behind itself.
    fn ids(xml: &[u8]) -> Vec<String> {
        let mut reader = Reader::from_reader(xml);
        reader.config_mut().expand_empty_elements = false;
        let mut buf = Vec::new();
        let mut out = Vec::new();
        loop {
            let event = reader.read_event_into(&mut buf).expect("well-formed");
            match event {
                Event::Eof => break,
                Event::Start(e) | Event::Empty(e) => {
                    if is_page_item_name(e.name().as_ref()) {
                        out.push(attr_value(&e, b"Self").unwrap_or_else(|| "<anon>".to_string()));
                    }
                }
                _ => {}
            }
            buf.clear();
        }
        out
    }

    /// Every event's span is used to reconstruct the file; if they did
    /// not tile the input exactly, every splice below would corrupt it.
    #[test]
    fn the_scan_spans_tile_the_document() {
        let roots = scan(SPREAD).expect("scan");
        // The four top-level items plus the group.
        assert_eq!(roots.len(), 4, "3 rectangles + 1 group at top level");
        for node in &roots {
            let text = String::from_utf8_lossy(&SPREAD[node.start..node.end]).into_owned();
            let id = node.id.clone().expect("Self");
            assert!(text.starts_with('<'), "{text}");
            assert!(text.contains(&format!("Self=\"{id}\"")), "{text}");
            assert!(text.ends_with('>'), "{text}");
        }
        let group = roots.last().expect("group");
        assert!(group.is_group);
        assert_eq!(group.children.len(), 2, "the group's two members");
        assert!(
            String::from_utf8_lossy(&SPREAD[group.start..group.end]).ends_with("</Group>"),
            "a group's span covers its whole subtree"
        );
    }

    /// THE PRIME INVARIANT: an unmutated spread is returned untouched —
    /// the same buffer, not a re-serialisation of it.
    #[test]
    fn an_unmutated_spread_is_returned_byte_identically() {
        let out = apply(&parsed(), SPREAD.to_vec()).expect("reorder");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(SPREAD)
        );
    }

    /// Bring-to-front: the model's z table names `a` last, so its
    /// element moves to the end of the run — carrying its own bytes.
    #[test]
    fn a_top_level_reshuffle_moves_the_element() {
        let mut spread = parsed();
        let a = spread.frames_in_order.remove(0);
        spread.frames_in_order.push(a);
        let out = apply(&spread, SPREAD.to_vec()).expect("reorder");
        let text = String::from_utf8_lossy(&out).into_owned();
        assert_eq!(ids(&out), vec!["b", "c", "grp", "g1", "g2", "a"], "{text}");
        // The moved element keeps every attribute it had.
        assert!(
            text.contains(
                r#"<Rectangle Self="a" GeometricBounds="0 0 10 10" FillColor="Color/Black"/>"#
            ),
            "{text}"
        );
        // Nothing was duplicated or dropped.
        assert_eq!(text.matches("<Rectangle").count(), 5, "{text}");
        assert_eq!(text.matches("<Group").count(), 1, "{text}");
        assert_eq!(
            text.len(),
            SPREAD.len(),
            "a permutation moves bytes, {text}"
        );
    }

    /// A spread-level reshuffle leaves the GROUP's own member order
    /// alone — different sibling list, different run.
    #[test]
    fn a_spread_reshuffle_does_not_touch_group_members() {
        let mut spread = parsed();
        spread.frames_in_order.reverse();
        let out = apply(&spread, SPREAD.to_vec()).expect("reorder");
        let text = String::from_utf8_lossy(&out).into_owned();
        assert_eq!(ids(&out), vec!["grp", "g1", "g2", "c", "b", "a"], "{text}");
        assert!(
            text.find(r#"Self="g1""#) < text.find(r#"Self="g2""#),
            "the group keeps its internal order: {text}"
        );
    }

    /// A reorder INSIDE a group moves only the group's members.
    #[test]
    fn a_group_member_reshuffle_stays_inside_the_group() {
        let mut spread = parsed();
        spread.groups[0].members.reverse();
        let out = apply(&spread, SPREAD.to_vec()).expect("reorder");
        let text = String::from_utf8_lossy(&out).into_owned();
        assert_eq!(ids(&out), vec!["a", "b", "c", "grp", "g2", "g1"], "{text}");
        assert!(
            text.contains("<Group Self=\"grp\" ItemTransform=\"1 0 0 1 0 0\">"),
            "the wrapper is untouched: {text}"
        );
    }

    /// The third sibling list: a B-18 container's pasted-in content
    /// reorders inside the container, and the spread's own order is not
    /// consulted for it.
    #[test]
    fn a_nested_child_reshuffle_stays_inside_its_container() {
        const NESTED: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Spread Self="sp1">
    <Rectangle Self="host" GeometricBounds="0 0 100 100" FillColor="Color/Black">
      <Rectangle Self="n1" GeometricBounds="0 0 10 10" FillColor="Color/Black"/>
      <Rectangle Self="n2" GeometricBounds="0 0 20 20" FillColor="Color/Paper"/>
    </Rectangle>
    <Rectangle Self="sib" GeometricBounds="0 0 30 30" FillColor="Color/Black"/>
  </Spread>
</idPkg:Spread>"#;
        let mut spread = idml_import::parse_spread(NESTED).expect("parse");
        assert_eq!(
            spread.nested_children["host"].len(),
            2,
            "fixture precondition: the parser lifted both children"
        );
        assert_eq!(
            String::from_utf8_lossy(&apply(&spread, NESTED.to_vec()).expect("reorder")),
            String::from_utf8_lossy(NESTED),
            "unmutated stays byte-identical"
        );

        spread
            .nested_children
            .get_mut("host")
            .expect("host")
            .reverse();
        let out = apply(&spread, NESTED.to_vec()).expect("reorder");
        let text = String::from_utf8_lossy(&out).into_owned();
        assert_eq!(ids(&out), vec!["host", "n2", "n1", "sib"], "{text}");
    }

    /// An element the model's list never names keeps its slot, and the
    /// named ones permute around it.
    #[test]
    fn an_unnamed_element_is_pinned_to_its_slot() {
        const WITH_STRAY: &[u8] =
            br#"<Spread Self="s"><Rectangle Self="a"/><Rectangle/><Rectangle Self="b"/></Spread>"#;
        let nodes = scan(WITH_STRAY).expect("scan");
        // `a` and `b` swap; the id-less rectangle must stay in slot 1.
        let perm = plan_permutation(&nodes, Some(&["b", "a"])).expect("a swap");
        assert_eq!(perm, vec![2, 1, 0]);
    }

    /// C-22 and C-23 compose: a document that BOTH gained an item and
    /// reshuffled comes out in one coherent z-order. The new item is
    /// still written by the anchor lane during the streaming pass; this
    /// pass then permutes source and inserted elements together, because
    /// it reads the order off the emitted document rather than the
    /// source one.
    #[test]
    fn an_insert_and_a_reshuffle_compose() {
        let mut spread = parsed();
        let mut fresh = spread.rectangles[0].clone();
        fresh.self_id = Some("new".to_string());
        spread.rectangles.push(fresh);
        let inserted = idml_import::FrameRef::Rectangle(spread.rectangles.len() - 1);
        spread.frames_in_order.insert(1, inserted);
        let a = spread.frames_in_order.remove(0);
        spread.frames_in_order.push(a);

        // Through the REAL writer, so the insert goes through
        // `write_new_item` and the reorder through this pass.
        let out = crate::rewrite::rewrite_spread(SPREAD, &spread).expect("rewrite");
        let text = String::from_utf8_lossy(&out).into_owned();
        assert_eq!(
            ids(&out),
            vec!["new", "b", "c", "grp", "g1", "g2", "a"],
            "{text}"
        );
    }

    /// Guard the empty-z-table rule: a spread whose table never
    /// materialised is skipped rather than reshuffled into kind order.
    #[test]
    fn an_empty_z_table_is_left_alone() {
        let mut spread = parsed();
        spread.frames_in_order.clear();
        let out = apply(&spread, SPREAD.to_vec()).expect("reorder");
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(SPREAD)
        );
    }
}
