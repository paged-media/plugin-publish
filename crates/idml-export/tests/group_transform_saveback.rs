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

//! A group's own `ItemTransform` saves back.
//!
//! The writer never patched a SOURCE `<Group>`'s `ItemTransform` from the
//! model and re-based the group's members against the source value. The
//! members therefore reopened where they were drawn, but a group moved or
//! turned in the editor reopened with its old frame: its selection box,
//! and every transform InDesign applies to it as a unit, sat where the
//! group had been.
//!
//! The model keeps a group's transform relative to the group holding it,
//! as IDML does, and a member's composed into spread space. So a group
//! moved as a unit changes ONE attribute in the file: its own
//! `ItemTransform`. Its members' on-disk transforms (relative to the
//! group) and any nested group's are what they were, and keep their
//! bytes; a group nobody touched keeps all of them.

use idml_export::rewrite::rewrite_spread;
use idml_import::{parse_spread, FrameRef, Spread};

type M = [f32; 6];

const SOURCE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<Group Self="go" ItemTransform="1 0 0 1 100 100">
<Rectangle Self="lo" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 20 20" FillColor="Color/Black"/>
<Group Self="gi" ItemTransform="0.8660254 0.5 -0.5 0.8660254 50 0">
<Rectangle Self="li" ItemTransform="1 0 0 1 5 0" GeometricBounds="0 0 20 40" FillColor="Color/Black"/>
</Group>
</Group>
<Group Self="gu" ItemTransform="1 0 0 1 300 300">
<Oval Self="ou" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 10 10" FillColor="Color/Black"/>
</Group>
</Spread>
</idPkg:Spread>"#;

fn mul(a: M, b: M) -> M {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

fn inv(m: M) -> M {
    let [a, b, c, d, tx, ty] = m;
    let det = a * d - b * c;
    let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
    [ia, ib, ic, id, -(ia * tx + ic * ty), -(ib * tx + id * ty)]
}

fn near(a: M, b: M, what: &str) {
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() < 1e-3, "{what}: {a:?} vs {b:?}");
    }
}

fn group_idx(s: &Spread, id: &str) -> usize {
    s.groups
        .iter()
        .position(|g| g.self_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("group {id}"))
}

fn group(s: &Spread, id: &str) -> Option<M> {
    s.groups[group_idx(s, id)].item_transform
}

/// Every leaf's composed transform, by id.
fn leaves(s: &Spread) -> Vec<(String, M)> {
    let id = |x: &Option<String>| x.clone().unwrap_or_default();
    let mut out: Vec<(String, M)> = s
        .rectangles
        .iter()
        .map(|r| (id(&r.self_id), r.item_transform.unwrap_or(I)))
        .chain(
            s.ovals
                .iter()
                .map(|o| (id(&o.self_id), o.item_transform.unwrap_or(I))),
        )
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

const I: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// What the engine's `SetGroupTransform` leaves in the model: the group's
/// own transform set, every leaf below it rebased by the spread-space
/// delta, nested groups' (parent-relative) transforms untouched.
fn set_group_transform(s: &mut Spread, id: &str, new: M) {
    let gi = group_idx(s, id);
    // The group's parent frame (the groups holding it, outermost first).
    let mut frame = I;
    let mut cur = gi;
    while let Some(p) = s
        .groups
        .iter()
        .position(|g| g.members.contains(&FrameRef::Group(cur)))
    {
        frame = mul(s.groups[p].item_transform.unwrap_or(I), frame);
        cur = p;
    }
    let old = s.groups[gi].item_transform.unwrap_or(I);
    let delta = mul(frame, mul(new, mul(inv(old), inv(frame))));
    fn walk(s: &Spread, gi: usize, out: &mut Vec<FrameRef>) {
        for m in &s.groups[gi].members {
            match *m {
                FrameRef::Group(c) => walk(s, c, out),
                leaf => out.push(leaf),
            }
        }
    }
    let mut members = Vec::new();
    walk(s, gi, &mut members);
    for m in members {
        let t = match m {
            FrameRef::Rectangle(i) => &mut s.rectangles[i].item_transform,
            FrameRef::Oval(i) => &mut s.ovals[i].item_transform,
            other => panic!("unexpected member {other:?}"),
        };
        *t = Some(mul(delta, t.unwrap_or(I)));
    }
    s.groups[gi].item_transform = Some(new);
}

/// The start tag of the element with `Self="id"`.
fn tag<'a>(xml: &'a str, id: &str) -> &'a str {
    let at = xml
        .find(&format!(r#"Self="{id}""#))
        .unwrap_or_else(|| panic!("{id} in {xml}"));
    let start = xml[..at].rfind('<').expect("tag start");
    let end = at + xml[at..].find('>').expect("tag end") + 1;
    &xml[start..end]
}

fn source() -> &'static str {
    std::str::from_utf8(SOURCE).unwrap()
}

#[test]
fn an_unmutated_spread_round_trips_byte_identically() {
    let spread = parse_spread(SOURCE).expect("parse");
    let out = rewrite_spread(SOURCE, &spread).expect("rewrite");
    assert_eq!(out, SOURCE, "{}", String::from_utf8_lossy(&out));
}

/// The outer group turned 90° about its origin: its own `ItemTransform`
/// is the one attribute that changes, and the file reopens with both
/// groups and every leaf where they were drawn.
#[test]
fn a_turned_outer_group_saves_its_own_transform_and_nothing_else() {
    let mut spread = parse_spread(SOURCE).expect("parse");
    let gi_before = group(&spread, "gi");
    set_group_transform(&mut spread, "go", [0.0, 1.0, -1.0, 0.0, 100.0, 100.0]);
    let drawn = leaves(&spread);

    let out = rewrite_spread(SOURCE, &spread).expect("rewrite");
    let xml = String::from_utf8(out.clone()).expect("utf8");
    assert_eq!(
        tag(&xml, "go"),
        r#"<Group Self="go" ItemTransform="0 1 -1 0 100 100">"#
    );
    // Patch, don't regenerate: everything else keeps its source bytes —
    // the nested group, the members (relative to their groups, which
    // they still are), and the untouched group.
    for id in ["gi", "lo", "li", "gu", "ou"] {
        assert_eq!(tag(&xml, id), tag(source(), id), "{id} keeps its bytes");
    }
    assert_eq!(
        xml.replace(
            r#"ItemTransform="0 1 -1 0 100 100""#,
            r#"ItemTransform="1 0 0 1 100 100""#
        ),
        source(),
        "the one changed attribute is the whole diff"
    );

    let back = parse_spread(&out).expect("reopen");
    near(
        group(&back, "go").unwrap(),
        [0.0, 1.0, -1.0, 0.0, 100.0, 100.0],
        "go",
    );
    near(group(&back, "gi").unwrap(), gi_before.unwrap(), "gi");
    for ((id, want), (id2, got)) in drawn.iter().zip(leaves(&back)) {
        assert_eq!(*id, id2);
        near(got, *want, id);
    }
}

/// A nested group moved inside its parent: its own transform is written,
/// relative to the parent, and the parent keeps its bytes.
#[test]
fn a_moved_nested_group_saves_relative_to_its_parent() {
    let mut spread = parse_spread(SOURCE).expect("parse");
    let mut moved = group(&spread, "gi").unwrap();
    moved[4] += 10.0;
    set_group_transform(&mut spread, "gi", moved);
    let drawn = leaves(&spread);

    let out = rewrite_spread(SOURCE, &spread).expect("rewrite");
    let xml = String::from_utf8(out.clone()).expect("utf8");
    assert_eq!(
        tag(&xml, "gi"),
        r#"<Group Self="gi" ItemTransform="0.866 0.5 -0.5 0.866 60 0">"#
    );
    for id in ["go", "lo", "li", "gu", "ou"] {
        assert_eq!(tag(&xml, id), tag(source(), id), "{id} keeps its bytes");
    }
    let back = parse_spread(&out).expect("reopen");
    let gi = group(&back, "gi").unwrap();
    assert!((gi[4] - 60.0).abs() < 1e-3, "{gi:?}");
    for ((id, want), (_, got)) in drawn.iter().zip(leaves(&back)) {
        near(got, *want, id);
    }
}

/// The model released `gi` from `go` (an ungroup, which does not save
/// back: the file keeps the `go` element). `gi`'s transform is then
/// relative to the spread in the model but written inside `go`, so it is
/// re-based against `go` — which gives back exactly its source spelling.
#[test]
fn a_group_the_model_released_from_its_wrapper_keeps_its_place_inside_it() {
    let mut spread = parse_spread(SOURCE).expect("parse");
    let go = group_idx(&spread, "go");
    let gi = group_idx(&spread, "gi");
    let go_t = spread.groups[go].item_transform;
    let gi_t = spread.groups[gi].item_transform;
    // The dissolve: `gi` takes `go`'s transform folded in, `go` leaves
    // the model, its members take its slot in the z-table.
    spread.groups[gi].item_transform = Some(mul(go_t.unwrap(), gi_t.unwrap()));
    let members = spread.groups[go].members.clone();
    spread.groups.remove(go);
    let fix = |r: FrameRef| match r {
        FrameRef::Group(j) if j > go => FrameRef::Group(j - 1),
        other => other,
    };
    let slot = spread
        .frames_in_order
        .iter()
        .position(|r| *r == FrameRef::Group(go))
        .expect("go is top level");
    spread.frames_in_order.remove(slot);
    for (k, m) in members.into_iter().enumerate() {
        spread.frames_in_order.insert(slot + k, fix(m));
    }
    for r in spread.frames_in_order.iter_mut() {
        *r = fix(*r);
    }
    for g in spread.groups.iter_mut() {
        for r in g.members.iter_mut() {
            *r = fix(*r);
        }
    }

    let out = rewrite_spread(SOURCE, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(tag(&xml, "gi"), tag(source(), "gi"), "{xml}");
}
