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

//! `Resources/Styles.xml` does not grow a second copy of every style.
//!
//! # The defect
//!
//! [`idml_export::resources::patch_styles`] injects the model styles the
//! source part doesn't define. It decided what "doesn't define" means
//! from a `seen` set built AS IT READ, and it injected at the FIRST
//! `</RootParagraphStyleGroup>`. A part with more than one root group of
//! a kind — the corpus's generated packages open with a group holding
//! only InDesign's reserved `$ID/[…]` entry and keep the document's real
//! styles in a SECOND group — therefore had every real style "unseen" at
//! the first close. All of them were injected there; the second group
//! then defined them again.
//!
//! Two elements with the same `Self` in one part, and the injected copy
//! is the LOSSY one: `patch_styles` writes only the handful of fields the
//! model carries, so the duplicate arrives without `NextStyle`,
//! `Justification`, `Hyphenation`, `HyphenationZone`,
//! `BulletsAndNumberingListType`, `AppliedNumberingList` — and, on the
//! object lane, without `StrokeColor`. Which of the two a reader honours
//! is its business, not ours; a save that changed nothing must not have
//! posed the question.
//!
//! Measured over 99 corpus packages, unmutated: 3 `Resources/Styles.xml`
//! entries differed, all three by GROWTH (+1,072 bytes total) — growth
//! being the severe kind of gap, because it means duplicated content.
//!
//! # The shape
//!
//! Same as the story rewrite's range misalignment and the nested-group
//! duplication before it: a single forward pass deciding "is this new?"
//! from whatever it happens to have read so far. Fixed the same way in
//! KIND — establish the whole-part fact in a pre-pass, then emit — though
//! the facts differ (a set of defined ids and a group count here, an
//! element→model map there), so the two share the invariant and not the
//! code.

use idml_export::resources::patch_styles;
use idml_import::{parse_stylesheet, ObjectStyleDef};

/// The real shape from `corpus/generated/styles-cascade.idml`: a first
/// `<RootParagraphStyleGroup>` carrying only the reserved default, and a
/// SECOND one carrying the document's own styles — with exactly the
/// fields `patch_styles` does not model (`NextStyle`, `Justification`,
/// `Hyphenation`, `AppliedNumberingList`) so a duplicate is visibly
/// lossy. Object styles get the same two-group treatment
/// (`corpus/generated/swatches.idml`).
const TWO_GROUP_STYLES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><RootCharacterStyleGroup><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]"/></RootCharacterStyleGroup><RootParagraphStyleGroup><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]" AppliedFont="Open Sans" PointSize="12" FillColor="Color/Black"/></RootParagraphStyleGroup><RootObjectStyleGroup><ObjectStyle Self="ObjectStyle/$ID/[None]" Name="$ID/[None]" FillColor="Swatch/None" StrokeColor="Swatch/None" StrokeWeight="0"/></RootObjectStyleGroup><RootParagraphStyleGroup><ParagraphStyle Self="ParagraphStyle/Title" Name="Title" AppliedFont="Open Sans" PointSize="24" FillColor="Color/Black" NextStyle="ParagraphStyle/Body"/><ParagraphStyle Self="ParagraphStyle/Body" Name="Body" AppliedFont="Open Sans" PointSize="12" FillColor="Color/Black" NextStyle="ParagraphStyle/Body"/><ParagraphStyle Self="ParagraphStyle/JustifiedHyphen" Name="JustifiedHyphen" AppliedFont="Open Sans" PointSize="12" FillColor="Color/Black" Justification="LeftJustified" Hyphenation="true" HyphenationZone="36"/></RootParagraphStyleGroup><RootCharacterStyleGroup><CharacterStyle Self="CharacterStyle/Emphasis" Name="Emphasis" FontStyle="Italic"/></RootCharacterStyleGroup><RootObjectStyleGroup><ObjectStyle Self="ObjectStyle/Base" Name="Base" FillColor="Color/InkFull" StrokeColor="Swatch/None" StrokeWeight="0"/><ObjectStyle Self="ObjectStyle/Derived" Name="Derived" BasedOn="ObjectStyle/Base" StrokeColor="Swatch/None" StrokeWeight="0"/></RootObjectStyleGroup></idPkg:Styles>"#;

/// How many elements named `<Tag Self="id"` the part carries.
fn definitions(xml: &str, tag: &str, id: &str) -> usize {
    xml.match_indices(&format!("<{tag} Self=\"{id}\"")).count()
}

/// Every `Self="…"` value on a `<Tag …>` element, in document order.
fn ids_of(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag} ");
    xml.match_indices(&open)
        .filter_map(|(i, _)| {
            let rest = &xml[i + open.len()..];
            let key = "Self=\"";
            let at = rest.find(key)? + key.len();
            let end = rest[at..].find('"')?;
            Some(rest[at..at + end].to_string())
        })
        .collect()
}

/// The premise, asserted rather than assumed: the fixture really does
/// carry two root groups per kind with the real styles in the SECOND,
/// and the parser really does load them. A single-group fixture would
/// make every assertion below vacuous.
#[test]
fn premise_the_part_has_two_root_groups_with_the_real_styles_in_the_second() {
    let xml = String::from_utf8_lossy(TWO_GROUP_STYLES).into_owned();
    assert_eq!(xml.matches("<RootParagraphStyleGroup>").count(), 2);
    assert_eq!(xml.matches("<RootCharacterStyleGroup>").count(), 2);
    assert_eq!(xml.matches("<RootObjectStyleGroup>").count(), 2);
    let first_close = xml.find("</RootParagraphStyleGroup>").expect("first close");
    let title_at = xml.find(r#"Self="ParagraphStyle/Title""#).expect("Title");
    assert!(
        title_at > first_close,
        "the document's own styles must live AFTER the first group close — \
         that is the whole-part fact the forward pass could not have"
    );

    let styles = parse_stylesheet(TWO_GROUP_STYLES).expect("parse");
    assert!(styles.paragraph_styles.contains_key("ParagraphStyle/Title"));
    assert!(styles
        .character_styles
        .contains_key("CharacterStyle/Emphasis"));
    assert!(styles.object_styles.contains_key("ObjectStyle/Base"));
}

/// THE PRIME INVARIANT: an unmutated style sheet re-emits its on-disk
/// bytes, so `write_idml` takes the raw-copy path for the entry.
#[test]
fn an_unmutated_two_group_style_sheet_round_trips_byte_identically() {
    let styles = parse_stylesheet(TWO_GROUP_STYLES).expect("parse");
    let out = patch_styles(TWO_GROUP_STYLES, &styles).expect("patch");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(TWO_GROUP_STYLES),
        "a style sheet nobody mutated must reproduce its on-disk bytes"
    );
}

/// THE DEFECT, named directly: no `Self` id may be defined twice, on any
/// of the three lanes.
#[test]
fn no_style_is_defined_twice() {
    let styles = parse_stylesheet(TWO_GROUP_STYLES).expect("parse");
    let out = patch_styles(TWO_GROUP_STYLES, &styles).expect("patch");
    let xml = String::from_utf8(out).expect("utf8");

    for (tag, ids) in [
        ("ParagraphStyle", ids_of(&xml, "ParagraphStyle")),
        ("CharacterStyle", ids_of(&xml, "CharacterStyle")),
        ("ObjectStyle", ids_of(&xml, "ObjectStyle")),
    ] {
        let mut sorted = ids.clone();
        sorted.sort();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            before,
            "a <{tag}> Self id is defined more than once:\n{ids:?}\n{xml}"
        );
    }
}

/// The injected duplicate was also LOSSY — this is what a user would
/// actually have lost. Asserted per attribute so a regression names the
/// field it dropped.
#[test]
fn the_fields_the_writer_does_not_model_survive() {
    let styles = parse_stylesheet(TWO_GROUP_STYLES).expect("parse");
    let out = patch_styles(TWO_GROUP_STYLES, &styles).expect("patch");
    let xml = String::from_utf8(out).expect("utf8");
    for attr in [
        r#"NextStyle="ParagraphStyle/Body""#,
        r#"Justification="LeftJustified""#,
        r#"Hyphenation="true""#,
        r#"HyphenationZone="36""#,
        r#"StrokeColor="Swatch/None""#,
    ] {
        assert!(
            xml.contains(attr),
            "{attr} must survive — the injected copy is the one that lacks it:\n{xml}"
        );
    }
    assert_eq!(
        definitions(&xml, "ParagraphStyle", "ParagraphStyle/Title"),
        1
    );
    assert_eq!(definitions(&xml, "ObjectStyle", "ObjectStyle/Base"), 1);
}

/// The pre-pass is a de-duplication, not a freeze: a style created since
/// load is still written — exactly once, and inside a root group.
#[test]
fn a_newly_created_style_is_still_injected_exactly_once() {
    let mut styles = parse_stylesheet(TWO_GROUP_STYLES).expect("parse");
    let def = ObjectStyleDef {
        self_id: "ObjectStyle/u0".to_string(),
        name: Some("Sidebar".to_string()),
        based_on: Some("ObjectStyle/Base".to_string()),
        ..Default::default()
    };
    styles.object_styles.insert(def.self_id.clone(), def);

    let out = patch_styles(TWO_GROUP_STYLES, &styles).expect("patch");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        definitions(&xml, "ObjectStyle", "ObjectStyle/u0"),
        1,
        "the new style must be defined once:\n{xml}"
    );
    // ...inside the LAST object group, which is where the document keeps
    // its own object styles.
    let last_group_open = xml.rfind("<RootObjectStyleGroup>").expect("group open");
    let def_at = xml.find(r#"Self="ObjectStyle/u0""#).expect("emitted");
    assert!(
        def_at > last_group_open,
        "a new style belongs next to the document's own, not in the \
         reserved-defaults group:\n{xml}"
    );
}

/// The paragraph lane's version of the same: a created paragraph style
/// lands once, in the last group.
#[test]
fn a_newly_created_paragraph_style_lands_once_in_the_last_group() {
    let mut styles = parse_stylesheet(TWO_GROUP_STYLES).expect("parse");
    let mut def = styles
        .paragraph_styles
        .get("ParagraphStyle/Body")
        .expect("Body")
        .clone();
    def.self_id = "ParagraphStyle/Caption".to_string();
    def.name = Some("Caption".to_string());
    styles
        .paragraph_styles
        .insert(def.self_id.clone(), def.clone());

    let out = patch_styles(TWO_GROUP_STYLES, &styles).expect("patch");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        definitions(&xml, "ParagraphStyle", "ParagraphStyle/Caption"),
        1,
        "{xml}"
    );
    let last_group_open = xml.rfind("<RootParagraphStyleGroup>").expect("group open");
    let def_at = xml
        .find(r#"Self="ParagraphStyle/Caption""#)
        .expect("emitted");
    assert!(def_at > last_group_open, "{xml}");
    // Nothing else duplicated on the way.
    assert_eq!(
        definitions(&xml, "ParagraphStyle", "ParagraphStyle/Title"),
        1
    );
}
