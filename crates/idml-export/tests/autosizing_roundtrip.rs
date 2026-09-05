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

//! Auto-sizing on an EXISTING frame reaches its `<TextFramePreference>`
//! (measured on InDesign 20.0.1: exactly that element is honoured; a
//! frame without it is reported overset).

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{AutoSizingReferencePoint, AutoSizingType};

#[test]
fn auto_sizing_set_on_a_source_frame_is_written_and_reads_back() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    let f = &mut doc.spreads[0].spread.text_frames[0];
    assert!(f.auto_sizing.is_none());
    f.auto_sizing = Some(AutoSizingType::HeightOnly);
    f.auto_sizing_reference_point = Some(AutoSizingReferencePoint::TopCenterPoint);

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    let frame = pkg::pos(&xml, "<TextFrame Self=\"tf1\"");
    let pref = pkg::pos(
        &xml,
        r#"<TextFramePreference AutoSizingType="HeightOnly" AutoSizingReferencePoint="TopCenterPoint"/>"#,
    );
    let end = pkg::pos(&xml, "</TextFrame>");
    assert!(frame < pref && pref < end, "inside the frame:\n{xml}");
    let re = pkg::open(&out);
    let f = &re.spreads[0].spread.text_frames[0];
    assert_eq!(f.auto_sizing, Some(AutoSizingType::HeightOnly));
    assert_eq!(
        f.auto_sizing_reference_point,
        Some(AutoSizingReferencePoint::TopCenterPoint)
    );
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

#[test]
fn an_existing_preference_is_patched_in_place_and_kept_when_equal() {
    let spread = pkg::SPREAD.replace(
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"><TextFramePreference TextColumnCount="2" AutoSizingType="WidthOnly" AutoSizingReferencePoint="TopLeftPoint" /></TextFrame>"#,
    );
    let source = pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ]);
    let doc = pkg::open(&source);
    assert_eq!(
        doc.spreads[0].spread.text_frames[0].auto_sizing,
        Some(AutoSizingType::WidthOnly)
    );
    assert_eq!(
        source,
        write_idml(&doc, &source).expect("write"),
        "unchanged: byte-identical"
    );

    let mut doc = doc;
    doc.spreads[0].spread.text_frames[0].auto_sizing = Some(AutoSizingType::HeightAndWidth);
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<TextFramePreference TextColumnCount="2" AutoSizingType="HeightAndWidth" AutoSizingReferencePoint="TopLeftPoint"/>"#),
        "patched in place, other attributes kept:\n{xml}"
    );
    assert_eq!(
        pkg::open(&out).spreads[0].spread.text_frames[0].auto_sizing,
        Some(AutoSizingType::HeightAndWidth)
    );
}
