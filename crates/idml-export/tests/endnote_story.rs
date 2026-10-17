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

//! The endnote story saved as InDesign 20.0.1 writes it (core ADR 174;
//! `endnote-probe`; the story below is its export, re-indented): each
//! note's range — the opening mark, `<?ACE 4?>`, the separator, the text,
//! the closing mark — inside its `<EndnoteRange Self SourceEndnote>`. An
//! unedited story saves byte for byte; a story the model gave a new range,
//! lost one or changed one is written again around the model's ranges.

use idml_export::rewrite::rewrite_story;

const NUM: char = paged_model::ENDNOTE_NUMBER_MARKER;
const BOM: char = '\u{FEFF}';

/// InDesign's export of an endnote story of two notes; `{BOM}` and
/// `{TAB}` stand for U+FEFF and the separator tab.
const STORY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
  <Story Self="u102" UserText="true" IsEndnoteStory="true" AppliedTOCStyle="n" TrackChanges="false" StoryTitle="$ID/" AppliedNamedGrid="n">
    <ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle">
      <CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
        <Content>Endnoten</Content>
        <Br />
        <EndnoteRange Self="u119" SourceEndnote="u118">
          <Content>{BOM}<?ACE 4?>{TAB}The first endnote.{BOM}</Content>
        </EndnoteRange>
        <Br />
        <EndnoteRange Self="u11e" SourceEndnote="u11d">
          <Content>{BOM}<?ACE 4?>{TAB}The second endnote.{BOM}</Content>
        </EndnoteRange>
      </CharacterStyleRange>
    </ParagraphStyleRange>
  </Story>
</idPkg:Story>
"#;

fn source() -> String {
    STORY.replace("{BOM}", "\u{FEFF}").replace("{TAB}", "\t")
}

fn text_of(story: &idml_import::Story) -> String {
    story.paragraphs[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect()
}

fn ranges_of(story: &idml_import::Story) -> Vec<(String, Option<String>)> {
    story.paragraphs[0]
        .endnote_ranges
        .iter()
        .map(|r| (r.self_id.clone(), r.source.clone()))
        .collect()
}

fn range(id: &str, source: &str) -> (String, Option<String>) {
    (id.to_string(), Some(source.to_string()))
}

#[test]
fn the_endnote_story_reads_its_ranges_and_saves_byte_identically() {
    let src = source();
    let story = idml_import::parse_story(src.as_bytes()).unwrap();
    assert!(story.is_endnote_story);
    assert_eq!(
        text_of(&story),
        format!(
            "Endnoten\n{BOM}{NUM}\tThe first endnote.{BOM}\n{BOM}{NUM}\tThe second endnote.{BOM}"
        )
    );
    assert_eq!(
        ranges_of(&story),
        [range("u119", "u118"), range("u11e", "u11d")]
    );
    let out = rewrite_story(src.as_bytes(), &story).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), src);
}

#[test]
fn a_new_range_is_written_in_its_place_as_indesign_writes_one() {
    let src = source();
    let mut story = idml_import::parse_story(src.as_bytes()).unwrap();
    let para = &mut story.paragraphs[0];
    let at = para.runs[0].text.find(&format!("\n{BOM}")).unwrap();
    para.runs[0]
        .text
        .insert_str(at, &format!("\n{BOM}{NUM}\tInserted first.{BOM}"));
    para.endnote_ranges.insert(
        0,
        paged_model::EndnoteRangeDef {
            self_id: "unew".into(),
            source: Some("uref".into()),
        },
    );
    let out = String::from_utf8(rewrite_story(src.as_bytes(), &story).unwrap()).unwrap();
    assert_eq!(out.matches("<EndnoteRange ").count(), 3, "{out}");
    let written = format!(
        "<EndnoteRange Self=\"unew\" SourceEndnote=\"uref\"><Content>{BOM}<?ACE 4?>\tInserted first.{BOM}</Content></EndnoteRange>"
    );
    assert!(out.contains(&written), "{out}");
    let back = idml_import::parse_story(out.as_bytes()).unwrap();
    assert_eq!(
        ranges_of(&back),
        [
            range("unew", "uref"),
            range("u119", "u118"),
            range("u11e", "u11d")
        ]
    );
    assert_eq!(text_of(&back), text_of(&story));
}

#[test]
fn a_deleted_and_an_edited_range_save() {
    let src = source();
    let mut story = idml_import::parse_story(src.as_bytes()).unwrap();
    let para = &mut story.paragraphs[0];
    let gone = format!("\n{BOM}{NUM}\tThe first endnote.{BOM}");
    para.runs[0].text = para.runs[0].text.replacen(&gone, "", 1).replacen(
        "The second endnote.",
        "The second endnote, edited.",
        1,
    );
    para.endnote_ranges.remove(0);
    let out = String::from_utf8(rewrite_story(src.as_bytes(), &story).unwrap()).unwrap();
    assert_eq!(out.matches("<EndnoteRange ").count(), 1, "{out}");
    let back = idml_import::parse_story(out.as_bytes()).unwrap();
    assert_eq!(
        text_of(&back),
        format!("Endnoten\n{BOM}{NUM}\tThe second endnote, edited.{BOM}")
    );
    assert_eq!(ranges_of(&back), [range("u11e", "u11d")]);
}
