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

//! InDesign 2025 is the oracle for the object-model export (ADR 131/132).
//!
//! `indesign/probe.sh` builds a document IN InDesign (`base.jsx`), runs the
//! ignored [`write_probe`] below to change it through the model and save it
//! with this crate's writer, then has InDesign open that file, save it as
//! an `.indd`, reopen it and record what it sees (`verify.jsx`). The
//! recording is committed under `fixtures/indesign-2025-object-model/` and
//! [`indesign_reads_the_object_model_export`] asserts it, so the facts stay
//! in CI without InDesign.
//!
//! The changes, each a defect the export had before (core's
//! `idml_export_object_model_state` and plugin-draw's round-trip pinned
//! them): a hidden rectangle and a locked text frame; a custom text
//! variable created and placed in the text; text on a path whose end
//! bracket the model leaves unset; a renamed page item.

use std::path::PathBuf;

use idml_import::{TextPath, TextVariable};

const RECORDING: &str = include_str!("fixtures/indesign-2025-object-model/verify.json");

fn item<'a>(spread: &'a idml_import::Spread, name: &str) -> &'a str {
    spread
        .item_names
        .iter()
        .find(|(_, n)| n.as_str() == name)
        .map(|(id, _)| id.as_str())
        .unwrap_or_else(|| panic!("no item named {name}"))
}

/// The model-side half of the probe. Ignored: it needs the InDesign-made
/// `base.idml` that `probe.sh` writes into `$PAGED_INDESIGN_PROBE`.
#[test]
#[ignore]
fn write_probe() {
    let Some(dir) = std::env::var_os("PAGED_INDESIGN_PROBE").map(PathBuf::from) else {
        eprintln!("SKIP: PAGED_INDESIGN_PROBE unset (run indesign/probe.sh)");
        return;
    };
    let src = std::fs::read(dir.join("base.idml")).expect("base.idml");
    let mut doc = idml_import::import_idml_doc(&src).expect("parse");
    let spread = &doc.spreads[0].spread;
    let body = item(spread, "body").to_string();
    let boxed = item(spread, "box").to_string();
    let line = item(spread, "baseline").to_string();
    let body_story = spread
        .text_frames
        .iter()
        .find(|f| f.self_id.as_deref() == Some(body.as_str()))
        .and_then(|f| f.parent_story.clone())
        .expect("body story");
    {
        let spread = &mut doc.spreads[0].spread;
        let r = spread
            .rectangles
            .iter_mut()
            .find(|r| r.self_id.as_deref() == Some(boxed.as_str()))
            .expect("box");
        r.visible = false;
        spread.item_names.insert(boxed.clone(), "price-box".into());
        let f = spread
            .text_frames
            .iter_mut()
            .find(|f| f.self_id.as_deref() == Some(body.as_str()))
            .expect("body");
        f.locked = true;
    }
    // A custom variable and an instance of it after "Hello " (what core's
    // createTextVariable + InsertField do to the model).
    doc.designmap.text_variables.push(TextVariable {
        self_id: "dTextVariablenPrice".into(),
        name: Some("Price".into()),
        variable_type: Some("CustomTextType".into()),
        contents: Some("9.99".into()),
        ..Default::default()
    });
    let si = doc
        .stories
        .iter()
        .position(|s| s.self_id == body_story)
        .expect("story");
    {
        let runs = &mut doc.stories[si].story.paragraphs[0].runs;
        let host = runs[0].clone();
        assert_eq!(host.text, "Hello world");
        let mut head = host.clone();
        head.text = "Hello ".into();
        let mut var = host.clone();
        var.text = "9.99".into();
        var.text_variable = Some("dTextVariablenPrice".into());
        let mut tail = host;
        tail.text = "world".into();
        *runs = vec![head, var, tail];
    }
    // Type on the line, in a story minted for it, from 10 pt to the end.
    let mut story = doc.stories[si].clone();
    story.self_id = "uprobetp".into();
    story.src = String::new();
    story.story.paragraphs.truncate(1);
    story.story.paragraphs[0].runs.truncate(1);
    story.story.paragraphs[0].runs[0].text = "Type on a path".into();
    story.story.paragraphs[0].runs[0].text_variable = None;
    doc.stories.push(story);
    {
        let spread = &mut doc.spreads[0].spread;
        let l = spread
            .graphic_lines
            .iter_mut()
            .find(|l| l.self_id.as_deref() == Some(line.as_str()))
            .expect("line");
        l.text_paths.push(TextPath {
            self_id: None,
            parent_story: "uprobetp".into(),
            path_alignment: None,
            path_type_alignment: None,
            path_effect: None,
            flip_path_effect: None,
            start_bracket: Some(10.0),
            end_bracket: None,
        });
    }
    doc.rebuild_indexes();
    let out = idml_export::write_idml(&doc, &src).expect("write");
    std::fs::write(dir.join("out.idml"), out).expect("write out.idml");
}

fn check(what: &str, v: &serde_json::Value) {
    let items = v["items"].as_array().expect("items");
    let named = |n: &str| {
        items
            .iter()
            .find(|i| i["name"] == n)
            .unwrap_or_else(|| panic!("{what}: no item named {n}: {v}"))
    };
    let boxed = named("price-box");
    assert_eq!(boxed["visible"], false, "{what}: the hidden box: {boxed}");
    assert_eq!(boxed["locked"], false, "{what}: {boxed}");
    let body = named("body");
    assert_eq!(body["locked"], true, "{what}: the locked frame: {body}");
    assert_eq!(body["visible"], true, "{what}: {body}");

    let vars = v["customVariables"].as_array().expect("vars");
    assert!(
        vars.iter()
            .any(|x| x["name"] == "Price" && x["contents"] == "9.99"),
        "{what}: the custom variable: {vars:?}"
    );
    let inst = v["variableInstances"].as_array().expect("instances");
    assert!(
        inst.iter().any(|x| x["variable"] == "Price"
            && x["resultText"] == "9.99"
            && x["story"].as_str().is_some_and(|s| s.contains("Hello "))),
        "{what}: the instance shows its value: {inst:?}"
    );

    let paths = v["textPaths"].as_array().expect("paths");
    let tp = paths
        .iter()
        .find(|p| p["contents"] == "Type on a path")
        .unwrap_or_else(|| panic!("{what}: the text on the path: {paths:?}"));
    assert_eq!(tp["overflows"], false, "{what}: {tp}");
    assert_eq!(tp["startBracket"], 10.0, "{what}: {tp}");
    let end = tp["endBracket"].as_f64().expect("end");
    assert!((end - 300.0).abs() < 0.01, "{what}: the 300 pt line: {tp}");
}

/// What InDesign 2025 recorded, opening the probe's export and again
/// after saving it as an `.indd` and reopening.
#[test]
fn indesign_reads_the_object_model_export() {
    let v: serde_json::Value = serde_json::from_str(RECORDING).expect("recording");
    check("opened", &v["opened"]);
    check("saved and reopened", &v["savedAndReopened"]);
}
