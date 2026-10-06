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

//! A text variable AUTHORED in paged reaches the IDML: core's
//! `createTextVariable` adds a definition to `doc.designmap.text_variables`
//! and `InsertField` (text-variable kind) adds a run tagged
//! `text_variable` whose text is the value. The writer used to carry only
//! what the source package already had, so InDesign saw neither the
//! definition nor the instance (core's `idml_export_object_model_state`
//! pinned that). Now the definition is written into `designmap.xml` in
//! InDesign's spelling and the instance as a `<TextVariableInstance>` in a
//! range of its own; the importer reads both back.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{CharacterRun, TextVariable};

const ID: &str = "dTextVariablenPrice";

fn price() -> TextVariable {
    TextVariable {
        self_id: ID.to_string(),
        name: Some("Price".to_string()),
        variable_type: Some("CustomTextType".to_string()),
        contents: Some("9.99".to_string()),
        ..Default::default()
    }
}

/// What core's `InsertField` does: the variable run clones the host
/// run's formatting, splitting it at `at` (a char offset inside "Hello
/// world").
fn insert_instance(doc: &mut paged_scene::Document, at: usize) {
    doc.designmap.text_variables.push(price());
    let runs = &mut doc.stories[0].story.paragraphs[0].runs;
    let host = runs[0].clone();
    let mut var = host.clone();
    var.text = "9.99".to_string();
    var.text_variable = Some(ID.to_string());
    let (a, b) = host.text.split_at(at);
    let (a, b) = (a.to_string(), b.to_string());
    let mut head = host.clone();
    head.text = a.clone();
    let mut tail = host;
    tail.text = b.clone();
    let mut out: Vec<CharacterRun> = Vec::new();
    if !a.is_empty() {
        out.push(head);
    }
    out.push(var);
    if !b.is_empty() {
        out.push(tail);
    }
    *runs = out;
}

fn texts(doc: &paged_scene::Document) -> Vec<(String, Option<String>)> {
    doc.stories[0].story.paragraphs[0]
        .runs
        .iter()
        .map(|r| (r.text.clone(), r.text_variable.clone()))
        .collect()
}

fn check(at: usize) {
    let src = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&src);
    insert_instance(&mut doc, at);
    let want = texts(&doc);
    let out = write_idml(&doc, &src).expect("write");

    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert!(
        dm.contains(
            r#"<TextVariable Self="dTextVariablenPrice" Name="Price" VariableType="CustomTextType"><CustomTextVariablePreference TextBefore="" TextAfter=""><Properties><Contents type="string">9.99</Contents></Properties></CustomTextVariablePreference></TextVariable>"#
        ),
        "the definition, in InDesign's spelling:\n{dm}"
    );
    // InDesign's order: definitions before the layers and spreads.
    assert!(pkg::pos(&dm, "<TextVariable ") < pkg::pos(&dm, "<Layer "), "{dm}");

    let story = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(
        story.contains(
            r#"<CharacterStyleRange PageNumberType="TextVariable"><TextVariableInstance Self="st1tvi0" Name="Price" ResultText="9.99" AssociatedTextVariable="dTextVariablenPrice"/></CharacterStyleRange>"#
        ),
        "the instance, in a range of its own:\n{story}"
    );

    // Reopened: the definition and the instance are the model's again, in
    // the same place, and a second save changes nothing.
    let doc2 = pkg::open(&out);
    let v = doc2
        .designmap
        .text_variables
        .iter()
        .find(|v| v.self_id == ID)
        .expect("definition read back");
    assert_eq!(v.name.as_deref(), Some("Price"));
    assert_eq!(v.variable_type.as_deref(), Some("CustomTextType"));
    assert_eq!(v.contents.as_deref(), Some("9.99"));
    assert_eq!(texts(&doc2), want, "{story}");
    let twice = write_idml(&doc2, &out).expect("write again");
    pkg::assert_same_package(&out, &twice);
}

#[test]
fn an_instance_inserted_inside_a_run_splits_it() {
    check(6);
}

#[test]
fn an_instance_inserted_at_the_start_of_a_run() {
    check(0);
}

#[test]
fn an_instance_inserted_at_the_end_of_a_run() {
    check("Hello world".len());
}

#[test]
fn a_story_written_whole_carries_the_instance() {
    let mut doc = pkg::open(&pkg::simple("", pkg::STORY_BODY, pkg::STYLES));
    insert_instance(&mut doc, 6);
    let part = idml_export::emit_story_part_for_test("u77", &doc.stories[0].story).expect("emit");
    let part = String::from_utf8(part).unwrap();
    assert!(
        part.contains(r#"<TextVariableInstance Self="u77tvi0" Name="Price" ResultText="9.99" AssociatedTextVariable="dTextVariablenPrice"/>"#),
        "{part}"
    );
    let reread = idml_import::parse_story(part.as_bytes()).expect("parse");
    assert_eq!(
        reread.paragraphs[0]
            .runs
            .iter()
            .map(|r| (r.text.clone(), r.text_variable.clone()))
            .collect::<Vec<_>>(),
        texts(&doc)
    );
}

#[test]
fn a_changed_definition_is_rewritten_and_a_deleted_one_dropped() {
    let src = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&src);
    doc.designmap.text_variables.push(price());
    let mut other = price();
    other.self_id = "dTextVariablenGone".to_string();
    other.name = Some("Gone".to_string());
    doc.designmap.text_variables.push(other);
    let with_both = write_idml(&doc, &src).expect("write");
    // Unmutated, the package with definitions saves byte-identical.
    let reopened = pkg::open(&with_both);
    let same = write_idml(&reopened, &with_both).expect("write");
    pkg::assert_same_package(&with_both, &same);

    let mut doc = pkg::open(&with_both);
    doc.designmap
        .text_variables
        .retain(|v| v.self_id != "dTextVariablenGone");
    doc.designmap.text_variables[0].contents = Some("12.50".to_string());
    let out = write_idml(&doc, &with_both).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert!(!dm.contains("Gone"), "{dm}");
    assert!(dm.contains(">12.50</Contents>"), "{dm}");
    assert!(!dm.contains("9.99"), "{dm}");
    let doc2 = pkg::open(&out);
    assert_eq!(doc2.designmap.text_variables.len(), 1);
    assert_eq!(
        doc2.designmap.text_variables[0].contents.as_deref(),
        Some("12.50")
    );
}
