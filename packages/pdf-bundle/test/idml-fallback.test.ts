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

import { writeFileSync } from "node:fs";
import { Buffer } from "node:buffer";

import JSZip from "jszip";
import { describe, it, expect } from "vitest";

import { buildIdmlFromRasters, type PdfPageRaster } from "../src/idml-fallback";

const MIME = "application/vnd.adobe.indesign-idml-package";

// A real 1×1 PNG (transparent) — so the emitted `.idml` is not just
// well-formed but carries decodable image bytes the engine could render.
const PNG_A = new Uint8Array(
  Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
    "base64",
  ),
);
// A second, distinct 1×1 PNG (opaque red) for the 2nd page.
const PNG_B = new Uint8Array(
  Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  ),
);

const PAGES: PdfPageRaster[] = [
  { widthPt: 612, heightPt: 792, pngBytes: PNG_A },
  { widthPt: 200, heightPt: 300, pngBytes: PNG_B },
];

/** Little-endian uint16 read. */
function u16(bytes: Uint8Array, off: number): number {
  return bytes[off] | (bytes[off + 1] << 8);
}

describe("buildIdmlFromRasters", () => {
  it("emits mimetype first, STORED, with exact bytes (OCF convention)", async () => {
    const idml = await buildIdmlFromRasters(PAGES);

    // First local file header at offset 0: signature PK\x03\x04.
    expect([idml[0], idml[1], idml[2], idml[3]]).toEqual([0x50, 0x4b, 0x03, 0x04]);
    // Compression method (offset 8) == 0 → STORED.
    expect(u16(idml, 8)).toBe(0);
    // Filename (offset 30, length at 26) is "mimetype".
    const fnLen = u16(idml, 26);
    const extraLen = u16(idml, 28);
    const fnStart = 30;
    const filename = Buffer.from(
      idml.subarray(fnStart, fnStart + fnLen),
    ).toString("latin1");
    expect(filename).toBe("mimetype");
    // The STORED payload immediately follows the header; its bytes are the
    // exact mimetype string.
    const dataStart = fnStart + fnLen + extraLen;
    const payload = Buffer.from(
      idml.subarray(dataStart, dataStart + MIME.length),
    ).toString("latin1");
    expect(payload).toBe(MIME);
  });

  it("designmap lists one spread per page; one Spreads/*.xml per page", async () => {
    const idml = await buildIdmlFromRasters(PAGES);
    const zip = await JSZip.loadAsync(idml);

    const designmap = await zip.file("designmap.xml")!.async("string");
    const spreadRefs = designmap.match(/<idPkg:Spread src="/g) ?? [];
    expect(spreadRefs.length).toBe(PAGES.length);

    const spreadEntries = Object.keys(zip.files).filter((n) =>
      /^Spreads\/.*\.xml$/.test(n),
    );
    expect(spreadEntries.length).toBe(PAGES.length);
  });

  it("each spread carries a Rectangle > Image > Contents that decodes to the page PNG", async () => {
    const idml = await buildIdmlFromRasters(PAGES);
    const zip = await JSZip.loadAsync(idml);

    for (let i = 0; i < PAGES.length; i++) {
      const spreadXml = await zip
        .file(`Spreads/Spread_p${i}.xml`)!
        .async("string");
      expect(spreadXml).toContain("<Rectangle");
      expect(spreadXml).toContain("<Image");

      const m = spreadXml.match(
        /<Contents><!\[CDATA\[([\s\S]*?)\]\]><\/Contents>/,
      );
      expect(m, `page ${i} must have inline <Contents>`).not.toBeNull();
      const decoded = new Uint8Array(Buffer.from(m![1], "base64"));
      expect(Array.from(decoded)).toEqual(Array.from(PAGES[i].pngBytes));
    }
  });

  it("writes a cross-verify sample to /tmp/pdf-fallback-sample.idml", async () => {
    const idml = await buildIdmlFromRasters(PAGES);
    writeFileSync("/tmp/pdf-fallback-sample.idml", idml);
    expect(idml.length).toBeGreaterThan(0);
  });
});
