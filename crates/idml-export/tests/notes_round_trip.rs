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

fn notes_in(xml: &str) -> usize {
    xml.matches("<Note ").count()
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
    assert_eq!(run.text, "Text with a note here and more text.");
    run.text = run.text.replacen("Text", "Edited text", 1);
    let out = String::from_utf8(rewrite_story(STORY.as_bytes(), &story).unwrap()).unwrap();
    assert_eq!(notes_in(&out), 2, "{out}");
    // The first note still stands inside "not|e", after the edit.
    let first = out.find("<Note ").unwrap();
    let before: String = out[..first].rsplit("<Content>").next().unwrap().to_string();
    assert!(before.starts_with("Edited text with a not"), "{before}");
    // Well formed, and it reads back as the edit.
    let back = idml_import::parse_story(out.as_bytes()).unwrap();
    let text: String = back.paragraphs[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(text, "Edited text with a note here and more text.");
}
