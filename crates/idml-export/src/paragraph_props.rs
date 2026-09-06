//! Tab stops, bullet character, numbering format, numbering list and
//! numbering expression — the paragraph overrides IDML spells as
//! `<Properties>` children rather than attributes, on a
//! `<ParagraphStyleRange>` and on a `<ParagraphStyle>` alike. InDesign
//! 20.0.1 writes, and reads, exactly this (measured on the corpus's
//! InDesign-authored packs and in the app, 2026-09-06 — `NumberingFormat`
//! and `AppliedNumberingList` as ATTRIBUTES are ignored outright):
//!
//! ```xml
//! <ParagraphStyleRange …>
//!   <Properties>
//!     <TabList type="list">
//!       <ListItem type="record">
//!         <Alignment type="enumeration">LeftAlign</Alignment>
//!         <AlignmentCharacter type="string">.</AlignmentCharacter>
//!         <Leader type="string"></Leader>
//!         <Position type="unit">34</Position>
//!       </ListItem>
//!     </TabList>
//!     <BulletChar BulletCharacterType="UnicodeOnly" BulletCharacterValue="42"/>
//!     <NumberingFormat type="string">1, 2, 3, 4...</NumberingFormat>
//!     <AppliedNumberingList type="object">NumberingList/Steps</AppliedNumberingList>
//!   </Properties>
//!   <CharacterStyleRange …>
//! ```
//!
//! A model paragraph's (or style's) values come out here. A source child
//! the model still matches passes through byte for byte; one the model
//! no longer carries is dropped; an element that gained any gets them
//! inside its existing `<Properties>`, or a new block as its first
//! child. An element with no model counterpart (the provenance names
//! none, the `Self` is unknown, or the part failed to parse) passes
//! through untouched, children included. Story-level ranges only; cell
//! paragraphs are the emitter's.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::styles::{ParagraphStyleDef, StyleSheet};
use idml_import::{Paragraph, Story, TabStop};

use crate::emit::{
    paragraph_has_properties, write_applied_numbering_list, write_bullet_char,
    write_numbering_expression, write_numbering_format, write_tab_list,
};
use crate::rewrite::resolve_paragraph;

/// What a range or a paragraph style owns among the children this pass
/// spells — one shape for both, so `<ParagraphStyle>` elements (keyed
/// by `Self`) go through the same pass as `<ParagraphStyleRange>`s
/// (keyed by provenance).
pub(crate) struct Owner<'a> {
    pub(crate) tabs: &'a [TabStop],
    pub(crate) bullet: Option<u32>,
    pub(crate) format: Option<&'a str>,
    pub(crate) list: Option<&'a str>,
    /// A style's `NumberingExpression`; a range has none.
    pub(crate) expression: Option<&'a str>,
}

impl<'a> Owner<'a> {
    pub(crate) fn of_paragraph(p: &'a Paragraph) -> Self {
        Owner {
            tabs: &p.tab_list,
            bullet: p.bullet_character,
            format: p.numbering_format.as_deref(),
            list: p.applied_numbering_list.as_deref(),
            expression: None,
        }
    }

    pub(crate) fn of_style(s: &'a ParagraphStyleDef) -> Self {
        Owner {
            tabs: &s.tab_list,
            bullet: s.bullet_character,
            format: s.numbering_format.as_deref(),
            list: s.applied_numbering_list.as_deref(),
            expression: s.numbering_expression.as_deref(),
        }
    }

    pub(crate) fn any(&self) -> bool {
        !self.tabs.is_empty()
            || self.bullet.is_some()
            || self.format.is_some()
            || self.list.is_some()
            || self.expression.is_some()
    }

    /// Every child, in InDesign's order, for a freshly written element.
    pub(crate) fn write_children(
        &self,
        writer: &mut Writer<Cursor<Vec<u8>>>,
    ) -> Result<(), quick_xml::Error> {
        if !self.tabs.is_empty() {
            write_tab_list(writer, self.tabs)?;
        }
        if let Some(cp) = self.bullet {
            write_bullet_char(writer, cp)?;
        }
        if let Some(f) = self.format {
            write_numbering_format(writer, f)?;
        }
        if let Some(l) = self.list {
            write_applied_numbering_list(writer, l)?;
        }
        if let Some(x) = self.expression {
            write_numbering_expression(writer, x)?;
        }
        Ok(())
    }
}

/// The open element and what its owner still owes.
struct Open<'a> {
    owner: Option<Owner<'a>>,
    /// Element depth below the element's start tag.
    depth: usize,
    /// Inside the element's direct `<Properties>` child.
    in_props: bool,
    /// The element's first child has been seen (so a missing
    /// `<Properties>` block, when owed, was written before it).
    first_child_seen: bool,
    tabs_done: bool,
    bullet_done: bool,
    format_done: bool,
    list_done: bool,
    expression_done: bool,
}

impl<'a> Open<'a> {
    fn new(owner: Option<Owner<'a>>) -> Self {
        Open {
            owner,
            depth: 0,
            in_props: false,
            first_child_seen: false,
            tabs_done: false,
            bullet_done: false,
            format_done: false,
            list_done: false,
            expression_done: false,
        }
    }
    fn owes_tabs(&self) -> bool {
        !self.tabs_done && self.owner.as_ref().is_some_and(|o| !o.tabs.is_empty())
    }
    fn owes_bullet(&self) -> bool {
        !self.bullet_done && self.owner.as_ref().is_some_and(|o| o.bullet.is_some())
    }
    fn owes_format(&self) -> bool {
        !self.format_done && self.owner.as_ref().is_some_and(|o| o.format.is_some())
    }
    fn owes_list(&self) -> bool {
        !self.list_done && self.owner.as_ref().is_some_and(|o| o.list.is_some())
    }
    fn owes_expression(&self) -> bool {
        !self.expression_done && self.owner.as_ref().is_some_and(|o| o.expression.is_some())
    }
    fn owes_any(&self) -> bool {
        self.owes_tabs()
            || self.owes_bullet()
            || self.owes_format()
            || self.owes_list()
            || self.owes_expression()
    }
}

/// The text-valued children.
#[derive(Clone, Copy)]
enum TextChild {
    Format,
    List,
    Expression,
}

/// InDesign's several spellings of "no numbering list", which the parser
/// reads as `None`.
fn is_no_list(text: &str) -> bool {
    matches!(text, "n" | "NumberingList/n" | "") || text.ends_with("[No numbering list]")
}

/// The text of a collected subtree.
fn subtree_text(inner: &[Event<'static>]) -> String {
    let mut text = String::new();
    for ev in inner {
        if let Event::Text(t) = ev {
            if let Ok(s) = t.decode() {
                text.push_str(&s);
            }
        }
    }
    text.trim().to_string()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn source_has_any(original: &[u8]) -> bool {
    contains(original, b"<TabList")
        || contains(original, b"<BulletChar")
        || contains(original, b"<NumberingFormat")
        || contains(original, b"<AppliedNumberingList")
        || contains(original, b"<NumberingExpression")
}

/// A story's ranges, matched to the model by provenance.
pub(crate) fn spell(original: &[u8], story: &Story) -> Result<Vec<u8>, quick_xml::Error> {
    let model_has = story.paragraphs.iter().any(paragraph_has_properties);
    if !model_has && !source_has_any(original) {
        return Ok(original.to_vec());
    }
    let Ok((_, provenance)) = idml_import::parse_story_with_provenance(original) else {
        return Ok(original.to_vec());
    };
    spell_with(original, b"ParagraphStyleRange", |_, pos| {
        resolve_paragraph(&provenance, pos, &story.paragraphs).map(Owner::of_paragraph)
    })
}

/// A style part's `<ParagraphStyle>` elements, matched to the model by
/// `Self`.
pub(crate) fn spell_styles(
    original: &[u8],
    styles: &StyleSheet,
) -> Result<Vec<u8>, quick_xml::Error> {
    let model_has = styles
        .paragraph_styles
        .values()
        .any(|s| Owner::of_style(s).any());
    if !model_has && !source_has_any(original) {
        return Ok(original.to_vec());
    }
    spell_with(original, b"ParagraphStyle", |e, _| {
        let mut owner = attr_string(e, b"Self")
            .and_then(|id| styles.paragraph_styles.get(&id))
            .map(Owner::of_style)?;
        // InDesign's own files spell the numbering expression as an
        // ATTRIBUTE (and read it there); one that already says what the
        // model says is not owed as a child.
        if attr_string(e, b"NumberingExpression").as_deref() == owner.expression {
            owner.expression = None;
        }
        Some(owner)
    })
}

/// The `<Properties>` children this pass owns.
fn is_owned_child(name: &[u8]) -> bool {
    matches!(
        name,
        b"TabList"
            | b"BulletChar"
            | b"NumberingFormat"
            | b"AppliedNumberingList"
            | b"NumberingExpression"
    )
}

fn spell_with<'m>(
    original: &[u8],
    element: &[u8],
    resolve: impl Fn(&BytesStart<'_>, u64) -> Option<Owner<'m>>,
) -> Result<Vec<u8>, quick_xml::Error> {
    let element_name = std::str::from_utf8(element).unwrap_or("ParagraphStyleRange");
    let mut reader = Reader::from_reader(original);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(original.len() + 256)));
    let mut buf = Vec::new();
    let mut table_depth = 0usize;
    let mut open: Option<Open> = None;

    loop {
        let pos = reader.buffer_position();
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.name().as_ref().to_vec();
                if name == b"Table" {
                    table_depth += 1;
                }
                if name == element && table_depth == 0 && open.is_none() {
                    open = Some(Open::new(resolve(&e, pos)));
                    writer.write_event(Event::Start(e.into_owned()))?;
                    buf.clear();
                    continue;
                }
                let Some(o) = open.as_mut() else {
                    writer.write_event(Event::Start(e.into_owned()))?;
                    buf.clear();
                    continue;
                };
                if o.depth == 0 {
                    if name == b"Properties" {
                        o.in_props = true;
                        o.first_child_seen = true;
                    } else if !o.first_child_seen {
                        o.first_child_seen = true;
                        write_missing_block(&mut writer, o)?;
                    }
                } else if o.depth == 1 && o.in_props && is_owned_child(&name) {
                    // The child's whole subtree, then the verdict.
                    let start = e.into_owned();
                    let inner = collect_subtree(&mut reader, &name)?;
                    match name.as_slice() {
                        b"TabList" => {
                            let stops = parse_tab_list(&inner);
                            settle_tabs(&mut writer, o, Some((&start, &inner)), &stops)?;
                        }
                        b"BulletChar" => {
                            let cp = attr_u32(&start, b"BulletCharacterValue");
                            settle_bullet(&mut writer, o, Some((&start, &inner)), cp)?;
                        }
                        other => {
                            let which = match other {
                                b"NumberingFormat" => TextChild::Format,
                                b"NumberingExpression" => TextChild::Expression,
                                _ => TextChild::List,
                            };
                            let text = subtree_text(&inner);
                            settle_text(&mut writer, o, Some((&start, &inner)), &text, which)?;
                        }
                    }
                    buf.clear();
                    continue;
                }
                o.depth += 1;
                writer.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                if name == element && table_depth == 0 && open.is_none() {
                    // A self-closing element: opened around the block
                    // when its owner owes one.
                    match resolve(&e, pos).filter(|o| o.any()) {
                        Some(owner) => {
                            writer.write_event(Event::Start(e.borrow()))?;
                            let mut o = Open::new(Some(owner));
                            write_missing_block(&mut writer, &mut o)?;
                            writer.write_event(Event::End(BytesEnd::new(element_name)))?;
                        }
                        None => writer.write_event(Event::Empty(e.into_owned()))?,
                    }
                    buf.clear();
                    continue;
                }
                let Some(o) = open.as_mut() else {
                    writer.write_event(Event::Empty(e.into_owned()))?;
                    buf.clear();
                    continue;
                };
                if o.depth == 0 {
                    if name == b"Properties" {
                        // `<Properties/>`: opened when the owner owes a
                        // child, kept self-closing otherwise.
                        o.first_child_seen = true;
                        if o.owes_any() {
                            writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                            write_missing_children(&mut writer, o)?;
                            writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                        } else {
                            writer.write_event(Event::Empty(e.into_owned()))?;
                        }
                        buf.clear();
                        continue;
                    }
                    if !o.first_child_seen {
                        o.first_child_seen = true;
                        write_missing_block(&mut writer, o)?;
                    }
                } else if o.depth == 1 && o.in_props && is_owned_child(&name) {
                    match name.as_slice() {
                        b"TabList" => settle_tabs(&mut writer, o, Some((&e.borrow(), &[])), &[])?,
                        b"BulletChar" => {
                            let cp = attr_u32(&e, b"BulletCharacterValue");
                            settle_bullet(&mut writer, o, Some((&e.borrow(), &[])), cp)?;
                        }
                        other => {
                            let which = match other {
                                b"NumberingFormat" => TextChild::Format,
                                b"NumberingExpression" => TextChild::Expression,
                                _ => TextChild::List,
                            };
                            settle_text(&mut writer, o, Some((&e.borrow(), &[])), "", which)?;
                        }
                    }
                    buf.clear();
                    continue;
                }
                writer.write_event(Event::Empty(e.into_owned()))?;
            }
            Event::End(e) => {
                let name = e.name().as_ref().to_vec();
                if name == b"Table" {
                    table_depth = table_depth.saturating_sub(1);
                }
                if let Some(o) = open.as_mut() {
                    if o.depth == 0 {
                        // The element closes; a block it still owes goes
                        // in before the end tag (an element with no
                        // children at all).
                        if !o.first_child_seen {
                            write_missing_block(&mut writer, o)?;
                        }
                        writer.write_event(Event::End(e.into_owned()))?;
                        open = None;
                        buf.clear();
                        continue;
                    }
                    o.depth -= 1;
                    if o.depth == 0 && o.in_props && name == b"Properties" {
                        write_missing_children(&mut writer, o)?;
                        o.in_props = false;
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

/// Every event up to and including the end tag matching `name`, the
/// reader having just returned that element's start tag.
fn collect_subtree(
    reader: &mut Reader<&[u8]>,
    name: &[u8],
) -> Result<Vec<Event<'static>>, quick_xml::Error> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut buf = Vec::new();
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        match &ev {
            Event::Eof => break,
            Event::Start(_) => depth += 1,
            Event::End(e) => {
                if depth == 0 {
                    debug_assert_eq!(e.name().as_ref(), name);
                    out.push(ev.into_owned());
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
        out.push(ev.into_owned());
        buf.clear();
    }
    Ok(out)
}

/// The stops a source `<TabList>` subtree spells — InDesign's
/// `<ListItem type="record">` of child elements and the generator's
/// `<TabStop Position=…/>` attributes alike, read the way the parser
/// reads them (an empty `<Leader>` is no leader).
fn parse_tab_list(inner: &[Event<'static>]) -> Vec<TabStop> {
    let mut stops = Vec::new();
    let mut pending: Option<(TabStop, bool)> = None;
    let mut field: Option<Vec<u8>> = None;
    let mut text = String::new();
    for ev in inner {
        match ev {
            Event::Start(e) => {
                let n = e.name().as_ref().to_vec();
                if n == b"ListItem" {
                    pending = Some((
                        TabStop {
                            position: 0.0,
                            alignment: None,
                            alignment_character: None,
                            leader: None,
                        },
                        false,
                    ));
                } else if pending.is_some() {
                    field = Some(n);
                    text.clear();
                }
            }
            Event::Empty(e) if e.name().as_ref() == b"TabStop" => {
                if let Some(position) = attr_f32(e, b"Position") {
                    stops.push(TabStop {
                        position,
                        alignment: attr_string(e, b"Alignment"),
                        alignment_character: attr_string(e, b"AlignmentCharacter"),
                        leader: attr_string(e, b"Leader"),
                    });
                }
            }
            Event::Text(t) => {
                if field.is_some() {
                    if let Ok(s) = t.decode() {
                        text.push_str(&s);
                    }
                }
            }
            Event::End(e) => {
                let n = e.name().as_ref().to_vec();
                let n = n.as_slice();
                if n == b"ListItem" {
                    if let Some((stop, has_position)) = pending.take() {
                        if has_position {
                            stops.push(stop);
                        }
                    }
                } else if field.as_deref() == Some(n) {
                    let value = text.trim().to_string();
                    if let Some((stop, has_position)) = pending.as_mut() {
                        match n {
                            b"Position" => {
                                if let Ok(v) = value.parse::<f32>() {
                                    stop.position = v;
                                    *has_position = true;
                                }
                            }
                            b"Alignment" if !value.is_empty() => stop.alignment = Some(value),
                            b"AlignmentCharacter" if !value.is_empty() => {
                                stop.alignment_character = Some(value)
                            }
                            b"Leader" if !value.is_empty() => stop.leader = Some(value),
                            _ => {}
                        }
                    }
                    field = None;
                    text.clear();
                }
            }
            _ => {}
        }
    }
    stops
}

fn attr_string(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.as_ref() == key)
            .then(|| {
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
            .flatten()
    })
}

fn attr_f32(e: &BytesStart, key: &[u8]) -> Option<f32> {
    attr_string(e, key).and_then(|s| s.trim().parse().ok())
}

fn attr_u32(e: &BytesStart, key: &[u8]) -> Option<u32> {
    attr_string(e, key).and_then(|s| s.trim().parse().ok())
}

fn write_events(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    start: &BytesStart<'_>,
    inner: &[Event<'static>],
) -> Result<(), quick_xml::Error> {
    if inner.is_empty() {
        writer.write_event(Event::Empty(start.borrow()))?;
    } else {
        writer.write_event(Event::Start(start.borrow()))?;
        for ev in inner {
            writer.write_event(ev.borrow())?;
        }
    }
    Ok(())
}

/// A source `<TabList>` (or none) against the model: verbatim when the
/// model spells the same stops, the model's spelling when it differs,
/// nothing when the model has none. An element without a model
/// counterpart keeps its source.
fn settle_tabs(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    o: &mut Open<'_>,
    source: Option<(&BytesStart<'_>, &[Event<'static>])>,
    source_stops: &[TabStop],
) -> Result<(), quick_xml::Error> {
    o.tabs_done = true;
    let Some(p) = o.owner.as_ref() else {
        if let Some((start, inner)) = source {
            write_events(writer, start, inner)?;
        }
        return Ok(());
    };
    if p.tabs.is_empty() {
        // InDesign writes an empty `<TabList type="list">` on its own
        // styles; with nothing to spell either way, it keeps its bytes.
        if let Some((start, inner)) = source {
            if source_stops.is_empty() {
                write_events(writer, start, inner)?;
            }
        }
        return Ok(());
    }
    match source {
        Some((start, inner)) if p.tabs == source_stops => write_events(writer, start, inner),
        _ => write_tab_list(writer, p.tabs),
    }
}

fn settle_bullet(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    o: &mut Open<'_>,
    source: Option<(&BytesStart<'_>, &[Event<'static>])>,
    source_cp: Option<u32>,
) -> Result<(), quick_xml::Error> {
    o.bullet_done = true;
    let Some(p) = o.owner.as_ref() else {
        if let Some((start, inner)) = source {
            write_events(writer, start, inner)?;
        }
        return Ok(());
    };
    let Some(cp) = p.bullet else {
        return Ok(());
    };
    match source {
        Some((start, inner)) if source_cp == Some(cp) => write_events(writer, start, inner),
        _ => write_bullet_char(writer, cp),
    }
}

/// A source `<NumberingFormat>` / `<AppliedNumberingList>` /
/// `<NumberingExpression>` (or none) against the model, like
/// [`settle_tabs`]. A "no list" spelling the parser reads as `None`
/// keeps its bytes.
fn settle_text(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    o: &mut Open<'_>,
    source: Option<(&BytesStart<'_>, &[Event<'static>])>,
    source_text: &str,
    which: TextChild,
) -> Result<(), quick_xml::Error> {
    match which {
        TextChild::Format => o.format_done = true,
        TextChild::List => o.list_done = true,
        TextChild::Expression => o.expression_done = true,
    }
    let Some(p) = o.owner.as_ref() else {
        if let Some((start, inner)) = source {
            write_events(writer, start, inner)?;
        }
        return Ok(());
    };
    let model = match which {
        TextChild::Format => p.format,
        TextChild::List => p.list,
        TextChild::Expression => p.expression,
    };
    let Some(value) = model else {
        if let (Some((start, inner)), TextChild::List) = (source, which) {
            if is_no_list(source_text) {
                write_events(writer, start, inner)?;
            }
        }
        return Ok(());
    };
    match source {
        Some((start, inner)) if source_text == value => write_events(writer, start, inner),
        _ => match which {
            TextChild::Format => write_numbering_format(writer, value),
            TextChild::List => write_applied_numbering_list(writer, value),
            TextChild::Expression => write_numbering_expression(writer, value),
        },
    }
}

/// The children the owner still owes, inside an open `<Properties>`.
fn write_missing_children(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    o: &mut Open<'_>,
) -> Result<(), quick_xml::Error> {
    if o.owes_tabs() {
        settle_tabs(writer, o, None, &[])?;
    }
    if o.owes_bullet() {
        settle_bullet(writer, o, None, None)?;
    }
    if o.owes_format() {
        settle_text(writer, o, None, "", TextChild::Format)?;
    }
    if o.owes_list() {
        settle_text(writer, o, None, "", TextChild::List)?;
    }
    if o.owes_expression() {
        settle_text(writer, o, None, "", TextChild::Expression)?;
    }
    Ok(())
}

/// A whole `<Properties>` block for an element that has none, when owed.
fn write_missing_block(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    o: &mut Open<'_>,
) -> Result<(), quick_xml::Error> {
    if !o.owes_any() {
        return Ok(());
    }
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    write_missing_children(writer, o)?;
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDESIGN: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
	<Story Self="s">
		<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body" LeftIndent="18" BulletsAndNumberingListType="BulletList">
			<Properties>
				<TabList type="list">
					<ListItem type="record">
						<Alignment type="enumeration">LeftAlign</Alignment>
						<AlignmentCharacter type="string">.</AlignmentCharacter>
						<Leader type="string"></Leader>
						<Position type="unit">34</Position>
					</ListItem>
				</TabList>
				<BulletChar BulletCharacterType="UnicodeOnly" BulletCharacterValue="42"/>
			</Properties>
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>Item</Content>
			</CharacterStyleRange>
		</ParagraphStyleRange>
		<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body">
			<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
				<Content>Plain</Content>
			</CharacterStyleRange>
		</ParagraphStyleRange>
	</Story>
</idPkg:Story>"#;

    fn story(xml: &[u8]) -> Story {
        idml_import::parse_story(xml).unwrap()
    }

    #[test]
    fn an_unmutated_indesign_story_is_byte_identical() {
        let s = story(INDESIGN);
        assert_eq!(s.paragraphs[0].tab_list.len(), 1);
        assert_eq!(s.paragraphs[0].bullet_character, Some(42));
        let out = spell(INDESIGN, &s).unwrap();
        assert_eq!(
            std::str::from_utf8(&out).unwrap(),
            std::str::from_utf8(INDESIGN).unwrap()
        );
    }

    #[test]
    fn changed_stops_are_respelled_and_cleared_ones_dropped() {
        let mut s = story(INDESIGN);
        s.paragraphs[0].tab_list[0].position = 72.0;
        s.paragraphs[0].tab_list.push(TabStop {
            position: 200.0,
            alignment: Some("RightAlign".into()),
            alignment_character: None,
            leader: Some(".".into()),
        });
        s.paragraphs[0].bullet_character = None;
        let out = String::from_utf8(spell(INDESIGN, &s).unwrap()).unwrap();
        assert!(
            out.contains(r#"<TabList type="list"><ListItem type="record"><Alignment type="enumeration">LeftAlign</Alignment><AlignmentCharacter type="string">.</AlignmentCharacter><Leader type="string"></Leader><Position type="unit">72</Position></ListItem><ListItem type="record"><Alignment type="enumeration">RightAlign</Alignment><AlignmentCharacter type="string">.</AlignmentCharacter><Leader type="string">.</Leader><Position type="unit">200</Position></ListItem></TabList>"#),
            "{out}"
        );
        assert!(!out.contains("BulletChar"), "{out}");
        assert!(
            out.contains(r#"LeftIndent="18""#),
            "attributes are not this pass's: {out}"
        );
    }

    #[test]
    fn a_paragraph_that_gained_stops_gets_a_properties_block_first() {
        let mut s = story(INDESIGN);
        s.paragraphs[1].tab_list.push(TabStop {
            position: 36.0,
            alignment: None,
            alignment_character: None,
            leader: None,
        });
        s.paragraphs[1].bullet_character = Some(8226);
        let out = String::from_utf8(spell(INDESIGN, &s).unwrap()).unwrap();
        let i = out
            .find(r#"<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Body">"#)
            .unwrap();
        let tail = &out[i..];
        let props = tail.find("<Properties>").unwrap();
        let csr = tail.find("<CharacterStyleRange").unwrap();
        assert!(props < csr, "{tail}");
        assert!(
            tail.contains(r#"<Position type="unit">36</Position></ListItem></TabList><BulletChar BulletCharacterType="UnicodeOnly" BulletCharacterValue="8226"/></Properties>"#),
            "{tail}"
        );
        // The first paragraph, untouched, keeps its bytes.
        assert!(
            out.contains("\t\t\t\t\t\t<Position type=\"unit\">34</Position>\n"),
            "{out}"
        );
    }

    const NUMBERED: &[u8] = br#"<idPkg:Story xmlns:idPkg="x"><Story Self="s"><ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/Numbered 1" BulletsAndNumberingListType="NumberedList"><Properties><NumberingFormat type="string">1, 2, 3, 4...</NumberingFormat><AppliedNumberingList type="object">NumberingList/Annual Steps</AppliedNumberingList></Properties><CharacterStyleRange><Content>Step</Content></CharacterStyleRange></ParagraphStyleRange><ParagraphStyleRange><CharacterStyleRange><Content>Plain</Content></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;

    #[test]
    fn numbering_children_round_trip_respell_and_appear() {
        let s = story(NUMBERED);
        assert_eq!(
            s.paragraphs[0].numbering_format.as_deref(),
            Some("1, 2, 3, 4...")
        );
        assert_eq!(
            s.paragraphs[0].applied_numbering_list.as_deref(),
            Some("NumberingList/Annual Steps")
        );
        assert_eq!(
            spell(NUMBERED, &s).unwrap(),
            NUMBERED.to_vec(),
            "unmutated: byte-identical"
        );

        let mut s = story(NUMBERED);
        s.paragraphs[0].numbering_format = Some("I, II, III, IV...".into());
        s.paragraphs[0].applied_numbering_list = None;
        s.paragraphs[1].applied_numbering_list = Some("NumberingList/Other".into());
        let out = String::from_utf8(spell(NUMBERED, &s).unwrap()).unwrap();
        assert!(
            out.contains(r#"<Properties><NumberingFormat type="string">I, II, III, IV...</NumberingFormat></Properties>"#),
            "{out}"
        );
        assert!(
            out.contains(r#"<ParagraphStyleRange><Properties><AppliedNumberingList type="object">NumberingList/Other</AppliedNumberingList></Properties><CharacterStyleRange><Content>Plain"#),
            "{out}"
        );
    }

    #[test]
    fn a_paragraph_style_gets_its_numbering_children() {
        let src = br#"<idPkg:Styles xmlns:idPkg="x"><RootParagraphStyleGroup Self="u9e"><ParagraphStyle Self="ParagraphStyle/Numbered 1" Name="Numbered 1" BulletsAndNumberingListType="NumberedList"/><ParagraphStyle Self="ParagraphStyle/Body" Name="Body"><Properties><BasedOn type="object">ParagraphStyle/X</BasedOn></Properties></ParagraphStyle></RootParagraphStyleGroup></idPkg:Styles>"#;
        let mut styles = idml_import::parse_stylesheet(src).unwrap();
        assert_eq!(
            spell_styles(src, &styles).unwrap(),
            src.to_vec(),
            "unchanged: byte-identical"
        );
        let n = styles
            .paragraph_styles
            .get_mut("ParagraphStyle/Numbered 1")
            .unwrap();
        n.numbering_format = Some("1, 2, 3, 4...".into());
        n.applied_numbering_list = Some("NumberingList/Annual Steps".into());
        n.numbering_expression = Some("^#.^t".into());
        let out = String::from_utf8(spell_styles(src, &styles).unwrap()).unwrap();
        assert!(
            out.contains(r#"<ParagraphStyle Self="ParagraphStyle/Numbered 1" Name="Numbered 1" BulletsAndNumberingListType="NumberedList"><Properties><NumberingFormat type="string">1, 2, 3, 4...</NumberingFormat><AppliedNumberingList type="object">NumberingList/Annual Steps</AppliedNumberingList><NumberingExpression type="string">^#.^t</NumberingExpression></Properties></ParagraphStyle>"#),
            "{out}"
        );
        assert!(
            out.contains(r#"<BasedOn type="object">ParagraphStyle/X</BasedOn></Properties>"#),
            "{out}"
        );
    }

    #[test]
    fn a_source_without_either_and_a_model_without_either_is_untouched() {
        let xml = br#"<idPkg:Story xmlns:idPkg="x"><Story Self="s"><ParagraphStyleRange><CharacterStyleRange><Content>a</Content></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;
        let s = story(xml);
        assert_eq!(spell(xml, &s).unwrap(), xml.to_vec());
    }
}
