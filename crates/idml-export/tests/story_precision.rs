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

//! An UNTOUCHED `Stories/*.xml` must save back as the bytes it arrived as.
//!
//! # The measurement came first
//!
//! `examples/corpus_sweep.rs` used to sweep `Spreads/*.xml` and nothing
//! else. Its headline gap count — 596, then 87, then 57 — described 823 of
//! the corpus's 12,668 entries; the STORY lane, 10,539 entries, was never
//! in the denominator. Putting it in raised the honest combined count from
//! 57 to **905** across 11,560 transformed entries, and the story lane
//! alone accounted for 845 of them.
//!
//! Three separate defects were hiding under that one number, and this file
//! pins all three.
//!
//! # 1. The same f32 truncation, never applied here
//!
//! `<CharacterStyleRange>` numerics went through `opt_f32_patch`, which
//! unconditionally re-emits through `format_f32` (4 decimals). InDesign
//! authors in millimetres and percent, so the corpus is full of
//! `StrokeWeight="0.9921259842519686"`, `PointSize="8.999999999999998"`,
//! `BaselineShift="4.097337047350078"`, `Tracking="-1.7763568394002505e-15"`
//! — 236 entries changed a stroke weight and 149 a point size on a save
//! that changed nothing.
//!
//! **Which predicate:** the SIMPLE one, `preserving_f32_patch` — the same
//! one page-item `StrokeWeight` uses, not the forward-replay
//! `TransformPlan` that `ItemTransform` needed. A group member's
//! `item_transform` is stored COMPOSED with its ancestors', so the on-disk
//! spelling has to be recovered by inverting that composition before it can
//! be compared; a character run has no such derivation. `idml-import` reads
//! every one of these as `attr(e, KEY).and_then(|s| s.parse().ok())` — no
//! composition, no unit conversion, and no inheritance from the applied
//! paragraph or character style (a `<CharacterStyle>`'s own `PointSize`
//! lands in the style registry and is resolved by consumers, never folded
//! back onto the run). So replaying the parser's derivation against the
//! source spelling IS parsing the spelling.
//!
//! # 2. U+2028 made every run look mutated
//!
//! 283 entries came back different with **no attribute difference at all**.
//! Every one of them contained U+2028 (Unicode LINE SEPARATOR — InDesign's
//! forced line break, Shift+Enter). The parser normalises U+2028 / U+2029
//! to `\n` when it builds `CharacterRun::text`; the rewriter's
//! reconstruction of the same string did not. So `run.text != body.text`
//! always, the "was this run edited?" test said yes, and the run was
//! re-serialised from the model — turning a forced LINE break into `<Br/>`,
//! which in IDML is a PARAGRAPH break. A no-op save was silently changing
//! the text's break semantics.
//!
//! # 3. An anchored object after content produced MALFORMED XML
//!
//! The inline body buffers Starts, Texts and Emptys and replays them at
//! `</CharacterStyleRange>` — but End events were written straight to the
//! writer, so they jumped ahead of everything buffered. A run shaped
//! `<Content>…</Content><Rectangle>…</Rectangle>` (an anchored object) or
//! `<HyperlinkTextSource>…<Content>…</Content></HyperlinkTextSource>` came
//! back with the subtree's closing tags stacked in front of its opening
//! ones. Seven corpus stories rewrote to XML that will not re-parse: a
//! KILLED save, and one a byte-count could never have named — which is why
//! the sweep now reports well-formedness as its own column.

use idml_export::rewrite::rewrite_story;

/// Real spellings and real element shapes, minimised from the corpus:
/// the millimetre-authored `StrokeWeight`, the `PointSize` an autofit
/// leaves behind, `real-estate-brochure`'s `BaselineShift`,
/// `square-company-profile`'s denormal `Tracking` and
/// `saas-product-launch`'s `HorizontalScale`.
const STORY: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Story Self="u1a2" AppliedTOCStyle="n" TrackChanges="false" StoryTitle="$ID/">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/BODY">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]" PointSize="8.999999999999998" StrokeWeight="0.9921259842519686" Tracking="-1.7763568394002505e-15" BaselineShift="4.097337047350078" HorizontalScale="97.50194646030498">
<Content>Anchored and untouched</Content>
</CharacterStyleRange>
</ParagraphStyleRange>
</Story>
</idPkg:Story>"#;

/// A run whose `<Content>` carries a real U+2028 (`&#x2028;` written as
/// the literal character, which is how InDesign serialises it) followed
/// by more text in the SAME `<Content>`. The parser turns that into a
/// `\n` in `CharacterRun::text`; the writer must not mistake the
/// normalisation for an edit.
const FORCED_LINE_BREAK: &[u8] = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<idPkg:Story xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\" DOMVersion=\"20.0\">
<Story Self=\"u2b3\">
<ParagraphStyleRange AppliedParagraphStyle=\"ParagraphStyle/BODY\">
<CharacterStyleRange AppliedCharacterStyle=\"CharacterStyle/$ID/[No character style]\">
<Content>first line\u{2028}second line</Content>
<Br />
<Content>after a real paragraph break</Content>
</CharacterStyleRange>
</ParagraphStyleRange>
</Story>
</idPkg:Story>"
    .as_bytes();

/// Two runs the End-ordering defect scrambled: an anchored `<Rectangle>`
/// that FOLLOWS the run's `<Content>`, and a `<HyperlinkTextSource>` that
/// WRAPS one. Both are exactly the shapes InDesign writes.
const ANCHORED_AFTER_CONTENT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Story xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Story Self="u3c4">
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/BODY">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
<Content>before the anchor</Content>
<Rectangle Self="ud99" ContentType="GraphicType" StoryTitle="$ID/">
<Properties>
<PathGeometry>
<GeometryPathType PathOpen="false">
<PathPointArray>
<PathPointType Anchor="-66.3 -63.0" LeftDirection="-66.3 -63.0" RightDirection="-66.3 -63.0" />
</PathPointArray>
</GeometryPathType>
</PathGeometry>
</Properties>
</Rectangle>
</CharacterStyleRange>
</ParagraphStyleRange>
<ParagraphStyleRange AppliedParagraphStyle="ParagraphStyle/TOC">
<CharacterStyleRange AppliedCharacterStyle="CharacterStyle/$ID/[No character style]">
<HyperlinkTextSource Self="u29535" Name=".169269" Hidden="true" AppliedCharacterStyle="n">
<Properties>
<AlternativeDestination Type="TocTextAnchor" IndexMarkerId="0" />
</Properties>
<Content>INTRODUCTION</Content>
</HyperlinkTextSource>
<Br />
</CharacterStyleRange>
</ParagraphStyleRange>
</Story>
</idPkg:Story>"#;

/// The writer's output precision (`rewrite::format_f32`, private): round
/// to 4 decimals, drop trailing zeros and a dangling `.`. Re-stated here
/// so the tests can assert what a RE-DERIVED value would look like.
fn fmt4(v: f32) -> String {
    let r = (f64::from(v) * 10_000.0).round() / 10_000.0;
    if r == 0.0 {
        return "0".to_string();
    }
    let mut s = format!("{r:.4}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Every `KEY="…"` value in document order.
fn values_of<'a>(xml: &'a str, key: &str) -> Vec<&'a str> {
    let needle = format!("{key}=\"");
    xml.match_indices(&needle)
        .map(|(i, _)| {
            let rest = &xml[i + needle.len()..];
            &rest[..rest.find('"').expect("closing quote")]
        })
        .collect()
}

/// `Ok(())` when `xml` is well-formed — mismatched or unclosed end tags
/// are hard errors, which is the check that catches a killed save.
fn well_formed(xml: &[u8]) -> Result<(), String> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let config = reader.config_mut();
    config.check_end_names = true;
    config.expand_empty_elements = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Eof) => return Ok(()),
            Ok(_) => buf.clear(),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn parsed() -> idml_import::Story {
    idml_import::parse_story(STORY).expect("parse")
}

/// The premise, asserted rather than assumed: the model genuinely cannot
/// spell these values back, so nothing below can pass for the wrong
/// reason (a fixture whose values happened to survive `format_f32` would
/// make every assertion vacuous).
#[test]
fn the_model_cannot_respell_the_source_values() {
    let story = parsed();
    let run = &story.paragraphs[0].runs[0];
    for (label, model, source) in [
        ("PointSize", run.point_size, "8.999999999999998"),
        ("StrokeWeight", run.stroke_weight, "0.9921259842519686"),
        ("Tracking", run.tracking, "-1.7763568394002505e-15"),
        ("BaselineShift", run.baseline_shift, "4.097337047350078"),
        ("HorizontalScale", run.horizontal_scale, "97.50194646030498"),
    ] {
        let v = model.unwrap_or_else(|| panic!("{label} parsed"));
        assert_ne!(
            fmt4(v),
            source,
            "{label}: premise — the writer's 4-decimal spelling must \
             differ from the source, or this file proves nothing"
        );
    }
}

/// The headline: an UNMUTATED story round-trips byte-identically. Without
/// the preserving rule every one of the five numbers above comes back
/// rounded.
#[test]
fn an_unmutated_story_round_trips_byte_identically() {
    let out = rewrite_story(STORY, &parsed()).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(STORY),
        "an unmutated story must save back as the bytes it arrived as"
    );
}

/// A CHANGED value is still written — from the model, at the writer's
/// precision. The preserving rule is a no-op check, not a freeze.
#[test]
fn a_character_size_edit_still_saves() {
    let mut story = parsed();
    story.paragraphs[0].runs[0].point_size = Some(12.5);
    let out = rewrite_story(STORY, &story).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(values_of(&xml, "PointSize"), vec!["12.5"], "{xml}");
    // The untouched neighbours keep their full source spelling.
    assert_eq!(
        values_of(&xml, "StrokeWeight"),
        vec!["0.9921259842519686"],
        "an edit to one attribute must not re-derive the others:\n{xml}"
    );
}

/// A CLEARED value still drops the attribute.
#[test]
fn a_cleared_character_attribute_still_drops() {
    let mut story = parsed();
    story.paragraphs[0].runs[0].baseline_shift = None;
    let out = rewrite_story(STORY, &story).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert!(
        !xml.contains("BaselineShift="),
        "a cleared attribute must be removed:\n{xml}"
    );
}

/// A value set on a run that never carried the attribute is still
/// appended (`character_extras`), at the writer's precision.
#[test]
fn a_newly_set_point_size_is_still_appended() {
    let story = idml_import::parse_story(FORCED_LINE_BREAK).expect("parse");
    let mut story = story;
    story.paragraphs[0].runs[0].point_size = Some(9.0);
    let out = rewrite_story(FORCED_LINE_BREAK, &story).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(values_of(&xml, "PointSize"), vec!["9"], "{xml}");
}

/// U+2028 is a forced LINE break; `<Br/>` is a PARAGRAPH break. An
/// unmutated save must not silently convert one into the other — which
/// it did for 283 corpus stories, every one of them a byte difference
/// with no attribute difference at all.
#[test]
fn a_forced_line_break_survives_an_unmutated_save() {
    let story = idml_import::parse_story(FORCED_LINE_BREAK).expect("parse");
    // Premise: the parser really does normalise it away, so the naive
    // comparison really would have mismatched.
    assert!(
        story.paragraphs[0].runs[0].text.contains('\n')
            && !story.paragraphs[0].runs[0].text.contains('\u{2028}'),
        "premise: the parser collapses U+2028 to a newline"
    );
    let out = rewrite_story(FORCED_LINE_BREAK, &story).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert!(
        xml.contains('\u{2028}'),
        "the forced line break must survive verbatim:\n{xml}"
    );
    assert_eq!(
        xml.matches("<Br").count(),
        1,
        "the one real <Br/> must stay the only one — a U+2028 that \
         became a second <Br/> has changed the text's break semantics:\n{xml}"
    );
    assert_eq!(
        xml.as_bytes(),
        FORCED_LINE_BREAK,
        "and the whole entry must be byte-identical"
    );
}

/// An anchored object AFTER the run's content, and a hyperlink source
/// WRAPPING it, both round-trip — and, first of all, both stay
/// well-formed. Before the fix the subtree's End tags were written
/// straight to the writer while its Starts sat in the run buffer, so
/// they came back stacked in front of each other.
#[test]
fn an_anchored_object_after_content_stays_well_formed() {
    let story = idml_import::parse_story(ANCHORED_AFTER_CONTENT).expect("parse");
    let out = rewrite_story(ANCHORED_AFTER_CONTENT, &story).expect("rewrite");
    well_formed(&out).unwrap_or_else(|e| {
        panic!(
            "the rewrite produced XML that will not re-parse ({e}):\n{}",
            String::from_utf8_lossy(&out)
        )
    });
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(ANCHORED_AFTER_CONTENT),
        "an unmutated story with an anchored object must be byte-identical"
    );
}

/// CLOSED. This used to name `samples/line-sheet.idml#Stories/
/// Story_u2b6.xml` as the one entry whose character numbers still moved,
/// and why: its first `<CharacterStyleRange>` holds a `<Table>` and no
/// text, `idml_import::parse_story` DROPS a run with empty text, and
/// `rewrite_story`'s positional cursor counted every element — so the
/// paragraph's runs were patched one position out and the range's
/// `PointSize="10"` / `FontStyle="Bold"` were overwritten with the NEXT
/// run's values. The two `AppliedCharacterStyle` divergences in
/// `samples/sample.idml` had the same root, and `<TextVariableInstance>`
/// broke the mapping the other way (the parser SPLITS one range into
/// three runs).
///
/// The parser now publishes where each source element landed
/// (`idml_import::StoryProvenance`) and the rewrite looks it up instead
/// of counting; see `tests/run_alignment_roundtrip.rs`. The exemption
/// list is empty and must stay empty.
const KNOWN_RUN_MISALIGNMENT: [&str; 0] = [];

/// The corpus lane the three defects were measured on. Opt-in: the corpus
/// is private and gitignored, so this no-ops cleanly wherever it is
/// absent.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test story_precision \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn corpus_stories_keep_their_numbers_and_stay_well_formed() {
    let Some(root) = corpus::root() else { return };
    // `line-sheet` is where the seven MALFORMED rewrites lived (anchored
    // objects after content); `real-estate-brochure` and
    // `square-company-profile` carry the full-precision character
    // numbers.
    let packages = [
        "samples/line-sheet.idml",
        "idml/packs/real-estate-brochure/template.idml",
        "idml/packs/square-company-profile/template.idml",
    ];
    const NUMERIC: [&str; 7] = [
        "PointSize",
        "StrokeWeight",
        "Tracking",
        "BaselineShift",
        "HorizontalScale",
        "VerticalScale",
        "Skew",
    ];
    let mut checked = 0usize;
    let mut long_spellings = 0usize;
    let mut moved: Vec<String> = Vec::new();
    for rel in packages {
        let package = root.join(rel);
        if !package.exists() {
            eprintln!("SKIP: {} not found", package.display());
            continue;
        }
        for (name, body) in corpus::stories(&package) {
            let story = idml_import::parse_story(&body).expect("parse");
            let out = rewrite_story(&body, &story).expect("rewrite");
            checked += 1;
            // A killed save first: every rewrite must re-parse.
            well_formed(&out)
                .unwrap_or_else(|e| panic!("{rel}#{name}: the rewrite will not re-parse ({e})"));
            let before = String::from_utf8_lossy(&body).into_owned();
            let after = String::from_utf8_lossy(&out).into_owned();
            for key in NUMERIC {
                let src = values_of(&before, key);
                long_spellings += src.iter().filter(|v| v.len() > 8).count();
                if values_of(&after, key) != src {
                    moved.push(format!("{rel}#{name}"));
                    break;
                }
            }
        }
    }
    assert!(checked > 0, "the packages had reachable stories");
    assert!(
        long_spellings > 0,
        "premise: the corpus really does carry full-precision character numbers"
    );
    assert_eq!(
        moved, KNOWN_RUN_MISALIGNMENT,
        "the only entries whose character numbers may still move are the \
         documented run-misalignment ones (see KNOWN_RUN_MISALIGNMENT); \
         anything else is a precision regression"
    );
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
