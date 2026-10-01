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

//! Two paragraph marks in a row are an EMPTY PARAGRAPH, wherever the
//! `<CharacterStyleRange>` boundary between them falls.
//!
//! # The defect
//!
//! The importer holds a `<Br/>` instead of appending it (the paragraph's
//! last mark is its terminator, not text — see `paragraph_terminator.rs`).
//! It held it in a FLAG, so a second mark before the next content folded
//! into the first. InDesign writes exactly that shape for a blank line that
//! carries the following range's formatting:
//!
//! ```text
//! <CharacterStyleRange …><Content>Overview</Content><Br /></CharacterStyleRange>
//! <CharacterStyleRange …><Br /><Content>Lorem ipsum…</Content></CharacterStyleRange>
//! ```
//!
//! Three paragraphs ("Overview", "", "Lorem…"); the model had two. The
//! writer then compared the model text with the source, rightly saw a
//! difference, and re-serialised the second range WITHOUT its leading
//! mark — so a save that changed nothing deleted the blank line and
//! rewrote the range's layout. The mirror shape (several marks ENDING a
//! range that another range follows) failed the same way. Found by the
//! opt-in corpus lane `corpus_sentinel_tints_survive_an_unmutated_save`
//! (five stories of `company-profile-canva-docx-id-psd`), broken since
//! the held-mark change `18857d9`; nothing in the default suite carried a
//! double mark across a range boundary.
//!
//! # What this file pins
//!
//! For each shape: the model keeps one newline per interior mark, and an
//! unmutated save is byte-identical. A real edit still writes, and keeps
//! the marks it does not touch.

use idml_export::rewrite::rewrite_story;
use idml_import::parse_story;

fn story(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<idPkg:Story xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\" DOMVersion=\"17.0\">\n\
\t<Story Self=\"u1\">\n\
\t\t<ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/Body\">\n\
{body}\
\t\t</ParagraphStyleRange>\n\
\t</Story>\n\
</idPkg:Story>"
    )
}

/// The model text of the story's single paragraph, run by run.
fn runs(src: &str) -> Vec<String> {
    let st = parse_story(src.as_bytes()).expect("parse");
    assert_eq!(st.paragraphs.len(), 1, "one ParagraphStyleRange");
    st.paragraphs[0]
        .runs
        .iter()
        .map(|r| r.text.clone())
        .collect()
}

fn assert_unmutated_save_is_identical(src: &str) {
    let st = parse_story(src.as_bytes()).expect("parse");
    let out = rewrite_story(src.as_bytes(), &st).expect("rewrite");
    assert_eq!(
        String::from_utf8(out).expect("utf-8"),
        src,
        "an unmutated save must not change a byte"
    );
}

/// The corpus shape: a range ends on a mark, the next one STARTS on one.
const MARK_BEFORE_CONTENT: &str = "\
\t\t\t<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/Head\" FontStyle=\"Bold\" FillTint=\"-1\">\n\
\t\t\t\t<Content>Overview</Content>\n\
\t\t\t\t<Br />\n\
\t\t\t</CharacterStyleRange>\n\
\t\t\t<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/Body\" PointSize=\"9\" FillTint=\"-1\">\n\
\t\t\t\t<Br />\n\
\t\t\t\t<Content>Lorem ipsum</Content>\n\
\t\t\t</CharacterStyleRange>\n";

#[test]
fn a_mark_before_content_after_a_mark_is_an_empty_paragraph() {
    let src = story(MARK_BEFORE_CONTENT);
    assert_eq!(
        runs(&src),
        ["Overview", "\n\nLorem ipsum"],
        "three paragraphs: Overview, an empty one, Lorem ipsum"
    );
    assert_unmutated_save_is_identical(&src);
}

/// The mirror: a range ENDS on several marks and another range follows.
/// All but the last stay in the range that holds them.
#[test]
fn several_marks_ending_a_range_stay_in_it() {
    let src = story(
        "\
\t\t\t<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/Body\">\n\
\t\t\t\t<Content>Ut enim</Content>\n\
\t\t\t\t<Br />\n\
\t\t\t\t<Br />\n\
\t\t\t\t<Br />\n\
\t\t\t</CharacterStyleRange>\n\
\t\t\t<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/Head\">\n\
\t\t\t\t<Content>// </Content>\n\
\t\t\t</CharacterStyleRange>\n",
    );
    assert_eq!(runs(&src), ["Ut enim\n\n", "\n// "]);
    assert_unmutated_save_is_identical(&src);
}

/// Both at once, plus a range that is nothing but marks.
#[test]
fn a_range_of_only_marks_between_two_marked_ranges() {
    let src = story(
        "\
\t\t\t<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/A\">\n\
\t\t\t\t<Content>one</Content>\n\
\t\t\t\t<Br />\n\
\t\t\t</CharacterStyleRange>\n\
\t\t\t<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/B\">\n\
\t\t\t\t<Br />\n\
\t\t\t\t<Br />\n\
\t\t\t</CharacterStyleRange>\n\
\t\t\t<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/C\">\n\
\t\t\t\t<Br />\n\
\t\t\t\t<Content>two</Content>\n\
\t\t\t</CharacterStyleRange>\n",
    );
    // Four marks before "two": one, three empty paragraphs, two.
    let text: String = runs(&src).concat();
    assert_eq!(text, "one\n\n\n\ntwo");
    assert_unmutated_save_is_identical(&src);
}

/// At the paragraph range's end only the LAST mark is the terminator; the
/// one before it is an empty paragraph, and stays.
#[test]
fn a_double_mark_at_the_ranges_end_keeps_one_empty_paragraph() {
    let src = story(
        "\
\t\t\t<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/Body\">\n\
\t\t\t\t<Content>last</Content>\n\
\t\t\t\t<Br />\n\
\t\t\t\t<Br />\n\
\t\t\t</CharacterStyleRange>\n",
    );
    assert_eq!(runs(&src), ["last\n"]);
    assert_unmutated_save_is_identical(&src);
}

/// A real edit of the range after the blank line still writes — and keeps
/// the range's leading mark, the empty paragraph the edit did not touch.
#[test]
fn an_edit_after_the_empty_paragraph_keeps_its_mark() {
    let src = story(MARK_BEFORE_CONTENT);
    let mut st = parse_story(src.as_bytes()).expect("parse");
    st.paragraphs[0].runs[1].text = "\n\nEdited".to_string();
    let out =
        String::from_utf8(rewrite_story(src.as_bytes(), &st).expect("rewrite")).expect("utf-8");
    assert!(
        out.contains("<Br/><Content>Edited</Content>"),
        "the edited range keeps its leading mark:\n{out}"
    );
    assert!(
        out.contains("<Content>Overview</Content>\n\t\t\t\t<Br />"),
        "the untouched range is byte-identical:\n{out}"
    );
    let back = parse_story(out.as_bytes()).expect("reparse");
    let text: String = back.paragraphs[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(
        text, "Overview\n\nEdited",
        "the blank line survives the edit"
    );
}
