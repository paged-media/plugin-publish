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

//! ADR 131 — page-item `Name`s and InDesign's `<?AID xxxx?>` label spelling,
//! against InDesign 2025's own output (tests/fixtures/indesign-2025).

use idml_import::{decode_aid_escapes, parse_spread_with_provenance};

const KVP: &str = include_str!("fixtures/indesign-2025/aid-label-keyvaluepair.xml");
const TEXT_FRAME: &str = include_str!("fixtures/indesign-2025/textframe-start-tag.xml");
const INJECTED: &str = include_str!("fixtures/indesign-2025/label-value.json");

/// A spread around the recorded text frame start tag and its recorded label.
fn recorded_spread() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Spread Self="ue7">
    <Page Self="p1" GeometricBounds="0 0 792 612"/>
    {}
      <Properties><Label>{}</Label></Properties>
    </TextFrame>
  </Spread>
</idPkg:Spread>"#,
        TEXT_FRAME.trim(),
        KVP.trim()
    )
}

#[test]
fn the_recorded_indesign_label_decodes_to_the_value_we_wrote() {
    assert!(KVP.contains("&lt;?AID d83d?&gt;&lt;?AID de00?&gt;"), "the recording carries the AID spelling");
    let (spread, _) = parse_spread_with_provenance(recorded_spread().as_bytes()).expect("parse");
    let labels = spread.labels.get("u100").expect("u100 labelled");
    let (_, value) = labels.iter().find(|(k, _)| k == "x-paged:aid").expect("x-paged:aid");
    assert!(!value.contains("<?AID"), "no AID escape survives the reader");
    assert_eq!(value.as_str(), INJECTED.trim_end_matches('\n'));
    assert!(value.contains("Grüße – 日本語 😀"));
}

#[test]
fn the_recorded_frame_name_is_read() {
    let (_, prov) = parse_spread_with_provenance(recorded_spread().as_bytes()).expect("parse");
    assert_eq!(prov.item_names().get("u100").map(String::as_str), Some("card-text"));
}

#[test]
fn names_on_all_six_kinds_groups_included_and_unnamed_items_skipped() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Spread Self="s1">
    <Page Self="p1" GeometricBounds="0 0 792 612"/>
    <TextFrame Self="t1" Name="headline" ParentStory="st1" ItemTransform="1 0 0 1 0 0"/>
    <Rectangle Self="r1" Name="photo &amp; caption" GeometricBounds="0 0 10 10" ItemTransform="1 0 0 1 0 0"/>
    <Oval Self="o1" Name="badge" GeometricBounds="0 0 10 10" ItemTransform="1 0 0 1 0 0"/>
    <Polygon Self="pg1" Name="star" GeometricBounds="0 0 10 10" ItemTransform="1 0 0 1 0 0"/>
    <GraphicLine Self="l1" Name="rule" GeometricBounds="0 0 0 10" ItemTransform="1 0 0 1 0 0"/>
    <Group Self="g1" Name="card" ItemTransform="1 0 0 1 0 0">
      <Rectangle Self="r2" Name="$ID/" GeometricBounds="0 0 10 10" ItemTransform="1 0 0 1 0 0"/>
      <Rectangle Self="r3" Name="" GeometricBounds="0 0 10 10" ItemTransform="1 0 0 1 0 0"/>
    </Group>
    <Rectangle Self="r4" GeometricBounds="0 0 10 10" ItemTransform="1 0 0 1 0 0"/>
  </Spread>
</idPkg:Spread>"#;
    let (_, prov) = parse_spread_with_provenance(xml.as_bytes()).expect("parse");
    let mut names: Vec<_> = prov.item_names().iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            ("g1", "card"),
            ("l1", "rule"),
            ("o1", "badge"),
            ("pg1", "star"),
            ("r1", "photo & caption"),
            ("t1", "headline"),
        ]
    );
}

#[cfg(feature = "item-names")]
#[test]
fn the_model_carries_the_names() {
    let (spread, prov) = parse_spread_with_provenance(recorded_spread().as_bytes()).expect("parse");
    assert_eq!(&spread.item_names, prov.item_names());
}

#[test]
fn aid_decoding_edges() {
    assert_eq!(decode_aid_escapes("plain"), "plain");
    assert_eq!(decode_aid_escapes("a <?AID d83d?><?AID de00?> b"), "a 😀 b");
    // Two characters back to back, and a BMP code unit.
    assert_eq!(decode_aid_escapes("<?AID d83d?><?AID de00?><?AID d83c?><?AID df89?><?AID 00e9?>"), "😀🎉é");
    // A lone surrogate is kept verbatim rather than lost.
    assert_eq!(decode_aid_escapes("x<?AID d83d?>y"), "x<?AID d83d?>y");
    // Not an escape: left alone.
    assert_eq!(decode_aid_escapes("<?AID zz?> <?AID "), "<?AID zz?> <?AID ");
}
