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

// `media.paged.pdf.importPdf` through `host.objects.invoke` on the real
// engine (headless) and the real PDFium + pdf-import wasm under Node — the
// Node CLI's `paged import x.pdf` path. The PDF is the engine's own export
// of an InDesign-saved fixture, so the lane needs no binary PDF fixture.
// The image path rasterises through a canvas, which Node lacks: it must
// refuse honestly and leave the open document alone.

import { readFileSync } from "node:fs";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { createHeadlessHost, validateValue, type HeadlessHost } from "@paged-media/plugin-sdk";

import { pdfBundle } from "../src/activate";
import { IMPORT_PDF_ARGS_TYPE, IMPORT_PDF_COMMAND, IMPORT_PDF_RESULT_TYPE } from "../src/commands";
import { _resetPdfMapperCache } from "../src/engine-loader";

const IDML = new URL("../../../crates/idml-export/tests/fixtures/indesign-20.0.1-navigation.idml", import.meta.url);

let h: HeadlessHost;
let pdf: Uint8Array;
let sourcePages: number;

/** The engine's PDF of the loaded document (begin → pages → finish). */
async function enginePdf(): Promise<Uint8Array> {
  const send = h.host.editor.client.send.bind(h.host.editor.client);
  const begun = await send({ kind: "exportPdfBegin", payload: { options: {} } });
  if (begun.kind !== "exportPdfBegun") throw new Error(begun.kind);
  for (let i = 0; i < begun.payload.pageCount; i++) {
    await send({ kind: "exportPdfPage", payload: { session: begun.payload.session } });
  }
  const done = await send({ kind: "exportPdfFinish", payload: { session: begun.payload.session } });
  if (done.kind !== "pdfExported") throw new Error(done.kind);
  return Uint8Array.from(done.payload.pdfBytes);
}

beforeEach(async () => {
  _resetPdfMapperCache();
  h = await createHeadlessHost();
  h.loadBundle(pdfBundle);
  sourcePages = (await h.load(new Uint8Array(readFileSync(IDML)))).length;
  pdf = await enginePdf();
}, 60_000);

afterEach(() => h.dispose());

const args = (over: Record<string, unknown> = {}) => ({ bytes: pdf, name: "navigation.pdf", mode: "auto", maxPages: 0, dpi: 150, ...over });

describe("[plugin-pdf.typed-commands] importPdf as a typed command", () => {
  it("is listed with the schema files' args and result", async () => {
    const [c] = (await h.objects.commands()).filter((x) => x.owner === "media.paged.pdf");
    expect(c?.id).toBe(IMPORT_PDF_COMMAND);
    expect(c?.args).toEqual(IMPORT_PDF_ARGS_TYPE);
    expect(c?.result).toEqual(IMPORT_PDF_RESULT_TYPE);
  });

  it("opens a PDF as an editable document under Node, a page per PDF page", async () => {
    const out = await h.objects.invoke(IMPORT_PDF_COMMAND, args({ mode: "editable" }));
    expect(validateValue(IMPORT_PDF_RESULT_TYPE, out)).toBeNull();
    expect(out).toEqual({ mode: "editable", pageCount: sourcePages });
    expect(await h.objects.query("page")).toHaveLength(sourcePages);
    // (The headless engine has no fonts registered, so its PDF carries no
    // text to reconstruct; the editable text path is extract.test.ts's.)
  }, 60_000);

  it("honours the page cap", async () => {
    const out = (await h.objects.invoke(IMPORT_PDF_COMMAND, args({ maxPages: 1 }))) as { pageCount: number };
    expect(out.pageCount).toBe(1);
    expect(await h.objects.query("page")).toHaveLength(1);
  }, 60_000);

  it("refuses args outside the schema or its ranges, and opens nothing", async () => {
    const before = await h.objects.query("page");
    await expect(h.objects.invoke(IMPORT_PDF_COMMAND, args({ mode: "vector" }))).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(IMPORT_PDF_COMMAND, args({ dpi: 10 }))).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(IMPORT_PDF_COMMAND, args({ maxPages: -1 }))).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(IMPORT_PDF_COMMAND, { bytes: pdf, name: "x" })).rejects.toThrow(/invalidValue/);
    expect(await h.objects.query("page")).toEqual(before);
  });

  it("the image path needs a canvas: under Node it fails honestly and keeps the document", async () => {
    const before = await h.objects.query("page");
    await expect(h.objects.invoke(IMPORT_PDF_COMMAND, args({ mode: "image" }))).rejects.toThrow();
    expect(await h.objects.query("page")).toEqual(before);
  }, 60_000);
});
