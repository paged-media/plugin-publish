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

//! A LINKED image (`image_link`, no bytes) on a frame whose element has
//! no placed-image child reaches the export as `<Image>` + `<Link
//! LinkResourceURI=…>` and reads back; an existing link is patched in
//! place, and left alone byte-for-byte when it agrees.

//! A page item's effects reach the part. The exporter used to write the
//! blending setting alone, so every drop shadow, glow, bevel, satin and
//! feather the annual's effects page authored reached InDesign — and the
//! export's own twin — as a plain square (measured 2026-09-06).

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{
    DropShadowSetting, FeatherParams, FrameEffects, InnerShadowParams, OuterGlowParams,
};

const SPREAD_WITH_RECT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<TextFrame Self="tf1" ParentStory="st1" GeometricBounds="100 100 400 400" ItemTransform="1 0 0 1 0 0"/>
<Rectangle Self="r1" GeometricBounds="450 100 600 300" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red"/>
</Spread></idPkg:Spread>"#;

fn source(spread: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Spreads/Spread_s1.xml", spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

#[test]
fn an_inserted_frame_carries_its_effects_and_reads_them_back() {
    let src = source(SPREAD_WITH_RECT);
    let mut doc = pkg::open(&src);
    let mut r = doc.spreads[0].spread.rectangles[0].clone();
    r.self_id = Some("r2".into());
    r.opacity = Some(80.0);
    r.drop_shadow = Some(DropShadowSetting {
        mode: "Drop".into(),
        x_offset: 4.0,
        y_offset: 4.0,
        size: 6.0,
        opacity_pct: 75.0,
        effect_color: Some("Color/Black".into()),
    });
    r.effects = Some(FrameEffects {
        inner_shadow: Some(InnerShadowParams {
            x_offset: Some(2.0),
            y_offset: Some(2.0),
            size: Some(5.0),
            opacity_pct: Some(50.0),
            effect_color: Some("Color/Black".into()),
            angle_deg: Some(135.0),
            distance: Some(2.83),
            choke_pct: Some(10.0),
            blend_mode: Some("Multiply".into()),
            noise_pct: Some(0.0),
        }),
        outer_glow: Some(OuterGlowParams {
            size: Some(9.0),
            opacity_pct: Some(60.0),
            effect_color: Some("Color/Red".into()),
            spread_pct: Some(20.0),
            blend_mode: Some("Screen".into()),
            noise_pct: Some(0.0),
        }),
        feather: Some(FeatherParams {
            width: Some(7.0),
            corner_type: Some("Diffusion".into()),
            noise_pct: Some(0.0),
            choke_pct: Some(0.0),
        }),
        ..Default::default()
    });
    doc.spreads[0].spread.rectangles.push(r);
    doc.spreads[0]
        .spread
        .frames_in_order
        .push(idml_import::FrameRef::Rectangle(1));
    let out = write_idml(&doc, &src).expect("write");
    let xml = pkg::entry(&out, "Spreads/Spread_s1.xml").expect("spread");
    assert!(
        xml.contains(r#"<TransparencySetting><BlendingSetting Opacity="80"/><DropShadowSetting Mode="Drop" Opacity="75" XOffset="4" YOffset="4" Size="6" Distance="5.6569" Angle="135" UseGlobalLight="false" KnockedOut="true" EffectColor="Color/Black"/><InnerShadowSetting Applied="true" Opacity="50" XOffset="2" YOffset="2" Size="5" Distance="2.83" Angle="135" ChokeAmount="10" Noise="0" BlendMode="Multiply" EffectColor="Color/Black"/><OuterGlowSetting Applied="true" Opacity="60" Size="9" Spread="20" Noise="0" BlendMode="Screen" EffectColor="Color/Red"/><FeatherSetting Applied="true" Width="7" ChokeAmount="0" Noise="0" CornerType="Diffusion"/></TransparencySetting>"#),
        "{xml}"
    );
    let re = pkg::open(&out);
    let back = &re.spreads[0].spread.rectangles[1];
    assert_eq!(back.opacity, Some(80.0));
    let shadow = back.drop_shadow.as_ref().expect("drop shadow");
    assert_eq!(
        (
            shadow.x_offset,
            shadow.y_offset,
            shadow.size,
            shadow.opacity_pct
        ),
        (4.0, 4.0, 6.0, 75.0)
    );
    let fx = back.effects.as_ref().expect("effects");
    assert_eq!(fx.outer_glow.as_ref().and_then(|g| g.size), Some(9.0));
    assert_eq!(
        fx.inner_shadow.as_ref().and_then(|s| s.choke_pct),
        Some(10.0)
    );
    assert_eq!(fx.feather.as_ref().and_then(|f| f.width), Some(7.0));
    assert!(fx.bevel.is_none());
}
