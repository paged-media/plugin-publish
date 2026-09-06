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

use idml_import::{DropShadowSetting, FrameEffects};

use crate::rewrite::{emit_empty_with_attrs, format_f32};

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

/// `<TransparencySetting>` with every setting the item carries; nothing
/// when it carries none.
pub(crate) fn write_transparency(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    opacity: Option<f32>,
    blend_mode: Option<&str>,
    drop_shadow: Option<&DropShadowSetting>,
    effects: Option<&FrameEffects>,
) -> Result<(), quick_xml::Error> {
    if !any(opacity, blend_mode, drop_shadow, effects) {
        return Ok(());
    }
    writer.write_event(Event::Start(BytesStart::new("TransparencySetting")))?;
    if opacity.is_some() || blend_mode.is_some() {
        let mut attrs: Vec<(&str, String)> = Vec::new();
        if let Some(o) = opacity {
            attrs.push(("Opacity", format_f32(o)));
        }
        if let Some(m) = blend_mode {
            attrs.push(("BlendMode", m.to_string()));
        }
        emit_empty_with_attrs(writer, "BlendingSetting", &attrs)?;
    }
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
            ("UseGlobalLight", "false".to_string()),
            ("KnockedOut", "true".to_string()),
        ];
        push_str(&mut attrs, "EffectColor", &d.effect_color);
        emit_empty_with_attrs(writer, "DropShadowSetting", &attrs)?;
    }
    let Some(e) = effects else {
        writer.write_event(Event::End(BytesEnd::new("TransparencySetting")))?;
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
        emit_empty_with_attrs(writer, "OuterGlowSetting", &attrs)?;
    }
    if let Some(g) = &e.inner_glow {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
        push_f32(&mut attrs, "Opacity", g.opacity_pct);
        push_f32(&mut attrs, "Size", g.size);
        push_f32(&mut attrs, "ChokeAmount", g.choke_pct);
        push_f32(&mut attrs, "Noise", g.noise_pct);
        push_str(&mut attrs, "BlendMode", &g.blend_mode);
        push_str(&mut attrs, "Source", &g.source);
        push_str(&mut attrs, "EffectColor", &g.effect_color);
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
        push_str(&mut attrs, "Technique", &b.technique);
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
            attrs.push(("Invert", i.to_string()));
        }
        emit_empty_with_attrs(writer, "SatinSetting", &attrs)?;
    }
    if let Some(f) = &e.feather {
        let mut attrs: Vec<(&'static str, String)> = vec![("Applied", "true".to_string())];
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
        push_f32(&mut attrs, "NoiseAmount", f.noise_pct);
        push_str(&mut attrs, "CornerType", &f.corner_type);
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
        if let Some((x, y)) = g.end_point {
            attrs.push((
                "GradientEnd",
                format!("{} {}", format_f32(x), format_f32(y)),
            ));
        }
        push_f32(&mut attrs, "GradientAngle", g.angle_deg);
        emit_empty_with_attrs(writer, "GradientFeatherSetting", &attrs)?;
    }
    writer.write_event(Event::End(BytesEnd::new("TransparencySetting")))?;
    Ok(())
}
