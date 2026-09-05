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

//! `<Image>` / `<Link>` save-back for LINKED images — a graphic frame the
//! model links to an asset by URI (`PlaceImage`, `image_link`) whose
//! element carries no placed-image child — and, given a link base, for
//! images the model holds only as BYTES.
//!
//! IDML can express this: the frame holds an `<Image>` with the
//! image's own `ItemTransform` (pixel space → frame space), a
//! `<Properties><GraphicBounds/>` box, and a `<Link LinkResourceURI=…>`
//! naming the asset. The attribute set mirrors an InDesign 2025 export
//! (`corpus/idml/samples/sample-3.idml`). InDesign then shows a
//! missing-link placeholder when the URI does not resolve on its host,
//! but keeps the frame, its geometry and the crop — the honest
//! interchange. A 134-page book had 31 such frames; its export had
//! `<Image>` 0.
//!
//! # The URI, with and without a link base
//!
//! Without [`ExportOptions::link_base`](crate::ExportOptions::link_base)
//! the URI is written as the model holds it. A relative `assets/…` path
//! stays relative: the document carries no asset base to resolve it
//! against (the renderer's `AssetResolver` is a runtime concern, not
//! part of the model). Measured in InDesign 20.0.1: a relative URI
//! resolves to NOTHING (`doc.links` = 0, every frame empty).
//!
//! With a base — an absolute directory the caller promises to populate
//! — every link is REBASED to `file:<base>/<basename>` in the exact
//! spelling InDesign writes (`file:/Users/…/Links/photo.psd`,
//! `file:C:/…/Links/CODE%201.jpg`: a single slash after the scheme,
//! spaces and other reserved bytes percent-encoded), and the writer
//! hands back one [`ExportedLink`] per distinct file: the basename to
//! write under the base, the bytes when the model holds them
//! (`image_bytes` — a placed-from-bytes or pixel-edited asset), and the
//! model's own URI so a link-only frame's file can be copied from
//! wherever it lives. A frame that holds bytes and NO link is given a
//! name from its id and the bytes' magic (`u123.png`), so a document
//! authored entirely from bytes still opens in InDesign with every
//! image bound. Two frames naming the same file share one entry; two
//! different files with the same basename are told apart (`logo.png`,
//! `logo-2.png`).
//!
//! Runs AFTER `rewrite_spread`, over its output, so a frame the model
//! INSERTED and a source frame are treated alike. A frame whose element
//! already carries a placed-image child keeps it, with the `<Link>`'s
//! URI patched when the model's (rebased) URI differs.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{Bounds, Spread};

use crate::rewrite::{
    attr_value, emit_empty_with_attrs, emit_start_with_attrs, format_f32, format_matrix,
    patch_start, Patch,
};
use crate::ExportedLink;

/// The files an export's links point at, accumulated across every
/// spread and master the writer visits. Created with the export's link
/// base (`None` ⇒ URIs are written as the model holds them and nothing
/// is collected).
#[derive(Debug, Default)]
pub(crate) struct LinkCollector {
    base: Option<String>,
    /// Embed the bytes a frame holds as the `<Image>`'s
    /// `<Properties><Contents>` (base64, InDesign's own embedded-link
    /// spelling, `StoredState="Embedded"`) — the pure-`.idml` export with
    /// no link base, where a relative URI would resolve to nothing. A
    /// `.paged` write never embeds: its model part holds the bytes.
    embed: bool,
    links: Vec<ExportedLink>,
    /// model uri → the file name it was given (so two frames naming the
    /// same asset share one file).
    by_source: HashMap<String, String>,
    taken: HashSet<String>,
}

impl LinkCollector {
    pub(crate) fn new(base: Option<&str>, embed: bool) -> Self {
        Self {
            base: base.map(|b| b.trim_end_matches('/').to_string()),
            embed,
            ..Self::default()
        }
    }

    pub(crate) fn into_links(self) -> Vec<ExportedLink> {
        self.links
    }

    fn rebased_uri(&self, file_name: &str) -> Option<String> {
        let base = self.base.as_deref()?;
        Some(format!(
            "file:{}/{}",
            percent_encode(base),
            percent_encode(file_name)
        ))
    }

    /// Claim a file name for `source_uri` (the model's URI, or a minted
    /// name for a bytes-only frame): the basename, disambiguated when a
    /// DIFFERENT source already took it.
    fn claim(&mut self, source_uri: &str, wanted: &str) -> String {
        if let Some(name) = self.by_source.get(source_uri) {
            return name.clone();
        }
        let (stem, ext) = match wanted.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (wanted.to_string(), String::new()),
        };
        let mut name = wanted.to_string();
        let mut n = 2;
        while self.taken.contains(&name) {
            name = format!("{stem}-{n}{ext}");
            n += 1;
        }
        self.taken.insert(name.clone());
        self.by_source.insert(source_uri.to_string(), name.clone());
        name
    }

    /// The URI to write for a frame linking `uri` (and holding `bytes`,
    /// maybe). Records the file when a base is set.
    fn uri_for_link(&mut self, uri: &str, bytes: Option<&[u8]>) -> String {
        if self.base.is_none() {
            return uri.to_string();
        }
        let file_name = self.claim(uri, &link_file_name(uri));
        let rebased = self.rebased_uri(&file_name).expect("base checked");
        match self.links.iter_mut().find(|l| l.file_name == file_name) {
            // A second frame on the same asset: keep the bytes if either
            // frame carries them.
            Some(existing) => {
                if existing.bytes.is_none() {
                    existing.bytes = bytes.map(|b| b.to_vec());
                }
            }
            None => self.links.push(ExportedLink {
                file_name,
                bytes: bytes.map(|b| b.to_vec()),
                source_uri: uri.to_string(),
            }),
        }
        rebased
    }

    /// The URI to write for a frame holding only bytes: a name minted
    /// from the frame id + the bytes' magic. `None` without a base, or
    /// when the bytes are in no format InDesign could open.
    fn uri_for_bytes(&mut self, host_id: &str, bytes: &[u8]) -> Option<String> {
        self.base.as_ref()?;
        let ext = sniff_image_extension(bytes)?;
        let wanted = format!("{}.{ext}", sanitize_file_stem(host_id));
        // Keyed by the minted name itself: a frame's bytes are their own
        // source.
        let key = format!("bytes:{host_id}");
        let file_name = self.claim(&key, &wanted);
        let rebased = self.rebased_uri(&file_name)?;
        self.links.push(ExportedLink {
            file_name,
            bytes: Some(bytes.to_vec()),
            source_uri: String::new(),
        });
        Some(rebased)
    }
}

/// The basename of a link URI as a file name on disk: the last path
/// segment, percent-decoded (InDesign writes `CODE%201.jpg` for the file
/// `CODE 1.jpg`). Shared with the loss ledger so both sides name the
/// same file.
pub fn link_file_name(uri: &str) -> String {
    let tail = uri.rsplit(['/', '\\']).next().unwrap_or(uri);
    percent_decode(tail)
}

/// Percent-encode a path for a `file:` URI the way InDesign spells it:
/// unreserved bytes (`A-Z a-z 0-9 - . _ ~`) plus the separators `/` and
/// `:` pass through; everything else (spaces, non-ASCII, `%` itself) is
/// `%XX` upper-case.
pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Undo [`percent_encode`]; a malformed `%` sequence is kept literally,
/// and a decoded byte string that is not UTF-8 falls back to the input.
fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// A frame id as a file stem: `/` (a `Rectangle/u12`-style id) → `_`,
/// like the story entry names.
fn sanitize_file_stem(id: &str) -> String {
    id.chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect()
}

/// The file extension for an encoded image, from its magic bytes —
/// the formats InDesign places. `None` for anything else.
pub fn sniff_image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some("tif")
    } else if bytes.starts_with(b"8BPS") {
        Some("psd")
    } else if bytes.starts_with(b"BM") && bytes.len() > 14 {
        Some("bmp")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if bytes.starts_with(b"%PDF") {
        Some("pdf")
    } else {
        None
    }
}

/// What the pass needs of a linked frame.
struct LinkSpec {
    /// The URI as it will be WRITTEN (rebased when a base is set).
    uri: String,
    transform: Option<[f32; 6]>,
    bounds: Bounds,
    space: Option<String>,
    /// The bytes to embed as `<Contents>` (see `LinkCollector::embed`).
    embed: Option<Vec<u8>>,
}

fn link_specs(spread: &Spread, links: &mut LinkCollector) -> HashMap<String, LinkSpec> {
    let mut out: HashMap<String, LinkSpec> = HashMap::new();
    let space_of = |id: &str| -> Option<String> {
        spread.image_metadata.get(id).and_then(|m| m.space.clone())
    };
    let mut add = |id: Option<&str>,
                   link: Option<&str>,
                   bytes: Option<&[u8]>,
                   transform: Option<[f32; 6]>,
                   bounds: Bounds| {
        let Some(id) = id else { return };
        let embed = if links.embed {
            bytes.map(|b| b.to_vec())
        } else {
            None
        };
        let uri = match (link, bytes) {
            (Some(uri), bytes) => links.uri_for_link(uri, bytes),
            (None, Some(bytes)) => match links.uri_for_bytes(id, bytes) {
                Some(uri) => uri,
                // No base to write the file under: an embedded picture
                // still names itself (InDesign shows the name in Links).
                None if embed.is_some() => match sniff_image_extension(bytes) {
                    Some(ext) => format!("{id}.{ext}"),
                    None => return,
                },
                None => return,
            },
            (None, None) => return,
        };
        out.insert(
            id.to_string(),
            LinkSpec {
                uri,
                transform,
                bounds,
                space: space_of(id),
                embed,
            },
        );
    };
    for r in &spread.rectangles {
        add(
            r.self_id.as_deref(),
            r.image_link.as_deref(),
            r.image_bytes.as_deref(),
            r.image_item_transform,
            r.bounds,
        );
    }
    for o in &spread.ovals {
        add(
            o.self_id.as_deref(),
            o.image_link.as_deref(),
            o.image_bytes.as_deref(),
            o.image_item_transform,
            o.bounds,
        );
    }
    for p in &spread.polygons {
        add(
            p.self_id.as_deref(),
            p.image_link.as_deref(),
            p.image_bytes.as_deref(),
            p.image_item_transform,
            p.bounds,
        );
    }
    out
}

/// InDesign's `LinkResourceFormat` name for an asset, by extension.
/// Every spelling here was read off an InDesign export in the corpus
/// (`$ID/Photoshop` for PSD, `$ID/WEBP ` WITH its trailing space, the
/// long PDF form) except `TIFF`, `GIF`, `Windows Bitmap`, `EPS` and
/// `SVG`, for which the corpus holds no placed file.
fn link_resource_format(uri: &str) -> &'static str {
    let ext = uri
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "$ID/JPEG",
        "png" => "$ID/Portable Network Graphics (PNG)",
        "tif" | "tiff" => "$ID/TIFF",
        "gif" => "$ID/GIF",
        "bmp" => "$ID/Windows Bitmap",
        "psd" => "$ID/Photoshop",
        "eps" => "$ID/EPS",
        "pdf" => "$ID/Adobe Portable Document Format (PDF)",
        "webp" => "$ID/WEBP ",
        "svg" => "$ID/SVG",
        _ => "$ID/",
    }
}

/// `<Contents>BASE64</Contents>` — InDesign's spelling of an embedded
/// picture's bytes, a typed child of the `<Image>`'s `<Properties>`
/// (the importer's Q-03 lane reads it back).
fn write_contents(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    bytes: &[u8],
) -> Result<(), quick_xml::Error> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    writer.write_event(Event::Start(BytesStart::new("Contents")))?;
    writer.write_event(Event::Text(quick_xml::events::BytesText::new(&encoded)))?;
    writer.write_event(Event::End(BytesEnd::new("Contents")))?;
    Ok(())
}

/// The frame's own `<Image>` block, in InDesign's spelling.
fn write_image(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    host_id: &str,
    spec: &LinkSpec,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![("Self", format!("{host_id}Image"))];
    if let Some(space) = &spec.space {
        attrs.push(("Space", space.clone()));
    }
    attrs.push((
        "ItemTransform",
        spec.transform
            .map(|m| format_matrix(&m))
            .unwrap_or_else(|| "1 0 0 1 0 0".to_string()),
    ));
    emit_start_with_attrs(writer, "Image", &attrs)?;
    writer.write_event(Event::Start(BytesStart::new("Properties")))?;
    // Without the pixel size (a link, not bytes) the picture box is the
    // frame's own box: with an identity transform the placeholder fills
    // the frame, which is what the model renders for an unresolved link.
    emit_empty_with_attrs(
        writer,
        "GraphicBounds",
        &[
            ("Left", format_f32(spec.bounds.left)),
            ("Top", format_f32(spec.bounds.top)),
            ("Right", format_f32(spec.bounds.right)),
            ("Bottom", format_f32(spec.bounds.bottom)),
        ],
    )?;
    if let Some(bytes) = &spec.embed {
        write_contents(writer, bytes)?;
    }
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    emit_empty_with_attrs(
        writer,
        "Link",
        &[
            ("Self", format!("{host_id}Link")),
            ("LinkResourceURI", spec.uri.clone()),
            (
                "LinkResourceFormat",
                link_resource_format(&spec.uri).to_string(),
            ),
            (
                "StoredState",
                if spec.embed.is_some() {
                    "Embedded"
                } else {
                    "Normal"
                }
                .to_string(),
            ),
            ("LinkClassID", "35906".to_string()),
            ("LinkClientID", "257".to_string()),
            ("LinkResourceModified", "false".to_string()),
            ("LinkObjectModified", "false".to_string()),
            ("ShowInUI", "true".to_string()),
            ("CanEmbed", "true".to_string()),
            ("CanUnembed", "true".to_string()),
            ("CanPackage", "true".to_string()),
            ("ImportPolicy", "NoAutoImport".to_string()),
            ("ExportPolicy", "NoAutoExport".to_string()),
            ("LinkResourceSize", "0~0".to_string()),
        ],
    )?;
    writer.write_event(Event::End(BytesEnd::new("Image")))?;
    Ok(())
}

fn is_host_name(name: &[u8]) -> bool {
    matches!(name, b"Rectangle" | b"Oval" | b"Polygon")
}

fn is_placed_name(name: &[u8]) -> bool {
    matches!(name, b"Image" | b"EPS" | b"PDF" | b"ImportedPage")
}

/// Patch a `<Link>` (or a placed-image element carrying the URI itself)
/// so `LinkResourceURI` names the model's asset; verbatim when it does.
fn patch_uri(
    e: &BytesStart,
    uri: &str,
    embedded: bool,
) -> Result<BytesStart<'static>, quick_xml::Error> {
    if attr_value(e, b"LinkResourceURI").is_none() {
        return Ok(e.clone().into_owned());
    }
    let rebuilt = patch_start(
        e,
        |k, raw| match k {
            b"LinkResourceURI" => Some(if raw == uri.as_bytes() {
                Patch::Keep
            } else {
                Patch::Set(uri.to_string())
            }),
            b"StoredState" if embedded => Some(if raw == b"Embedded" {
                Patch::Keep
            } else {
                Patch::Set("Embedded".to_string())
            }),
            _ => None,
        },
        &[],
    )?;
    Ok(
        if e.as_ref().trim_ascii_end() == rebuilt.as_ref().trim_ascii_end() {
            e.clone().into_owned()
        } else {
            rebuilt
        },
    )
}

/// Bring every linked frame's placed-image child in line with the model.
/// `links` accumulates the files the export's URIs point at (see
/// [`LinkCollector`]).
pub(crate) fn rewrite_images(
    original: &[u8],
    spread: &Spread,
    links: &mut LinkCollector,
) -> Result<Vec<u8>, quick_xml::Error> {
    let specs = link_specs(spread, links);
    if specs.is_empty() {
        return Ok(original.to_vec());
    }
    let mut reader = Reader::from_reader(original);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();
    let mut depth = 0usize;
    // The open linked host: (depth, id, spec, placed child seen).
    let mut open: Option<(usize, String, &LinkSpec, bool)> = None;
    // Whether the placed element's own `<Properties>` block already
    // carries a `<Contents>` child (an embedded picture in the source).
    let mut contents_seen = false;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                let name = e.name().as_ref().to_vec();
                if open.is_none() && is_host_name(&name) {
                    if let Some(id) = attr_value(&e, b"Self") {
                        if let Some(spec) = specs.get(id.as_str()) {
                            open = Some((depth, id, spec, false));
                            contents_seen = false;
                        }
                    }
                    writer.write_event(Event::Start(e.into_owned()))?;
                } else if let Some((d, _, spec, _)) = open.as_ref() {
                    if is_placed_name(&name) && depth == d + 1 {
                        let (d, id, spec, _) = open.take().expect("checked");
                        open = Some((d, id, spec, true));
                        writer.write_event(Event::Start(patch_uri(
                            &e,
                            &spec.uri,
                            spec.embed.is_some(),
                        )?))?;
                    } else if name == b"Link" && depth == d + 2 {
                        writer.write_event(Event::Start(patch_uri(
                            &e,
                            &spec.uri,
                            spec.embed.is_some(),
                        )?))?;
                    } else {
                        if name == b"Contents" && depth == d + 3 {
                            contents_seen = true;
                        }
                        writer.write_event(Event::Start(e.into_owned()))?;
                    }
                } else {
                    writer.write_event(Event::Start(e.into_owned()))?;
                }
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                if open.is_none() && is_host_name(&name) {
                    let spec = attr_value(&e, b"Self")
                        .and_then(|id| specs.get(id.as_str()).map(|s| (id, s)));
                    match spec {
                        Some((id, spec)) => {
                            // A self-closing frame: expand it around its image.
                            writer.write_event(Event::Start(e.borrow()))?;
                            write_image(&mut writer, &id, spec)?;
                            writer.write_event(Event::End(BytesEnd::new(
                                String::from_utf8_lossy(&name).into_owned(),
                            )))?;
                        }
                        None => writer.write_event(Event::Empty(e.into_owned()))?,
                    }
                } else if let Some((d, _, spec, _)) = open.as_ref() {
                    if is_placed_name(&name) && depth + 1 == d + 1 {
                        let (d, id, spec, _) = open.take().expect("checked");
                        open = Some((d, id, spec, true));
                        writer.write_event(Event::Empty(patch_uri(
                            &e,
                            &spec.uri,
                            spec.embed.is_some(),
                        )?))?;
                    } else if name == b"Link" && depth + 1 == d + 2 {
                        writer.write_event(Event::Empty(patch_uri(
                            &e,
                            &spec.uri,
                            spec.embed.is_some(),
                        )?))?;
                    } else {
                        if name == b"Contents" && depth + 1 == d + 3 {
                            contents_seen = true;
                        }
                        writer.write_event(Event::Empty(e.into_owned()))?;
                    }
                } else {
                    writer.write_event(Event::Empty(e.into_owned()))?;
                }
            }
            Event::End(e) => {
                if let Some((d, id, spec, seen)) = open.as_ref() {
                    // The placed element's `<Properties>` closes without
                    // a `<Contents>`: the bytes to embed go in first.
                    if *seen
                        && depth == d + 2
                        && e.name().as_ref() == b"Properties"
                        && !contents_seen
                    {
                        if let Some(bytes) = &spec.embed {
                            write_contents(&mut writer, bytes)?;
                            contents_seen = true;
                        }
                    }
                    if *d == depth && is_host_name(e.name().as_ref()) {
                        if !seen {
                            write_image(&mut writer, id, spec)?;
                        }
                        open = None;
                    }
                }
                depth = depth.saturating_sub(1);
                writer.write_event(Event::End(e))?;
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

    #[test]
    fn percent_encoding_matches_indesigns_spelling() {
        assert_eq!(
            percent_encode("/Users/x/Line Sheet Template/Links"),
            "/Users/x/Line%20Sheet%20Template/Links"
        );
        assert_eq!(
            percent_encode("C:/Dropbox/a-b_c.d~e"),
            "C:/Dropbox/a-b_c.d~e"
        );
        assert_eq!(
            percent_encode("Grüße 100%.png"),
            "Gr%C3%BC%C3%9Fe%20100%25.png"
        );
        assert_eq!(percent_decode("CODE%201A.jpg"), "CODE 1A.jpg");
        assert_eq!(percent_decode("100%.png"), "100%.png");
        assert_eq!(percent_decode("Gr%C3%BC%C3%9Fe.png"), "Grüße.png");
    }

    #[test]
    fn link_file_name_is_the_decoded_basename() {
        assert_eq!(
            link_file_name("file:C:/Users/SDR1/Desktop/Links/CODE%201.jpg"),
            "CODE 1.jpg"
        );
        assert_eq!(link_file_name("assets/photos/apples.jpg"), "apples.jpg");
        assert_eq!(link_file_name("apples.jpg"), "apples.jpg");
    }

    #[test]
    fn the_collector_dedupes_and_disambiguates() {
        let mut c = LinkCollector::new(Some("/tmp/Links/"), false);
        let a = c.uri_for_link("assets/x/logo.png", None);
        let a2 = c.uri_for_link("assets/x/logo.png", Some(&[1, 2]));
        let b = c.uri_for_link("assets/y/logo.png", None);
        assert_eq!(a, "file:/tmp/Links/logo.png");
        assert_eq!(a2, a, "the same asset keeps its file");
        assert_eq!(b, "file:/tmp/Links/logo-2.png");
        let links = c.into_links();
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].file_name, "logo.png");
        assert_eq!(
            links[0].bytes.as_deref(),
            Some(&[1u8, 2][..]),
            "bytes from the second frame on the same asset are kept"
        );
        assert_eq!(links[0].source_uri, "assets/x/logo.png");
        assert_eq!(links[1].file_name, "logo-2.png");
        assert_eq!(links[1].source_uri, "assets/y/logo.png");
    }

    #[test]
    fn bytes_only_frames_get_a_name_from_their_magic() {
        let mut c = LinkCollector::new(Some("/tmp/Links"), false);
        let png = b"\x89PNG\r\n\x1a\n....".to_vec();
        assert_eq!(
            c.uri_for_bytes("Rectangle/u12", &png).as_deref(),
            Some("file:/tmp/Links/Rectangle_u12.png")
        );
        assert_eq!(c.uri_for_bytes("u13", b"not an image"), None);
        let links = c.into_links();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].file_name, "Rectangle_u12.png");
        assert_eq!(links[0].bytes.as_deref(), Some(&png[..]));
        assert_eq!(links[0].source_uri, "");
    }

    #[test]
    fn without_a_base_nothing_is_rebased_or_collected() {
        let mut c = LinkCollector::new(None, false);
        assert_eq!(c.uri_for_link("assets/a.png", Some(&[1])), "assets/a.png");
        assert_eq!(c.uri_for_bytes("u1", b"\x89PNG\r\n\x1a\n"), None);
        assert!(c.into_links().is_empty());
    }

    #[test]
    fn sniffing_covers_the_formats_indesign_places() {
        assert_eq!(sniff_image_extension(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(
            sniff_image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("jpg")
        );
        assert_eq!(sniff_image_extension(b"8BPS\0\x01"), Some("psd"));
        assert_eq!(sniff_image_extension(b"II*\0"), Some("tif"));
        assert_eq!(sniff_image_extension(b"MM\0*"), Some("tif"));
        assert_eq!(sniff_image_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_image_extension(b"GIF89a"), Some("gif"));
        assert_eq!(sniff_image_extension(b"%PDF-1.7"), Some("pdf"));
        assert_eq!(sniff_image_extension(b"hello"), None);
    }
}
