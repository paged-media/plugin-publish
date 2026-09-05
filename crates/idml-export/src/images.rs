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
//! element carries no placed-image child.
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
//! The URI is written as the model holds it. A relative `assets/…` path
//! stays relative: the document carries no asset base to resolve it
//! against (the renderer's `AssetResolver` is a runtime concern, not
//! part of the model).
//!
//! Runs AFTER `rewrite_spread`, over its output, so a frame the model
//! INSERTED and a source frame are treated alike. A frame that ALSO
//! holds decoded bytes (`image_bytes` — the editor keeps the pixels of a
//! placed asset, and may have edited them) still gets its `<Link>`: the
//! asset is what IDML can name, and InDesign re-reads it; the pixels
//! themselves stay a measured loss (IDML cannot embed them). A frame
//! whose element already carries a placed-image child keeps it, with
//! the `<Link>`'s URI patched when the model's differs.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

use idml_import::{Bounds, Spread};

use crate::rewrite::{
    attr_value, emit_empty_with_attrs, emit_start_with_attrs, format_f32, format_matrix,
    patch_start, Patch,
};

/// What the pass needs of a linked frame.
struct LinkSpec<'a> {
    uri: &'a str,
    transform: Option<[f32; 6]>,
    bounds: Bounds,
    space: Option<&'a str>,
}

fn link_specs(spread: &Spread) -> HashMap<&str, LinkSpec<'_>> {
    let mut out: HashMap<&str, LinkSpec<'_>> = HashMap::new();
    let space_of = |id: &str| -> Option<&str> {
        spread
            .image_metadata
            .get(id)
            .and_then(|m| m.space.as_deref())
    };
    for r in &spread.rectangles {
        if let (Some(id), Some(uri)) = (r.self_id.as_deref(), r.image_link.as_deref()) {
            out.insert(
                id,
                LinkSpec {
                    uri,
                    transform: r.image_item_transform,
                    bounds: r.bounds,
                    space: space_of(id),
                },
            );
        }
    }
    for o in &spread.ovals {
        if let (Some(id), Some(uri)) = (o.self_id.as_deref(), o.image_link.as_deref()) {
            out.insert(
                id,
                LinkSpec {
                    uri,
                    transform: o.image_item_transform,
                    bounds: o.bounds,
                    space: space_of(id),
                },
            );
        }
    }
    for p in &spread.polygons {
        if let (Some(id), Some(uri)) = (p.self_id.as_deref(), p.image_link.as_deref()) {
            out.insert(
                id,
                LinkSpec {
                    uri,
                    transform: p.image_item_transform,
                    bounds: p.bounds,
                    space: space_of(id),
                },
            );
        }
    }
    out
}

/// InDesign's `LinkResourceFormat` name for an asset, by extension.
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
        "pdf" => "$ID/Adobe PDF",
        "svg" => "$ID/SVG",
        _ => "$ID/",
    }
}

/// The frame's own `<Image>` block, in InDesign's spelling.
fn write_image(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    host_id: &str,
    spec: &LinkSpec<'_>,
) -> Result<(), quick_xml::Error> {
    let mut attrs: Vec<(&str, String)> = vec![("Self", format!("{host_id}Image"))];
    if let Some(space) = spec.space {
        attrs.push(("Space", space.to_string()));
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
    writer.write_event(Event::End(BytesEnd::new("Properties")))?;
    emit_empty_with_attrs(
        writer,
        "Link",
        &[
            ("Self", format!("{host_id}Link")),
            ("LinkResourceURI", spec.uri.to_string()),
            (
                "LinkResourceFormat",
                link_resource_format(spec.uri).to_string(),
            ),
            ("StoredState", "Normal".to_string()),
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
fn patch_uri(e: &BytesStart, uri: &str) -> Result<BytesStart<'static>, quick_xml::Error> {
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
pub fn rewrite_images(original: &[u8], spread: &Spread) -> Result<Vec<u8>, quick_xml::Error> {
    let specs = link_specs(spread);
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
    let mut open: Option<(usize, String, &LinkSpec<'_>, bool)> = None;
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
                        }
                    }
                    writer.write_event(Event::Start(e.into_owned()))?;
                } else if let Some((d, _, spec, _)) = open.as_ref() {
                    if is_placed_name(&name) && depth == d + 1 {
                        let (d, id, spec, _) = open.take().expect("checked");
                        open = Some((d, id, spec, true));
                        writer.write_event(Event::Start(patch_uri(&e, spec.uri)?))?;
                    } else if name == b"Link" && depth == d + 2 {
                        writer.write_event(Event::Start(patch_uri(&e, spec.uri)?))?;
                    } else {
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
                        writer.write_event(Event::Empty(patch_uri(&e, spec.uri)?))?;
                    } else if name == b"Link" && depth + 1 == d + 2 {
                        writer.write_event(Event::Empty(patch_uri(&e, spec.uri)?))?;
                    } else {
                        writer.write_event(Event::Empty(e.into_owned()))?;
                    }
                } else {
                    writer.write_event(Event::Empty(e.into_owned()))?;
                }
            }
            Event::End(e) => {
                if let Some((d, id, spec, seen)) = open.as_ref() {
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
