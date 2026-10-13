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

//! Two attributes as InDesign 20.0.1 reads them:
//! - `ShowMasterItems` lives on `<Spread>`; InDesign ignores it on `<Page>`
//!   (core tools/indesign-export/master-items-probe), so a minted spread
//!   carries it there and its pages do not.
//! - Word and letter spacing and glyph scaling set on a
//!   `<ParagraphStyleRange>` are the paragraph's own: read, saved
//!   byte-identically when unedited, rewritten when edited.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;

fn minted_spread(page_hides_master_items: bool) -> String {
    let source = pkg::simple("", pkg::STORY_BODY, pkg::STYLES);
    let mut doc = pkg::open(&source);
    let mut spread = idml_import::Spread {
        self_id: Some("s2".into()),
        ..Default::default()
    };
    spread.pages.push(idml_import::Page {
        self_id: Some("p2".into()),
        bounds: idml_import::Bounds {
            top: 0.0,
            left: 0.0,
            bottom: 792.0,
            right: 612.0,
        },
        applied_master: None,
        item_transform: None,
        master_page_transform: None,
        override_list: Vec::new(),
        name: None,
        show_master_items: page_hides_master_items.then_some(false),
    });
    doc.spreads.push(paged_scene::ParsedSpread {
        src: "Spreads/Spread_s2.xml".into(),
        spread,
    });
    let out = write_idml(&doc, &source).expect("write");
    pkg::entry(&out, "Spreads/Spread_s2.xml").expect("minted spread")
}

#[test]
fn a_minted_spread_carries_show_master_items_on_the_spread() {
    let hidden = minted_spread(true);
    let spread_tag = &hidden[hidden.find("<Spread ").unwrap()..];
    let spread_tag = &spread_tag[..spread_tag.find('>').unwrap()];
    assert!(
        spread_tag.contains(r#"ShowMasterItems="false""#),
        "{hidden}"
    );
    let page_tag = &hidden[hidden.find("<Page ").unwrap()..];
    let page_tag = &page_tag[..page_tag.find('>').unwrap()];
    assert!(!page_tag.contains("ShowMasterItems"), "{hidden}");

    let shown = minted_spread(false);
    assert!(shown.contains(r#"ShowMasterItems="true""#), "{shown}");
}

#[test]
fn range_spacing_is_read_and_saved() {
    let body = r#"<ParagraphStyleRange MinimumLetterSpacing="-5" DesiredLetterSpacing="10" MaximumLetterSpacing="25" DesiredWordSpacing="90" MinimumGlyphScaling="97" MaximumGlyphScaling="103"><CharacterStyleRange><Content>Hello world</Content></CharacterStyleRange></ParagraphStyleRange>"#;
    let source = pkg::simple("", body, pkg::STYLES);
    let mut doc = pkg::open(&source);
    {
        let p = &doc.stories[0].story.paragraphs[0];
        assert_eq!(p.minimum_letter_spacing, Some(-5.0));
        assert_eq!(p.desired_letter_spacing, Some(10.0));
        assert_eq!(p.maximum_letter_spacing, Some(25.0));
        assert_eq!(p.desired_word_spacing, Some(90.0));
        assert_eq!(p.minimum_word_spacing, None);
        assert_eq!(p.minimum_glyph_scaling, Some(97.0));
        assert_eq!(p.maximum_glyph_scaling, Some(103.0));
    }
    // Unedited: the package comes back as it went in.
    pkg::assert_same_package(&source, &write_idml(&doc, &source).expect("write"));
    // Edited: the values are rewritten in place.
    doc.stories[0].story.paragraphs[0].desired_letter_spacing = Some(20.0);
    doc.stories[0].story.paragraphs[0].maximum_word_spacing = Some(120.0);
    let out = write_idml(&doc, &source).expect("write");
    let story = pkg::entry(&out, &doc.stories[0].src).expect("story");
    assert!(story.contains(r#"DesiredLetterSpacing="20""#), "{story}");
    assert!(story.contains(r#"MaximumWordSpacing="120""#), "{story}");
    assert!(story.contains(r#"MinimumLetterSpacing="-5""#), "{story}");
}
