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

//! A text frame's layout options survive an export: its columns and
//! their gutter, column balance, vertical justification, the
//! first-baseline policy and its minimum, and ignore-wrap — the
//! `<TextFramePreference>` attributes the importer reads and the engine
//! lays out.
//!
//! None of them was written. A two-column frame, a balanced one, a
//! vertically justified one and every first-baseline policy came back
//! from an export as one top-aligned column set from the ascent, on a
//! frame the export minted and on a frame the source already carried.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, FirstBaselineOffset, Paragraph, Story, VerticalJustification};

/// Every layout option, set to something other than InDesign's default.
fn lay_out(f: &mut idml_import::TextFrame, fbo: FirstBaselineOffset) {
    f.column_count = Some(2);
    f.column_gutter = Some(10.0);
    f.column_balance = Some(true);
    f.vertical_justification = Some(VerticalJustification::Justify);
    f.first_baseline_offset = Some(fbo);
    f.minimum_first_baseline_offset = Some(14.5);
    f.ignore_wrap = Some(true);
}

fn assert_laid_out(f: &idml_import::TextFrame, fbo: FirstBaselineOffset) {
    assert_eq!(f.column_count, Some(2), "column count");
    assert_eq!(f.column_gutter, Some(10.0), "gutter");
    assert_eq!(f.column_balance, Some(true), "balance");
    assert_eq!(
        f.vertical_justification,
        Some(VerticalJustification::Justify),
        "vertical justification"
    );
    assert_eq!(f.first_baseline_offset, Some(fbo), "first baseline");
    assert_eq!(f.minimum_first_baseline_offset, Some(14.5), "its minimum");
    assert_eq!(f.ignore_wrap, Some(true), "ignore wrap");
}

fn frame<'a>(doc: &'a paged_scene::Document, id: &str) -> &'a idml_import::TextFrame {
    doc.spreads[0]
        .spread
        .text_frames
        .iter()
        .find(|f| f.self_id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("frame {id}"))
}

/// A frame the export mints, once per first-baseline policy.
#[test]
fn a_minted_frame_keeps_its_layout_options() {
    for fbo in [
        FirstBaselineOffset::AscentOffset,
        FirstBaselineOffset::CapHeight,
        FirstBaselineOffset::XHeight,
        FirstBaselineOffset::EmBoxHeight,
        FirstBaselineOffset::LeadingOffset,
        FirstBaselineOffset::FixedHeight,
    ] {
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
        lay_out(doc.spreads[0].spread.text_frames.last_mut().unwrap(), fbo);
        let out = write_idml(&doc, &source).expect("write");
        let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
        assert!(
            xml.contains(r#"VerticalJustification="JustifyAlign""#),
            "InDesign's enumerator:\n{xml}"
        );
        assert_laid_out(frame(&pkg::open(&out), "tf_u9"), fbo);
    }
}

/// The em-box policy is `EmboxHeight` in InDesign's own export; it reads
/// the wire's `EmBoxHeight` as `AscentOffset`.
#[test]
fn the_em_box_policy_is_spelled_the_way_indesign_reads_it() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.spreads[0].spread.text_frames[0].first_baseline_offset =
        Some(FirstBaselineOffset::EmBoxHeight);
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"FirstBaselineOffset="EmboxHeight""#) && !xml.contains("EmBoxHeight"),
        "{xml}"
    );
}

/// A SOURCE frame with no preference at all gains one carrying the
/// options the model set on it.
#[test]
fn a_source_frame_gains_the_options_the_model_set() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    lay_out(
        &mut doc.spreads[0].spread.text_frames[0],
        FirstBaselineOffset::CapHeight,
    );
    let out = write_idml(&doc, &source).expect("write");
    assert_laid_out(
        frame(&pkg::open(&out), "tf1"),
        FirstBaselineOffset::CapHeight,
    );
}

/// A source frame's own preference is patched in place: the options the
/// model changed are rewritten, the ones it did not keep their bytes,
/// and the preference's other attributes and children survive.
#[test]
fn a_source_preference_is_patched_in_place() {
    let spread = pkg::SPREAD.replace(
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"><TextFramePreference TextColumnCount="1" TextColumnGutter="12.000" TextColumnFixedWidth="300" FirstBaselineOffset="AscentOffset" MinimumFirstBaselineOffset="0" VerticalJustification="TopAlign" VerticalThreshold="0" IgnoreWrap="false" VerticalBalanceColumns="false"><Properties><InsetSpacing type="unit">4</InsetSpacing></Properties></TextFramePreference></TextFrame>"#,
    );
    let source = pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ]);
    let mut doc = pkg::open(&source);
    {
        let f = &mut doc.spreads[0].spread.text_frames[0];
        assert_eq!(f.column_gutter, Some(12.0));
        f.column_count = Some(3);
        f.vertical_justification = Some(VerticalJustification::Bottom);
    }
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<TextFramePreference TextColumnCount="3" TextColumnGutter="12.000" TextColumnFixedWidth="300" FirstBaselineOffset="AscentOffset" MinimumFirstBaselineOffset="0" VerticalJustification="BottomAlign" VerticalThreshold="0" IgnoreWrap="false" VerticalBalanceColumns="false"><Properties><InsetSpacing type="unit">4</InsetSpacing></Properties></TextFramePreference>"#),
        "only the changed options move:\n{xml}"
    );
    let back = pkg::open(&out);
    let f = frame(&back, "tf1");
    assert_eq!(f.column_count, Some(3));
    assert_eq!(
        f.vertical_justification,
        Some(VerticalJustification::Bottom)
    );
    assert_eq!(f.inset_spacing, Some([4.0; 4]));

    // And an unmutated save of that package changes nothing.
    let doc = pkg::open(&source);
    pkg::assert_same_package(&source, &write_idml(&doc, &source).expect("write"));
}

/// A part an older export wrote with the wire's `EmBoxHeight` is
/// rewritten in InDesign's spelling on the next save.
#[test]
fn an_older_em_box_spelling_is_normalised_on_save() {
    let spread = pkg::SPREAD.replace(
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"><TextFramePreference FirstBaselineOffset="EmBoxHeight"/></TextFrame>"#,
    );
    let source = pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ]);
    let doc = pkg::open(&source);
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<TextFramePreference FirstBaselineOffset="EmboxHeight"/>"#),
        "{xml}"
    );
}
