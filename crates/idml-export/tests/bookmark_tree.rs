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

//! The bookmark tree saved as InDesign 20.0.1 writes and reads it (core
//! ADR 173; `bookmarks-probe`): a child's `<Bookmark>` inside its
//! parent's, a page destination a bookmark made hidden and fitted to the
//! window. An unchanged tree replays byte for byte; a changed one is
//! written whole from the model, after every destination, since InDesign
//! drops a bookmark whose destination it has not read yet.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{Bookmark, HyperlinkDestination, HyperlinkDestinationKind};

/// InDesign's own export of two bookmarks, one nested (`bookmarks-probe`),
/// after the stories as it writes them.
const DESIGNMAP: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document DOMVersion="20.0" Self="d1" xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
<Layer Self="ub6" Name="Layer 1" Visible="true" Locked="false" Printable="true"/>
<idPkg:Spread src="Spreads/Spread_s1.xml"/>
<idPkg:Story src="Stories/Story_st1.xml"/>
<HyperlinkPageDestination Self="HyperlinkPageDestination/Lesezeichen 1" Name="Lesezeichen 1" NameManually="true" DestinationPage="p1" ViewSetting="FitWindow" ViewPercentage="100" Hidden="true" DestinationUniqueKey="1">
	<Properties>
		<ViewBounds Top="1e+256" Left="1e+256" Bottom="1e+256" Right="1e+256" />
	</Properties>
</HyperlinkPageDestination>
<Bookmark Self="u102" Name="Page one" Destination="HyperlinkPageDestination/Lesezeichen 1">
	<Bookmark Self="u107" Name="Under page one" Destination="HyperlinkPageDestination/Lesezeichen 1" />
</Bookmark>
<Bookmark Self="u105" Name="Second" Destination="HyperlinkPageDestination/Lesezeichen 1" />
</Document>"#;

fn source() -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", DESIGNMAP),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", pkg::SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

/// `(name, parent's name)` of every saved bookmark, in order.
fn saved_tree(out: &[u8]) -> Vec<(String, Option<String>)> {
    let dm = &pkg::open(out).designmap;
    dm.bookmarks
        .iter()
        .map(|b| {
            let parent = dm.bookmark_parents.get(&b.self_id).and_then(|p| {
                dm.bookmarks
                    .iter()
                    .find(|x| &x.self_id == p)
                    .and_then(|x| x.name.clone())
            });
            (b.name.clone().unwrap_or_default(), parent)
        })
        .collect()
}

fn s(v: &str) -> String {
    v.to_string()
}

#[test]
fn an_unchanged_tree_saves_byte_identically() {
    let source = source();
    let doc = pkg::open(&source);
    assert_eq!(doc.designmap.bookmarks.len(), 3);
    let out = write_idml(&doc, &source).expect("write");
    assert_eq!(
        pkg::entry(&out, "designmap.xml").unwrap(),
        DESIGNMAP,
        "the designmap replays"
    );
}

#[test]
fn a_renamed_and_moved_tree_is_written_whole_and_nested() {
    let source = source();
    let mut doc = pkg::open(&source);
    let dm = &mut doc.designmap;
    dm.bookmarks
        .iter_mut()
        .find(|b| b.self_id == "u105")
        .unwrap()
        .name = Some("Renamed".into());
    // "Under page one" moves under "Renamed" (after it, in order).
    let child = dm.bookmarks.remove(1);
    dm.bookmarks.push(child);
    dm.bookmark_parents.insert("u107".into(), "u105".into());
    let out = write_idml(&doc, &source).expect("write");
    assert_eq!(
        saved_tree(&out),
        [
            (s("Page one"), None),
            (s("Renamed"), None),
            (s("Under page one"), Some(s("Renamed")))
        ]
    );
    let dm = pkg::entry(&out, "designmap.xml").unwrap();
    assert!(
        dm.contains(r#"<Bookmark Self="u105" Name="Renamed" Destination="HyperlinkPageDestination/Lesezeichen 1"><Bookmark Self="u107" Name="Under page one" Destination="HyperlinkPageDestination/Lesezeichen 1"/></Bookmark>"#),
        "{dm}"
    );
    // The source's destination stands where it stood, before the tree.
    assert!(
        pkg::pos(&dm, "<HyperlinkPageDestination ") < pkg::pos(&dm, "<Bookmark "),
        "{dm}"
    );
    assert_eq!(dm.matches("<Bookmark ").count(), 3, "{dm}");
}

#[test]
fn a_new_page_bookmark_is_written_after_its_destination_in_indesigns_spelling() {
    let source = source();
    let mut doc = pkg::open(&source);
    let dm = &mut doc.designmap;
    dm.hyperlink_destinations.push(HyperlinkDestination {
        self_id: "HyperlinkPageDestination/u200".into(),
        kind: HyperlinkDestinationKind::Page("p1".into()),
    });
    dm.bookmarks.insert(
        2,
        Bookmark {
            self_id: "u200".into(),
            name: Some("New under page one".into()),
            destination: Some("HyperlinkPageDestination/u200".into()),
        },
    );
    dm.bookmark_parents.insert("u200".into(), "u102".into());
    let out = write_idml(&doc, &source).expect("write");
    assert_eq!(
        saved_tree(&out),
        [
            (s("Page one"), None),
            (s("Under page one"), Some(s("Page one"))),
            (s("New under page one"), Some(s("Page one"))),
            (s("Second"), None)
        ]
    );
    let dm = pkg::entry(&out, "designmap.xml").unwrap();
    let dest = pkg::pos(
        &dm,
        r#"<HyperlinkPageDestination Self="HyperlinkPageDestination/u200""#,
    );
    assert!(dest < pkg::pos(&dm, r#"<Bookmark Self="u200""#), "{dm}");
    assert!(
        dm.contains(
            r#"DestinationPage="p1" ViewSetting="FitWindow" ViewPercentage="100" Hidden="true""#
        ),
        "{dm}"
    );
    assert!(
        dm.contains(r#"<ViewBounds Top="1e+256" Left="1e+256" Bottom="1e+256" Right="1e+256"/>"#),
        "{dm}"
    );
}

#[test]
fn a_deleted_bookmark_leaves_the_designmap_with_its_children() {
    let source = source();
    let mut doc = pkg::open(&source);
    let dm = &mut doc.designmap;
    dm.bookmarks.retain(|b| b.self_id == "u105");
    dm.bookmark_parents.clear();
    let out = write_idml(&doc, &source).expect("write");
    assert_eq!(saved_tree(&out), [(s("Second"), None)]);
    let dm = pkg::entry(&out, "designmap.xml").unwrap();
    assert_eq!(dm.matches("<Bookmark ").count(), 1, "{dm}");
}
