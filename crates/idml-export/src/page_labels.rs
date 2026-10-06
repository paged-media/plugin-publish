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

//! A page's `Properties/Label` — its plugin metadata (a slide's notes and
//! transition) — written from the model's `spread.labels`, which keys
//! pages by their `Self` as it keys items.
//!
//! The page-item rewrite owns item labels; pages are not page items, so
//! they get this pass of their own. A page whose label the model leaves
//! as the source had it is not touched, and a spread with no such page
//! comes back byte-identical.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use idml_import::Spread;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use crate::rewrite::{self, attr_value};

/// Label entries by page id.
type PageLabels = HashMap<String, Vec<(String, String)>>;

/// Each page's source label entries, and which pages have a direct
/// `<Properties>` child.
fn source_labels(original: &[u8]) -> Result<(PageLabels, HashSet<String>), quick_xml::Error> {
    let mut reader = Reader::from_reader(original);
    let mut buf = Vec::new();
    let mut labels = PageLabels::new();
    let mut with_props = HashSet::new();
    // (page id, depth of the page element) while a page is open.
    let mut page: Option<(String, usize)> = None;
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                if e.name().as_ref() == b"Page" {
                    page = attr_value(&e, b"Self").map(|id| (id, depth));
                } else if let Some((id, d)) = &page {
                    if e.name().as_ref() == b"Properties" && depth == d + 1 {
                        with_props.insert(id.clone());
                    }
                }
            }
            Event::Empty(e) => {
                if let Some((id, _)) = &page {
                    if e.name().as_ref() == b"KeyValuePair" {
                        if let Some((k, v)) = rewrite::key_value_pair(&e) {
                            let entries = labels.entry(id.clone()).or_default();
                            match entries.iter_mut().find(|(ek, _)| *ek == k) {
                                Some(slot) => slot.1 = v,
                                None => entries.push((k, v)),
                            }
                        }
                    }
                }
            }
            Event::End(e) => {
                if e.name().as_ref() == b"Page" {
                    page = None;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        buf.clear();
    }
    Ok((labels, with_props))
}

/// Write each page's label from the model.
pub(crate) fn rewrite_page_labels(
    original: &[u8],
    spread: &Spread,
) -> Result<Vec<u8>, quick_xml::Error> {
    let model = |id: &str| spread.labels.get(id).filter(|v| !v.is_empty());
    let (source, with_props) = source_labels(original)?;
    let changed: HashSet<String> = spread
        .pages
        .iter()
        .filter_map(|p| p.self_id.clone())
        .filter(|id| model(id) != source.get(id).filter(|v| !v.is_empty()))
        .collect();
    if changed.is_empty() {
        return Ok(original.to_vec());
    }

    let write_label = |w: &mut Writer<Cursor<Vec<u8>>>, id: &str| -> Result<(), quick_xml::Error> {
        rewrite::write_item_label(w, spread, id)
    };
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut depth = 0usize;
    // The changed page open now, and the depth of its element.
    let mut page: Option<(String, usize)> = None;
    // Depth of the page's own `<Properties>` while it is open.
    let mut props: Option<usize> = None;
    // Depth of a source `<Label>` being skipped.
    let mut skipping: Option<usize> = None;
    loop {
        let ev = reader.read_event_into(&mut buf)?;
        if let Some(d) = skipping {
            match &ev {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    if depth == d {
                        skipping = None;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            buf.clear();
            continue;
        }
        match ev {
            Event::Eof => break,
            Event::Empty(e) if e.name().as_ref() == b"Page" => {
                let id = attr_value(&e, b"Self").filter(|id| changed.contains(id));
                match id.filter(|id| model(id).is_some()) {
                    Some(id) => {
                        writer.write_event(Event::Start(e.into_owned()))?;
                        writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                        write_label(&mut writer, &id)?;
                        writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                        writer.write_event(Event::End(BytesEnd::new("Page")))?;
                    }
                    None => writer.write_event(Event::Empty(e.into_owned()))?,
                }
            }
            Event::Start(e) => {
                depth += 1;
                let name = e.name().as_ref().to_vec();
                if name == b"Page" {
                    let id = attr_value(&e, b"Self").filter(|id| changed.contains(id));
                    writer.write_event(Event::Start(e.into_owned()))?;
                    if let Some(id) = id {
                        if !with_props.contains(&id) && model(&id).is_some() {
                            writer.write_event(Event::Start(BytesStart::new("Properties")))?;
                            write_label(&mut writer, &id)?;
                            writer.write_event(Event::End(BytesEnd::new("Properties")))?;
                        }
                        page = Some((id, depth));
                    }
                } else if let Some((_, pd)) = &page {
                    if name == b"Properties" && depth == pd + 1 {
                        props = Some(depth);
                        writer.write_event(Event::Start(e.into_owned()))?;
                    } else if name == b"Label" && props == Some(depth - 1) {
                        skipping = Some(depth);
                    } else {
                        writer.write_event(Event::Start(e.into_owned()))?;
                    }
                } else {
                    writer.write_event(Event::Start(e.into_owned()))?;
                }
            }
            Event::Empty(e) if e.name().as_ref() == b"Label" && props == Some(depth) => {}
            Event::End(e) => {
                if props == Some(depth) && e.name().as_ref() == b"Properties" {
                    if let Some((id, _)) = &page {
                        write_label(&mut writer, id)?;
                    }
                    props = None;
                }
                if page.as_ref().is_some_and(|(_, d)| *d == depth) {
                    page = None;
                }
                depth = depth.saturating_sub(1);
                writer.write_event(Event::End(e.into_owned()))?;
            }
            other => writer.write_event(other)?,
        }
        buf.clear();
    }
    Ok(writer.into_inner().into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spread_with(labels: &[(&str, &[(&str, &str)])]) -> Spread {
        let mut s = idml_import::parse_spread(XML.as_bytes()).unwrap();
        s.labels.clear();
        for (id, kv) in labels {
            s.labels.insert(
                id.to_string(),
                kv.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            );
        }
        s
    }

    const XML: &str = r#"<Spread Self="s"><Page Self="p1" GeometricBounds="0 0 1 1"><Properties><PageColor type="enumeration">UseMasterColor</PageColor><Label><KeyValuePair Key="a" Value="1"/></Label></Properties></Page><Page Self="p2" GeometricBounds="0 0 1 1"/></Spread>"#;

    #[test]
    fn an_unchanged_label_is_byte_identical() {
        let s = spread_with(&[("p1", &[("a", "1")])]);
        assert_eq!(
            rewrite_page_labels(XML.as_bytes(), &s).unwrap(),
            XML.as_bytes()
        );
    }

    #[test]
    fn labels_are_replaced_added_and_dropped() {
        let s = spread_with(&[("p1", &[("a", "2")]), ("p2", &[("b", "x\ny")])]);
        let out = String::from_utf8(rewrite_page_labels(XML.as_bytes(), &s).unwrap()).unwrap();
        assert!(out.contains(r#"<PageColor type="enumeration">UseMasterColor</PageColor><Label><KeyValuePair Key="a" Value="2"/></Label></Properties>"#), "{out}");
        assert!(out.contains(r#"<Page Self="p2" GeometricBounds="0 0 1 1"><Properties><Label><KeyValuePair Key="b""#), "{out}");
        let back = idml_import::parse_spread(out.as_bytes()).unwrap();
        assert_eq!(
            back.labels["p2"],
            vec![("b".to_string(), "x\ny".to_string())]
        );

        let none = spread_with(&[]);
        let out = String::from_utf8(rewrite_page_labels(XML.as_bytes(), &none).unwrap()).unwrap();
        assert!(!out.contains("KeyValuePair"), "{out}");
        assert!(out.contains("<Properties><PageColor"), "{out}");
    }
}
