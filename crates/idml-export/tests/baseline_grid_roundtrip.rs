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

//! Align to baseline grid, in InDesign's spelling: `GridAlignment` and
//! `GridAlignFirstLineOnly` on a paragraph style and on a range, the
//! document grid in `Resources/Preferences.xml`'s `<GridPreference>`,
//! and a frame's own grid in its `<BaselineFrameGridOption>`. Read, an
//! unchanged package written back byte for byte, an edited alignment
//! written where InDesign reads it and read back.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use paged_model::{BaselineFrameGridRelativeOption, GridAlignment};

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Styles xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<RootCharacterStyleGroup Self="u9d"><CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]"/></RootCharacterStyleGroup>
<RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]"/><ParagraphStyle Self="ParagraphStyle/Body" Name="Body" GridAlignment="AlignBaseline" GridAlignFirstLineOnly="false"/></RootParagraphStyleGroup>
</idPkg:Styles>"#;

const PREFS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<TextPreference UseOpticalSize="false"/>
<GridPreference BaselineGridShown="false" BaselineStart="36" BaselineDivision="14" BaselineGridRelativeOption="TopOfMarginOfBaselineGridRelativeOption"/>
</idPkg:Preferences>"#;

const SPREAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"><TextFramePreference TextColumnCount="1"/><BaselineFrameGridOption UseCustomBaselineFrameGrid="true" StartingOffsetForBaselineFrameGrid="5" BaselineFrameGridRelativeOption="TopOfInset" BaselineFrameGridIncrement="9"><Properties><BaselineFrameGridColor type="enumeration">LightBlue</BaselineFrameGridColor></Properties></BaselineFrameGridOption></TextFrame>
</Spread></idPkg:Spread>"#;

const STORY: &str = r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body"><CharacterStyleRange><Content>On the grid</Content><Br/></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body" GridAlignment="None"><CharacterStyleRange><Content>Off the grid</Content></CharacterStyleRange></ParagraphStyleRange>"#;

fn source() -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", STYLES),
        ("Resources/Preferences.xml", PREFS),
        ("Spreads/Spread_s1.xml", SPREAD),
        ("Stories/Story_st1.xml", &pkg::story(STORY)),
    ])
}

#[test]
fn indesigns_spelling_is_read() {
    let doc = pkg::open(&source());
    let body = &doc.styles.paragraph_styles["ParagraphStyle/Body"];
    assert_eq!(body.grid_alignment, Some(GridAlignment::AlignBaseline));
    assert_eq!(body.grid_align_first_line_only, Some(false));
    let paragraphs = &doc.stories[0].story.paragraphs;
    assert_eq!(paragraphs[0].grid_alignment, None);
    assert_eq!(paragraphs[1].grid_alignment, Some(GridAlignment::None));
    // The range's None wins over the style's AlignBaseline.
    assert_eq!(
        doc.resolved_paragraph_attrs(&paragraphs[0]).grid_alignment,
        Some(GridAlignment::AlignBaseline)
    );
    assert_eq!(
        doc.resolved_paragraph_attrs(&paragraphs[1]).grid_alignment,
        Some(GridAlignment::None)
    );
    let grid = doc.designmap.grid_preference.baseline_grid();
    assert!(grid.from_top_margin);
    assert_eq!((grid.start, grid.division), (36.0, 14.0));
    let frame = &doc.spreads[0].spread.text_frames[0];
    assert_eq!(
        frame.baseline_frame_grid.and_then(|g| g.custom()),
        Some((BaselineFrameGridRelativeOption::TopOfInset, 5.0, 9.0))
    );
}

#[test]
fn an_unchanged_package_is_written_back_byte_for_byte() {
    let src = source();
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    for part in [
        "Resources/Styles.xml",
        "Resources/Preferences.xml",
        "Spreads/Spread_s1.xml",
        "Stories/Story_st1.xml",
    ] {
        assert_eq!(pkg::entry(&out, part), pkg::entry(&src, part), "{part}");
    }
}

#[test]
fn an_edited_alignment_is_written_where_indesign_reads_it_and_reads_back() {
    let src = source();
    let mut doc = pkg::open(&src);
    {
        let paragraphs = &mut doc.stories[0].story.paragraphs;
        paragraphs[0].grid_alignment = Some(GridAlignment::AlignBaseline);
        paragraphs[0].grid_align_first_line_only = Some(true);
        paragraphs[1].grid_alignment = None;
    }
    doc.styles
        .paragraph_styles
        .get_mut("ParagraphStyle/Body")
        .unwrap()
        .grid_align_first_line_only = Some(true);
    let out = write_idml(&doc, &src).expect("write");

    let story = pkg::entry(&out, "Stories/Story_st1.xml").expect("story");
    assert!(
        story.contains(r#"GridAlignment="AlignBaseline" GridAlignFirstLineOnly="true""#),
        "{story}"
    );
    assert_eq!(story.matches("GridAlignment=").count(), 1, "{story}");
    let styles = pkg::entry(&out, "Resources/Styles.xml").expect("styles");
    assert!(
        styles.contains(r#"GridAlignment="AlignBaseline" GridAlignFirstLineOnly="true""#),
        "{styles}"
    );
    // The frame's own grid is the source's element, untouched.
    assert_eq!(
        pkg::entry(&out, "Spreads/Spread_s1.xml"),
        pkg::entry(&src, "Spreads/Spread_s1.xml")
    );

    let back = pkg::open(&out);
    let paragraphs = &back.stories[0].story.paragraphs;
    assert_eq!(
        paragraphs[0].grid_alignment,
        Some(GridAlignment::AlignBaseline)
    );
    assert_eq!(paragraphs[0].grid_align_first_line_only, Some(true));
    assert_eq!(paragraphs[1].grid_alignment, None);
    let body = &back.styles.paragraph_styles["ParagraphStyle/Body"];
    assert_eq!(body.grid_alignment, Some(GridAlignment::AlignBaseline));
    assert_eq!(body.grid_align_first_line_only, Some(true));
    assert_eq!(
        back.spreads[0].spread.text_frames[0].baseline_frame_grid,
        doc.spreads[0].spread.text_frames[0].baseline_frame_grid
    );
}
