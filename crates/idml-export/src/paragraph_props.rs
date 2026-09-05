//! Tab stops and bullet characters of a `<ParagraphStyleRange>` — the
//! two paragraph overrides IDML spells as `<Properties>` children rather
//! than attributes. InDesign 20.0.1 writes, and reads, exactly this
//! (measured on the corpus's InDesign-authored packs, 2026-09-06):
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
//!   </Properties>
//!   <CharacterStyleRange …>
//! ```
//!
//! A model paragraph's `tab_list` / `bullet_character` come out here. A
//! source child the model still matches passes through byte for byte; one
//! the model no longer carries is dropped; a paragraph that gained either
//! gets them inside its existing `<Properties>`, or a new block as its
//! first child. A range with no model counterpart (the provenance names
//! none, or the story failed to parse) passes through untouched, children
//! included. Story-level ranges only; cell paragraphs are the emitter's.

use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{Paragraph, Story, TabStop};

use crate::emit::{paragraph_has_properties, write_bullet_char, write_tab_list};
use crate::rewrite::resolve_paragraph;

/// The open story-level range and what its model paragraph still owes.
struct Open<'a> {
    para: Option<&'a Paragraph>,
    /// Element depth below the range's start tag.
    depth: usize,
    /// Inside the range's direct `<Properties>` child.
    in_props: bool,
    /// The range's first child has been seen (so a missing `<Properties>`
    /// block, when owed, was written before it).
    first_child_seen: bool,
    tabs_done: bool,
    bullet_done: bool,
}

impl Open<'_> {
    fn owes_tabs(&self) -> bool {
        !self.tabs_done && self.para.is_some_and(|p| !p.tab_list.is_empty())
    }
    fn owes_bullet(&self) -> bool {
        !self.bullet_done && self.para.is_some_and(|p| p.bullet_character.is_some())
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

pub(crate) fn spell(original: &[u8], story: &Story) -> Result<Vec<u8>, quick_xml::Error> {
    let model_has = story.paragraphs.iter().any(paragraph_has_properties);
    let source_has = contains(original, b"<TabList") || contains(original, b"<BulletChar");
    if !model_has && !source_has {
        return Ok(original.to_vec());
    }
    let Ok((_, provenance)) = idml_import::parse_story_with_provenance(original) else {
        return Ok(original.to_vec());
    };

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
                if name == b"ParagraphStyleRange" && table_depth == 0 && open.is_none() {
                    open = Some(Open {
                        para: resolve_paragraph(&provenance, pos, &story.paragraphs),
                        depth: 0,
                        in_props: false,
                        first_child_seen: false,
                        tabs_done: false,
                        bullet_done: false,
                    });
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
                } else if o.depth == 1
                    && o.in_props
                    && (name == b"TabList" || name == b"BulletChar")
                {
                    // The child's whole subtree, then the verdict.
                    let start = e.into_owned();
                    let inner = collect_subtree(&mut reader, &name)?;
                    match name.as_slice() {
                        b"TabList" => {
                            let stops = parse_tab_list(&inner);
                            settle_tabs(&mut writer, o, Some((&start, &inner)), &stops)?;
                        }
                        _ => {
                            let cp = attr_u32(&start, b"BulletCharacterValue");
                            settle_bullet(&mut writer, o, Some((&start, &inner)), cp)?;
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
                if name == b"ParagraphStyleRange" && table_depth == 0 && open.is_none() {
                    // A self-closing range: opened around the block when
                    // its paragraph owes one.
                    let para = resolve_paragraph(&provenance, pos, &story.paragraphs);
                    match para.filter(|p| paragraph_has_properties(p)) {
                        Some(p) => {
                            writer.write_event(Event::Start(e.borrow()))?;
                            let mut o = Open {
                                para: Some(p),
                                depth: 0,
                                in_props: false,
                                first_child_seen: false,
                                tabs_done: false,
                                bullet_done: false,
                            };
                            write_missing_block(&mut writer, &mut o)?;
                            writer.write_event(Event::End(BytesEnd::new("ParagraphStyleRange")))?;
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
                        // `<Properties/>`: opened when the paragraph owes
                        // a child, kept self-closing otherwise.
                        o.first_child_seen = true;
                        if o.owes_tabs() || o.owes_bullet() {
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
                } else if o.depth == 1 && o.in_props && name == b"TabList" {
                    settle_tabs(&mut writer, o, Some((&e.borrow(), &[])), &[])?;
                    buf.clear();
                    continue;
                } else if o.depth == 1 && o.in_props && name == b"BulletChar" {
                    let cp = attr_u32(&e, b"BulletCharacterValue");
                    settle_bullet(&mut writer, o, Some((&e.borrow(), &[])), cp)?;
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
                        // The range closes; a block it still owes goes
                        // in before the end tag (a range with no children
                        // at all).
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
/// nothing when the model has none. A range without a model counterpart
/// keeps its source.
fn settle_tabs(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    o: &mut Open<'_>,
    source: Option<(&BytesStart<'_>, &[Event<'static>])>,
    source_stops: &[TabStop],
) -> Result<(), quick_xml::Error> {
    o.tabs_done = true;
    let Some(p) = o.para else {
        if let Some((start, inner)) = source {
            write_events(writer, start, inner)?;
        }
        return Ok(());
    };
    if p.tab_list.is_empty() {
        return Ok(());
    }
    match source {
        Some((start, inner)) if p.tab_list == source_stops => write_events(writer, start, inner),
        _ => write_tab_list(writer, &p.tab_list),
    }
}

fn settle_bullet(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    o: &mut Open<'_>,
    source: Option<(&BytesStart<'_>, &[Event<'static>])>,
    source_cp: Option<u32>,
) -> Result<(), quick_xml::Error> {
    o.bullet_done = true;
    let Some(p) = o.para else {
        if let Some((start, inner)) = source {
            write_events(writer, start, inner)?;
        }
        return Ok(());
    };
    let Some(cp) = p.bullet_character else {
        return Ok(());
    };
    match source {
        Some((start, inner)) if source_cp == Some(cp) => write_events(writer, start, inner),
        _ => write_bullet_char(writer, cp),
    }
}

/// The children the paragraph still owes, inside an open `<Properties>`.
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
    Ok(())
}

/// A whole `<Properties>` block for a range that has none, when owed.
fn write_missing_block(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    o: &mut Open<'_>,
) -> Result<(), quick_xml::Error> {
    if !(o.owes_tabs() || o.owes_bullet()) {
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

    #[test]
    fn a_source_without_either_and_a_model_without_either_is_untouched() {
        let xml = br#"<idPkg:Story xmlns:idPkg="x"><Story Self="s"><ParagraphStyleRange><CharacterStyleRange><Content>a</Content></CharacterStyleRange></ParagraphStyleRange></Story></idPkg:Story>"#;
        let s = story(xml);
        assert_eq!(spell(xml, &s).unwrap(), xml.to_vec());
    }
}
