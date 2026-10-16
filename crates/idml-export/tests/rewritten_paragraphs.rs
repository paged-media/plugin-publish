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

//! A story whose paragraphs were rewritten whole — as Update Table of
//! Contents rewrites a TOC story (core ADR 165) — saves the model's text
//! only. A source range past a paragraph's last model run held text the
//! story no longer has; it used to be replayed, so a stale page number
//! (`8`) outlived the update in the saved file.

use idml_export::rewrite::rewrite_story;
use idml_import::{CharacterRun, Paragraph};

const STORY: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Story Self="u1" AppliedTOCStyle="TOCStyle/Contents">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/TOC 1"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Content>Getting Started	</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/TOC Page"><Content>7</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Br/></CharacterStyleRange></ParagraphStyleRange>
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/TOC 1"><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]"><Content>An Old Heading	</Content></CharacterStyleRange><CharacterStyleRange AppliedCharacterStyle="CharacterStyle/TOC Page"><Content>8</Content></CharacterStyleRange></ParagraphStyleRange>
</Story>
</idPkg:Story>"#;

fn run(text: &str, style: Option<&str>) -> CharacterRun {
    CharacterRun {
        text: text.to_string(),
        character_style: style.map(str::to_string),
        ..CharacterRun::default()
    }
}

fn texts(story: &idml_import::Story) -> Vec<String> {
    story
        .paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
        .collect()
}

#[test]
fn a_range_past_the_models_last_run_saves_no_text() {
    let mut story = idml_import::parse_story(STORY).expect("parse");
    assert_eq!(
        texts(&story),
        ["Getting Started\t7", "An Old Heading\t8"],
        "the source as read"
    );
    story.paragraphs = vec![
        Paragraph {
            paragraph_style: Some("ParagraphStyle/TOC 1".into()),
            runs: vec![
                run("Going Further\t", None),
                run("2", Some("CharacterStyle/TOC Page")),
            ],
            ..Paragraph::default()
        },
        Paragraph {
            paragraph_style: Some("ParagraphStyle/TOC 2".into()),
            runs: vec![run("1 First Steps", None)],
            ..Paragraph::default()
        },
    ];
    let out = rewrite_story(STORY, &story).expect("rewrite");
    let back = idml_import::parse_story(&out).expect("re-parse");
    assert_eq!(
        texts(&back),
        ["Going Further\t2", "1 First Steps"],
        "{}",
        String::from_utf8_lossy(&out)
    );
    assert_eq!(
        back.paragraphs[0].runs[1].character_style.as_deref(),
        Some("CharacterStyle/TOC Page")
    );
}

#[test]
fn an_unchanged_story_still_saves_byte_identically() {
    let story = idml_import::parse_story(STORY).expect("parse");
    assert_eq!(rewrite_story(STORY, &story).expect("rewrite"), STORY);
}
