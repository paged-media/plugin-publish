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

//! IDML re-serialization — the save-back foundation (W3.B1).
//!
//! Turns a (possibly mutated) [`paged_scene::Document`] back into a
//! valid IDML package so an edited document can be saved.
//!
//! # Strategy: carry-through fidelity
//!
//! The parser keeps a *subset* of every entry's attributes; most entries
//! (fonts, preferences, tags, metadata, the XML backing store) are not
//! modeled at all. Regenerating those from the model would silently drop
//! everything the parser didn't read. So this writer does NOT regenerate
//! the package from scratch. Instead it copies the original package
//! verbatim and **patches only what the model can faithfully express**:
//!
//! * **Pass-through (byte-identical).** Every entry except the changed
//!   Spreads / MasterSpreads / Stories is copied straight out of the
//!   source ZIP with its original compressed bytes (via
//!   [`zip::write::ZipWriter::raw_copy_file`]), so `mimetype` stays
//!   first + stored and untouched entries round-trip bit-for-bit.
//!   The `.paged` CONTAINER parts (`manifest.json`, `paged/**`) are the
//!   one exception: [`write_idml`] drops them so its product is a pure
//!   IDML package; only [`write_paged`] carries them (see
//!   [`write_package`]).
//! * **Patched (streaming rewrite).** `Spreads/*.xml`,
//!   `MasterSpreads/*.xml` and `Stories/*.xml`
//!   are rewritten with a quick-xml reader→writer pass that copies the
//!   original token stream and overwrites only the attributes / text the
//!   model owns (see [`rewrite`]). Unknown attributes, child elements,
//!   `<Properties>`, processing instructions, and comments pass through
//!   untouched. When the rewrite produces bytes identical to the source
//!   (the document wasn't mutated in that entry), the entry is copied
//!   verbatim instead — so an unmutated round-trip is byte-identical
//!   across the *whole* package.
//!
//! # API shape
//!
//! [`write_idml`] takes `(&Document, original_bytes)` rather than reading
//! the source package off `Document` — even though `Document` *does*
//! retain the original entries (`Document.source`'s entries). Taking the
//! original bytes explicitly keeps the ZIP container structure (entry
//! order, compression, the stored-mimetype rule, local-header layout)
//! available for a faithful re-zip, which the decompressed entry map
//! alone can't reconstruct. No parse-side change is needed.
//!
//! # What is save-able (patch list)
//!
//! The patch surface is the intersection of (a) attributes the parser
//! round-trips onto the model and (b) the page-item / story properties
//! the mutation layer (`paged_mutate::PropertyPath`) can change. On top
//! of that property-patch foundation, W1.15 adds STRUCTURAL save-back:
//! page-item inserts / removes within a spread, new swatches / gradients
//! / styles injected into the Resources entries (see [`resources`]),
//! table-cell text + style edits, and group-member transforms. C-8 adds
//! NEW-ENTRY emission: a story minted post-parse (InsertTextFrame's
//! `parent_story`, `src: ""`) and a spread minted by `InsertPage` are
//! serialised as full parts and referenced from `designmap.xml` (see
//! [`emit`]). See [`rewrite`] for the per-element inventory and the
//! documented losses (removed PAGES still leave an orphaned entry).

use std::collections::HashSet;
use std::io::{Cursor, Read, Write};

use paged_scene::{Document, ParsedStory};

pub mod based_on;
pub mod cell_insets;
pub mod dangling;
mod emit;
pub mod face;
pub mod fonts;
pub mod guides;
pub mod images;
mod navigation;
mod paged;
pub mod preferences;
mod reorder;
pub mod resources;
pub mod rewrite;
pub mod text_frame_prefs;
pub mod text_wrap;

/// The `Self` of the one `<CrossReferenceFormat>` the exporter emits for
/// cross-reference sources that name none (InDesign drops those).
pub const XREF_FORMAT_ID: &str = "paged-xref-format";

pub use fonts::FontFace;
pub use paged::{idml_parts_hash, is_container_part, write_paged, MANIFEST_NAME, PAGED_PREFIX};

/// What a caller can ask of [`write_idml_with`] beyond the bytes.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// An ABSOLUTE directory the caller will write the returned
    /// [`ExportedLink`]s into. When set, every placed image's
    /// `LinkResourceURI` is rewritten to `file:<link_base>/<basename>`
    /// in InDesign's own spelling so the links resolve on the host that
    /// opens the file — see [`images`]. `None` writes each URI as the
    /// model holds it (a relative `assets/…` path stays relative, which
    /// InDesign cannot resolve) and returns no links.
    pub link_base: Option<String>,
    /// Faces the host knows from font BYTES (its registry, one per
    /// variable-font named instance): a face the document applies is
    /// declared in `Resources/Fonts.xml` with these names and design
    /// axes when listed here — see [`fonts`] — and with synthesised
    /// names otherwise.
    pub fonts: Vec<FontFace>,
    /// The face the engine composes text with when nothing in a run's
    /// cascade names one — the host's fallback font. When given, it is
    /// stated as the document's `<TextDefault>` (see [`preferences`])
    /// and declared in `Resources/Fonts.xml`, so InDesign sets unstyled
    /// text the way the engine did instead of in its own application
    /// default (Minion Pro 12 pt — 4,449 characters of the annual,
    /// measured 2026-09-05).
    pub default_face: Option<FontFace>,
}

/// One file the export's image links point at. The caller writes
/// `bytes` (when present) as `<link_base>/<file_name>`, or copies the
/// file `source_uri` names there — then every `<Link>` in the package
/// resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedLink {
    /// The basename the link was written with (percent-DEcoded: the
    /// name on disk, `CODE 1.jpg` for a URI ending `CODE%201.jpg`).
    pub file_name: String,
    /// The image's encoded file bytes when the model holds them
    /// (`image_bytes`); `None` for a link-only frame, whose file the
    /// caller copies from `source_uri`.
    pub bytes: Option<Vec<u8>>,
    /// The URI the model holds for the asset — empty for an image that
    /// exists only as bytes (its name was minted from the frame id).
    pub source_uri: String,
}

/// The product of [`write_idml_with`].
#[derive(Debug, Clone)]
pub struct IdmlExport {
    /// The `.idml` package.
    pub bytes: Vec<u8>,
    /// The files the package's image links point at — empty unless
    /// [`ExportOptions::link_base`] was given.
    pub links: Vec<ExportedLink>,
}

/// Errors raised while re-serializing a document.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("source package is not a readable ZIP: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("i/o while assembling the package: {0}")]
    Io(#[from] std::io::Error),
    #[error("xml rewrite of {entry}: {source}")]
    Rewrite {
        entry: String,
        #[source]
        source: quick_xml::Error,
    },
}

/// Re-serialize `doc` back into an IDML package, carrying through the
/// untouched bytes of `original` and patching only the model-owned
/// attributes of the Spreads / Stories.
///
/// `original` must be the IDML byte stream `doc` was parsed from (or one
/// structurally equivalent to it — same entries, same `Self` ids). The
/// returned `Vec<u8>` is a valid `.idml` package: `mimetype` first +
/// stored, every other source entry preserved, the Spreads /
/// MasterSpreads / Stories reflecting the current model state.
///
/// An unmutated document round-trips byte-identically, with two
/// deliberate exceptions measured against InDesign 20.0.1: a story no
/// frame references is dropped with its designmap reference (InDesign
/// discards it — see [`placed_story_ids`]), and `Resources/Fonts.xml`
/// gains any face the document applies that it does not declare
/// (undeclared faces come back NOT_AVAILABLE — see [`fonts`]). A
/// package that places every story and declares every face — every
/// InDesign-authored one — is untouched. A mutated document differs
/// only in the Spreads / MasterSpreads / Stories whose model the
/// mutation touched.
/// Serialise one minted story part — the exact bytes [`write_idml`]
/// would add for a story the model minted. Exposed so the paragraph-mark
/// contract can be pinned directly (see
/// `tests/paragraph_terminator.rs`) instead of only through a full
/// package write.
pub fn emit_story_part_for_test(
    self_id: &str,
    story: &idml_import::Story,
) -> Result<Vec<u8>, quick_xml::Error> {
    emit::story_part(&emit::sanitize_id(self_id), story, "20.0", &[], None)
}

pub fn write_idml(doc: &Document, original: &[u8]) -> Result<Vec<u8>, WriteError> {
    write_idml_with(doc, original, &ExportOptions::default()).map(|e| e.bytes)
}

/// [`write_idml`] with [`ExportOptions`]: a link base to rebase every
/// placed image's URI under (and the files to write there come back in
/// the result), and the host's byte-derived faces for `Fonts.xml`.
pub fn write_idml_with(
    doc: &Document,
    original: &[u8],
    opts: &ExportOptions,
) -> Result<IdmlExport, WriteError> {
    write_package(doc, original, false, opts)
}

/// The story ids some frame REFERENCES: every `TextFrame`'s
/// `ParentStory` and every `TextPath`'s on every spread and master, plus
/// — transitively — the stories anchored frames inside those stories
/// host. A story outside this set is an ORPHAN: nothing places it, and
/// InDesign discards it on open (measured on 20.0.1 — a book whose
/// engine had kept a story after its frame was deleted opened with one
/// story fewer than the package carried). Ids are returned as the
/// frames spell them; use [`story_is_placed`] to test a model story,
/// which also tries the sanitized entry-stem spelling a minted story
/// takes on reopen.
pub fn placed_story_ids(doc: &Document) -> HashSet<String> {
    use idml_import::{AnchoredFrame, Paragraph, Spread};

    let mut placed: HashSet<String> = HashSet::new();
    // Both spellings of every reference, so a frame minted with the
    // model id and a story reopened under the sanitized entry stem
    // (or the reverse) still find each other.
    fn insert(placed: &mut HashSet<String>, id: &str) {
        placed.insert(emit::sanitize_id(id));
        placed.insert(id.to_string());
    }
    fn seed(spread: &Spread, placed: &mut HashSet<String>) {
        for f in &spread.text_frames {
            if let Some(id) = &f.parent_story {
                insert(placed, id);
            }
        }
        for r in &spread.rectangles {
            for tp in &r.text_paths {
                insert(placed, &tp.parent_story);
            }
        }
        for p in &spread.polygons {
            for tp in &p.text_paths {
                insert(placed, &tp.parent_story);
            }
        }
        for l in &spread.graphic_lines {
            for tp in &l.text_paths {
                insert(placed, &tp.parent_story);
            }
        }
    }
    for s in &doc.spreads {
        seed(&s.spread, &mut placed);
    }
    for m in doc.master_spreads.values() {
        seed(&m.spread, &mut placed);
    }
    // Anchored frames host stories of their own; a story reachable only
    // through one is placed by transitivity. Iterate to a fixpoint.
    fn anchored(frames: &[AnchoredFrame], out: &mut Vec<String>) {
        for f in frames {
            if let Some(id) = &f.parent_story {
                out.push(id.clone());
            }
            anchored(&f.children, out);
        }
    }
    fn hosted(paras: &[Paragraph], out: &mut Vec<String>) {
        for p in paras {
            anchored(&p.anchored_frames, out);
            if let Some(t) = &p.table {
                for c in &t.cells {
                    hosted(&c.paragraphs, out);
                }
            }
            for f in &p.footnotes {
                hosted(&f.paragraphs, out);
            }
        }
    }
    let mut visited: HashSet<String> = HashSet::new();
    loop {
        let mut found: Vec<String> = Vec::new();
        for s in &doc.stories {
            if !story_is_placed(&placed, &s.self_id) || !visited.insert(s.self_id.clone()) {
                continue;
            }
            hosted(&s.story.paragraphs, &mut found);
        }
        let before = placed.len();
        for id in &found {
            insert(&mut placed, id);
        }
        if placed.len() == before {
            break;
        }
    }
    placed
}

/// Whether the story `id` is in `placed` (see [`placed_story_ids`]),
/// under either its own spelling or the sanitized entry-stem spelling
/// (`Story/u12` ⇄ `Story_u12`) a frame may reference it by.
pub fn story_is_placed(placed: &HashSet<String>, id: &str) -> bool {
    placed.contains(id) || placed.contains(&emit::sanitize_id(id))
}

/// The one writer both [`write_idml`] and [`write_paged`] run. The only
/// thing that differs between the two products is whether the CONTAINER
/// parts of `original` — `manifest.json` and everything under `paged/`
/// — survive the copy-through walk:
///
/// * `keep_container_parts = false` ⇒ a PURE `.idml`: only IDML parts
///   (`mimetype`, `designmap.xml`, `META-INF/`, `Resources/`, `XML/`,
///   `MasterSpreads/`, `Spreads/`, `Stories/`, and any other entry that
///   is not a container part — the same definition [`idml_parts_hash`]
///   uses). This closes the "an `.idml` exported from a loaded `.paged`
///   is the container under another name" gap: the engine's load sniff
///   prefers a carried-through `document.pgm` over the IDML parts, so a
///   parity gate that re-opened such an export compared the model with
///   itself and reported zero differing pages. A source with no
///   container parts is unaffected (still byte-identical when unmutated).
/// * `keep_container_parts = true` ⇒ the `.paged` lane, where the
///   carried-through plugin parts + manifest are the whole point.
pub(crate) fn write_package(
    doc: &Document,
    original: &[u8],
    keep_container_parts: bool,
    opts: &ExportOptions,
) -> Result<IdmlExport, WriteError> {
    let mut src = zip::ZipArchive::new(Cursor::new(original))?;
    // The files every placed image's `<Link>` will point at (empty
    // without a link base — see `images`).
    // A pure `.idml` with no link base embeds every picture's bytes
    // (see `images::LinkCollector::embed`); a `.paged` write never does.
    let mut links = images::LinkCollector::new(
        opts.link_base.as_deref(),
        !keep_container_parts && opts.link_base.is_none(),
    );
    let out = Cursor::new(Vec::<u8>::new());
    let mut zip = zip::write::ZipWriter::new(out);

    // Pre-build the patched bodies, keyed by entry path. Only entries
    // whose rewrite differs from the source land here; an entry that
    // rewrites identically is dropped so it takes the verbatim path
    // below (preserving byte-identity + original compression).
    let mut patched: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();

    // C-8 — new-entry emission. A spread / story the model carries but
    // the source archive doesn't (a spread minted by `InsertPage`, a
    // story minted by InsertTextFrame's `parent_story` with `src: ""`)
    // is serialised as a FULL part and appended to the package, then
    // referenced from `designmap.xml`. An unmutated document mints
    // nothing, so none of this fires and the round-trip stays
    // byte-identical.
    let dom_version = doc
        .designmap
        .dom_version
        .clone()
        .unwrap_or_else(|| "20.0".to_string());
    let mut new_entries: Vec<(String, Vec<u8>)> = Vec::new();
    // `(anchor src, new src)` — the anchor is the nearest PRECEDING
    // spread with a source entry, so the designmap ref (whose order is
    // page order) lands next to its host.
    let mut new_spread_refs: Vec<(Option<String>, String)> = Vec::new();
    let mut new_story_srcs: Vec<String> = Vec::new();
    // The layer new guides bind to (`ItemLayer`): the document's first.
    let default_layer: Option<&str> = doc.designmap.layers.first().map(|l| l.self_id.as_str());

    for (i, spread) in doc.spreads.iter().enumerate() {
        if let Some(orig) = entry_bytes(&mut src, &spread.src)? {
            // Auto-sizing prefs and guides first (their own passes), then
            // the page-item rewrite over the patched bytes.
            let with_prefs = text_frame_prefs::rewrite_text_frame_prefs(&orig, &spread.spread)
                .map_err(|source| WriteError::Rewrite {
                    entry: spread.src.clone(),
                    source,
                })?;
            let with_guides = guides::rewrite_guides(
                &with_prefs,
                &spread.spread.guides,
                spread.spread.self_id.as_deref(),
                default_layer,
            )
            .map_err(|source| WriteError::Rewrite {
                entry: spread.src.clone(),
                source,
            })?;
            let rewritten =
                rewrite::rewrite_spread(&with_guides, &spread.spread).map_err(|source| {
                    WriteError::Rewrite {
                        entry: spread.src.clone(),
                        source,
                    }
                })?;
            // Linked images last, over the rewritten bytes, so inserted
            // frames get theirs too.
            let new = images::rewrite_images(&rewritten, &spread.spread, &mut links).map_err(
                |source| WriteError::Rewrite {
                    entry: spread.src.clone(),
                    source,
                },
            )?;
            // … and every wrap the model composes around (see [`text_wrap`]).
            let new = text_wrap::rewrite_text_wrap(&new, &spread.spread).map_err(|source| {
                WriteError::Rewrite {
                    entry: spread.src.clone(),
                    source,
                }
            })?;
            if new != orig.as_slice() {
                patched.insert(spread.src.clone(), new);
            }
        } else if !spread.src.is_empty() {
            let body = emit::spread_part(&spread.spread, &dom_version, default_layer).map_err(
                |source| WriteError::Rewrite {
                    entry: spread.src.clone(),
                    source,
                },
            )?;
            let body =
                images::rewrite_images(&body, &spread.spread, &mut links).map_err(|source| {
                    WriteError::Rewrite {
                        entry: spread.src.clone(),
                        source,
                    }
                })?;
            let body = text_wrap::rewrite_text_wrap(&body, &spread.spread).map_err(|source| {
                WriteError::Rewrite {
                    entry: spread.src.clone(),
                    source,
                }
            })?;
            let anchor = doc.spreads[..i]
                .iter()
                .rev()
                .find(|prev| src.by_name(&prev.src).is_ok())
                .map(|prev| prev.src.clone());
            new_spread_refs.push((anchor, spread.src.clone()));
            new_entries.push((spread.src.clone(), body));
        }
    }
    // MASTER spreads. They parse with the same `parse_spread` into
    // `Document::master_spreads` and rewrite with the same
    // `rewrite_spread` — but until this loop existed the writer iterated
    // `doc.spreads` only, so every master part took the verbatim copy
    // path below. That made byte-identity trivially safe and made an
    // EDIT TO A MASTER PAGE VANISH: the user changed a master, saved,
    // reopened, and the change was gone with no error anywhere.
    //
    // A master with no source entry is NOT minted here. `emit` mints
    // spreads and stories because `InsertPage` / `InsertTextFrame` can
    // create them; nothing creates a master spread today, so a model
    // master the archive doesn't carry would be a shape this writer has
    // never seen. Skipping it keeps that an untouched hole rather than
    // an untested emitter.
    //
    // Iterated in `src` order because `master_spreads` is a `HashMap`
    // and a save must not depend on hash iteration order.
    let mut masters: Vec<&paged_scene::ParsedMasterSpread> = doc.master_spreads.values().collect();
    masters.sort_by(|a, b| a.src.cmp(&b.src));
    for master in masters {
        if let Some(orig) = entry_bytes(&mut src, &master.src)? {
            let with_prefs = text_frame_prefs::rewrite_text_frame_prefs(&orig, &master.spread)
                .map_err(|source| WriteError::Rewrite {
                    entry: master.src.clone(),
                    source,
                })?;
            let with_guides = guides::rewrite_guides(
                &with_prefs,
                &master.spread.guides,
                master.spread.self_id.as_deref(),
                default_layer,
            )
            .map_err(|source| WriteError::Rewrite {
                entry: master.src.clone(),
                source,
            })?;
            let rewritten =
                rewrite::rewrite_spread(&with_guides, &master.spread).map_err(|source| {
                    WriteError::Rewrite {
                        entry: master.src.clone(),
                        source,
                    }
                })?;
            let new = images::rewrite_images(&rewritten, &master.spread, &mut links).map_err(
                |source| WriteError::Rewrite {
                    entry: master.src.clone(),
                    source,
                },
            )?;
            let new = text_wrap::rewrite_text_wrap(&new, &master.spread).map_err(|source| {
                WriteError::Rewrite {
                    entry: master.src.clone(),
                    source,
                }
            })?;
            if new != orig.as_slice() {
                patched.insert(master.src.clone(), new);
            }
        }
    }
    // The text destinations (`TextAnchor`) each story must carry as an
    // inline marker — InDesign's spelling; the designmap spelling binds
    // nothing (measured). Keyed by story id, matching either the model's
    // id or its sanitized entry-stem form.
    let anchors_for = |story: &paged_scene::ParsedStory| -> Vec<(String, Option<String>)> {
        let sanitized = emit::sanitize_id(&story.self_id);
        doc.designmap
            .hyperlink_destinations
            .iter()
            .filter_map(|d| match &d.kind {
                idml_import::HyperlinkDestinationKind::TextAnchor(target)
                    if *target == story.self_id || *target == sanitized =>
                {
                    Some((d.self_id.clone(), None))
                }
                _ => None,
            })
            .collect()
    };
    // The inner width of the text column a story flows in — the column
    // fallback for a table the model never sized (`emit::write_table`).
    // `frame_for_story` is keyed by the parsed story id; a minted story's
    // frame names it by the model id, so both spellings are tried.
    let host_width_for = |story: &paged_scene::ParsedStory| -> Option<f32> {
        let frame = doc
            .frame_for_story
            .get(&story.self_id)
            .or_else(|| doc.frame_for_story.get(&emit::sanitize_id(&story.self_id)))?;
        let insets = frame.inset_spacing.unwrap_or([0.0; 4]);
        let inner = frame.bounds.right - frame.bounds.left - insets[1] - insets[3];
        let columns = frame.column_count.unwrap_or(1).max(1) as f32;
        let gutter = frame.column_gutter.unwrap_or(12.0);
        let width = (inner - gutter * (columns - 1.0)) / columns;
        (width > 0.0).then_some(width)
    };
    // Whether any carried-through story has a cross-reference source
    // with no format (InDesign drops it); the designmap then gains the
    // exporter's one format and the source is pointed at it.
    let mut needs_xref_format = false;
    // Orphan stories — referenced by no frame on any spread or master
    // (nor by an anchored frame inside a placed story). InDesign
    // discards them on open (measured), so writing their parts only
    // makes the package's story count disagree with what InDesign
    // shows; the engine keeps one after its frame is deleted. Neither
    // the part nor its designmap reference is written; the loss ledger
    // names each one so the drop is loud. A source whose every story is
    // placed is unaffected (byte-identical when unmutated).
    let placed = placed_story_ids(doc);
    let mut dropped_story_srcs: Vec<String> = Vec::new();
    for story in &doc.stories {
        if !story_is_placed(&placed, &story.self_id) {
            dropped_story_srcs.push(story_entry_src(story));
            continue;
        }
        if let Some(orig) = entry_bytes(&mut src, &story.src)? {
            let anchors = anchors_for(story);
            let unformatted =
                rewrite::unformatted_xref_sources(&orig).map_err(|source| WriteError::Rewrite {
                    entry: story.src.clone(),
                    source,
                })?;
            needs_xref_format |= unformatted > 0;
            let injected = rewrite::inject_story_navigation(&orig, &anchors, Some(XREF_FORMAT_ID))
                .map_err(|source| WriteError::Rewrite {
                    entry: story.src.clone(),
                    source,
                })?;
            let new =
                rewrite::rewrite_story_in_frame(&injected, &story.story, host_width_for(story))
                    .map_err(|source| WriteError::Rewrite {
                        entry: story.src.clone(),
                        source,
                    })?;
            let new = face::patch_story_face(&new, &doc.styles).map_err(|source| {
                WriteError::Rewrite {
                    entry: story.src.clone(),
                    source,
                }
            })?;
            let new = dangling::drop_dangling_style_refs(&new, &doc.styles).map_err(|source| {
                WriteError::Rewrite {
                    entry: story.src.clone(),
                    source,
                }
            })?;
            let new =
                cell_insets::spell_cell_insets(&new, &cell_insets::cell_insets_of(&story.story))
                    .map_err(|source| WriteError::Rewrite {
                        entry: story.src.clone(),
                        source,
                    })?;
            if new != orig.as_slice() {
                patched.insert(story.src.clone(), new);
            }
        } else {
            // Minted post-parse (`src: ""`); derive the entry name from
            // the `Self` id (`/` → `_` — `derive_story_id` re-derives the
            // sanitized id from this stem on reopen).
            let entry_src = if story.src.is_empty() {
                emit::story_src_for(&story.self_id)
            } else {
                story.src.clone()
            };
            if let Some(orig) = entry_bytes(&mut src, &entry_src)? {
                // The derived entry already exists: a `.paged` checkpoint
                // wrote this minted story's part, and the reloaded model
                // (`document.pgm`) still says `src: ""`. This used to be
                // read as a name collision and the story was SKIPPED —
                // the checkpoint's part rode through verbatim, stale text
                // and all, and every table the model added after the
                // checkpoint was lost (16 of 16 in the annual). It is the
                // story's source part: patch it like any other.
                let anchors = anchors_for(story);
                let unformatted = rewrite::unformatted_xref_sources(&orig).map_err(|source| {
                    WriteError::Rewrite {
                        entry: entry_src.clone(),
                        source,
                    }
                })?;
                needs_xref_format |= unformatted > 0;
                let injected =
                    rewrite::inject_story_navigation(&orig, &anchors, Some(XREF_FORMAT_ID))
                        .map_err(|source| WriteError::Rewrite {
                            entry: entry_src.clone(),
                            source,
                        })?;
                let new =
                    rewrite::rewrite_story_in_frame(&injected, &story.story, host_width_for(story))
                        .map_err(|source| WriteError::Rewrite {
                            entry: entry_src.clone(),
                            source,
                        })?;
                let new = face::patch_story_face(&new, &doc.styles).map_err(|source| {
                    WriteError::Rewrite {
                        entry: entry_src.clone(),
                        source,
                    }
                })?;
                let new =
                    dangling::drop_dangling_style_refs(&new, &doc.styles).map_err(|source| {
                        WriteError::Rewrite {
                            entry: entry_src.clone(),
                            source,
                        }
                    })?;
                let new = cell_insets::spell_cell_insets(
                    &new,
                    &cell_insets::cell_insets_of(&story.story),
                )
                .map_err(|source| WriteError::Rewrite {
                    entry: entry_src.clone(),
                    source,
                })?;
                if new != orig.as_slice() {
                    patched.insert(entry_src, new);
                }
                continue;
            }
            let body = emit::story_part(
                &emit::sanitize_id(&story.self_id),
                &story.story,
                &dom_version,
                &anchors_for(story),
                host_width_for(story),
            )
            .map_err(|source| WriteError::Rewrite {
                entry: entry_src.clone(),
                source,
            })?;
            let body = face::patch_story_face(&body, &doc.styles).map_err(|source| {
                WriteError::Rewrite {
                    entry: entry_src.clone(),
                    source,
                }
            })?;
            let body =
                dangling::drop_dangling_style_refs(&body, &doc.styles).map_err(|source| {
                    WriteError::Rewrite {
                        entry: entry_src.clone(),
                        source,
                    }
                })?;
            let body =
                cell_insets::spell_cell_insets(&body, &cell_insets::cell_insets_of(&story.story))
                    .map_err(|source| WriteError::Rewrite {
                    entry: entry_src.clone(),
                    source,
                })?;
            new_entries.push((entry_src.clone(), body));
            new_story_srcs.push(entry_src);
        }
    }

    // Reference the new parts: a minimal designmap.xml insertion next to
    // the existing `<idPkg:Spread>` / `<idPkg:Story>` elements — then the
    // document-level resources (sections, the hyperlink block, the
    // conditions) brought in line with the model AND with InDesign's
    // spelling (see [`navigation`]). Both are pure pass-throughs for a
    // document that changed nothing and is already spelled canonically.
    const DESIGNMAP_SRC: &str = "designmap.xml";
    const GRAPHIC_SRC: &str = "Resources/Graphic.xml";
    const STYLES_SRC: &str = "Resources/Styles.xml";
    const FONTS_SRC: &str = "Resources/Fonts.xml";
    let styles_orig = entry_bytes(&mut src, STYLES_SRC)?;
    // Every face the document applies, declared in `Resources/Fonts.xml`
    // (see [`fonts`]). A part that already declares them all passes
    // through untouched; a package with no part gets one, referenced
    // from the designmap below.
    let faces = fonts::used_faces(doc, &opts.fonts, opts.default_face.as_ref());
    let mut mint_fonts = false;
    if let Some(orig) = entry_bytes(&mut src, FONTS_SRC)? {
        let new = fonts::patch_fonts(&orig, &faces, &opts.fonts).map_err(|source| {
            WriteError::Rewrite {
                entry: FONTS_SRC.to_string(),
                source,
            }
        })?;
        if new != orig.as_slice() {
            patched.insert(FONTS_SRC.to_string(), new);
        }
    } else if !faces.is_empty() {
        let body =
            fonts::fonts_part(&faces, &dom_version).map_err(|source| WriteError::Rewrite {
                entry: FONTS_SRC.to_string(),
                source,
            })?;
        new_entries.push((FONTS_SRC.to_string(), body));
        mint_fonts = true;
    }
    // The optical-size text preference the engine composed under (see
    // [`preferences`]): a part that spells it passes through untouched.
    // A package with no part is left without one (every generator and
    // InDesign itself write the part; minting it here would change the
    // entry list of a package that changed nothing).
    const PREFERENCES_SRC: &str = "Resources/Preferences.xml";
    if let Some(orig) = entry_bytes(&mut src, PREFERENCES_SRC)? {
        let new = preferences::patch_preferences(&orig, opts.default_face.as_ref()).map_err(
            |source| WriteError::Rewrite {
                entry: PREFERENCES_SRC.to_string(),
                source,
            },
        )?;
        if new != orig.as_slice() {
            patched.insert(PREFERENCES_SRC.to_string(), new);
        }
    }
    if let Some(orig) = entry_bytes(&mut src, DESIGNMAP_SRC)? {
        let mut new = orig.clone();
        if !(new_spread_refs.is_empty() && new_story_srcs.is_empty()) {
            new = emit::patch_designmap(&new, &new_spread_refs, &new_story_srcs, &[]).map_err(
                |source| WriteError::Rewrite {
                    entry: DESIGNMAP_SRC.to_string(),
                    source,
                },
            )?;
        }
        if !dropped_story_srcs.is_empty() {
            new = emit::patch_designmap(&new, &[], &[], &dropped_story_srcs).map_err(|source| {
                WriteError::Rewrite {
                    entry: DESIGNMAP_SRC.to_string(),
                    source,
                }
            })?;
        }
        if mint_fonts {
            new =
                emit::ensure_fonts_ref(&new, FONTS_SRC).map_err(|source| WriteError::Rewrite {
                    entry: DESIGNMAP_SRC.to_string(),
                    source,
                })?;
        }
        let plan = navigation_plan(doc, &orig, styles_orig.as_deref(), needs_xref_format).map_err(
            |source| WriteError::Rewrite {
                entry: DESIGNMAP_SRC.to_string(),
                source,
            },
        )?;
        new = navigation::patch_designmap_navigation(&new, &plan).map_err(|source| {
            WriteError::Rewrite {
                entry: DESIGNMAP_SRC.to_string(),
                source,
            }
        })?;
        if new != orig.as_slice() {
            patched.insert(DESIGNMAP_SRC.to_string(), new);
        }
    }

    // W1.15 lane 2 — new resources. Swatches / gradients created by ops
    // are injected into `Resources/Graphic.xml`; new paragraph / character
    // styles into `Resources/Styles.xml`. Both patchers are pure
    // pass-throughs when the model carries nothing the source lacks, so
    // an unmutated round-trip leaves these entries byte-identical (and the
    // entry takes the verbatim copy path below).
    if let Some(orig) = entry_bytes(&mut src, GRAPHIC_SRC)? {
        let new = resources::patch_graphic(&orig, &doc.palette).map_err(|source| {
            WriteError::Rewrite {
                entry: GRAPHIC_SRC.to_string(),
                source,
            }
        })?;
        if new != orig.as_slice() {
            patched.insert(GRAPHIC_SRC.to_string(), new);
        }
    }
    if let Some(orig) = styles_orig {
        // Conditions leave Styles.xml (they live in the designmap — see
        // `navigation`), then the style groups are patched.
        let stripped =
            resources::strip_conditions(&orig).map_err(|source| WriteError::Rewrite {
                entry: STYLES_SRC.to_string(),
                source,
            })?;
        let new = resources::patch_styles(&stripped, &doc.styles).map_err(|source| {
            WriteError::Rewrite {
                entry: STYLES_SRC.to_string(),
                source,
            }
        })?;
        // … and every paragraph style that names a font spells its
        // face (see [`face`]).
        let new =
            face::patch_styles_face(&new, &doc.styles).map_err(|source| WriteError::Rewrite {
                entry: STYLES_SRC.to_string(),
                source,
            })?;
        // … and every parent is a typed child, not an attribute (see
        // [`based_on`]).
        let new = based_on::patch_based_on(&new).map_err(|source| WriteError::Rewrite {
            entry: STYLES_SRC.to_string(),
            source,
        })?;
        if new != orig.as_slice() {
            patched.insert(STYLES_SRC.to_string(), new);
        }
    }

    // Walk the source archive in its original order. Each entry is
    // either substituted (patched body, re-deflated) or copied verbatim
    // with its already-compressed bytes.
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for i in 0..src.len() {
        let name = {
            let entry = src.by_index_raw(i)?;
            if entry.is_dir() {
                // Directory entries (rare in IDML) copy through as-is.
                drop(entry);
                let entry = src.by_index_raw(i)?;
                zip.raw_copy_file(entry)?;
                continue;
            }
            entry.name().to_string()
        };

        if !keep_container_parts && paged::is_container_part(&name) {
            // A pure `.idml` carries no `.paged` container part — see
            // `write_package`'s doc. `write_paged` keeps them.
            continue;
        }
        if dropped_story_srcs.contains(&name) {
            // An orphan story's part (see the stories loop above).
            continue;
        }
        if let Some(body) = patched.get(&name) {
            zip.start_file(&name, deflated)?;
            zip.write_all(body)?;
        } else {
            let entry = src.by_index_raw(i)?;
            zip.raw_copy_file(entry)?;
        }
    }

    // C-8 — the minted parts are appended after every source entry
    // (entry order within the ZIP is irrelevant to the parser; the
    // designmap drives discovery, and `mimetype` stays first).
    for (name, body) in &new_entries {
        zip.start_file(name.as_str(), deflated)?;
        zip.write_all(body)?;
    }

    let cursor = zip.finish()?;
    Ok(IdmlExport {
        bytes: cursor.into_inner(),
        links: links.into_links(),
    })
}

/// The entry a story's part lives at: its source path, or — minted
/// post-parse (`src: ""`) — the name derived from its id.
fn story_entry_src(story: &ParsedStory) -> String {
    if story.src.is_empty() {
        emit::story_src_for(&story.self_id)
    } else {
        story.src.clone()
    }
}

/// Read one entry's decompressed bytes out of the source archive.
/// `None` when the manifest names a path the package doesn't actually
/// carry (tolerated: that resource simply isn't patched).
fn entry_bytes<R: Read + std::io::Seek>(
    src: &mut zip::ZipArchive<R>,
    path: &str,
) -> Result<Option<Vec<u8>>, WriteError> {
    let mut entry = match src.by_name(path) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(Some(buf))
}

/// Prepare the designmap pass's view of the model: sections with their
/// derived `Length`, the URL / page destinations (text anchors are story
/// markers), hyperlinks, bookmarks, conditions + sets, and the indicator
/// colours scraped from wherever the source spelled them.
fn navigation_plan<'a>(
    doc: &'a Document,
    designmap: &[u8],
    styles: Option<&[u8]>,
    needs_xref_format: bool,
) -> Result<navigation::NavigationPlan<'a>, quick_xml::Error> {
    // Page order → each section's length: pages from its start up to the
    // next section's start. A section whose start page is unknown gets 1.
    let pages: Vec<&str> = doc
        .spreads
        .iter()
        .flat_map(|s| s.spread.pages.iter())
        .filter_map(|p| p.self_id.as_deref())
        .collect();
    let mut starts: Vec<(usize, usize)> = doc
        .designmap
        .sections
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let page = s.page_start.as_deref()?;
            pages.iter().position(|p| *p == page).map(|at| (at, i))
        })
        .collect();
    starts.sort();
    let mut lengths: Vec<usize> = vec![1; doc.designmap.sections.len()];
    for (k, (at, i)) in starts.iter().enumerate() {
        let end = starts
            .get(k + 1)
            .map(|(next, _)| *next)
            .unwrap_or(pages.len());
        lengths[*i] = end.saturating_sub(*at).max(1);
    }
    let sections = doc
        .designmap
        .sections
        .iter()
        .zip(lengths)
        .map(|(s, length)| navigation::SectionSpec {
            self_id: s.self_id.clone(),
            page_start: s.page_start.clone(),
            length,
            continue_numbering: s.continue_numbering,
            include_prefix: s.include_prefix,
            start_at: s.start_at,
            section_prefix: s.section_prefix.clone(),
            marker: s.marker.clone(),
            numbering_style: s.numbering_style,
        })
        .collect();
    let destinations = doc
        .designmap
        .hyperlink_destinations
        .iter()
        .filter(|d| !matches!(d.kind, idml_import::HyperlinkDestinationKind::TextAnchor(_)))
        .collect();
    let mut indicator_colors = resources::scan_condition_colors(designmap)?;
    if let Some(styles) = styles {
        for (id, color) in resources::scan_condition_colors(styles)? {
            indicator_colors.entry(id).or_insert(color);
        }
    }
    Ok(navigation::NavigationPlan {
        sections,
        destinations,
        hyperlinks: &doc.designmap.hyperlinks,
        bookmarks: &doc.designmap.bookmarks,
        conditions: &doc.styles.conditions,
        condition_sets: &doc.styles.condition_sets,
        indicator_colors,
        xref_format: needs_xref_format.then(|| XREF_FORMAT_ID.to_string()),
    })
}
