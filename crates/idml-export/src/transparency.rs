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
//! no longer matches is rewritten. InDesign's own spelling therefore
//! keeps its bytes on an unmutated round trip. The model's blocks — the
//! object's `<TransparencySetting>` and, since R-10, the stroke, fill and
//! (text frame) content targets — replace the source's (or are dropped
//! when the model has none); an item without them gains them after
//! `<Properties>`, where InDesign writes them. A graphic frame's picture
//! keeps its own block inside `<Image>`, and a `<Group>`'s block carries
//! `IsolateBlending` / `KnockoutGroup`. A frame's own `IsolateBlending` /
//! `KnockoutGroup` (the model keeps them for groups only) are carried
//! over from the source block rather than dropped.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{DropShadowSetting, FrameEffects, GroupTransparency, Spread, TargetTransparency};

use crate::effects::{self, ItemTransparency, Target};
use crate::rewrite::attr_value;

/// What kind of host an item is: which targets it writes where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostKind {
    /// A text frame: its content target is `<ContentTransparencySetting>`.
    Text,
    /// A graphic frame: its content target is the picture's own
    /// `<TransparencySetting>`, inside `<Image>`.
    Graphic,
    /// A line: object and stroke only.
    Line,
    /// A group: its own setting, with isolate / knockout.
    Group,
}

/// What an item says about its transparency (R-10: the object and each
/// of its targets).
#[derive(Debug, Clone)]
struct Owner {
    kind: HostKind,
    opacity: Option<f32>,
    blend_mode: Option<String>,
    drop_shadow: Option<DropShadowSetting>,
    effects: Option<FrameEffects>,
    group: Option<GroupTransparency>,
    stroke: (Option<TargetTransparency>, Option<DropShadowSetting>),
    fill: (Option<TargetTransparency>, Option<DropShadowSetting>),
    content: (Option<TargetTransparency>, Option<DropShadowSetting>),
}

impl Owner {
    fn frame(
        kind: HostKind,
        opacity: Option<f32>,
        blend_mode: &Option<String>,
        drop_shadow: &Option<DropShadowSetting>,
        effects: &Option<FrameEffects>,
    ) -> Self {
        Owner {
            kind,
            opacity,
            blend_mode: blend_mode.clone(),
            drop_shadow: drop_shadow.clone(),
            effects: effects.clone(),
            group: None,
            stroke: (None, None),
            fill: (None, None),
            content: (None, None),
        }
    }

    fn target(t: &(Option<TargetTransparency>, Option<DropShadowSetting>)) -> Target<'_> {
        Target {
            setting: t.0.as_ref(),
            drop_shadow: t.1.as_ref(),
        }
    }

    /// The item's own blocks — object, stroke, fill and a text frame's
    /// content. `flags` are InDesign's frame-level `IsolateBlending` /
    /// `KnockoutGroup`, which the model keeps for groups only and which
    /// are carried over from the source block.
    fn view(&self, flags: (bool, bool)) -> ItemTransparency<'_> {
        if let Some(g) = &self.group {
            return crate::rewrite::group_item_transparency(g);
        }
        ItemTransparency {
            opacity: self.opacity,
            blend_mode: self.blend_mode.as_deref(),
            drop_shadow: self.drop_shadow.as_ref(),
            effects: self.effects.as_ref(),
            isolate_blending: flags.0,
            knockout_group: flags.1,
            stroke: Self::target(&self.stroke),
            fill: Self::target(&self.fill),
            content: if self.kind == HostKind::Text {
                Self::target(&self.content)
            } else {
                Target::default()
            },
        }
    }

    fn block(&self, flags: (bool, bool)) -> Result<Vec<u8>, quick_xml::Error> {
        let mut w = Writer::new(Cursor::new(Vec::new()));
        effects::write_item_transparency(&mut w, &self.view(flags))?;
        Ok(w.into_inner().into_inner())
    }

    /// A graphic frame's picture block (the `<Image>`'s own
    /// `<TransparencySetting>`); empty for every other kind.
    fn picture_block(&self) -> Result<Vec<u8>, quick_xml::Error> {
        let mut w = Writer::new(Cursor::new(Vec::new()));
        if self.kind == HostKind::Graphic {
            effects::write_target(&mut w, "TransparencySetting", &Self::target(&self.content))?;
        }
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
        let mut o = Owner::frame(HostKind::Text, t.opacity, &t.blend_mode, &t.drop_shadow, &t.effects);
        o.stroke = (t.stroke_transparency.clone(), t.stroke_drop_shadow.clone());
        o.fill = (t.fill_transparency.clone(), t.fill_drop_shadow.clone());
        o.content = (t.content_transparency.clone(), t.content_drop_shadow.clone());
        add(t.self_id.as_deref(), o);
    }
    for r in &spread.rectangles {
        let mut o =
            Owner::frame(HostKind::Graphic, r.opacity, &r.blend_mode, &r.drop_shadow, &r.effects);
        o.stroke = (r.stroke_transparency.clone(), r.stroke_drop_shadow.clone());
        o.fill = (r.fill_transparency.clone(), r.fill_drop_shadow.clone());
        o.content = (r.content_transparency.clone(), r.content_drop_shadow.clone());
        add(r.self_id.as_deref(), o);
    }
    for v in &spread.ovals {
        let mut o =
            Owner::frame(HostKind::Graphic, v.opacity, &v.blend_mode, &v.drop_shadow, &v.effects);
        o.stroke = (v.stroke_transparency.clone(), v.stroke_drop_shadow.clone());
        o.fill = (v.fill_transparency.clone(), v.fill_drop_shadow.clone());
        o.content = (v.content_transparency.clone(), v.content_drop_shadow.clone());
        add(v.self_id.as_deref(), o);
    }
    for p in &spread.polygons {
        let mut o =
            Owner::frame(HostKind::Graphic, p.opacity, &p.blend_mode, &p.drop_shadow, &p.effects);
        o.stroke = (p.stroke_transparency.clone(), None);
        o.fill = (p.fill_transparency.clone(), p.fill_drop_shadow.clone());
        o.content = (p.content_transparency.clone(), p.content_drop_shadow.clone());
        add(p.self_id.as_deref(), o);
    }
    for l in &spread.graphic_lines {
        let mut o = Owner::frame(HostKind::Line, l.opacity, &l.blend_mode, &l.drop_shadow, &l.effects);
        o.stroke = (l.stroke_transparency.clone(), None);
        add(l.self_id.as_deref(), o);
    }
    for g in &spread.groups {
        let mut o = Owner::frame(HostKind::Group, None, &None, &None, &None);
        o.group = Some(g.transparency.clone());
        add(g.self_id.as_deref(), o);
    }
    out
}

fn is_host_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"TextFrame" | b"Rectangle" | b"Oval" | b"Polygon" | b"GraphicLine" | b"Group"
    )
}

/// The elements an item's own transparency is spelled in.
fn is_wrapper(name: &[u8]) -> bool {
    matches!(
        name,
        b"TransparencySetting"
            | b"StrokeTransparencySetting"
            | b"FillTransparencySetting"
            | b"ContentTransparencySetting"
    )
}

fn is_picture(name: &[u8]) -> bool {
    matches!(name, b"Image" | b"EPS" | b"PDF" | b"ImportedPage")
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

/// InDesign's frame-level `IsolateBlending` / `KnockoutGroup` — set on a
/// frame's own `<TransparencySetting><BlendingSetting>` (InDesign 20.0.1
/// writes them when asked, isolating the frame's fill, stroke and content
/// from what lies under it). The model keeps the pair for groups only, so
/// a frame's rewritten block carries the source's values over instead of
/// dropping them. Keyed by the frame's `Self`; only frames with a flag
/// set appear.
fn source_frame_flags(original: &[u8]) -> HashMap<String, (bool, bool)> {
    let mut out = HashMap::new();
    let mut reader = Reader::from_reader(original);
    let mut buf = Vec::new();
    // Element names from the root, with the host id where one opened.
    let mut path: Vec<(Vec<u8>, Option<String>)> = Vec::new();
    loop {
        let (e, empty) = match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => (e.into_owned(), false),
            Ok(Event::Empty(e)) => (e.into_owned(), true),
            Ok(Event::End(_)) => {
                path.pop();
                buf.clear();
                continue;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        let name = e.name().as_ref().to_vec();
        if name == b"BlendingSetting" {
            let n = path.len();
            if n >= 2 && path[n - 1].0 == b"TransparencySetting" {
                let (host, id) = &path[n - 2];
                if is_host_name(host) && host.as_slice() != b"Group" {
                    let flag = |k: &[u8]| attr_value(&e, k).is_some_and(|v| v == "true");
                    let flags = (flag(b"IsolateBlending"), flag(b"KnockoutGroup"));
                    if let (Some(id), true) = (id, flags.0 || flags.1) {
                        out.insert(id.clone(), flags);
                    }
                }
            }
        }
        if !empty {
            let id = is_host_name(&name)
                .then(|| attr_value(&e, b"Self"))
                .flatten();
            path.push((name, id));
        }
        buf.clear();
    }
    out
}

pub fn rewrite_transparency(original: &[u8], spread: &Spread) -> Result<Vec<u8>, quick_xml::Error> {
    let changed = changed_owners(original, spread);
    if changed.is_empty() {
        return Ok(original.to_vec());
    }
    let flags = source_frame_flags(original);

    struct Open {
        owner: Owner,
        flags: (bool, bool),
        depth: usize,
        settled: bool,
        /// Inside the host's `<Image>` (its direct child): whether the
        /// picture's block has been written.
        picture: Option<bool>,
    }

    impl Open {
        fn block(&self) -> Result<Vec<u8>, quick_xml::Error> {
            self.owner.block(self.flags)
        }
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
                    if o.depth == 0 && is_wrapper(&name) {
                        // The source's blocks, against the model's.
                        skip_subtree(&mut reader, &name)?;
                        // Blocks already placed after `<Properties>` are
                        // not placed again.
                        if !o.settled {
                            raw(&mut writer, &o.block()?)?;
                        }
                        o.settled = true;
                        buf.clear();
                        continue;
                    }
                    if o.depth == 1 && o.picture.is_some() && name == b"TransparencySetting" {
                        // The picture's own block, against the model's.
                        skip_subtree(&mut reader, &name)?;
                        if o.picture == Some(false) {
                            raw(&mut writer, &o.owner.picture_block()?)?;
                        }
                        o.picture = Some(true);
                        buf.clear();
                        continue;
                    }
                    if o.depth == 0 && !o.settled && name != b"Properties" {
                        // The first child after `<Properties>`: the blocks
                        // go before it.
                        raw(&mut writer, &o.block()?)?;
                        o.settled = true;
                    }
                    if o.depth == 0 && is_picture(&name) {
                        o.picture = Some(false);
                    }
                    o.depth += 1;
                }
                if is_host_name(&name) {
                    if let Some(owner) = attr_value(&e, b"Self").and_then(|id| changed.get(&id)) {
                        let id = attr_value(&e, b"Self").unwrap_or_default();
                        stack.push(Open {
                            owner: owner.clone(),
                            flags: flags.get(&id).copied().unwrap_or((false, false)),
                            depth: 0,
                            settled: false,
                            picture: None,
                        });
                    }
                }
                writer.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                if let Some(o) = stack.last_mut() {
                    if o.depth == 0 && is_wrapper(&name) {
                        if !o.settled {
                            raw(&mut writer, &o.block()?)?;
                        }
                        o.settled = true;
                        buf.clear();
                        continue;
                    }
                    if o.depth == 1 && o.picture.is_some() && name == b"TransparencySetting" {
                        if o.picture == Some(false) {
                            raw(&mut writer, &o.owner.picture_block()?)?;
                        }
                        o.picture = Some(true);
                        buf.clear();
                        continue;
                    }
                    if o.depth == 0 && !o.settled && name != b"Properties" {
                        raw(&mut writer, &o.block()?)?;
                        o.settled = true;
                    }
                    if o.depth == 0 && name == b"Properties" {
                        // `<Properties/>`: the blocks follow it.
                        writer.write_event(Event::Empty(e.into_owned()))?;
                        if !o.settled {
                            raw(&mut writer, &o.block()?)?;
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
                        // The host closes with no child to anchor the blocks.
                        if !o.settled {
                            raw(&mut writer, &o.block()?)?;
                        }
                        writer.write_event(Event::End(e.into_owned()))?;
                        stack.pop();
                        // The enclosing changed host counted this element
                        // at its start; it ends here.
                        if let Some(parent) = stack.last_mut() {
                            parent.depth = parent.depth.saturating_sub(1);
                        }
                        buf.clear();
                        continue;
                    }
                    if o.depth == 1 && o.picture.is_some() && is_picture(&name) {
                        // The picture closes: its block, if the source had
                        // none, goes last, where InDesign writes it.
                        if o.picture == Some(false) {
                            raw(&mut writer, &o.owner.picture_block()?)?;
                        }
                        o.picture = None;
                    }
                    o.depth -= 1;
                    if o.depth == 0 && name == b"Properties" && !o.settled {
                        // Right after `</Properties>`, where InDesign
                        // writes it.
                        writer.write_event(Event::End(e.into_owned()))?;
                        raw(&mut writer, &o.block()?)?;
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
                technique: None,
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
