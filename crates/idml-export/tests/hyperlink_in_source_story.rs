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

//! A hyperlink inserted into a story that already has a SOURCE part:
//! `InsertHyperlink` splits the run and tags the middle piece; the source
//! range carries no `<HyperlinkTextSource>`. The patch path must write
//! the split pieces (the tail used to be LOST: "Hello world" saved as
//! "Hello") and wrap the tagged one in InDesign's inner-form source —
//! InDesign 20.0.1 kept 3 of 7 hyperlinks in the annual, dropping exactly
//! the four whose sources lived in source stories.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, Hyperlink, HyperlinkDestination, HyperlinkDestinationKind};

fn link(doc: &mut paged_scene::Document) {
    doc.designmap
        .hyperlink_destinations
        .push(HyperlinkDestination {
            self_id: "HyperlinkURLDestination/u9".into(),
            kind: HyperlinkDestinationKind::Url("https://paged.media".into()),
        });
    doc.designmap.hyperlinks.push(Hyperlink {
        self_id: "Hyperlink/u9".into(),
        name: Some("web".into()),
        source: Some("HyperlinkTextSource/u9".into()),
        destination: Some("HyperlinkURLDestination/u9".into()),
    });
}

#[test]
fn a_run_split_by_insert_hyperlink_saves_every_piece_and_wraps_the_link() {
    let source = pkg::simple("", "<ParagraphStyleRange><CharacterStyleRange PointSize=\"12\"><Content>Visit paged now</Content></CharacterStyleRange></ParagraphStyleRange>", pkg::STYLES);
    let mut doc = pkg::open(&source);
    link(&mut doc);
    let para = &mut doc.stories[0].story.paragraphs[0];
    let base = para.runs[0].clone();
    para.runs = vec![
        CharacterRun {
            text: "Visit ".into(),
            ..base.clone()
        },
        CharacterRun {
            text: "paged".into(),
            hyperlink_source: Some("HyperlinkTextSource/u9".into()),
            ..base.clone()
        },
        CharacterRun {
            text: " now".into(),
            ..base
        },
    ];

    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(
        xml.contains(r#"<CharacterStyleRange PointSize="12"><Content>Visit </Content></CharacterStyleRange><CharacterStyleRange PointSize="12"><HyperlinkTextSource Self="HyperlinkTextSource/u9" Name="u9" Hidden="false"><Content>paged</Content></HyperlinkTextSource></CharacterStyleRange><CharacterStyleRange PointSize="12"><Content> now</Content></CharacterStyleRange>"#),
        "{xml}"
    );
    let re = pkg::open(&out);
    let runs = &re.stories[0].story.paragraphs[0].runs;
    let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, vec!["Visit ", "paged", " now"]);
    assert_eq!(
        runs[1].hyperlink_source.as_deref(),
        Some("HyperlinkTextSource/u9")
    );
    assert_eq!(runs[1].point_size, Some(12.0));
    assert_eq!(re.designmap.hyperlinks.len(), 1);
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}

#[test]
fn a_whole_run_tagged_after_the_part_was_written_gets_its_wrapper() {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    link(&mut doc);
    doc.stories[0].story.paragraphs[0].runs[0].hyperlink_source =
        Some("HyperlinkTextSource/u9".into());
    let out = write_idml(&doc, &source).expect("write");
    let xml = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(
        xml.contains(r#"<CharacterStyleRange><HyperlinkTextSource Self="HyperlinkTextSource/u9" Name="u9" Hidden="false"><Content>Hello world</Content></HyperlinkTextSource></CharacterStyleRange>"#),
        "{xml}"
    );
    let re = pkg::open(&out);
    assert_eq!(
        re.stories[0].story.paragraphs[0].runs[0]
            .hyperlink_source
            .as_deref(),
        Some("HyperlinkTextSource/u9")
    );
    pkg::assert_same_package(&out, &write_idml(&re, &out).expect("re-write"));
}
