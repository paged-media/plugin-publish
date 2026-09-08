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

//! A text frame's insets survive the export in the spelling InDesign
//! reads.
//!
//! Measured on InDesign 20.0.1: asked to open an IDML that carries
//! `<TextFramePreference InsetSpacing="4 4 4 4">`, InDesign reports
//! `textFramePreferences.insetSpacing = [0]` — the attribute is ignored
//! outright. Across 271 real-world packages InDesign writes the
//! attribute form zero times, against 13,893 `type="list"` children and
//! 45 `type="unit"` ones.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Paragraph, Story};

/// Mint a text frame carrying `insets`, and return the exported spread.
fn export_with_minted_insets(insets: [f32; 4]) -> (Vec<u8>, String) {
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
        .inset_spacing = Some(insets);
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    (out, xml)
}

#[test]
fn a_uniform_inset_is_written_as_the_scalar_child_indesign_reads() {
    let (out, xml) = export_with_minted_insets([6.0; 4]);
    assert!(
        xml.contains(r#"<InsetSpacing type="unit">6</InsetSpacing>"#),
        "scalar child, not an attribute:\n{xml}"
    );
    assert!(
        !xml.contains("InsetSpacing=\""),
        "the attribute spelling InDesign ignores must not appear:\n{xml}"
    );
    let re = pkg::open(&out);
    let minted = re.spreads[0]
        .spread
        .text_frames
        .iter()
        .find(|f| f.self_id.as_deref() == Some("tf_u9"))
        .expect("minted frame");
    assert_eq!(minted.inset_spacing, Some([6.0; 4]));
}

#[test]
fn four_different_insets_are_written_as_a_typed_list() {
    let (out, xml) = export_with_minted_insets([6.0, 8.0, 10.0, 12.0]);
    assert!(
        xml.contains(r#"<InsetSpacing type="list">"#),
        "list child:\n{xml}"
    );
    for v in ["6", "8", "10", "12"] {
        assert!(
            xml.contains(&format!(r#"<ListItem type="unit">{v}</ListItem>"#)),
            "IDML order is top, left, bottom, right — missing {v}:\n{xml}"
        );
    }
    let re = pkg::open(&out);
    let minted = re.spreads[0]
        .spread
        .text_frames
        .iter()
        .find(|f| f.self_id.as_deref() == Some("tf_u9"))
        .expect("minted frame");
    assert_eq!(minted.inset_spacing, Some([6.0, 8.0, 10.0, 12.0]));
}

/// A frame the model gives no insets keeps writing nothing, so an
/// untouched package stays byte-identical.
#[test]
fn no_insets_writes_no_preference() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let doc = pkg::open(&source);
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(!xml.contains("InsetSpacing"), "nothing written:\n{xml}");
    pkg::assert_same_package(&source, &out);
}

// ---------------------------------------------------------------------
// An EXISTING frame whose insets the model changed
// ---------------------------------------------------------------------

/// A source frame carrying its own typed `<InsetSpacing>` follows the
/// model when the model moves it — and the rest of its `<Properties>`
/// survives untouched.
#[test]
fn a_changed_inset_on_a_source_frame_is_written_back() {
    let spread = pkg::SPREAD.replace(
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"><TextFramePreference TextColumnCount="2"><Properties><InsetSpacing type="unit">4</InsetSpacing><VerticalThreshold type="unit">7</VerticalThreshold></Properties></TextFramePreference></TextFrame>"#,
    );
    let source = pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ]);
    let mut doc = pkg::open(&source);
    assert_eq!(
        doc.spreads[0].spread.text_frames[0].inset_spacing,
        Some([4.0; 4])
    );
    doc.spreads[0].spread.text_frames[0].inset_spacing = Some([9.0, 3.0, 9.0, 3.0]);

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<InsetSpacing type="list">"#)
            && xml.contains(r#"<ListItem type="unit">9</ListItem>"#),
        "the model's insets replace the source's:\n{xml}"
    );
    assert!(
        !xml.contains(r#"<InsetSpacing type="unit">4</InsetSpacing>"#),
        "the source's insets are gone, not duplicated:\n{xml}"
    );
    assert!(
        xml.contains(r#"<VerticalThreshold type="unit">7</VerticalThreshold>"#),
        "its sibling property survives:\n{xml}"
    );
    assert!(
        xml.contains(r#"TextColumnCount="2""#),
        "the preference's own attributes survive:\n{xml}"
    );
    let re = pkg::open(&out);
    assert_eq!(
        re.spreads[0].spread.text_frames[0].inset_spacing,
        Some([9.0, 3.0, 9.0, 3.0])
    );
}

/// A source frame with a preference but NO `<Properties>` gains one.
#[test]
fn a_frame_whose_preference_has_no_properties_gains_them() {
    let spread = pkg::SPREAD.replace(
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"><TextFramePreference TextColumnCount="1"/></TextFrame>"#,
    );
    let source = pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ]);
    let mut doc = pkg::open(&source);
    assert_eq!(doc.spreads[0].spread.text_frames[0].inset_spacing, None);
    doc.spreads[0].spread.text_frames[0].inset_spacing = Some([5.0; 4]);

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<Properties><InsetSpacing type="unit">5</InsetSpacing></Properties>"#),
        "a Properties block is created for them:\n{xml}"
    );
    let re = pkg::open(&out);
    assert_eq!(
        re.spreads[0].spread.text_frames[0].inset_spacing,
        Some([5.0; 4])
    );
}

/// An UNCHANGED inset is not rewritten — the source's own bytes stand,
/// so a package nobody edited comes back identical.
#[test]
fn an_unchanged_inset_leaves_the_source_alone() {
    let spread = pkg::SPREAD.replace(
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>"#,
        r#"<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"><TextFramePreference TextColumnCount="1"><Properties><InsetSpacing type="list"><ListItem type="unit">1</ListItem><ListItem type="unit">2</ListItem><ListItem type="unit">3</ListItem><ListItem type="unit">4</ListItem></InsetSpacing></Properties></TextFramePreference></TextFrame>"#,
    );
    let source = pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ]);
    let doc = pkg::open(&source);
    assert_eq!(
        doc.spreads[0].spread.text_frames[0].inset_spacing,
        Some([1.0, 2.0, 3.0, 4.0])
    );
    let out = write_idml(&doc, &source).expect("write");
    pkg::assert_same_package(&source, &out);
}
