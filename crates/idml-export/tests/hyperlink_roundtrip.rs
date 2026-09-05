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

//! Hyperlink save-back in the spelling InDesign 20.0.1 was MEASURED to
//! read (2026-09-05, variants V0–VF):
//!
//! * the navigation block sits AFTER the last `<idPkg:Story>` include;
//! * `<Hyperlink>` names its destination as a typed Properties child,
//!   never as a `Destination` attribute (which, next to a
//!   `DestinationUniqueKey`, makes the file unopenable);
//! * a text destination is an inline story marker, never a designmap
//!   element;
//! * a minted story wraps a linked run's `<Content>` in a
//!   `<HyperlinkTextSource>` INSIDE the range.
//!
//! The engine's older spelling (block before the stories, attribute
//! destination, designmap text destination) is NORMALISED on export, so
//! an existing book exports correctly without re-authoring.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{
    CharacterRun, Hyperlink, HyperlinkDestination, HyperlinkDestinationKind, Paragraph, Story,
};

fn url_link(doc: &mut paged_scene::Document, n: u32, source: &str) {
    doc.designmap
        .hyperlink_destinations
        .push(HyperlinkDestination {
            self_id: format!("HyperlinkURLDestination/u{n}"),
            kind: HyperlinkDestinationKind::Url("https://paged.media".into()),
        });
    doc.designmap.hyperlinks.push(Hyperlink {
        self_id: format!("Hyperlink/u{n}"),
        name: Some("web".into()),
        source: Some(source.to_string()),
        destination: Some(format!("HyperlinkURLDestination/u{n}")),
    });
}

#[test]
fn a_new_hyperlink_is_written_after_the_stories_in_indesigns_spelling() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    url_link(&mut doc, 9, "HyperlinkTextSource/u9");
    // A minted story with the linked run.
    doc.stories.push(paged_scene::ParsedStory {
        src: String::new(),
        self_id: "Story/u9".into(),
        story: Story {
            paragraphs: vec![Paragraph {
                runs: vec![
                    CharacterRun {
                        text: "Visit ".into(),
                        ..Default::default()
                    },
                    CharacterRun {
                        text: "paged".into(),
                        hyperlink_source: Some("HyperlinkTextSource/u9".into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        },
    });

    let out = write_idml(&doc, &source).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    let last_story = dm.rfind("<idPkg:Story ").expect("story include");
    let dest = pkg::pos(
        &dm,
        "<HyperlinkURLDestination Self=\"HyperlinkURLDestination/u9\"",
    );
    let link = pkg::pos(&dm, "<Hyperlink Self=\"Hyperlink/u9\"");
    assert!(
        last_story < dest && dest < link,
        "block after the stories:\n{dm}"
    );
    assert!(
        dm.contains(r#"<Properties><BorderColor type="enumeration">Black</BorderColor><Destination type="object">HyperlinkURLDestination/u9</Destination></Properties>"#),
        "typed Properties child:\n{dm}"
    );
    assert!(
        !dm.contains(r#" Destination="HyperlinkURLDestination/u9""#),
        "no attribute form:\n{dm}"
    );
    assert!(
        dm.contains(
            r#"DestinationURL="https://paged.media" Hidden="false" DestinationUniqueKey="1""#
        ),
        "{dm}"
    );
    assert!(dm.contains(r#"Source="HyperlinkTextSource/u9" Visible="false" Highlight="None" Width="Thin" BorderStyle="Solid" Hidden="false" DestinationUniqueKey="1""#), "{dm}");

    let story = pkg::entry(&out, "Stories/Story_Story_u9.xml").expect("minted story");
    assert!(
        story.contains(r#"<CharacterStyleRange><HyperlinkTextSource Self="HyperlinkTextSource/u9" Name="u9" Hidden="false"><Content>paged</Content></HyperlinkTextSource></CharacterStyleRange>"#),
        "source INSIDE the range:\n{story}"
    );

    let re = pkg::open(&out);
    let h = re
        .designmap
        .hyperlinks
        .iter()
        .find(|h| h.self_id == "Hyperlink/u9")
        .expect("hyperlink back");
    assert_eq!(
        h.destination.as_deref(),
        Some("HyperlinkURLDestination/u9"),
        "read from the typed child"
    );
    assert_eq!(h.source.as_deref(), Some("HyperlinkTextSource/u9"));
    assert!(re
        .designmap
        .hyperlink_destinations
        .iter()
        .any(|d| d.self_id == "HyperlinkURLDestination/u9"));
    let minted = re
        .stories
        .iter()
        .find(|s| s.self_id == "Story_u9")
        .expect("minted story back");
    let runs = &minted.story.paragraphs[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, "Visit ");
    assert_eq!(runs[0].hyperlink_source, None);
    assert_eq!(runs[1].text, "paged");
    assert_eq!(
        runs[1].hyperlink_source.as_deref(),
        Some("HyperlinkTextSource/u9")
    );
    // Second save: byte-identical (already canonical).
    assert_eq!(out, write_idml(&re, &out).expect("re-write"));
}

/// The engine's pre-2026-09 spelling: block BEFORE the stories,
/// `Destination` attribute, a designmap `HyperlinkTextDestination`.
const OLD_BLOCK: &str = r#"<HyperlinkURLDestination Self="HyperlinkURLDestination/a" Name="https://paged.media" DestinationURL="https://paged.media" Hidden="false"/>
<HyperlinkTextDestination Self="HyperlinkTextDestination/t" Name="HyperlinkTextDestination/t" DestinationText="st1" Hidden="false"/>
<Hyperlink Self="Hyperlink/a" Name="url-link" Source="HyperlinkTextSource/a" Destination="HyperlinkURLDestination/a" Visible="true" Hidden="false"/>
<Hyperlink Self="Hyperlink/t" Name="xref" Source="CrossReferenceSource/x" Destination="HyperlinkTextDestination/t" Visible="true" Hidden="false"/>
<Bookmark Self="Bookmark/b" Name="Intro" Destination="HyperlinkTextDestination/t"/>
"#;
const OLD_STORY: &str = r#"<ParagraphStyleRange><HyperlinkTextSource Self="HyperlinkTextSource/a" Name="a" Hidden="false"><CharacterStyleRange><Content>paged</Content></CharacterStyleRange></HyperlinkTextSource><CrossReferenceSource Self="CrossReferenceSource/x" Name="x"><CharacterStyleRange><Content>see intro</Content></CharacterStyleRange></CrossReferenceSource></ParagraphStyleRange>"#;

#[test]
fn the_old_spelling_is_normalised_on_export_and_reads_back() {
    let source = pkg::simple(OLD_BLOCK, OLD_STORY, pkg::STYLES);
    let doc = pkg::open(&source);
    assert_eq!(doc.designmap.hyperlinks.len(), 2);
    assert_eq!(doc.designmap.hyperlink_destinations.len(), 2);

    let out = write_idml(&doc, &source).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    let last_story = dm.rfind("<idPkg:Story ").expect("story include");
    for needle in [
        "<HyperlinkURLDestination Self=\"HyperlinkURLDestination/a\"",
        "<Hyperlink Self=\"Hyperlink/a\"",
        "<Hyperlink Self=\"Hyperlink/t\"",
        "<Bookmark Self=\"Bookmark/b\"",
    ] {
        assert!(
            pkg::pos(&dm, needle) > last_story,
            "{needle} moved after the stories:\n{dm}"
        );
    }
    assert!(
        !dm.contains("<HyperlinkTextDestination"),
        "text destination left the designmap:\n{dm}"
    );
    assert!(
        !dm.contains(" Destination=\"HyperlinkURLDestination"),
        "no attribute destination:\n{dm}"
    );
    assert!(
        dm.contains(r#"<Destination type="object">HyperlinkURLDestination/a</Destination>"#),
        "{dm}"
    );
    assert!(
        dm.contains(r#"<Destination type="object">HyperlinkTextDestination/t</Destination>"#),
        "{dm}"
    );
    assert!(
        dm.contains(
            r#"<Bookmark Self="Bookmark/b" Name="Intro" Destination="HyperlinkTextDestination/t"/>"#
        ),
        "bookmarks keep the attribute form InDesign writes:\n{dm}"
    );
    // The cross-reference source gained the exporter's format.
    assert!(
        dm.contains("<CrossReferenceFormat Self=\"paged-xref-format\""),
        "{dm}"
    );
    let story = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(story.contains(r#"<HyperlinkTextDestination Self="HyperlinkTextDestination/t" Name="t" Hidden="false"/>"#), "inline marker injected:\n{story}");
    assert!(story.contains(r#"<CrossReferenceSource Self="CrossReferenceSource/x" Name="x" AppliedFormat="paged-xref-format">"#), "{story}");

    let re = pkg::open(&out);
    assert_eq!(re.designmap.hyperlinks.len(), 2);
    let t = re
        .designmap
        .hyperlinks
        .iter()
        .find(|h| h.self_id == "Hyperlink/t")
        .unwrap();
    assert_eq!(t.destination.as_deref(), Some("HyperlinkTextDestination/t"));
    let anchor = re
        .designmap
        .hyperlink_destinations
        .iter()
        .find(|d| d.self_id == "HyperlinkTextDestination/t")
        .expect("text anchor back from the inline marker");
    assert!(matches!(&anchor.kind, HyperlinkDestinationKind::TextAnchor(s) if s == "st1"));
    assert_eq!(re.designmap.bookmarks.len(), 1);
    // Both runs still tagged (wrapping form read back).
    let runs = &re.stories[0].story.paragraphs[0].runs;
    assert_eq!(
        runs[0].hyperlink_source.as_deref(),
        Some("HyperlinkTextSource/a")
    );
    assert_eq!(
        runs[1].hyperlink_source.as_deref(),
        Some("CrossReferenceSource/x")
    );
    // Now canonical: a second save is byte-identical.
    assert_eq!(out, write_idml(&re, &out).expect("re-write"));
}

#[test]
fn a_deleted_hyperlink_leaves_the_designmap() {
    let source = pkg::simple(OLD_BLOCK, OLD_STORY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    doc.designmap
        .hyperlinks
        .retain(|h| h.self_id != "Hyperlink/a");
    doc.designmap
        .hyperlink_destinations
        .retain(|d| d.self_id != "HyperlinkURLDestination/a");
    let out = write_idml(&doc, &source).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert!(
        !dm.contains("Hyperlink/a\"") && !dm.contains("HyperlinkURLDestination/a\""),
        "{dm}"
    );
    let re = pkg::open(&out);
    assert_eq!(re.designmap.hyperlinks.len(), 1);
}

#[test]
fn a_hyperlink_the_model_carries_twice_is_written_once() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    url_link(&mut doc, 9, "HyperlinkTextSource/u9");
    let dup = doc.designmap.hyperlinks[0].clone();
    doc.designmap.hyperlinks.push(dup);
    let out = write_idml(&doc, &source).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert_eq!(
        dm.matches("<Hyperlink Self=\"Hyperlink/u9\"").count(),
        1,
        "{dm}"
    );
    assert_eq!(pkg::open(&out).designmap.hyperlinks.len(), 1);
}
