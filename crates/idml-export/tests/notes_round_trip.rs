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

//! A story holding editorial notes saves whole. InDesign
//! 20.0.1 writes a `<Note>` inline in its range, with ranges of its own
//! (`notes-probe`; the story below is its export). The writer read those
//! ranges as the story's next paragraph and dropped the note's start tag
//! and the text before it, so even an unedited save came out malformed;
//! now a note, hidden text or a tracked deletion is carried whole, and a
//! rewritten run writes it back where it stood.
//!
//! A note is a character of its paragraph, as InDesign keeps one (core ADR
//! 172): the parser leaves a `NOTE_MARKER` there and the note on the
//! paragraph, and the writer writes each note back at its marker — the
//! source's subtree when the note is unchanged, the model's otherwise.

use idml_export::rewrite::rewrite_story;

const STORY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
	<Story Self="udf" UserText="true" IsEndnoteStory="false" AppliedTOCStyle="n" TrackChanges="false" StoryTitle="$ID/" AppliedNamedGrid="n">
		<StoryPreference OpticalMarginAlignment="false" OpticalMarginSize="12" FrameType="TextFrameType" StoryOrientation="Horizontal" StoryDirection="LeftToRightDirection" />
		<InCopyExportOption IncludeGraphicProxies="true" IncludeAllResources="false" />
		<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle">
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>Text with a not</Content>
				<Note Collapsed="false" CreationDate="2026-10-16T09:59:49" ModificationDate="2026-10-16T09:59:49" UserName="Unbekannter Benutzername" AppliedDocumentUser="dDocumentUser0">
					<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/[No paragraph style]">
						<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
							<Content>Check this claim.</Content>
						</CharacterStyleRange>
					</ParagraphStyleRange>
				</Note>
				<Content>e here and more text.</Content>
				<Note Collapsed="false" CreationDate="2026-10-16T09:59:49" ModificationDate="2026-10-16T09:59:49" UserName="Unbekannter Benutzername" AppliedDocumentUser="dDocumentUser0">
					<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/[No paragraph style]">
						<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
							<Content>Second note, at the end.</Content>
						</CharacterStyleRange>
					</ParagraphStyleRange>
				</Note>
			</CharacterStyleRange>
		</ParagraphStyleRange>
	</Story>
</idPkg:Story>
"#;

const NOTE: char = paged_model::NOTE_MARKER;

fn notes_in(xml: &str) -> usize {
    xml.matches("<Note ").count()
}

fn text_of(story: &idml_import::Story) -> String {
    story.paragraphs[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect()
}

/// The source's second `<Note>` element, whole.
fn second_note_source() -> &'static str {
    let start = STORY.match_indices("<Note ").nth(1).unwrap().0;
    let end = STORY[start..].find("</Note>").unwrap() + start + "</Note>".len();
    &STORY[start..end]
}

#[test]
fn each_note_is_a_character_of_the_text_and_its_text_is_the_notes() {
    let story = idml_import::parse_story(STORY.as_bytes()).unwrap();
    assert_eq!(
        text_of(&story),
        format!("Text with a not{NOTE}e here and more text.{NOTE}")
    );
    let notes = &story.paragraphs[0].notes;
    assert_eq!(notes.len(), 2);
    assert_eq!(
        (notes[0].anchor, notes[0].text.as_str()),
        (15, "Check this claim.")
    );
    assert_eq!(
        (notes[1].anchor, notes[1].text.as_str()),
        (37, "Second note, at the end.")
    );
    assert_eq!(
        notes[0].user_name.as_deref(),
        Some("Unbekannter Benutzername")
    );
    assert_eq!(notes[0].document_user.as_deref(), Some("dDocumentUser0"));
    assert_eq!(
        notes[0].creation_date.as_deref(),
        Some("2026-10-16T09:59:49")
    );
    assert!(!notes[0].collapsed);
}

#[test]
fn an_unedited_story_with_notes_saves_byte_identically() {
    let story = idml_import::parse_story(STORY.as_bytes()).unwrap();
    let out = rewrite_story(STORY.as_bytes(), &story).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), STORY);
}

#[test]
fn an_edit_beside_a_note_saves_and_the_note_keeps_its_place() {
    let mut story = idml_import::parse_story(STORY.as_bytes()).unwrap();
    let run = &mut story.paragraphs[0].runs[0];
    run.text = run.text.replacen("Text", "Edited text", 1);
    let out = String::from_utf8(rewrite_story(STORY.as_bytes(), &story).unwrap()).unwrap();
    assert_eq!(notes_in(&out), 2, "{out}");
    // The first note still stands inside "not|e", after the edit.
    let first = out.find("<Note ").unwrap();
    let before: String = out[..first].rsplit("<Content>").next().unwrap().to_string();
    assert!(before.starts_with("Edited text with a not"), "{before}");
    // Both notes are their source's, byte for byte.
    assert!(out.contains(second_note_source()), "{out}");
    // Well formed, and it reads back as the edit.
    let back = idml_import::parse_story(out.as_bytes()).unwrap();
    assert_eq!(
        text_of(&back),
        format!("Edited text with a not{NOTE}e here and more text.{NOTE}")
    );
    assert_eq!(back.paragraphs[0].notes[0].text, "Check this claim.");
}

#[test]
fn an_edited_note_saves_its_new_text_and_the_others_as_they_were() {
    let mut story = idml_import::parse_story(STORY.as_bytes()).unwrap();
    story.paragraphs[0].notes[0].text = "Checked: it holds.".into();
    story.paragraphs[0].notes[0].modification_date = Some("2026-10-17T08:00:00".into());
    let out = String::from_utf8(rewrite_story(STORY.as_bytes(), &story).unwrap()).unwrap();
    assert_eq!(notes_in(&out), 2, "{out}");
    assert!(out.contains(second_note_source()), "{out}");
    let back = idml_import::parse_story(out.as_bytes()).unwrap();
    let notes = &back.paragraphs[0].notes;
    assert_eq!(notes[0].text, "Checked: it holds.");
    assert_eq!(
        notes[0].modification_date.as_deref(),
        Some("2026-10-17T08:00:00")
    );
    assert_eq!(
        notes[0].creation_date.as_deref(),
        Some("2026-10-16T09:59:49")
    );
    assert_eq!(notes[0].anchor, 15);
    assert_eq!(notes[1].text, "Second note, at the end.");
}

#[test]
fn a_deleted_note_leaves_the_saved_story_with_its_text() {
    let mut story = idml_import::parse_story(STORY.as_bytes()).unwrap();
    let para = &mut story.paragraphs[0];
    para.runs[0].text = para.runs[0].text.replacen(NOTE, "", 1);
    para.notes.remove(0);
    para.notes[0].anchor -= 1;
    let out = String::from_utf8(rewrite_story(STORY.as_bytes(), &story).unwrap()).unwrap();
    assert_eq!(notes_in(&out), 1, "{out}");
    assert!(out.contains(second_note_source()), "{out}");
    let back = idml_import::parse_story(out.as_bytes()).unwrap();
    assert_eq!(
        text_of(&back),
        format!("Text with a note here and more text.{NOTE}")
    );
}

#[test]
fn an_inserted_note_is_written_as_indesign_writes_one() {
    let mut story = idml_import::parse_story(STORY.as_bytes()).unwrap();
    let para = &mut story.paragraphs[0];
    para.runs[0].text.insert(4, NOTE);
    para.shift_anchors(4, 1);
    para.notes.insert(
        0,
        paged_model::Note {
            id: "new".into(),
            anchor: 4,
            text: "A new note.\rIts second paragraph.".into(),
            user_name: Some("Reviewer".into()),
            creation_date: Some("2026-10-17T09:00:00".into()),
            modification_date: Some("2026-10-17T09:00:00".into()),
            ..Default::default()
        },
    );
    let out = String::from_utf8(rewrite_story(STORY.as_bytes(), &story).unwrap()).unwrap();
    assert_eq!(notes_in(&out), 3, "{out}");
    assert!(
        out.contains(
            "<Content>Text</Content><Note Collapsed=\"false\" CreationDate=\"2026-10-17T09:00:00\""
        ),
        "{out}"
    );
    assert!(
        out.contains("<Content>A new note.</Content><Br/><Content>Its second paragraph.</Content>"),
        "{out}"
    );
    let back = idml_import::parse_story(out.as_bytes()).unwrap();
    let notes = &back.paragraphs[0].notes;
    assert_eq!(notes.len(), 3);
    assert_eq!(
        (notes[0].anchor, notes[0].text.as_str()),
        (4, "A new note.\rIts second paragraph.")
    );
    assert_eq!(notes[0].user_name.as_deref(), Some("Reviewer"));
    assert_eq!(
        (notes[1].anchor, notes[1].text.as_str()),
        (16, "Check this claim.")
    );
    assert_eq!(
        text_of(&back),
        format!("Text{NOTE} with a not{NOTE}e here and more text.{NOTE}")
    );
}
