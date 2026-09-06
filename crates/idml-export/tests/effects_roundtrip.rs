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
        xml.contains(r#"<TransparencySetting><BlendingSetting Opacity="80"/><DropShadowSetting Mode="Drop" Opacity="75" XOffset="4" YOffset="4" Size="6" Distance="5.6569" Angle="135" UseGlobalLight="false" KnockedOut="true" EffectColor="Color/Black"/><InnerShadowSetting Applied="true" Opacity="50" XOffset="2" YOffset="2" Size="5" Distance="2.83" Angle="135" ChokeAmount="10" Noise="0" BlendMode="Multiply" EffectColor="Color/Black"/><OuterGlowSetting Applied="true" Opacity="60" Size="9" Spread="20" Noise="0" BlendMode="Screen" EffectColor="Color/Red"/><FeatherSetting Mode="Standard" Width="7" ChokeAmount="0" Noise="0" CornerType="Diffusion"/></TransparencySetting>"#),
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

/// The enum-typed and oddly-named effect attributes InDesign reads
/// (measured 2026-09-06 on its own export): `InnerGlowSetting Spread`
/// (not ChokeAmount) and `Source="EdgeSourced"`, `Technique=
/// "SmoothContour"`, `SatinSetting InvertEffect`, `DirectionalFeather
/// Noise`, `GradientFeatherSetting Angle` + `Length`. One unknown
/// enumerator left the whole effect unreadable and the object white.
#[test]
fn effect_enumerators_and_attribute_names_are_indesigns() {
    let src = source(SPREAD_WITH_RECT);
    let mut doc = pkg::open(&src);
    let mut r = doc.spreads[0].spread.rectangles[0].clone();
    r.self_id = Some("r3".into());
    r.effects = Some(FrameEffects {
        inner_glow: Some(idml_import::InnerGlowParams {
            size: Some(9.0),
            opacity_pct: Some(80.0),
            effect_color: Some("Color/Red".into()),
            choke_pct: Some(8.0),
            blend_mode: Some("Screen".into()),
            source: Some("EdgeGlow".into()),
            noise_pct: Some(0.0),
        }),
        bevel: Some(idml_import::BevelEmbossParams {
            depth_pct: Some(120.0),
            size: Some(8.0),
            angle_deg: Some(120.0),
            altitude_deg: Some(30.0),
            highlight_color: Some("Color/Red".into()),
            shadow_color: Some("Color/Black".into()),
            highlight_opacity_pct: Some(80.0),
            shadow_opacity_pct: Some(70.0),
            style: Some("InnerBevel".into()),
            direction: Some("Up".into()),
            technique: Some("Smooth".into()),
            soften: Some(2.0),
        }),
        satin: Some(idml_import::SatinParams {
            size: Some(11.0),
            angle_deg: Some(20.0),
            distance: Some(7.0),
            effect_color: Some("Color/Black".into()),
            opacity_pct: Some(55.0),
            blend_mode: Some("Multiply".into()),
            invert: Some(false),
        }),
        directional_feather: Some(idml_import::DirectionalFeatherParams {
            left_width: Some(14.0),
            right_width: Some(3.0),
            top_width: Some(8.0),
            bottom_width: Some(0.0),
            angle_deg: Some(0.0),
            noise_pct: Some(0.0),
            choke_pct: Some(0.0),
            corner_type: None,
        }),
        gradient_feather: Some(idml_import::GradientFeatherParams {
            gradient_type: Some("Linear".into()),
            start_point: Some((0.0, 0.0)),
            end_point: Some((30.0, 40.0)),
            angle_deg: Some(0.0),
            stops: Vec::new(),
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
    for expected in [
        r#"<InnerGlowSetting Applied="true" Opacity="80" Size="9" Spread="8" Noise="0" BlendMode="Screen" Source="EdgeSourced" EffectColor="Color/Red"/>"#,
        r#"Style="InnerBevel" Direction="Up" Technique="SmoothContour"/>"#,
        r#"BlendMode="Multiply" EffectColor="Color/Black" InvertEffect="false"/>"#,
        r#"<DirectionalFeatherSetting Applied="true" LeftWidth="14" RightWidth="3" TopWidth="8" BottomWidth="0" Angle="0" ChokeAmount="0" Noise="0"/>"#,
        r#"<GradientFeatherSetting Applied="true" Type="Linear" GradientStart="0 0" Length="50" Angle="0"/>"#,
    ] {
        assert!(xml.contains(expected), "missing {expected}\n{xml}");
    }
    for forbidden in [
        "ChokeAmount=\"8\"",
        "EdgeGlow",
        "\"Smooth\"",
        " Invert=",
        "NoiseAmount",
        "GradientAngle",
        "GradientEnd",
    ] {
        assert!(!xml.contains(forbidden), "still writes {forbidden}\n{xml}");
    }
    let re = pkg::open(&out);
    let fx = re.spreads[0].spread.rectangles[1]
        .effects
        .as_ref()
        .expect("effects");
    assert_eq!(fx.inner_glow.as_ref().and_then(|g| g.choke_pct), Some(8.0));
    assert_eq!(fx.satin.as_ref().and_then(|s| s.invert), Some(false));
    assert_eq!(
        fx.directional_feather.as_ref().and_then(|f| f.noise_pct),
        Some(0.0)
    );
    assert_eq!(
        fx.gradient_feather.as_ref().and_then(|g| g.angle_deg),
        Some(0.0)
    );
}
