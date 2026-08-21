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

//! `<PathPointArray>` contour indices must mean the same thing in the
//! writer as they do in the model.
//!
//! # The defect
//!
//! The writer walked every `<PathPointArray>` inside a page item and
//! matched the Nth one against the model's Nth contour. The parser fills
//! `subpath_starts` from a smaller set, so the two indices drift, in two
//! ways that both reach real corpus templates:
//!
//! * **A trailing EMPTY `<GeometryPathType>`.** The parser drops it as a
//!   "spurious subpath marker" (its start index points past the last
//!   anchor). `envato/packs/the-brochure` `Polygon u1659` has 8 contours
//!   with the 8th empty; `soccer-career-flyer-templates` `Polygon u687a`
//!   has 7, same shape. Indexing `subpath_starts[7]` PANICKED — and the
//!   wasm worker runs `panic = abort`, so it killed the save outright
//!   rather than surfacing a catchable error.
//! * **A `<PathPointArray>` that is not the frame's outline at all** — a
//!   placed picture's box or a clipping path. The parser skips those
//!   (`in_image_depth` / `in_clipping_path`); the writer counted them.
//!   `envato/packs/business-magazine-template` `Polygon u11560` carries
//!   a 45-point `Image > TextWrapPreference` contour, and an UNMUTATED
//!   save overwrote it with the host polygon's 39 anchors. No panic — a
//!   silent geometry loss, which is the worse of the two.
//!
//! # The behaviour chosen, and why
//!
//! A contour the model has no entry for passes through VERBATIM. Not
//! `.get(contour)` returning an empty slice — the caller would read
//! "model empty, disk not" as a divergence and rewrite the contour,
//! turning a visible crash into a silent geometry loss. Not "refuse the
//! save" either: there is no edit being refused. The dropped marker is
//! EMPTY, so there is nothing the model could write and nothing the user
//! can lose, and the contours the model DOES know still save normally —
//! refusing would throw away a legitimate edit to protect bytes that
//! were never at risk.

use idml_export::rewrite::rewrite_spread;

/// `the-brochure`'s shape, minimised: three `<GeometryPathType>`
/// contours, the last one an empty `<PathPointArray>` container (that is
/// exactly how InDesign writes it — a Start/End pair around whitespace,
/// not a self-closing tag).
const TRAILING_EMPTY: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Polygon Self="p1" ItemTransform="1 0 0 1 0 0" FillColor="Color/Black">
	<Properties>
		<PathGeometry>
			<GeometryPathType PathOpen="false">
				<PathPointArray>
					<PathPointType Anchor="0 0" LeftDirection="0 0" RightDirection="0 0" />
					<PathPointType Anchor="0 50" LeftDirection="0 50" RightDirection="0 50" />
					<PathPointType Anchor="50 50" LeftDirection="50 50" RightDirection="50 50" />
				</PathPointArray>
			</GeometryPathType>
			<GeometryPathType PathOpen="false">
				<PathPointArray>
					<PathPointType Anchor="10 10" LeftDirection="10 10" RightDirection="10 10" />
					<PathPointType Anchor="10 20" LeftDirection="10 20" RightDirection="10 20" />
					<PathPointType Anchor="20 20" LeftDirection="20 20" RightDirection="20 20" />
				</PathPointArray>
			</GeometryPathType>
			<GeometryPathType PathOpen="false">
				<PathPointArray>
				</PathPointArray>
			</GeometryPathType>
		</PathGeometry>
	</Properties>
</Polygon>
</Spread>
</idPkg:Spread>"#;

/// `business-magazine-template`'s shape, minimised: a single-contour
/// polygon that hosts a placed `<Image>` whose `<TextWrapPreference>`
/// carries a path of its own, with a DIFFERENT point count.
const FOREIGN_CONTOUR: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" DOMVersion="20.0">
<Spread Self="s1">
<Page Self="pg1" GeometricBounds="0 0 792 612"/>
<Polygon Self="p1" ItemTransform="1 0 0 1 0 0" FillColor="Color/Black">
	<Properties>
		<PathGeometry>
			<GeometryPathType PathOpen="false">
				<PathPointArray>
					<PathPointType Anchor="0 0" LeftDirection="0 0" RightDirection="0 0" />
					<PathPointType Anchor="0 50" LeftDirection="0 50" RightDirection="0 50" />
					<PathPointType Anchor="50 50" LeftDirection="50 50" RightDirection="50 50" />
				</PathPointArray>
			</GeometryPathType>
		</PathGeometry>
	</Properties>
	<Image Self="img1" ItemTransform="1 0 0 1 0 0">
		<TextWrapPreference Inverse="false" TextWrapMode="Contour">
			<Properties>
				<PathGeometry>
					<GeometryPathType PathOpen="false">
						<PathPointArray>
							<PathPointType Anchor="1 1" LeftDirection="1 1" RightDirection="1 1" />
							<PathPointType Anchor="1 9" LeftDirection="1 9" RightDirection="1 9" />
							<PathPointType Anchor="9 9" LeftDirection="9 9" RightDirection="9 9" />
							<PathPointType Anchor="9 1" LeftDirection="9 1" RightDirection="9 1" />
						</PathPointArray>
					</GeometryPathType>
				</PathGeometry>
			</Properties>
		</TextWrapPreference>
		<Link Self="lnk1" LinkResourceURI="file:///art.jpg"/>
	</Image>
</Polygon>
</Spread>
</idPkg:Spread>"#;

fn anchors_in(xml: &str) -> Vec<&str> {
    xml.match_indices(r#"Anchor=""#)
        .map(|(i, _)| {
            let rest = &xml[i + 8..];
            &rest[..rest.find('"').expect("closing quote")]
        })
        .collect()
}

/// The parse-side premise: the parser really does record ONE FEWER
/// contour than the XML carries, because it drops the trailing empty
/// marker. Asserted so this file fails loudly if that ever changes,
/// rather than passing for the wrong reason.
#[test]
fn parser_drops_the_trailing_empty_contour() {
    let spread = idml_import::parse_spread(TRAILING_EMPTY).expect("parse");
    let p = &spread.polygons[0];
    assert_eq!(p.anchors.len(), 6, "both real contours were read");
    assert_eq!(
        p.subpath_starts,
        vec![0, 3],
        "two contours recorded for three <GeometryPathType> elements"
    );
}

/// THE DEFECT, closed. This used to panic on `subpath_starts[2]` —
/// `panic = abort` in the wasm worker, so the user's save just died.
#[test]
fn trailing_empty_contour_round_trips_instead_of_panicking() {
    let spread = idml_import::parse_spread(TRAILING_EMPTY).expect("parse");
    let out = rewrite_spread(TRAILING_EMPTY, &spread).expect("rewrite");
    assert_eq!(
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(TRAILING_EMPTY),
        "an unmutated spread must round-trip byte-identically"
    );
}

/// THE OTHER HALF, closed. A `<PathPointArray>` belonging to a placed
/// image (here its text-wrap contour) is none of the frame's business:
/// it must survive untouched, and it must not shift the frame's own
/// contour numbering.
#[test]
fn a_placed_image_contour_is_not_overwritten_by_the_frame() {
    let spread = idml_import::parse_spread(FOREIGN_CONTOUR).expect("parse");
    let out = rewrite_spread(FOREIGN_CONTOUR, &spread).expect("rewrite");
    let xml = String::from_utf8(out.clone()).expect("utf8");
    assert_eq!(
        anchors_in(&xml),
        anchors_in(&String::from_utf8_lossy(FOREIGN_CONTOUR)),
        "every anchor, frame and image alike, must come back unchanged"
    );
    assert_eq!(
        out, FOREIGN_CONTOUR,
        "an unmutated spread must round-trip byte-identically:\n{xml}"
    );
}

/// The fix must not disable the anchor-edit lane. A real
/// `FramePathPoint` edit still writes back — to the frame's OWN contour,
/// leaving the image's alone.
#[test]
fn a_real_path_edit_still_saves_and_leaves_the_image_alone() {
    let mut spread = idml_import::parse_spread(FOREIGN_CONTOUR).expect("parse");
    spread.polygons[0].anchors[0].anchor = (7.0, 8.0);
    spread.polygons[0].anchors[0].left = (7.0, 8.0);
    spread.polygons[0].anchors[0].right = (7.0, 8.0);

    let out = rewrite_spread(FOREIGN_CONTOUR, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert!(
        xml.contains(r#"Anchor="7 8""#),
        "the moved anchor must be written:\n{xml}"
    );
    for a in ["1 1", "1 9", "9 9", "9 1"] {
        assert!(
            xml.contains(&format!(r#"Anchor="{a}""#)),
            "the image's text-wrap contour must survive a frame edit:\n{xml}"
        );
    }
}

/// A multi-contour edit still addresses the right contour — the
/// trailing empty marker must not shift the numbering of the ones that
/// precede it.
#[test]
fn an_edit_to_the_second_contour_lands_on_the_second_contour() {
    let mut spread = idml_import::parse_spread(TRAILING_EMPTY).expect("parse");
    // Anchor index 3 is the first point of contour 1 (`subpath_starts`
    // is `[0, 3]`).
    spread.polygons[0].anchors[3].anchor = (11.0, 12.0);
    spread.polygons[0].anchors[3].left = (11.0, 12.0);
    spread.polygons[0].anchors[3].right = (11.0, 12.0);

    let out = rewrite_spread(TRAILING_EMPTY, &spread).expect("rewrite");
    let xml = String::from_utf8(out).expect("utf8");
    assert_eq!(
        anchors_in(&xml),
        vec!["0 0", "0 50", "50 50", "11 12", "10 20", "20 20"],
        "only the edited anchor moves, and the empty contour stays empty:\n{xml}"
    );
}

/// The two corpus templates the panic was measured on, plus the one the
/// silent overwrite was measured on. Opt-in: the corpus is private and
/// gitignored, so this no-ops cleanly wherever it is absent.
///
/// ```text
/// PAGED_IDML_CORPUS=1 cargo test -p idml-export --test path_contour_roundtrip \
///     -- --ignored --nocapture
/// ```
#[test]
#[ignore = "private corpus: opt-in (PAGED_IDML_CORPUS=1 + the corpus mount)"]
fn corpus_templates_survive_an_unmutated_save() {
    let Some(root) = corpus::root() else { return };
    let mut checked = 0usize;
    for pack in [
        "idml/packs/the-brochure/template.idml",
        "idml/packs/soccer-career-flyer-templates/template.idml",
        "idml/packs/business-magazine-template/template.idml",
    ] {
        let package = corpus::package(&root, pack);
        for (name, body) in corpus::spreads(&package) {
            let spread = idml_import::parse_spread(&body).expect("parse");
            // Would have aborted the whole process before the fix.
            let out = rewrite_spread(&body, &spread).expect("rewrite");
            checked += 1;
            let before = String::from_utf8_lossy(&body).into_owned();
            let after = String::from_utf8_lossy(&out).into_owned();
            assert_eq!(
                anchors_in(&after).len(),
                anchors_in(&before).len(),
                "{pack}#{name}: an unmutated save must not add or drop a \
                 single path anchor"
            );
        }
    }
    assert!(checked > 0, "at least one template was reachable");
}

/// Shared corpus plumbing for the opt-in lanes.
#[path = "support/corpus.rs"]
mod corpus;
