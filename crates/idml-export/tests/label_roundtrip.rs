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

//! A `<Label>` must not be reformatted on save — and when it IS
//! rewritten, its multi-line values must survive the round-trip.
//!
//! # This one was mis-diagnosed, and the mis-diagnosis is the point
//!
//! The defect was recorded as "equivalent XML, different bytes — lowest
//! priority, and possibly correct to leave alone". It is not equivalent
//! XML. It is silent data loss, and it compounds on every save.
//!
//! The writer rebuilt the `<Label>` from the model on every save and
//! wrote each `KeyValuePair Value` through
//! `BytesStart::push_attribute`, which escapes the five XML entities and
//! nothing else. So a value holding a newline was emitted with a LITERAL
//! newline inside the attribute — and XML 1.0 §3.3.3 requires a parser
//! to replace every literal tab, newline and carriage return in an
//! attribute value with a SPACE before reporting it. A character
//! reference is exempt, which is why InDesign writes `&#xa;`.
//!
//! Measured on `idml/samples/sample-3.idml`, whose labels carry an embedded
//! ecscript document: what went in as
//!
//! ```text
//! <?xml version="1.0" …?>\n<ecscript …>\n<sourcedata/>\n</ecscript>\n
//! ```
//!
//! came back as the same text with every `\n` turned into a space. Save
//! twice and the line structure is gone for good. `<Label>` is the
//! plugin-metadata carrier — JSON lives there too — so this is the
//! generic "your plugin's saved state came back mangled" bug.
//!
//! # Two fixes, because there are two defects
//!
//! * `escape_attr` now emits `&#x9;` / `&#xA;` / `&#xD;`. That is
//!   required for any label the model genuinely CHANGED, which no
//!   amount of byte-preservation would have covered.
//! * The replace/keep decision moved from the `<Label>` to the
//!   `</Label>`, so a label nobody edited keeps its source bytes — the
//!   same stance `patch_start` takes attribute by attribute. That is
//!   what makes the unmutated save byte-identical, indentation and all.
//!
//! # What this file pins
//!
//! The premise (attribute-value normalization really does eat a literal
//! newline — asserted against the XML reader, not assumed), the defect
//! (a mutated label's value survives a reparse; an unmutated one keeps
//! its bytes), and the invariants: a changed label is still rewritten, a
//! dropped one still dropped, a new one still synthesised.

use idml_export::rewrite::rewrite_spread;

/// The `sample-3` shape: a rectangle whose `<Label>` holds a multi-line
/// value spelled with `&#xa;`, indented the way InDesign indents it.
const SPREAD: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<idPkg:Spread xmlns:idPkg=\"http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging\" DOMVersion=\"20.0\">\n\
<Spread Self=\"s1\">\n\
\t<Page Self=\"pg1\" GeometricBounds=\"0 0 792 612\"/>\n\
\t<Rectangle Self=\"r1\" ItemTransform=\"1 0 0 1 0 0\" GeometricBounds=\"0 0 20 20\" FillColor=\"Color/Black\">\n\
\t\t<Properties>\n\
\t\t\t<Label>\n\
\t\t\t\t<KeyValuePair Key=\"ECRuleFieldDataKey\" Value=\"&lt;?xml version=&quot;1.0&quot;?&gt;&#xa;&lt;ecscript&gt;&#xa;&lt;/ecscript&gt;&#xa;\" />\n\
\t\t\t</Label>\n\
\t\t</Properties>\n\
\t</Rectangle>\n\
</Spread>\n\
</idPkg:Spread>";

const KEY: &str = "ECRuleFieldDataKey";
const VALUE: &str = "<?xml version=\"1.0\"?>\n<ecscript>\n</ecscript>\n";

fn spread() -> idml_import::Spread {
    idml_import::parse_spread(SPREAD).expect("parse")
}

/// Re-read a saved spread and return item `r1`'s label entries — the
/// only question that matters: does the value come back?
fn reparsed_label(xml: &[u8]) -> Vec<(String, String)> {
    idml_import::parse_spread(xml)
        .expect("reparse")
        .labels
        .get("r1")
        .cloned()
        .unwrap_or_default()
}

// -------------------------------------------------------------------
// Premises.
// -------------------------------------------------------------------

/// The parser reads `&#xa;` as a real newline, so the model value is
/// multi-line and there is something to lose.
#[test]
fn premise_the_model_holds_the_newlines() {
    let entries = spread().labels.get("r1").cloned().unwrap_or_default();
    assert_eq!(
        entries,
        vec![(KEY.to_string(), VALUE.to_string())],
        "premise: the label's value reaches the model with its newlines"
    );
}

/// And a LITERAL newline in an attribute value does not survive a read —
/// asserted against the same reader the parser uses, because this is the
/// fact the whole file rests on and "equivalent XML" was the wrong call.
#[test]
fn premise_a_literal_newline_in_an_attribute_becomes_a_space() {
    let literal = b"<Root><KeyValuePair Key=\"k\" Value=\"a\nb\"/></Root>";
    let reference = b"<Root><KeyValuePair Key=\"k\" Value=\"a&#xA;b\"/></Root>";
    assert_eq!(
        first_value(literal),
        "a b",
        "premise: XML 1.0 §3.3.3 normalizes a literal newline to a space"
    );
    assert_eq!(
        first_value(reference),
        "a\nb",
        "premise: …and exempts the character reference"
    );
}

fn first_value(xml: &[u8]) -> String {
    let mut r = quick_xml::Reader::from_reader(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf).expect("well-formed") {
            quick_xml::events::Event::Eof => return String::new(),
            quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                if e.name().as_ref() == b"KeyValuePair" {
                    for a in e.attributes().flatten() {
                        if a.key.as_ref() == b"Value" {
                            return a
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .expect("decode")
                                .into_owned();
                        }
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
}

// -------------------------------------------------------------------
// The defect.
// -------------------------------------------------------------------

/// An unmutated label keeps its source bytes — indentation, ` />`
/// spacing, `&#xa;` spelling and all.
#[test]
fn an_unmutated_label_round_trips_byte_identically() {
    let out = rewrite_spread(SPREAD, &spread()).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(SPREAD),
        "an unmutated save must not change a byte"
    );
}

/// A MUTATED label is rewritten — and its multi-line value comes back
/// out of the saved bytes unchanged. This is the half no amount of
/// byte-preservation could have covered.
#[test]
fn a_mutated_labels_newlines_survive_the_save() {
    let mut s = spread();
    let edited = format!("{VALUE}<!-- edited -->\nlast line\n");
    s.labels
        .insert("r1".to_string(), vec![(KEY.to_string(), edited.clone())]);
    let out = rewrite_spread(SPREAD, &s).expect("rewrite");
    assert_eq!(
        reparsed_label(&out),
        vec![(KEY.to_string(), edited)],
        "the value must survive the round-trip, newlines included:\n{}",
        String::from_utf8_lossy(&out)
    );
    assert!(
        !String::from_utf8_lossy(&out).contains("Value=\"<"),
        "…and the value must still be escaped:\n{}",
        String::from_utf8_lossy(&out)
    );
}

/// Tab and carriage return are normalized to spaces by the same rule, so
/// they need the same treatment.
#[test]
fn tabs_and_carriage_returns_survive_too() {
    let mut s = spread();
    let edited = "a\tb\rc\nd".to_string();
    s.labels
        .insert("r1".to_string(), vec![(KEY.to_string(), edited.clone())]);
    let out = rewrite_spread(SPREAD, &s).expect("rewrite");
    assert_eq!(
        reparsed_label(&out),
        vec![(KEY.to_string(), edited)],
        "tab / CR / LF all survive:\n{}",
        String::from_utf8_lossy(&out)
    );
}

// -------------------------------------------------------------------
// The invariants the fix must not cost.
// -------------------------------------------------------------------

/// Keeping an UNCHANGED label verbatim must not keep a changed one.
#[test]
fn a_changed_label_is_still_rewritten() {
    let mut s = spread();
    s.labels.insert(
        "r1".to_string(),
        vec![("other".to_string(), "value".to_string())],
    );
    let xml = String::from_utf8(rewrite_spread(SPREAD, &s).expect("rewrite")).expect("utf-8");
    assert!(
        xml.contains(r#"Key="other""#),
        "the new entry must be written:\n{xml}"
    );
    assert!(!xml.contains(KEY), "…and the old one must go:\n{xml}");
}

/// A label the model no longer carries is still dropped.
#[test]
fn a_dropped_label_is_still_dropped() {
    let mut s = spread();
    s.labels.remove("r1");
    let xml = String::from_utf8(rewrite_spread(SPREAD, &s).expect("rewrite")).expect("utf-8");
    assert!(!xml.contains("<Label>"), "the label must go:\n{xml}");
    assert!(!xml.contains("KeyValuePair"), "…with its entries:\n{xml}");
}

/// And a label on an item whose source had none is still synthesised.
#[test]
fn a_new_label_is_still_synthesised() {
    const BARE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Rectangle Self="r1" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 20 20"/>
</Spread>
</idPkg:Spread>"#;
    let mut s = idml_import::parse_spread(BARE).expect("parse");
    s.labels.insert(
        "r1".to_string(),
        vec![("k".to_string(), "line1\nline2".to_string())],
    );
    let out = rewrite_spread(BARE, &s).expect("rewrite");
    assert_eq!(
        reparsed_label(&out),
        vec![("k".to_string(), "line1\nline2".to_string())],
        "a synthesised label round-trips too:\n{}",
        String::from_utf8_lossy(&out)
    );
}

/// The THIRD label-writing lane, which is easy to miss: an item that
/// changes parent is dropped from its old position and rebuilt by the
/// `write_new_*` emitters, whose `write_item_label` had its own copy of
/// the escaping. So a grouped item's plugin metadata was flattened by a
/// move even though nobody touched it.
#[test]
fn a_moved_items_label_keeps_its_newlines() {
    const SRC: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Rectangle Self="r1" ItemTransform="1 0 0 1 0 0" GeometricBounds="0 0 20 20">
<Properties>
<Label>
<KeyValuePair Key="k" Value="line1&#xa;line2"/>
</Label>
</Properties>
</Rectangle>
</Spread>
</idPkg:Spread>"#;
    let mut s = idml_import::parse_spread(SRC).expect("parse");
    assert_eq!(
        s.labels.get("r1").cloned().unwrap_or_default(),
        vec![("k".to_string(), "line1\nline2".to_string())],
        "premise: the label reaches the model with its newline"
    );
    // Group the rectangle. `CreateGroup` over a SOURCE item is exactly
    // the move lane: the element is dropped and re-emitted inside the
    // new `<Group>`.
    s.groups.push(idml_import::Group {
        self_id: Some("g1".to_string()),
        members: vec![idml_import::FrameRef::Rectangle(0)],
        transparency: Default::default(),
        item_transform: None,
        corner_radius: None,
        corner_option: None,
        corners: Default::default(),
    });
    let out = rewrite_spread(SRC, &s).expect("rewrite");
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(
        text.contains("<Group Self=\"g1\""),
        "premise: the item really did move into a new group:\n{text}"
    );
    assert_eq!(
        reparsed_label(&out),
        vec![("k".to_string(), "line1\nline2".to_string())],
        "the moved item's label keeps its newline:\n{text}"
    );
}

/// The corpus entries the defect was measured on. Opt-in.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test label_roundtrip \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn corpus_sample_3_labels_survive_an_unmutated_save() {
    let Some(root) = corpus::root() else { return };
    let package = corpus::package(&root, "idml/samples/sample-3.idml");
    let mut with_labels = 0usize;
    for (name, body) in corpus::spreads(&package) {
        let text = String::from_utf8_lossy(&body).into_owned();
        if !text.contains("<Label>") {
            continue;
        }
        with_labels += 1;
        assert!(
            text.contains("&#xa;"),
            "premise: {name}'s label really does carry a character reference"
        );
        let spread = idml_import::parse_spread(&body).expect("parse");
        let out = rewrite_spread(&body, &spread).expect("rewrite");
        assert_eq!(
            String::from_utf8_lossy(&out),
            text,
            "{name}: an unmutated save must not change a byte"
        );
    }
    assert!(
        with_labels > 0,
        "premise: the package really does carry <Label>s"
    );
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
