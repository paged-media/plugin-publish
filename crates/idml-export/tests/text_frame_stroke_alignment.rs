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

//! A text frame's `StrokeAlignment` reaches the model and comes back out.
//!
//! On a text frame it decides where the stroke paints AND how far the
//! stroke insets the text (InDesign 20.0.1, core's `stroke-inset`
//! fixture: half the weight centred, all of it inside, none outside). It
//! was read only on rectangles, ovals and polygons, so an inside- or
//! outside-stroked text frame composed and painted as centred.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story};

const BARE: &str = r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#;

/// A package whose one text frame carries `StrokeAlignment="{align}"`
/// (spelled after the stroke, where InDesign writes it).
fn package_with(align: Option<&str>) -> Vec<u8> {
    let frame = match align {
        Some(a) => format!(
            r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0" StrokeColor="Color/Black" StrokeWeight="6" StrokeAlignment="{a}"/>"#
        ),
        None => BARE.to_string(),
    };
    assert!(
        pkg::SPREAD.contains(BARE),
        "the fixture frame is still spelled so"
    );
    let spread = pkg::SPREAD.replace(BARE, &frame);
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

#[test]
fn each_alignment_is_read_off_the_text_frame() {
    for a in ["CenterAlignment", "InsideAlignment", "OutsideAlignment"] {
        let doc = pkg::open(&package_with(Some(a)));
        assert_eq!(
            doc.spreads[0].spread.text_frames[0]
                .stroke_alignment
                .as_deref(),
            Some(a)
        );
    }
    let doc = pkg::open(&package_with(None));
    assert_eq!(doc.spreads[0].spread.text_frames[0].stroke_alignment, None);
}

#[test]
fn an_unmutated_save_is_byte_identical() {
    for a in [Some("InsideAlignment"), Some("OutsideAlignment"), None] {
        let source = package_with(a);
        let doc = pkg::open(&source);
        let out = write_idml(&doc, &source).expect("write");
        pkg::assert_same_package(&source, &out);
    }
}

#[test]
fn a_changed_alignment_is_written_in_place() {
    let source = package_with(Some("InsideAlignment"));
    let mut doc = pkg::open(&source);
    doc.spreads[0].spread.text_frames[0].stroke_alignment = Some("OutsideAlignment".into());
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"StrokeWeight="6" StrokeAlignment="OutsideAlignment"/>"#),
        "patched where the source spelled it:\n{xml}"
    );
    assert!(!xml.contains("InsideAlignment"), "{xml}");
    let re = pkg::open(&out);
    assert_eq!(
        re.spreads[0].spread.text_frames[0]
            .stroke_alignment
            .as_deref(),
        Some("OutsideAlignment")
    );
}

#[test]
fn an_alignment_the_source_never_spelled_is_appended() {
    let source = package_with(None);
    let mut doc = pkg::open(&source);
    doc.spreads[0].spread.text_frames[0].stroke_alignment = Some("InsideAlignment".into());
    let out = write_idml(&doc, &source).expect("write");
    let re = pkg::open(&out);
    assert_eq!(
        re.spreads[0].spread.text_frames[0]
            .stroke_alignment
            .as_deref(),
        Some("InsideAlignment")
    );
}

#[test]
fn an_alignment_cleared_to_the_default_is_dropped() {
    let source = package_with(Some("InsideAlignment"));
    let mut doc = pkg::open(&source);
    doc.spreads[0].spread.text_frames[0].stroke_alignment = None;
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(!xml.contains("StrokeAlignment"), "{xml}");
}

#[test]
fn a_minted_text_frame_writes_its_alignment() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                runs: vec![CharacterRun {
                    text: "minted".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        },
    });
    pkg::place_story(&mut doc, "tf_u9", "Story/u9");
    doc.spreads[0]
        .spread
        .text_frames
        .last_mut()
        .unwrap()
        .stroke_alignment = Some("OutsideAlignment".into());
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"StrokeAlignment="OutsideAlignment""#),
        "{xml}"
    );
    let re = pkg::open(&out);
    let minted = re.spreads[0]
        .spread
        .text_frames
        .iter()
        .find(|f| f.self_id.as_deref() == Some("tf_u9"))
        .expect("minted frame");
    assert_eq!(minted.stroke_alignment.as_deref(), Some("OutsideAlignment"));
}
