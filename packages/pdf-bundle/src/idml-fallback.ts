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

// Phase 0 — build a minimal IDML/OCF package (in TypeScript, with JSZip)
// carrying ONE inline-image `<Rectangle>` per PDF page. Handed to
// `host.nativeDocument.open`, it routes through the engine's EXISTING IDML
// importer + inline-image path — no Rust/wasm involved.
//
// The skeleton is a faithful port of the engine's own blank-document
// builder (`core/crates/paged-canvas/src/blank.rs`): the same ZIP entry set,
// order, and XML strings. Two deliberate differences:
//
//   1. multi-spread — one `<idPkg:Spread>` per page in the designmap, one
//      `Spreads/Spread_p{i}.xml` per page (blank.rs emits exactly one).
//   2. each spread carries a full-page `<Rectangle>` whose nested `<Image>`
//      inlines the page PNG as base64 `<Contents>`.
//
// The `<Rectangle>` + `<Image>` shape mirrors the generator's inline-image
// path (`core/crates/paged-gen/src/builders/page_item.rs`) and is validated
// against the parser (`plugin-publish/crates/idml-import/src/spread.rs`,
// Q-03): `<Contents>` is base64 of the raw PNG bytes, decoded onto
// `Rectangle.image_bytes`; the `<Image>` element flips `has_image_element`.

import JSZip from "jszip";

/** One rasterized PDF page: its point size + the PNG-encoded pixels. */
export interface PdfPageRaster {
  widthPt: number;
  heightPt: number;
  pngBytes: Uint8Array;
}

// IDML/OCF package mimetype. MUST be the first ZIP entry and STORED.
const MIME = "application/vnd.adobe.indesign-idml-package";
const NS = "http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging";
const XML_DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n';

function xml(body: string): string {
  return XML_DECL + body;
}

function emptyPkg(tag: string): string {
  return xml(`<idPkg:${tag} xmlns:idPkg="${NS}" DOMVersion="20.0"/>`);
}

function container(): string {
  return xml(
    '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">' +
      '<rootfiles><rootfile full-path="designmap.xml" media-type="text/xml"/></rootfiles></container>',
  );
}

// Mirror of blank.rs::graphic — the `Color/Black` + `Swatch/None` the
// Rectangle's `FillColor`/`StrokeColor="Swatch/None"` reference so the
// palette resolves cleanly.
function graphic(): string {
  return xml(
    `<idPkg:Graphic xmlns:idPkg="${NS}" DOMVersion="20.0">` +
      '<Color Self="Color/Black" Model="Process" Space="CMYK" ColorValue="0 0 0 100" Name="Black"/>' +
      '<Swatch Self="Swatch/None" Name="None"/></idPkg:Graphic>',
  );
}

// Mirror of blank.rs::styles — the default `[No character style]` /
// `[No paragraph style]` roots a parsed IDML always carries.
function styles(): string {
  return xml(
    `<idPkg:Styles xmlns:idPkg="${NS}" DOMVersion="20.0">` +
      '<RootCharacterStyleGroup Self="rcs">' +
      '<CharacterStyle Self="CharacterStyle/$ID/[No character style]" Name="$ID/[No character style]"/>' +
      "</RootCharacterStyleGroup>" +
      '<RootParagraphStyleGroup Self="rps">' +
      '<ParagraphStyle Self="ParagraphStyle/$ID/[No paragraph style]" Name="$ID/[No paragraph style]"/>' +
      "</RootParagraphStyleGroup></idPkg:Styles>",
  );
}

function backing(): string {
  return xml(
    `<idPkg:BackingStory xmlns:idPkg="${NS}" DOMVersion="20.0">` +
      '<XmlStory Self="backing"/></idPkg:BackingStory>',
  );
}

// Multi-spread designmap: one `<idPkg:Spread>` ref per page. Mirrors
// blank.rs::designmap otherwise (aid PI + resource/master/backing refs).
// `StoryList=""` — no stories in the image-only fallback.
function designmap(spreadSrcs: string[]): string {
  const spreadRefs = spreadSrcs
    .map((src) => `<idPkg:Spread src="${src}"/>`)
    .join("\n");
  return xml(
    '<?aid style="50" type="document" readerVersion="6.0" featureSet="257" product="20.0(32)"?>\n' +
      `<Document xmlns:idPkg="${NS}" DOMVersion="20.0" Self="d" StoryList="" Name="imported.pdf">\n` +
      '<idPkg:Graphic src="Resources/Graphic.xml"/>\n' +
      '<idPkg:Fonts src="Resources/Fonts.xml"/>\n' +
      '<idPkg:Styles src="Resources/Styles.xml"/>\n' +
      '<idPkg:Preferences src="Resources/Preferences.xml"/>\n' +
      '<idPkg:MasterSpread src="MasterSpreads/MasterSpread_um.xml"/>\n' +
      `${spreadRefs}\n` +
      '<idPkg:BackingStory src="XML/BackingStory.xml"/>\n' +
      "</Document>",
  );
}

// `GeometricBounds` is InDesign's "y0 x0 y1 x1" order, so a `w × h` page is
// `0 0 h w` (mirror blank.rs).
function boundsStr(widthPt: number, heightPt: number): string {
  return `0 0 ${fmt(heightPt)} ${fmt(widthPt)}`;
}

function masterSpread(widthPt: number, heightPt: number): string {
  const bounds = boundsStr(widthPt, heightPt);
  return xml(
    `<idPkg:MasterSpread xmlns:idPkg="${NS}" DOMVersion="20.0">` +
      '<MasterSpread Self="um" Name="A">' +
      `<Page Self="ump" Name="A" GeometricBounds="${bounds}" ItemTransform="1 0 0 1 0 0"/>` +
      "</MasterSpread></idPkg:MasterSpread>",
  );
}

// Serialise a coordinate like the Rust `format_f32`: whole numbers print
// without a fraction (`612`, not `612.0`), keeping the emitted XML close to
// what the generator produces and the parser round-trips cleanly. `String`
// already renders integer-valued floats without a trailing `.0`.
function fmt(n: number): string {
  return String(n);
}

function base64(bytes: Uint8Array): string {
  // Chunked to stay well under any argument-count limits for large pages.
  let binary = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    const slice = bytes.subarray(i, i + CHUNK);
    binary += String.fromCharCode.apply(null, slice as unknown as number[]);
  }
  // Present in both browser (btoa) and node (Buffer) runtimes.
  if (typeof btoa === "function") return btoa(binary);
  return Buffer.from(binary, "binary").toString("base64");
}

// The four page corners, anchored at the item origin (0,0) — exactly the
// walk `write_path_geometry` emits in page_item.rs:
//   (0,0) (0,h) (w,h) (w,0)
// With an identity `ItemTransform` these anchors ARE spread-space, so the
// AABB the parser derives (`bounds_from_anchors`) equals the page bounds.
function pathGeometry(widthPt: number, heightPt: number): string {
  const w = fmt(widthPt);
  const h = fmt(heightPt);
  const corners: Array<[string, string]> = [
    ["0", "0"],
    ["0", h],
    [w, h],
    [w, "0"],
  ];
  const points = corners
    .map(([x, y]) => {
      const xy = `${x} ${y}`;
      return `<PathPointType Anchor="${xy}" LeftDirection="${xy}" RightDirection="${xy}"/>`;
    })
    .join("");
  return (
    "<PathGeometry>" +
    '<GeometryPathType PathOpen="false">' +
    "<PathPointArray>" +
    points +
    "</PathPointArray>" +
    "</GeometryPathType>" +
    "</PathGeometry>"
  );
}

// One page's `<idPkg:Spread>`: a single page at its pt size holding one
// full-page inline-image `<Rectangle>`. Ids are derived from the page index
// so the output is deterministic across runs.
function spread(page: PdfPageRaster, index: number): string {
  const bounds = boundsStr(page.widthPt, page.heightPt);
  const geometry = pathGeometry(page.widthPt, page.heightPt);
  const contents = base64(page.pngBytes);
  const spreadId = `spread_p${index}`;
  const pageId = `page_p${index}`;
  const rectId = `rect_p${index}`;
  const imageId = `img_p${index}`;
  return xml(
    `<idPkg:Spread xmlns:idPkg="${NS}" DOMVersion="20.0">\n` +
      `<Spread Self="${spreadId}" PageCount="1" BindingLocation="0" ShowMasterItems="true" AllowPageShuffle="true" ItemTransform="1 0 0 1 0 0">\n` +
      `<Page Self="${pageId}" Name="${index + 1}" AppliedMaster="um" ItemTransform="1 0 0 1 0 0" GeometricBounds="${bounds}" MasterPageTransform="1 0 0 1 0 0"/>\n` +
      // No GeometricBounds attr on the Rectangle: like page_item.rs it relies
      // on the PathGeometry anchors + identity ItemTransform for its bounds.
      `<Rectangle Self="${rectId}" AppliedObjectStyle="ObjectStyle/$ID/[None]" Visible="true" Name="$ID/" ItemTransform="1 0 0 1 0 0" FillColor="Swatch/None" StrokeColor="Swatch/None" StrokeWeight="0">\n` +
      "<Properties>" +
      geometry +
      "</Properties>\n" +
      // FrameFittingOption is a direct child of the Rectangle (sibling of
      // Properties), emitted right before the <Image> — matching page_item.rs
      // and what the parser's Rectangle walker expects. FitContentToFrame +
      // zero crops fills the frame with the page raster (its pixel aspect
      // already equals the page aspect).
      '<FrameFittingOption LeftCrop="0" TopCrop="0" RightCrop="0" BottomCrop="0" FittingOnEmptyFrame="FitContentToFrame"/>\n' +
      `<Image Self="${imageId}" ItemTransform="1 0 0 1 0 0">\n` +
      "<Properties>" +
      // The Image's own PathGeometry describes its (here page-sized) extents,
      // then the base64 <Contents> the parser decodes onto image_bytes.
      geometry +
      `<Contents><![CDATA[${contents}]]></Contents>` +
      "</Properties>\n" +
      "</Image>\n" +
      "</Rectangle>\n" +
      "</Spread></idPkg:Spread>",
  );
}

/**
 * Build a minimal IDML package (bytes) with one inline-image `<Rectangle>`
 * per page. The ZIP is assembled with `mimetype` STORED + first (OCF
 * convention), every other entry deflated — identical to blank.rs's `zip`
 * assembly. The result parses through the engine's `import_idml_archive`.
 *
 * Async because JSZip v3 offers no synchronous buffer output
 * (`generate()` was removed in v3 — only `generateAsync` exists).
 */
export async function buildIdmlFromRasters(
  pages: PdfPageRaster[],
): Promise<Uint8Array> {
  if (pages.length === 0) {
    throw new Error("buildIdmlFromRasters: no pages to build");
  }
  const zip = new JSZip();

  // mimetype first + STORED (OCF convention). JSZip preserves insertion
  // order, and `compression: "STORE"` leaves the bytes uncompressed.
  zip.file("mimetype", MIME, { compression: "STORE" });

  const spreadSrcs = pages.map((_, i) => `Spreads/Spread_p${i}.xml`);

  zip.file("designmap.xml", designmap(spreadSrcs), { compression: "DEFLATE" });
  zip.file("META-INF/container.xml", container(), { compression: "DEFLATE" });
  zip.file("Resources/Graphic.xml", graphic(), { compression: "DEFLATE" });
  zip.file("Resources/Fonts.xml", emptyPkg("Fonts"), { compression: "DEFLATE" });
  zip.file("Resources/Styles.xml", styles(), { compression: "DEFLATE" });
  zip.file("Resources/Preferences.xml", emptyPkg("Preferences"), {
    compression: "DEFLATE",
  });
  zip.file(
    "MasterSpreads/MasterSpread_um.xml",
    masterSpread(pages[0].widthPt, pages[0].heightPt),
    { compression: "DEFLATE" },
  );
  pages.forEach((page, i) => {
    zip.file(spreadSrcs[i], spread(page, i), { compression: "DEFLATE" });
  });
  zip.file("XML/BackingStory.xml", backing(), { compression: "DEFLATE" });

  // `streamFiles: false` (the default) writes real sizes into each local
  // header rather than a trailing data descriptor — so the STORED mimetype's
  // header is the exact shape an OCF consumer expects. Per-file `compression`
  // wins over this generate-level default, so mimetype stays STORED.
  return zip.generateAsync({
    type: "uint8array",
    compression: "DEFLATE",
    streamFiles: false,
  });
}
