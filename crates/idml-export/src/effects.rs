//! A page item's transparency and effects — `<TransparencySetting>` with
//! its `<BlendingSetting>` and the effect settings InDesign reads:
//! `DropShadowSetting`, `InnerShadowSetting`, `OuterGlowSetting`,
//! `InnerGlowSetting`, `BevelAndEmbossSetting`, `SatinSetting`,
//! `FeatherSetting`, `DirectionalFeatherSetting`,
//! `GradientFeatherSetting`. The attribute names are the ones the
//! importer reads back (`idml_import::spread`), which are InDesign's.
//! The exporter used to write the blending setting alone, so every
//! effect the annual's "One parameter at a time" page authored reached
//! InDesign — and the export's own twin — as a plain red square
//! (measured 2026-09-06).

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Writer;

use idml_import::{DropShadowSetting, FrameEffects, TargetTransparency};

use crate::rewrite::{emit_empty_with_attrs, format_f32};

/// Everything a page item says about its transparency: the object's own
/// setting and its three targets (R-10). `content` is a text frame's
/// text (`<ContentTransparencySetting>`); a graphic frame's picture is
/// the `<Image>`'s own `<TransparencySetting>` and is written there, not
/// here. A `<Group>` adds how its members composite.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ItemTransparency<'a> {
    pub opacity: Option<f32>,
    pub blend_mode: Option<&'a str>,
    pub drop_shadow: Option<&'a DropShadowSetting>,
    pub effects: Option<&'a FrameEffects>,
    /// `<BlendingSetting IsolateBlending>` — a group's only.
    pub isolate_blending: bool,
    /// `<BlendingSetting KnockoutGroup>` — a group's only.
    pub knockout_group: bool,
    pub stroke: Target<'a>,
    pub fill: Target<'a>,
    pub content: Target<'a>,
}

/// One target: its setting and its drop shadow.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Target<'a> {
    pub setting: Option<&'a TargetTransparency>,
    pub drop_shadow: Option<&'a DropShadowSetting>,
}

impl Target<'_> {
    fn any(&self) -> bool {
        let t = self.setting;
        any(
            t.and_then(|t| t.opacity),
            t.and_then(|t| t.blend_mode.as_deref()),
            self.drop_shadow,
            t.and_then(|t| t.effects.as_ref()),
        )
    }
}

impl ItemTransparency<'_> {
    fn object_any(&self) -> bool {
        any(
            self.opacity,
            self.blend_mode,
            self.drop_shadow,
            self.effects,
        ) || self.isolate_blending
            || self.knockout_group
    }

}

/// Whether there is anything to write at all.
pub(crate) fn any(
    opacity: Option<f32>,
    blend_mode: Option<&str>,
    drop_shadow: Option<&DropShadowSetting>,
    effects: Option<&FrameEffects>,
) -> bool {
    opacity.is_some()
        || blend_mode.is_some()
        || drop_shadow.is_some_and(|d| d.mode != "None")
        || effects.is_some_and(|e| {
            e.inner_shadow.is_some()
                || e.outer_glow.is_some()
                || e.inner_glow.is_some()
                || e.bevel.is_some()
                || e.satin.is_some()
                || e.feather.is_some()
                || e.directional_feather.is_some()
                || e.gradient_feather.is_some()
        })
}

fn push_f32(out: &mut Vec<(&'static str, String)>, key: &'static str, v: Option<f32>) {
    if let Some(v) = v {
        out.push((key, format_f32(v)));
    }
}

fn push_str(out: &mut Vec<(&'static str, String)>, key: &'static str, v: &Option<String>) {
    if let Some(v) = v {
        out.push((key, v.clone()));
    }
}

fn push_bool(out: &mut Vec<(&'static str, String)>, key: &'static str, v: Option<bool>) {
    if let Some(v) = v {
        out.push((key, v.to_string()));
    }
}

/// The object's `<TransparencySetting>` and then each target's, in
/// InDesign's order — stroke, fill, content (asked of InDesign 20.0.1).
/// Each block is written only when it carries something.
pub(crate) fn write_item_transparency(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    t: &ItemTransparency<'_>,
) -> Result<(), quick_xml::Error> {
    if t.object_any() {
        let blending = Blending {
            opacity: t.opacity,
            blend_mode: t.blend_mode,
            isolate_blending: t.isolate_blending,
            knockout_group: t.knockout_group,
        };
        write_setting(
            writer,
            "TransparencySetting",
            &blending,
            t.drop_shadow,
            t.effects,
        )?;
    }
    write_target(writer, "StrokeTransparencySetting", &t.stroke)?;
    write_target(writer, "FillTransparencySetting", &t.fill)?;
    write_target(writer, "ContentTransparencySetting", &t.content)
}

/// One target's block (`<FillTransparencySetting>` …, or a picture's
/// `<TransparencySetting>`); nothing when it carries nothing.
pub(crate) fn write_target(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    element: &str,
    target: &Target<'_>,
) -> Result<(), quick_xml::Error> {
    if !target.any() {
        return Ok(());
    }
    let setting = target.setting;
    let blending = Blending {
        opacity: setting.and_then(|s| s.opacity),
        blend_mode: setting.and_then(|s| s.blend_mode.as_deref()),
        ..Default::default()
    };
    write_setting(
        writer,
        element,
        &blending,
        target.drop_shadow,
        setting.and_then(|s| s.effects.as_ref()),
    )
}

/// A `<BlendingSetting>`'s values: a target's opacity and blend mode,
/// and — on a group — how its members composite.
#[derive(Debug, Clone, Copy, Default)]
struct Blending<'a> {
    opacity: Option<f32>,
    blend_mode: Option<&'a str>,
    isolate_blending: bool,
    knockout_group: bool,
}

/// `<BlendingSetting>` with the values set; nothing when none is.
fn write_blending(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    b: &Blending<'_>,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&'static str, String)> = Vec::new();
    push_f32(&mut attrs, "Opacity", b.opacity);
    if let Some(m) = b.blend_mode {
        attrs.push(("BlendMode", m.to_string()));
    }
    if b.isolate_blending {
        attrs.push(("IsolateBlending", "true".to_string()));
    }
    if b.knockout_group {
        attrs.push(("KnockoutGroup", "true".to_string()));
    }
    if attrs.is_empty() {
        return Ok(());
    }
    emit_empty_with_attrs(writer, "BlendingSetting", &attrs)
}

fn write_setting(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    element: &str,
    blending: &Blending<'_>,
    drop_shadow: Option<&DropShadowSetting>,
    effects: Option<&FrameEffects>,
) -> Result<(), quick_xml::Error> {
    writer.write_event(Event::Start(BytesStart::new(element)))?;
    write_blending(writer, blending)?;
    if let Some(d) = drop_shadow.filter(|d| d.mode != "None") {
        // InDesign carries the offset twice: as XOffset/YOffset and as
        // Distance/Angle (135° for an offset down and to the right).
        let distance = (d.x_offset * d.x_offset + d.y_offset * d.y_offset).sqrt();
        let angle = d.y_offset.atan2(-d.x_offset).to_degrees();
        let mut attrs: Vec<(&'static str, String)> = vec![
            ("Mode", d.mode.clone()),
            ("Opacity", format_f32(d.opacity_pct)),
            ("XOffset", format_f32(d.x_offset)),
            ("YOffset", format_f32(d.y_offset)),
            ("Size", format_f32(d.size)),
            ("Distance", format_f32(distance)),
            ("Angle", format_f32(angle)),
            // R-11: the model's own values; absent ones are InDesign's
            // defaults, written as before.
            (
                "UseGlobalLight",
                d.use_global_light.unwrap_or(false).to_string(),
            ),
            ("KnockedOut", d.knocked_out.unwrap_or(true).to_string()),
        ];
        push_str(&mut attrs, "EffectColor", &d.effect_color);
        push_str(&mut attrs, "BlendMode", &d.blend_mode);
        push_f32(&mut attrs, "Spread", d.spread_pct);
        push_f32(&mut attrs, "Noise", d.noise_pct);
        push_bool(&mut attrs, "HonorOtherEffects", d.honor_other_effects);
        emit_empty_with_attrs(writer, "DropShadowSetting", &attrs)?;
    }
    let Some(e) = effects else {
        writer.write_event(Event::End(BytesEnd::new(element)))?;
        return Ok(());
    };
    if let Some(s) = &e.inner_shadow {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
        push_f32(&mut attrs, "Opacity", s.opacity_pct);
        push_f32(&mut attrs, "XOffset", s.x_offset);
        push_f32(&mut attrs, "YOffset", s.y_offset);
        push_f32(&mut attrs, "Size", s.size);
        push_f32(&mut attrs, "Distance", s.distance);
        push_f32(&mut attrs, "Angle", s.angle_deg);
        push_f32(&mut attrs, "ChokeAmount", s.choke_pct);
        push_f32(&mut attrs, "Noise", s.noise_pct);
        push_str(&mut attrs, "BlendMode", &s.blend_mode);
        push_str(&mut attrs, "EffectColor", &s.effect_color);
        push_bool(&mut attrs, "UseGlobalLight", s.use_global_light);
        emit_empty_with_attrs(writer, "InnerShadowSetting", &attrs)?;
    }
    if let Some(g) = &e.outer_glow {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
        push_f32(&mut attrs, "Opacity", g.opacity_pct);
        push_f32(&mut attrs, "Size", g.size);
        push_f32(&mut attrs, "Spread", g.spread_pct);
        push_f32(&mut attrs, "Noise", g.noise_pct);
        push_str(&mut attrs, "BlendMode", &g.blend_mode);
        push_str(&mut attrs, "EffectColor", &g.effect_color);
        push_str(&mut attrs, "Technique", &g.technique);
        emit_empty_with_attrs(writer, "OuterGlowSetting", &attrs)?;
    }
    if let Some(g) = &e.inner_glow {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
        push_f32(&mut attrs, "Opacity", g.opacity_pct);
        push_f32(&mut attrs, "Size", g.size);
        // InDesign's inner glow has no ChokeAmount: its choke is `Spread`.
        push_f32(&mut attrs, "Spread", g.choke_pct);
        push_f32(&mut attrs, "Noise", g.noise_pct);
        push_str(&mut attrs, "BlendMode", &g.blend_mode);
        push_str(
            &mut attrs,
            "Source",
            &g.source.as_deref().map(inner_glow_source),
        );
        push_str(&mut attrs, "EffectColor", &g.effect_color);
        push_str(&mut attrs, "Technique", &g.technique);
        emit_empty_with_attrs(writer, "InnerGlowSetting", &attrs)?;
    }
    if let Some(b) = &e.bevel {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
        push_f32(&mut attrs, "Depth", b.depth_pct);
        push_f32(&mut attrs, "Size", b.size);
        push_f32(&mut attrs, "Angle", b.angle_deg);
        push_f32(&mut attrs, "Altitude", b.altitude_deg);
        push_f32(&mut attrs, "Soften", b.soften);
        push_f32(&mut attrs, "HighlightOpacity", b.highlight_opacity_pct);
        push_f32(&mut attrs, "ShadowOpacity", b.shadow_opacity_pct);
        push_str(&mut attrs, "HighlightColor", &b.highlight_color);
        push_str(&mut attrs, "ShadowColor", &b.shadow_color);
        push_str(&mut attrs, "Style", &b.style);
        push_str(&mut attrs, "Direction", &b.direction);
        push_str(
            &mut attrs,
            "Technique",
            &b.technique.as_deref().map(bevel_technique),
        );
        push_str(&mut attrs, "HighlightBlendMode", &b.highlight_blend_mode);
        push_str(&mut attrs, "ShadowBlendMode", &b.shadow_blend_mode);
        push_bool(&mut attrs, "UseGlobalLight", b.use_global_light);
        emit_empty_with_attrs(writer, "BevelAndEmbossSetting", &attrs)?;
    }
    if let Some(s) = &e.satin {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
        push_f32(&mut attrs, "Opacity", s.opacity_pct);
        push_f32(&mut attrs, "Size", s.size);
        push_f32(&mut attrs, "Angle", s.angle_deg);
        push_f32(&mut attrs, "Distance", s.distance);
        push_str(&mut attrs, "BlendMode", &s.blend_mode);
        push_str(&mut attrs, "EffectColor", &s.effect_color);
        if let Some(i) = s.invert {
            attrs.push(("InvertEffect", i.to_string()));
        }
        emit_empty_with_attrs(writer, "SatinSetting", &attrs)?;
    }
    if let Some(f) = &e.feather {
        // A basic feather has no `Applied`: InDesign switches it on with
        // `Mode="Standard"` (default "None").
        let mut attrs: Vec<(&'static str, String)> = vec![("Mode", "Standard".to_string())];
        push_f32(&mut attrs, "Width", f.width);
        push_f32(&mut attrs, "ChokeAmount", f.choke_pct);
        push_f32(&mut attrs, "Noise", f.noise_pct);
        push_str(&mut attrs, "CornerType", &f.corner_type);
        emit_empty_with_attrs(writer, "FeatherSetting", &attrs)?;
    }
    if let Some(f) = &e.directional_feather {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
        push_f32(&mut attrs, "LeftWidth", f.left_width);
        push_f32(&mut attrs, "RightWidth", f.right_width);
        push_f32(&mut attrs, "TopWidth", f.top_width);
        push_f32(&mut attrs, "BottomWidth", f.bottom_width);
        push_f32(&mut attrs, "Angle", f.angle_deg);
        push_f32(&mut attrs, "ChokeAmount", f.choke_pct);
        push_f32(&mut attrs, "Noise", f.noise_pct);
        push_str(&mut attrs, "FollowShapeMode", &f.follow_shape_mode);
        emit_empty_with_attrs(writer, "DirectionalFeatherSetting", &attrs)?;
    }
    if let Some(g) = &e.gradient_feather {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
        push_str(&mut attrs, "Type", &g.gradient_type);
        if let Some((x, y)) = g.start_point {
            attrs.push((
                "GradientStart",
                format!("{} {}", format_f32(x), format_f32(y)),
            ));
        }
        if let (Some((sx, sy)), Some((ex, ey))) = (g.start_point, g.end_point) {
            let length = ((ex - sx).powi(2) + (ey - sy).powi(2)).sqrt();
            attrs.push(("Length", format_f32(length)));
        }
        push_f32(&mut attrs, "Angle", g.angle_deg);
        push_f32(&mut attrs, "HiliteAngle", g.hilite_angle);
        push_f32(&mut attrs, "HiliteLength", g.hilite_length);
        emit_empty_with_attrs(writer, "GradientFeatherSetting", &attrs)?;
    }
    writer.write_event(Event::End(BytesEnd::new(element)))?;
    Ok(())
}

/// InDesign's `InnerGlowSetting Source` enumerators (measured 2026-09-06
/// on its own export): `EdgeSourced` / `CenterSourced`. The model still
/// carries the older `EdgeGlow` / `CenterGlow` from files we wrote
/// ourselves; either spelling goes out as InDesign's.
fn inner_glow_source(s: &str) -> String {
    match s {
        "EdgeGlow" => "EdgeSourced".to_string(),
        "CenterGlow" => "CenterSourced".to_string(),
        other => other.to_string(),
    }
}

/// InDesign's `BevelAndEmbossSetting Technique` enumerators:
/// `SmoothContour` / `ChiselHard` / `ChiselSoft`. A bare `Smooth` (the
/// engine's word) is `SmoothContour` to InDesign — an unknown enumerator
/// left the whole effect unreadable and the object painted white.
fn bevel_technique(s: &str) -> String {
    match s {
        "Smooth" => "SmoothContour".to_string(),
        other => other.to_string(),
    }
}
