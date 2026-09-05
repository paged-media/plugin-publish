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

//! Conditional-text save-back in the spelling InDesign 20.0.1 was
//! MEASURED to read (2026-09-05): `<Condition>` / `<ConditionSet>` as
//! direct children of `<Document>` in `designmap.xml`, the indicator
//! colour and the set members as typed Properties children, followed by
//! a `<ConditionalTextPreference/>`. The engine's older spelling — inside
//! `Resources/Styles.xml`, wrapped in an invented
//! `<RootConditionalTextGroup>` that hides everything from InDesign, the
//! colour as an attribute, the members as a `Conditions="a b"` attribute
//! that makes every set capture ALL conditions — is normalised on export.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const OLD_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]"/></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]"/></RootParagraphStyleGroup>
<RootConditionalTextGroup><Condition Self="Condition/Draft" Name="Draft" Visible="true" IndicatorMethod="UseHighlight" IndicatorColor="Yellow"/><Condition Self="Condition/Print" Name="Print-only" Visible="false" IndicatorMethod="UseUnderline" IndicatorColor="Green"/><ConditionSet Self="ConditionSet/Review" Name="Review" Conditions="Condition/Draft Condition/Print"/></RootConditionalTextGroup>
</idPkg:Styles>"#;

const CANONICAL_BLOCK: &str = r#"<Condition Self="Condition/Draft" Name="Draft" IndicatorMethod="UseHighlight" Visible="true"><Properties><IndicatorColor type="enumeration">Yellow</IndicatorColor></Properties></Condition><Condition Self="Condition/Print" Name="Print-only" IndicatorMethod="UseUnderline" Visible="false"><Properties><IndicatorColor type="enumeration">Green</IndicatorColor></Properties></Condition><ConditionSet Self="ConditionSet/Review" Name="Review"><Properties><SetConditions><VisibilityPair Condition="Condition/Draft" Visibility="true"/><VisibilityPair Condition="Condition/Print" Visibility="true"/></SetConditions></Properties></ConditionSet><ConditionalTextPreference ShowConditionIndicators="ShowIndicators" ActiveConditionSet="n"/>"#;

fn check_model(doc: &paged_scene::Document) {
    assert_eq!(
        doc.styles.conditions.len(),
        2,
        "{:?}",
        doc.styles.conditions
    );
    let draft = &doc.styles.conditions["Condition/Draft"];
    assert_eq!(draft.name.as_deref(), Some("Draft"));
    assert_eq!(draft.visible, Some(true));
    assert_eq!(draft.indicator_method.as_deref(), Some("UseHighlight"));
    assert_eq!(
        doc.styles.conditions["Condition/Print"].visible,
        Some(false)
    );
    let set = &doc.styles.condition_sets["ConditionSet/Review"];
    assert_eq!(
        set.conditions,
        vec!["Condition/Draft".to_string(), "Condition/Print".to_string()]
    );
}

#[test]
fn the_wrapper_spelling_reads_and_is_moved_to_the_designmap_on_export() {
    let source = pkg::simple("", pkg::STORY_BODY, OLD_STYLES);
    let doc = pkg::open(&source);
    check_model(&doc);

    let out = write_idml(&doc, &source).expect("write");
    let styles = pkg::entry(&out, "Resources/Styles.xml").expect("styles");
    assert!(
        !styles.contains("Condition"),
        "conditions left Styles.xml:\n{styles}"
    );
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert!(dm.contains(CANONICAL_BLOCK), "canonical block:\n{dm}");
    // After the layers, before the spread includes.
    let layer = pkg::pos(&dm, "<Layer ");
    let block = pkg::pos(&dm, "<Condition Self=\"Condition/Draft\"");
    let spread = pkg::pos(&dm, "<idPkg:Spread ");
    assert!(layer < block && block < spread, "{dm}");

    let re = pkg::open(&out);
    check_model(&re);
    assert_eq!(
        out,
        write_idml(&re, &out).expect("re-write"),
        "now canonical: byte-identical"
    );
}

#[test]
fn a_canonical_source_round_trips_byte_identically_and_a_flip_is_patched() {
    let source = pkg::simple(
        &format!("{CANONICAL_BLOCK}\n"),
        pkg::STORY_BODY,
        pkg::STYLES,
    );
    let doc = pkg::open(&source);
    check_model(&doc);
    assert_eq!(source, write_idml(&doc, &source).expect("write"));

    let mut flipped = doc;
    flipped
        .styles
        .conditions
        .get_mut("Condition/Draft")
        .unwrap()
        .visible = Some(false);
    let out = write_idml(&flipped, &source).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert!(dm.contains(r#"<Condition Self="Condition/Draft" Name="Draft" IndicatorMethod="UseHighlight" Visible="false"><Properties><IndicatorColor type="enumeration">Yellow</IndicatorColor></Properties></Condition>"#), "patched in place, colour kept:\n{dm}");
    assert_eq!(
        pkg::open(&out).styles.conditions["Condition/Draft"].visible,
        Some(false)
    );
}

#[test]
fn the_unwrapped_styles_spelling_reads_too() {
    let styles = OLD_STYLES
        .replace("<RootConditionalTextGroup>", "")
        .replace("</RootConditionalTextGroup>", "");
    let source = pkg::simple("", pkg::STORY_BODY, &styles);
    check_model(&pkg::open(&source));
}
