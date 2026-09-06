//! A page item's `<TransparencySetting>` — the blending setting and every
//! effect — kept in step with the model on an item the source part
//! already has. The emitter writes the block for a minted item (see
//! [`crate::effects`]); an existing item's block used to pass through
//! verbatim, so an effect applied after the item was first saved never
//! reached the part: the annual's effects page, minted at chapter time
//! with its blending alone and dressed later, exported plain squares
//! (measured 2026-09-06).
//!
//! The rule is semantic, not textual: the source part is parsed the way
//! the importer parses it, and only an item whose transparency the model
//! no longer matches is rewritten. InDesign's own richer spelling of the
//! same setting (`IsolateBlending`, `KnockoutGroup`, …) therefore keeps
//! its bytes on an unmutated round trip. The model's block replaces the
//! source's (or is dropped when the model has none); an item without one
//! gains it after `<Properties>`, where InDesign writes it.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{DropShadowSetting, FrameEffects, Spread};

use crate::effects;
use crate::rewrite::attr_value;

/// What an item says about its transparency.
#[derive(Debug, Clone)]
struct Owner {
    opacity: Option<f32>,
    blend_mode: Option<String>,
    drop_shadow: Option<DropShadowSetting>,
    effects: Option<FrameEffects>,
}

impl Owner {
    fn any(&self) -> bool {
        effects::any(
            self.opacity,
            self.blend_mode.as_deref(),
            self.drop_shadow.as_ref(),
            self.effects.as_ref(),
        )
    }

    /// The block as the emitter spells it.
    fn block(&self) -> Result<Vec<u8>, quick_xml::Error> {
        let mut w = Writer::new(Cursor::new(Vec::new()));
        effects::write_transparency(
            &mut w,
            self.opacity,
            self.blend_mode.as_deref(),
            self.drop_shadow.as_ref(),
            self.effects.as_ref(),
        )?;
        Ok(w.into_inner().into_inner())
    }
}

fn owners_of(spread: &Spread) -> HashMap<String, Owner> {
    let mut out = HashMap::new();
    let mut add = |id: Option<&str>, o: Owner| {
        if let Some(id) = id {
            out.insert(id.to_string(), o);
        }
    };
    for t in &spread.text_frames {
        add(
            t.self_id.as_deref(),
            Owner {
                opacity: t.opacity,
                blend_mode: t.blend_mode.clone(),
                drop_shadow: t.drop_shadow.clone(),
                effects: t.effects.clone(),
            },
        );
    }
    for r in &spread.rectangles {
        add(
            r.self_id.as_deref(),
            Owner {
                opacity: r.opacity,
                blend_mode: r.blend_mode.clone(),
                drop_shadow: r.drop_shadow.clone(),
                effects: r.effects.clone(),
            },
        );
    }
    for o in &spread.ovals {
        add(
            o.self_id.as_deref(),
            Owner {
                opacity: o.opacity,
                blend_mode: o.blend_mode.clone(),
                drop_shadow: o.drop_shadow.clone(),
                effects: o.effects.clone(),
            },
        );
    }
    for p in &spread.polygons {
        add(
            p.self_id.as_deref(),
            Owner {
                opacity: p.opacity,
                blend_mode: p.blend_mode.clone(),
                drop_shadow: None,
                effects: p.effects.clone(),
            },
        );
    }
    for l in &spread.graphic_lines {
        add(
            l.self_id.as_deref(),
            Owner {
                opacity: None,
                blend_mode: None,
                drop_shadow: None,
                effects: l.effects.clone(),
            },
        );
    }
    out
}

fn is_host_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"TextFrame" | b"Rectangle" | b"Oval" | b"Polygon" | b"GraphicLine"
    )
}

/// The items whose transparency the model no longer matches, keyed by
/// `Self`.
fn changed_owners(original: &[u8], spread: &Spread) -> HashMap<String, Owner> {
    let model = owners_of(spread);
    let Ok(source) = idml_import::parse_spread(original) else {
        return HashMap::new();
    };
    let source = owners_of(&source);
    model
        .into_iter()
        .filter(|(id, o)| match source.get(id) {
            Some(s) => format!("{s:?}") != format!("{o:?}"),
            // An item the source lacks is minted, and the emitter's.
            None => false,
        })
        .collect()
}

pub fn rewrite_transparency(original: &[u8], spread: &Spread) -> Result<Vec<u8>, quick_xml::Error> {
    let changed = changed_owners(original, spread);
    if changed.is_empty() {
        return Ok(original.to_vec());
    }

    struct Open {
        owner: Owner,
        depth: usize,
        settled: bool,
    }

    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(original.len() + 512)));
    let mut buf = Vec::new();
    let mut stack: Vec<Open> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.name().as_ref().to_vec();
                if let Some(o) = stack.last_mut() {
                    if o.depth == 0 && name == b"TransparencySetting" {
                        // The source block, against the model's.
                        skip_subtree(&mut reader, &name)?;
                        // A block already placed after `<Properties>`
                        // is not placed again.
                        if !o.settled && o.owner.any() {
                            raw(&mut writer, &o.owner.block()?)?;
                        }
                        o.settled = true;
                        buf.clear();
                        continue;
                    }
                    if o.depth == 0 && !o.settled && name != b"Properties" {
                        // The first child after `<Properties>`: the block
                        // goes before it.
                        if o.owner.any() {
                            raw(&mut writer, &o.owner.block()?)?;
                        }
                        o.settled = true;
                    }
                    o.depth += 1;
                }
                if is_host_name(&name) {
                    if let Some(owner) = attr_value(&e, b"Self").and_then(|id| changed.get(&id)) {
                        stack.push(Open {
                            owner: owner.clone(),
                            depth: 0,
                            settled: false,
                        });
                    }
                }
                writer.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                if let Some(o) = stack.last_mut() {
                    if o.depth == 0 && name == b"TransparencySetting" {
                        if !o.settled && o.owner.any() {
                            raw(&mut writer, &o.owner.block()?)?;
                        }
                        o.settled = true;
                        buf.clear();
                        continue;
                    }
                    if o.depth == 0 && !o.settled && name != b"Properties" {
                        if o.owner.any() {
                            raw(&mut writer, &o.owner.block()?)?;
                        }
                        o.settled = true;
                    }
                    if o.depth == 0 && name == b"Properties" {
                        // `<Properties/>`: the block follows it.
                        writer.write_event(Event::Empty(e.into_owned()))?;
                        if !o.settled && o.owner.any() {
                            raw(&mut writer, &o.owner.block()?)?;
                        }
                        o.settled = true;
                        buf.clear();
                        continue;
                    }
                }
                // A self-closing host item has no children to hold a block;
                // it is left as it is (an effect on it is a loss the ledger
                // will name).
                writer.write_event(Event::Empty(e.into_owned()))?;
            }
            Event::End(e) => {
                let name = e.name().as_ref().to_vec();
                if let Some(o) = stack.last_mut() {
                    if o.depth == 0 {
                        // The host closes with no child to anchor the block.
                        if !o.settled && o.owner.any() {
                            raw(&mut writer, &o.owner.block()?)?;
                        }
                        writer.write_event(Event::End(e.into_owned()))?;
                        stack.pop();
                        buf.clear();
                        continue;
                    }
                    o.depth -= 1;
                    if o.depth == 0 && name == b"Properties" && !o.settled {
                        // Right after `</Properties>`, where InDesign
                        // writes it.
                        writer.write_event(Event::End(e.into_owned()))?;
                        if o.owner.any() {
                            raw(&mut writer, &o.owner.block()?)?;
                        }
                        o.settled = true;
                        buf.clear();
                        continue;
                    }
                }
                writer.write_event(Event::End(e.into_owned()))?;
            }
            other => writer.write_event(other.into_owned())?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

/// Bytes already spelled (the emitter's block), through the writer's
/// cursor so they land where the stream is, not at the buffer's end.
fn raw(writer: &mut Writer<Cursor<Vec<u8>>>, bytes: &[u8]) -> Result<(), quick_xml::Error> {
    std::io::Write::write_all(writer.get_mut(), bytes)
        .map_err(|e| quick_xml::Error::Io(std::sync::Arc::new(e)))
}

/// Consume every event up to and including the end tag matching `name`,
/// the reader having just returned that element's start tag.
fn skip_subtree(reader: &mut Reader<&[u8]>, name: &[u8]) -> Result<(), quick_xml::Error> {
    let mut depth = 0usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(_) => depth += 1,
            Event::End(e) => {
                if depth == 0 {
                    debug_assert_eq!(e.name().as_ref(), name);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

// `BytesStart` is named in the signature of `attr_value`'s callers only.
#[allow(dead_code)]
fn _uses(_: &BytesStart<'_>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use idml_import::OuterGlowParams;

    const SPREAD: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1" PageCount="1">
<Page Self="p1" Name="1" GeometricBounds="0 0 792 612" ItemTransform="1 0 0 1 0 0"/>
<Rectangle Self="r1" GeometricBounds="450 100 600 300" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red"><Properties><Label><KeyValuePair Key="k" Value="v"/></Label></Properties><TransparencySetting><BlendingSetting Opacity="40" BlendMode="Normal" IsolateBlending="false" KnockoutGroup="false"/></TransparencySetting></Rectangle>
<Rectangle Self="r2" GeometricBounds="100 100 200 200" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red"><Properties><Label><KeyValuePair Key="k" Value="v"/></Label></Properties></Rectangle>
<Oval Self="o1" GeometricBounds="100 300 200 400" ItemTransform="1 0 0 1 0 0"/>
</Spread></idPkg:Spread>"#;

    #[test]
    fn an_unmutated_spread_keeps_indesigns_richer_spelling() {
        let spread = idml_import::parse_spread(SPREAD).unwrap();
        assert_eq!(spread.rectangles[0].opacity, Some(40.0));
        assert_eq!(
            rewrite_transparency(SPREAD, &spread).unwrap(),
            SPREAD.to_vec()
        );
    }

    #[test]
    fn an_effect_applied_later_replaces_or_adds_the_block() {
        let mut spread = idml_import::parse_spread(SPREAD).unwrap();
        spread.rectangles[0].effects = Some(FrameEffects {
            outer_glow: Some(OuterGlowParams {
                size: Some(9.0),
                opacity_pct: Some(60.0),
                effect_color: Some("Color/Red".into()),
                spread_pct: None,
                blend_mode: None,
                noise_pct: None,
            }),
            ..Default::default()
        });
        spread.rectangles[1].opacity = Some(25.0);
        let out = String::from_utf8(rewrite_transparency(SPREAD, &spread).unwrap()).unwrap();
        assert!(
            out.contains(r#"<Rectangle Self="r1" GeometricBounds="450 100 600 300" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red"><Properties><Label><KeyValuePair Key="k" Value="v"/></Label></Properties><TransparencySetting><BlendingSetting Opacity="40" BlendMode="Normal"/><OuterGlowSetting Applied="true" Opacity="60" Size="9" EffectColor="Color/Red"/></TransparencySetting></Rectangle>"#),
            "{out}"
        );
        assert!(
            out.contains(r#"<Rectangle Self="r2" GeometricBounds="100 100 200 200" ItemTransform="1 0 0 1 0 0" FillColor="Color/Red"><Properties><Label><KeyValuePair Key="k" Value="v"/></Label></Properties><TransparencySetting><BlendingSetting Opacity="25"/></TransparencySetting></Rectangle>"#),
            "{out}"
        );
        assert!(
            out.contains(
                r#"<Oval Self="o1" GeometricBounds="100 300 200 400" ItemTransform="1 0 0 1 0 0"/>"#
            ),
            "{out}"
        );
        // A cleared setting drops the block.
        spread.rectangles[0].opacity = None;
        spread.rectangles[0].blend_mode = None;
        spread.rectangles[0].effects = None;
        let out = String::from_utf8(rewrite_transparency(SPREAD, &spread).unwrap()).unwrap();
        assert!(
            out.contains(r#"<Properties><Label><KeyValuePair Key="k" Value="v"/></Label></Properties></Rectangle>"#),
            "{out}"
        );
    }
}
