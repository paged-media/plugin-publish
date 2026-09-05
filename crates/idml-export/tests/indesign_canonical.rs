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
