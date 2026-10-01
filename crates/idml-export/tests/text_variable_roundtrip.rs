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

//! Text variables and page markers through an unmutated and a mutated save.
//!
//! The story is InDesign 20.0.1's own spelling of a master header (its IDML
//! export of a DOM-built document, 2026-10-01; thoughts ADR 033): variable
//! instances in `PageNumberType="TextVariable"` ranges, next / previous page
//! numbers as `<?ACE 18?>` in `PageNumberType` ranges, the section marker as
//! `<?ACE 19?>`. The importer now keeps a variable instance whose
//! `ResultText` is empty (RFI C-39), which adds a model run the writer's
//! provenance must account for.

use idml_export::rewrite::rewrite_story;
use idml_import::parse_story;

const STORY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
	<Story Self="u118" UserText="true" IsEndnoteStory="false" AppliedTOCStyle="n" TrackChanges="false" StoryTitle="$ID/" AppliedNamedGrid="n">
		<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/$ID/NormalParagraphStyle">
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>rhF=[</Content>
			</CharacterStyleRange>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PageNumberType="TextVariable">
				<TextVariableInstance Self="u12e" Name="RH First" ResultText="&lt;RH First&gt;" AssociatedTextVariable="dTextVariablenRH First" />
			</CharacterStyleRange>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>] cust=[</Content>
			</CharacterStyleRange>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PageNumberType="TextVariable">
				<TextVariableInstance Self="u135" Name="Cust" ResultText="" AssociatedTextVariable="dTextVariablenCust" />
			</CharacterStyleRange>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>]</Content>
				<Br />
				<Content>pg=[<?ACE 18?>]</Content>
				<Br />
				<Content>next=[</Content>
			</CharacterStyleRange>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PageNumberType="NextPageNumber">
				<Content><?ACE 18?></Content>
			</CharacterStyleRange>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>] prev=[</Content>
			</CharacterStyleRange>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PageNumberType="PreviousPageNumber">
				<Content><?ACE 18?></Content>
			</CharacterStyleRange>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>] sec=[<?ACE 19?>] tail</Content>
			</CharacterStyleRange>
		</ParagraphStyleRange>
	</Story>
</idPkg:Story>"#;

#[test]
fn an_unmutated_save_is_byte_identical() {
    let story = parse_story(STORY.as_bytes()).expect("parse");
    let out = rewrite_story(STORY.as_bytes(), &story).expect("rewrite");
    assert_eq!(String::from_utf8(out).unwrap(), STORY);
}

#[test]
fn an_edit_after_an_empty_result_instance_patches_its_own_run() {
    let mut story = parse_story(STORY.as_bytes()).expect("parse");
    let run = story
        .paragraphs
        .iter_mut()
        .flat_map(|p| p.runs.iter_mut())
        .find(|r| r.text == "] cust=[")
        .expect("the run between the two instances");
    run.text = "] custom=[".to_string();
    let out = String::from_utf8(rewrite_story(STORY.as_bytes(), &story).expect("rewrite")).unwrap();
    // A replaced run body is written without the indentation that followed
    // it (the writer's behaviour for every edited run), so compare with
    // inter-element whitespace dropped.
    let squash = |s: &str| -> String {
        s.split('\n')
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .concat()
    };
    assert_eq!(
        squash(&out),
        squash(&STORY.replace(
            "<Content>] cust=[</Content>",
            "<Content>] custom=[</Content>"
        )),
        "only the edited run changes; the instances and markers stay verbatim"
    );
}
