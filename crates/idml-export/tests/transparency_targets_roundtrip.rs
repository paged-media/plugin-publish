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

//! R-10 / R-11 / A-18 — a page item's transparency targets kept apart,
//! a group's isolate and knockout kept, and the effect attributes and the
//! global light read and written back.
//!
//! The XML is InDesign 20.0.1's own spelling, read off a document built
//! through its DOM: the object's `<TransparencySetting>`, then
//! `<StrokeTransparencySetting>`, `<FillTransparencySetting>` and — on a
//! text frame — `<ContentTransparencySetting>`; a placed picture's own
//! `<TransparencySetting>` inside `<Image>`; a group's
//! `<BlendingSetting KnockoutGroup IsolateBlending>`. The importer used to
//! collapse every `<BlendingSetting>` onto the object (the last one read
//! winning, the picture's included), and the exporter rewrote a mutated
//! item's block without the targets.

#[path = "support/pkg.rs"]
mod pkg;

use idml_export::write_idml;
use idml_import::{FrameRef, TargetTransparency};

const SPREAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<Rectangle Self="r1" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red" StrokeColor="Color/Black" StrokeWeight="6"><Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="50 50" LeftDirection="50 50" RightDirection="50 50"/><PathPointType Anchor="50 150" LeftDirection="50 150" RightDirection="50 150"/><PathPointType Anchor="150 150" LeftDirection="150 150" RightDirection="150 150"/><PathPointType Anchor="150 50" LeftDirection="150 50" RightDirection="150 50"/></PathPointArray></GeometryPathType></PathGeometry></Properties><TransparencySetting><BlendingSetting BlendMode="Multiply" Opacity="50" IsolateBlending="true" /><DropShadowSetting Mode="Drop" BlendMode="Screen" Noise="20" Spread="30" UseGlobalLight="true" KnockedOut="false" HonorOtherEffects="true" /><OuterGlowSetting Applied="true" Technique="Precise" /></TransparencySetting><StrokeTransparencySetting><BlendingSetting BlendMode="Multiply" Opacity="30" /><DropShadowSetting Mode="Drop" /></StrokeTransparencySetting><FillTransparencySetting><BlendingSetting BlendMode="Screen" Opacity="40" /><DropShadowSetting Mode="Drop" XOffset="11" YOffset="11" Size="5" /></FillTransparencySetting><ContentTransparencySetting><BlendingSetting Opacity="20" /></ContentTransparencySetting><Image Self="i1" ItemTransform="1 0 0 1 60 60"><Properties/><Link Self="l1" LinkResourceURI="file:quad.png"/><TransparencySetting><BlendingSetting BlendMode="Multiply" Opacity="30" /></TransparencySetting></Image></Rectangle>
<TextFrame Self="tf1" ParentStory="st1" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red"><Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="200 50" LeftDirection="200 50" RightDirection="200 50"/><PathPointType Anchor="200 150" LeftDirection="200 150" RightDirection="200 150"/><PathPointType Anchor="400 150" LeftDirection="400 150" RightDirection="400 150"/><PathPointType Anchor="400 50" LeftDirection="400 50" RightDirection="400 50"/></PathPointArray></GeometryPathType></PathGeometry></Properties><TransparencySetting><BevelAndEmbossSetting Applied="true" HighlightBlendMode="Overlay" ShadowBlendMode="Darken" /><DirectionalFeatherSetting Applied="true" FollowShapeMode="AllEdges" /></TransparencySetting><FillTransparencySetting><BlendingSetting Opacity="70" /></FillTransparencySetting><ContentTransparencySetting><BlendingSetting BlendMode="Multiply" Opacity="60" /><DropShadowSetting Mode="Drop" /></ContentTransparencySetting></TextFrame>
<GraphicLine Self="gl1" ItemTransform="1 0 0 1 0 0" StrokeColor="Color/Black" StrokeWeight="4"><Properties><PathGeometry><GeometryPathType PathOpen="true"><PathPointArray><PathPointType Anchor="50 300" LeftDirection="50 300" RightDirection="50 300"/><PathPointType Anchor="250 300" LeftDirection="250 300" RightDirection="250 300"/></PathPointArray></GeometryPathType></PathGeometry></Properties><TransparencySetting><BlendingSetting BlendMode="Multiply" Opacity="50" /><GradientFeatherSetting Applied="true" HiliteAngle="10" HiliteLength="56.7" /></TransparencySetting><StrokeTransparencySetting><BlendingSetting Opacity="40" /></StrokeTransparencySetting><FillTransparencySetting><BlendingSetting Opacity="30" /></FillTransparencySetting></GraphicLine>
<Group Self="g1" ItemTransform="1 0 0 1 0 0"><TransparencySetting><BlendingSetting KnockoutGroup="true" IsolateBlending="true" /></TransparencySetting><Rectangle Self="r2" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red"><Properties><PathGeometry><GeometryPathType PathOpen="false"><PathPointArray><PathPointType Anchor="50 400" LeftDirection="50 400" RightDirection="50 400"/><PathPointType Anchor="50 450" LeftDirection="50 450" RightDirection="50 450"/><PathPointType Anchor="150 450" LeftDirection="150 450" RightDirection="150 450"/><PathPointType Anchor="150 400" LeftDirection="150 400" RightDirection="150 400"/></PathPointArray></GeometryPathType></PathGeometry></Properties><TransparencySetting><BlendingSetting BlendMode="Multiply" /></TransparencySetting></Rectangle></Group>
</Spread></idPkg:Spread>"#;

const PREFERENCES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Preferences xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0"><TextPreference UseOpticalSize="false"/><TransparencyPreference BlendingSpace="CMYK" GlobalLightAngle="45" GlobalLightAltitude="20" /></idPkg:Preferences>"#;

fn source(spread: &str) -> Vec<u8> {
    pkg::package(&[
        ("designmap.xml", &pkg::designmap("")),
        ("Resources/Styles.xml", pkg::STYLES),
        ("Resources/Preferences.xml", PREFERENCES),
        ("Spreads/Spread_s1.xml", spread),
        ("Stories/Story_st1.xml", &pkg::story(pkg::STORY_BODY)),
    ])
}

fn target(t: &Option<TargetTransparency>) -> (Option<f32>, Option<&str>) {
    t.as_ref()
        .map(|t| (t.opacity, t.blend_mode.as_deref()))
        .unwrap_or((None, None))
}

#[test]
fn every_target_is_read_apart() {
    let doc = pkg::open(&source(SPREAD));
    let s = &doc.spreads[0].spread;
    let r = &s.rectangles[0];
    // The object keeps its own setting — not the picture's, read last.
    assert_eq!(
        (r.opacity, r.blend_mode.as_deref()),
        (Some(50.0), Some("Multiply"))
    );
    assert_eq!(
        target(&r.stroke_transparency),
        (Some(30.0), Some("Multiply"))
    );
    assert_eq!(target(&r.fill_transparency), (Some(40.0), Some("Screen")));
    // A graphic frame's content is its picture's own setting; the
    // frame's `<ContentTransparencySetting>` is ignored, as InDesign does.
    assert_eq!(
        target(&r.content_transparency),
        (Some(30.0), Some("Multiply"))
    );
    assert!(r.stroke_drop_shadow.is_some(), "stroke target's shadow");
    let fill_shadow = r.fill_drop_shadow.as_ref().expect("fill target's shadow");
    assert_eq!((fill_shadow.x_offset, fill_shadow.size), (11.0, 5.0));
    // R-11: the object shadow's remaining attributes.
    let ds = r.drop_shadow.as_ref().expect("object shadow");
    assert_eq!(ds.blend_mode.as_deref(), Some("Screen"));
    assert_eq!((ds.spread_pct, ds.noise_pct), (Some(30.0), Some(20.0)));
    assert_eq!(
        (ds.knocked_out, ds.honor_other_effects, ds.use_global_light),
        (Some(false), Some(true), Some(true))
    );
    let fx = r.effects.as_ref().expect("object effects");
    assert_eq!(
        fx.outer_glow.as_ref().and_then(|g| g.technique.as_deref()),
        Some("Precise")
    );

    let t = &s.text_frames[0];
    assert_eq!(
        t.opacity, None,
        "a text frame's targets leave its object alone"
    );
    assert_eq!(target(&t.fill_transparency), (Some(70.0), None));
    assert_eq!(
        target(&t.content_transparency),
        (Some(60.0), Some("Multiply"))
    );
    assert!(t.content_drop_shadow.is_some(), "the text's own shadow");
    let fx = t.effects.as_ref().expect("text frame effects");
    let bevel = fx.bevel.as_ref().expect("bevel");
    assert_eq!(
        (
            bevel.highlight_blend_mode.as_deref(),
            bevel.shadow_blend_mode.as_deref()
        ),
        (Some("Overlay"), Some("Darken"))
    );
    assert_eq!(
        fx.directional_feather
            .as_ref()
            .and_then(|d| d.follow_shape_mode.as_deref()),
        Some("AllEdges")
    );

    let l = &s.graphic_lines[0];
    assert_eq!(
        (l.opacity, l.blend_mode.as_deref()),
        (Some(50.0), Some("Multiply"))
    );
    assert_eq!(target(&l.stroke_transparency), (Some(40.0), None));
    let gf = l
        .effects
        .as_ref()
        .and_then(|e| e.gradient_feather.as_ref())
        .expect("gradient feather");
    assert_eq!(
        (gf.hilite_angle, gf.hilite_length),
        (Some(10.0), Some(56.7))
    );

    let g = &s.groups[0];
    assert!(g.transparency.isolate_blending && g.transparency.knockout_group);
    assert_eq!(s.rectangles[1].blend_mode.as_deref(), Some("Multiply"));

    let light = doc.designmap.transparency_preference;
    assert_eq!(
        (light.global_light_angle, light.global_light_altitude),
        (Some(45.0), Some(20.0))
    );
}

#[test]
fn an_unchanged_document_keeps_indesigns_spelling() {
    let src = source(SPREAD);
    let doc = pkg::open(&src);
    let out = write_idml(&doc, &src).expect("write");
    assert_eq!(
        pkg::entry(&out, "Spreads/Spread_s1.xml"),
        pkg::entry(&src, "Spreads/Spread_s1.xml"),
        "a model that agrees with the part leaves it byte-identical"
    );
    assert_eq!(
        pkg::entry(&out, "Resources/Preferences.xml"),
        pkg::entry(&src, "Resources/Preferences.xml"),
    );
}

#[test]
fn changed_targets_flags_and_light_are_written_back() {
    let src = source(SPREAD);
    let mut doc = pkg::open(&src);
    {
        let s = &mut doc.spreads[0].spread;
        let r = &mut s.rectangles[0];
        r.fill_transparency.as_mut().unwrap().opacity = Some(25.0);
        r.content_transparency.as_mut().unwrap().blend_mode = Some("Screen".into());
        r.drop_shadow.as_mut().unwrap().spread_pct = Some(10.0);
        s.text_frames[0].stroke_transparency = Some(TargetTransparency {
            opacity: Some(80.0),
            ..Default::default()
        });
        s.graphic_lines[0].opacity = Some(75.0);
        s.groups[0].transparency.knockout_group = false;
        doc.designmap.transparency_preference.global_light_angle = Some(60.0);
    }
    let out = write_idml(&doc, &src).expect("write");
    let re = pkg::open(&out);
    let s = &re.spreads[0].spread;
    let r = &s.rectangles[0];
    assert_eq!(target(&r.fill_transparency), (Some(25.0), Some("Screen")));
    assert_eq!(
        target(&r.stroke_transparency),
        (Some(30.0), Some("Multiply"))
    );
    assert_eq!(
        target(&r.content_transparency),
        (Some(30.0), Some("Screen"))
    );
    assert_eq!(
        (r.opacity, r.blend_mode.as_deref()),
        (Some(50.0), Some("Multiply"))
    );
    let ds = r.drop_shadow.as_ref().unwrap();
    assert_eq!((ds.spread_pct, ds.noise_pct), (Some(10.0), Some(20.0)));
    assert_eq!(ds.blend_mode.as_deref(), Some("Screen"));
    assert_eq!(ds.use_global_light, Some(true));
    assert!(r.fill_drop_shadow.is_some() && r.stroke_drop_shadow.is_some());
    assert_eq!(
        target(&s.text_frames[0].stroke_transparency),
        (Some(80.0), None)
    );
    assert_eq!(
        target(&s.text_frames[0].content_transparency),
        (Some(60.0), Some("Multiply"))
    );
    assert_eq!(s.graphic_lines[0].opacity, Some(75.0));
    assert_eq!(
        target(&s.graphic_lines[0].stroke_transparency),
        (Some(40.0), None)
    );
    let g = &s.groups[0];
    assert!(g.transparency.isolate_blending && !g.transparency.knockout_group);
    assert_eq!(
        re.designmap.transparency_preference.global_light_angle,
        Some(60.0)
    );

    // The rectangle's own `IsolateBlending` (which the model keeps for
    // groups only) survives its rewritten block.
    let part = pkg::entry(&out, "Spreads/Spread_s1.xml").unwrap();
    let r1 = &part[part.find(r#"Self="r1""#).unwrap()..part.find(r#"Self="tf1""#).unwrap()];
    assert!(r1.contains(r#"IsolateBlending="true""#), "{r1}");
    // InDesign's order: object, stroke, fill; the picture's inside <Image>.
    let (o, st, fi) = (
        r1.find("<TransparencySetting>").unwrap(),
        r1.find("<StrokeTransparencySetting>").unwrap(),
        r1.find("<FillTransparencySetting>").unwrap(),
    );
    assert!(o < st && st < fi, "{r1}");
    assert!(!r1.contains("<ContentTransparencySetting>"), "{r1}");
    let image = &r1[r1.find("<Image").unwrap()..];
    assert!(image.contains(r#"<TransparencySetting><BlendingSetting Opacity="30" BlendMode="Screen"/></TransparencySetting></Image>"#), "{image}");
}

#[test]
fn inserted_items_and_groups_carry_their_targets() {
    let src = source(SPREAD);
    let mut doc = pkg::open(&src);
    let spread = &mut doc.spreads[0].spread;
    let mut rect = spread.rectangles[1].clone();
    rect.self_id = Some("r9".into());
    rect.blend_mode = None;
    rect.stroke_color = Some("Color/Black".into());
    rect.stroke_weight = Some(4.0);
    rect.fill_transparency = Some(TargetTransparency {
        opacity: Some(35.0),
        blend_mode: Some("Multiply".into()),
        ..Default::default()
    });
    rect.stroke_transparency = Some(TargetTransparency {
        opacity: Some(45.0),
        ..Default::default()
    });
    spread.rectangles.push(rect);
    let ri = spread.rectangles.len() - 1;
    let mut group = spread.groups[0].clone();
    group.self_id = Some("g9".into());
    group.members = vec![FrameRef::Rectangle(ri)];
    group.transparency.knockout_group = false;
    spread.groups.push(group);
    let gi = spread.groups.len() - 1;
    spread.frames_in_order.push(FrameRef::Group(gi));

    let out = write_idml(&doc, &src).expect("write");
    let re = pkg::open(&out);
    let s = &re.spreads[0].spread;
    let r = s
        .rectangles
        .iter()
        .find(|r| r.self_id.as_deref() == Some("r9"))
        .expect("inserted rectangle");
    assert_eq!(target(&r.fill_transparency), (Some(35.0), Some("Multiply")));
    assert_eq!(target(&r.stroke_transparency), (Some(45.0), None));
    let g = s
        .groups
        .iter()
        .find(|g| g.self_id.as_deref() == Some("g9"))
        .expect("inserted group");
    assert!(g.transparency.isolate_blending && !g.transparency.knockout_group);
}
