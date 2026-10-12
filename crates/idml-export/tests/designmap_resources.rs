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

//! Design-map resources: InDesign writes `<NumberingList>` and
//! `<ColorGroup>` (with `<ColorGroupSwatch>` children) in `designmap.xml`.
//! The importer reads them there, and a save does not define a design-map
//! list a second time in Styles.xml.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

const DESIGNMAP_BODY: &str = r#"<NumberingList Self="NumberingList/$ID/[Default]" Name="$ID/[Default]" ContinueNumbersAcrossStories="false" ContinueNumbersAcrossDocuments="false"/>
<NumberingList Self="NumberingList/Steps" Name="Steps" ContinueNumbersAcrossStories="true" ContinueNumbersAcrossDocuments="false"/>
<ColorGroup Self="ColorGroup/[Root Color Group]" Name="[Root Color Group]" IsRootColorGroup="true"><ColorGroupSwatch Self="u18ColorGroupSwatch0" SwatchItemRef="Swatch/None"/></ColorGroup>
<ColorGroup Self="ColorGroup/Brand" Name="Brand" IsRootColorGroup="false"><ColorGroupSwatch Self="u1" SwatchItemRef="Color/Red"/><ColorGroupSwatch Self="u2" SwatchItemRef="Color/Blue"/></ColorGroup>
"#;

#[test]
fn designmap_lists_and_groups_are_read_where_indesign_writes_them() {
    let doc = pkg::open(&pkg::simple(DESIGNMAP_BODY, pkg::STORY_BODY, pkg::STYLES));
    let steps = &doc.styles.numbering_lists["NumberingList/Steps"];
    assert_eq!(steps.continue_across_stories, Some(true));
    assert!(doc
        .styles
        .numbering_lists
        .contains_key("NumberingList/$ID/[Default]"));
    let brand = &doc.palette.color_groups["ColorGroup/Brand"];
    assert_eq!(brand.members, ["Color/Red", "Color/Blue"]);
    assert!(
        !doc.palette
            .color_groups
            .contains_key("ColorGroup/[Root Color Group]"),
        "the root group is InDesign's container, not a group"
    );
}

#[test]
fn a_save_does_not_define_a_designmap_list_twice() {
    let src = pkg::simple(DESIGNMAP_BODY, pkg::STORY_BODY, pkg::STYLES);
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    let styles = pkg::entry(&out, "Resources/Styles.xml").expect("styles");
    assert!(!styles.contains("NumberingList/Steps"), "{styles}");
    let dm = pkg::entry(&out, "designmap.xml").expect("designmap");
    assert_eq!(
        dm.matches("Self=\"NumberingList/Steps\"").count(),
        1,
        "{dm}"
    );
}
