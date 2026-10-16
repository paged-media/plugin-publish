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

//! Type on a path is written once, however often the document is saved.
//!
//! A text path attached through the wire carries no `Self` of its own;
//! the writer names it after its story (`<story>tp0`). The next save
//! looked for the model's id — none — among the paths the source part
//! already carried, did not find it, and appended the path again: a
//! book saved through 21 checkpoints carried 21 copies of one path, and
//! the reader set the story once per copy, heavier and larger each time.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const LINE: &str = r#"<Properties><PathGeometry><GeometryPathType PathOpen="true"><PathPointArray><PathPointType Anchor="100 100" LeftDirection="100 100" RightDirection="100 100"/><PathPointType Anchor="300 100" LeftDirection="300 100" RightDirection="300 100"/></PathPointArray></GeometryPathType></PathGeometry></Properties>"#;

fn source(text_paths: &str) -> Vec<u8> {
    let spread = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="600 100 700 400" ItemTransform="1 0 0 1 0 0"/>
<Polygon Self="line" ItemTransform="1 0 0 1 0 0">{LINE}{text_paths}</Polygon>
</Spread></idPkg:Spread>"#
    );
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", &spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

/// The path as the wire attaches it: no `Self`.
fn wire_path() -> idml_import::TextPath {
    idml_import::TextPath {
        self_id: None,
        parent_story: "st1".to_string(),
        path_alignment: None,
        path_type_alignment: Some("CenterPathType".to_string()),
        path_effect: None,
        flip_path_effect: None,
        start_bracket: None,
        end_bracket: None,
    }
}

fn paths_on_line(doc: &paged_scene::Document) -> usize {
    doc.spreads[0]
        .spread
        .polygons
        .iter()
        .find(|p| p.self_id.as_deref() == Some("line"))
        .expect("host")
        .text_paths
        .len()
}

/// Save, then save the saved package again with the SAME model — the
/// engine reloads its native model, not the package, so the path still
/// has no id on the second save.
#[test]
fn a_path_without_an_id_is_written_once_across_saves() {
    let mut src = source("");
    let mut doc = pkg::open(&src);
    doc.spreads[0].spread.polygons[0].text_paths = vec![wire_path()];
    for save in 1..=3 {
        let out = write_idml(&doc, &src).expect("write");
        let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
        assert_eq!(
            xml.matches("<TextPath ").count(),
            1,
            "save {save} carries one path:\n{xml}"
        );
        assert_eq!(paths_on_line(&pkg::open(&out)), 1, "save {save}");
        src = out;
    }
}

/// A part an earlier export filled with copies is repaired: one path
/// per `Self` survives.
#[test]
fn copies_an_earlier_export_appended_are_dropped() {
    let copy = r#"<TextPath Self="st1tp0" ParentStory="st1" TextAlignment="CenterTextAlignment" EndBracket="200"/>"#;
    let src = source(&copy.repeat(21));
    let mut doc = pkg::open(&src);
    assert_eq!(paths_on_line(&doc), 21, "the damaged part, as read");
    doc.spreads[0].spread.polygons[0].text_paths = vec![wire_path()];
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert_eq!(xml.matches("<TextPath ").count(), 1, "{xml}");
    assert!(xml.contains(copy), "the first copy keeps its bytes:\n{xml}");
    assert_eq!(paths_on_line(&pkg::open(&out)), 1);
}
