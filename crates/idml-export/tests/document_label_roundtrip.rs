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

//! The document's own labels travel through IDML.
//!
//! A plugin can keep state on the DOCUMENT (`DesignMap::labels`, one
//! `KeyValuePair` per key) rather than on a page item. The native model
//! kept it; IDML did not — it was neither read from nor written to the
//! designmap, so a save as IDML dropped it. Its IDML home is the one an
//! item's labels use, on the `<Document>` element:
//! `<Document><Properties><Label><KeyValuePair Key Value/>`.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

fn source(designmap: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", designmap),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

/// The helper's designmap with `properties` as the `<Document>`'s first
/// child.
fn designmap_with(properties: &str) -> String {
    pkg::designmap("").replacen(
        "<Layer Self=\"ub6\"",
        &format!("{properties}\n<Layer Self=\"ub6\""),
        1,
    )
}

#[test]
fn document_labels_are_read_and_nothing_else_is_taken_for_one() {
    let dm = designmap_with(
        r#"<Properties><Label><KeyValuePair Key="x-paged:web" Value="{&quot;v&quot;:1,&quot;data&quot;:{}}"/><KeyValuePair Key="vendor" Value="acme"/></Label></Properties>"#,
    )
    // A label on something ELSE in the designmap is not the document's.
    .replace(
        r#"Printable="true"/>"#,
        r#"Printable="true"><Properties><Label><KeyValuePair Key="layer" Value="no"/></Label></Properties></Layer>"#,
    );
    let doc = pkg::open(&source(&dm));
    assert_eq!(
        doc.designmap.labels,
        vec![
            (
                "x-paged:web".to_string(),
                r#"{"v":1,"data":{}}"#.to_string()
            ),
            ("vendor".to_string(), "acme".to_string()),
        ]
    );
}

#[test]
fn document_labels_are_written_kept_and_dropped() {
    let src = source(&pkg::designmap(""));
    let mut doc = pkg::open(&src);
    assert!(doc.designmap.labels.is_empty());

    // Added: written, and read back exactly — a multi-line JSON value
    // keeps its newlines.
    let value = "{\"v\":1,\n\"data\":{\"title\":\"A & B\"}}".to_string();
    doc.designmap.labels = vec![("x-paged:web".into(), value.clone())];
    let out = write_idml(&doc, &src).expect("write");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert!(dm.contains("<Properties><Label><KeyValuePair"), "{dm}");
    let back = pkg::open(&out);
    assert_eq!(back.designmap.labels, vec![("x-paged:web".into(), value)]);

    // Unchanged: the designmap is not touched.
    let again = write_idml(&back, &out).expect("re-write");
    assert_eq!(
        pkg::entry(&again, "designmap.xml"),
        pkg::entry(&out, "designmap.xml"),
        "a model that agrees with the part leaves it byte-identical"
    );

    // Changed in place: the label is replaced, not duplicated.
    let mut changed = back.clone();
    changed.designmap.labels = vec![("x-paged:web".into(), "{\"v\":2}".into())];
    let out2 = write_idml(&changed, &out).expect("write changed");
    let dm2 = pkg::entry(&out2, "designmap.xml").expect("designmap");
    assert_eq!(dm2.matches("<Label>").count(), 1, "{dm2}");
    assert_eq!(
        pkg::open(&out2).designmap.labels,
        vec![("x-paged:web".to_string(), "{\"v\":2}".to_string())]
    );

    // Removed: the label goes, and the empty wrapper with it.
    let mut cleared = back;
    cleared.designmap.labels.clear();
    let out3 = write_idml(&cleared, &out).expect("write cleared");
    let dm3 = pkg::entry(&out3, "designmap.xml").expect("designmap");
    assert!(!dm3.contains("<Label>"), "{dm3}");
    assert!(!dm3.contains("<Properties></Properties>"), "{dm3}");
    assert!(pkg::open(&out3).designmap.labels.is_empty());
}
