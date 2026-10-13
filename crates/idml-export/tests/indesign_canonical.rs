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

//! InDesign 20.0.1's OWN export of a two-page document with one
//! condition, one condition set, URL + page + text-anchor hyperlinks and
//! one bookmark (`tests/fixtures/indesign-20.0.1-navigation.idml`,
//! authored 2026-09-05 for the navigation experiment). The importer must
//! read every one of them in InDesign's spelling, and the exporter must
//! leave the package byte-for-byte alone when nothing changed — the
//! in-place lanes of `navigation` measured against the real thing.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::HyperlinkDestinationKind;

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/indesign-20.0.1-navigation.idml"
    ))
    .expect("fixture present")
}

#[test]
fn the_importer_reads_indesigns_navigation_spelling() {
    let doc = pkg::open(&fixture());
    // Conditions: designmap children with typed Properties.
    assert_eq!(doc.styles.conditions.len(), 1);
    let draft = &doc.styles.conditions["Condition/Draft"];
    assert_eq!(draft.name.as_deref(), Some("Draft"));
    assert_eq!(draft.visible, Some(true));
    assert_eq!(draft.indicator_method.as_deref(), Some("UseUnderline"));
    assert_eq!(doc.styles.condition_sets.len(), 1);
    assert_eq!(
        doc.styles.condition_sets["ConditionSet/Review"].conditions,
        vec!["Condition/Draft".to_string()],
        "members come from the VisibilityPair children"
    );
    // Hyperlinks: destination from the typed child, not the key.
    assert_eq!(doc.designmap.hyperlinks.len(), 3);
    let by_name = |n: &str| {
        doc.designmap
            .hyperlinks
            .iter()
            .find(|h| h.name.as_deref() == Some(n))
            .unwrap_or_else(|| panic!("hyperlink {n}"))
    };
    assert_eq!(
        by_name("registry-url").destination.as_deref(),
        Some("HyperlinkURLDestination/paged")
    );
    assert_eq!(by_name("registry-url").source.as_deref(), Some("ufe"));
    assert_eq!(
        by_name("back-to-p2").destination.as_deref(),
        Some("HyperlinkPageDestination/page2")
    );
    assert_eq!(
        by_name("xref-overleaf").destination.as_deref(),
        Some("HyperlinkTextDestination/overleaf")
    );
    // Destinations: URL + page from the designmap, the text anchor from
    // the inline story marker.
    let dests = &doc.designmap.hyperlink_destinations;
    assert_eq!(dests.len(), 3, "{dests:#?}");
    assert!(dests.iter().any(
        |d| matches!(&d.kind, HyperlinkDestinationKind::Url(u) if u == "https://paged.media")
    ));
    assert!(dests
        .iter()
        .any(|d| matches!(&d.kind, HyperlinkDestinationKind::Page(p) if p == "ue3")));
    assert!(dests
        .iter()
        .any(|d| d.self_id == "HyperlinkTextDestination/overleaf"
            && matches!(&d.kind, HyperlinkDestinationKind::TextAnchor(s) if s == "ue5")));
    assert_eq!(doc.designmap.bookmarks.len(), 1);
    assert_eq!(
        doc.designmap.bookmarks[0].destination.as_deref(),
        Some("HyperlinkPageDestination/page2")
    );
    // Text sources INSIDE the range: the linked words are their own runs.
    let runs = &doc.stories[0].story.paragraphs[0].runs;
    let tagged: Vec<(&str, &str)> = runs
        .iter()
        .filter_map(|r| r.hyperlink_source.as_deref().map(|s| (r.text.as_str(), s)))
        .collect();
    assert!(tagged.contains(&("paged", "ufe")), "{tagged:?}");
    assert!(tagged.contains(&("media", "u103")), "{tagged:?}");
    assert!(runs
        .iter()
        .any(|r| r.text == "Visit" && r.applied_conditions == vec!["Condition/Draft".to_string()]));
    assert!(
        tagged.iter().any(|(_, s)| *s == "u107"),
        "the cross-reference source wraps its ranges: {tagged:?}"
    );
    // Section: typed PageNumberStyle child (Arabic here).
    assert_eq!(doc.designmap.sections.len(), 1);
    assert_eq!(doc.designmap.sections[0].page_start.as_deref(), Some("ud4"));
}

/// Every ENTRY byte-identical. The ZIP container itself is re-framed by
/// the `zip` crate (local-header layout, descriptors), which is not the
/// byte-identity guarantee — that is about the parts InDesign reads.
#[test]
fn indesigns_own_package_round_trips_every_entry_byte_identically() {
    let source = fixture();
    let doc = pkg::open(&source);
    let out = write_idml(&doc, &source).expect("write");
    pkg::assert_same_entries(&source, &out);
}

/// The inks, the page setup and the story's table-of-contents style, as
/// InDesign 20.0.1 spells them: four process `<Ink>`s in Graphic.xml,
/// page size / facing pages / binding on `<DocumentPreference>`, and
/// `AppliedTOCStyle="n"` (none) on an ordinary `<Story>`.
#[test]
fn the_importer_reads_indesigns_inks_page_setup_and_story_toc_style() {
    let doc = pkg::open(&fixture());
    let inks: Vec<(&str, Option<f32>, Option<i32>)> = doc
        .palette
        .inks
        .iter()
        .map(|i| (i.self_id.as_str(), i.angle, i.trap_order))
        .collect();
    assert_eq!(
        inks,
        [
            ("Ink/$ID/Process Cyan", Some(75.0), Some(1)),
            ("Ink/$ID/Process Magenta", Some(15.0), Some(2)),
            ("Ink/$ID/Process Yellow", Some(0.0), Some(3)),
            ("Ink/$ID/Process Black", Some(45.0), Some(4)),
        ]
    );
    let black = &doc.palette.inks[3];
    assert_eq!(black.name.as_deref(), Some("$ID/Process Black"));
    assert_eq!(black.frequency, Some(70.0));
    assert_eq!(black.neutral_density, Some(1.7));
    assert_eq!(black.convert_to_process, Some(false));
    assert_eq!(black.print_ink, Some(true));
    assert_eq!(black.ink_type.as_deref(), Some("Normal"));
    assert_eq!(black.alias_ink_name, None);

    let p = &doc.designmap.document_preference;
    assert!((p.page_width.unwrap() - 595.2756).abs() < 1e-3, "{p:?}");
    assert!((p.page_height.unwrap() - 841.8898).abs() < 1e-3);
    assert_eq!(p.facing_pages, Some(true));
    assert_eq!(p.page_binding.as_deref(), Some("LeftToRight"));

    assert_eq!(doc.stories.len(), 1);
    assert_eq!(doc.stories[0].story.applied_toc_style, None);
}

/// An edit of the story's text rewrites its part and nothing else: the
/// `<Story>` tag (its `AppliedTOCStyle="n"` included), the inks and the
/// document preference stay InDesign's bytes.
#[test]
fn a_story_edit_in_indesigns_package_keeps_the_story_tag_inks_and_preferences() {
    let source = fixture();
    let mut doc = pkg::open(&source);
    let run = doc.stories[0]
        .story
        .paragraphs
        .iter_mut()
        .flat_map(|p| p.runs.iter_mut())
        .find(|r| !r.text.trim().is_empty())
        .expect("a run with text");
    run.text.push_str(" (edited)");
    let out = write_idml(&doc, &source).expect("write");
    let story = pkg::entry(&out, "Stories/Story_ue5.xml").expect("story part");
    assert_ne!(
        Some(story.clone()),
        pkg::entry(&source, "Stories/Story_ue5.xml"),
        "the part was rewritten"
    );
    assert!(story.contains(" (edited)"), "{story}");
    assert!(
        story.contains(r#"<Story Self="ue5" UserText="true" IsEndnoteStory="false" AppliedTOCStyle="n" TrackChanges="false" StoryTitle="$ID/" AppliedNamedGrid="n">"#),
        "{story}"
    );
    for part in ["Resources/Graphic.xml", "Resources/Preferences.xml"] {
        assert_eq!(pkg::entry(&out, part), pkg::entry(&source, part), "{part}");
    }
}
