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

// The publish bundle's typed commands through `host.objects.invoke`, on the
// real engine (the published canvas-wasm, headless) — the path the Node CLI
// takes for `paged export --format pdf|idml|paged` and `paged import`.
// Args are validated against the bundle's schema before the handler runs
// (never coerced), and every exporter answers the one ExportResult shape.

import { readFileSync } from "node:fs";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { applyDefaults, createHeadlessHost, validateValue, type HeadlessHost } from "@paged-media/plugin-sdk";
import type { ValueType } from "@paged-media/plugin-api";

import { publishBundle } from "../src/activate";
import {
  EXPORT_IDML_COMMAND,
  EXPORT_PAGED_COMMAND,
  EXPORT_PDF_ARGS_TYPE,
  EXPORT_PDF_COMMAND,
  EXPORT_RESULT_TYPE,
  IMPORT_IDML_COMMAND,
  PDF_EXPORT_DEFAULTS,
  type ExportResult,
  parsePageRange,
  pdfExportArgs,
  pdfWireOptions,
  type PdfExportArgs,
} from "../src/commands";
import importResult from "../schema/import-result.json";
import exportPdfArgsFile from "../schema/export-pdf.args.json";
import exportIdmlArgsFile from "../schema/export-idml.args.json";
import importIdmlArgsFile from "../schema/import-idml.args.json";

const IDML = new URL("../../../crates/idml-export/tests/fixtures/indesign-20.0.1-navigation.idml", import.meta.url);

let h: HeadlessHost;
let pageCount: number;

beforeEach(async () => {
  h = await createHeadlessHost();
  h.loadBundle(publishBundle);
  const out = (await h.objects.invoke(IMPORT_IDML_COMMAND, {
    bytes: new Uint8Array(readFileSync(IDML)),
    name: "navigation",
  })) as { pages: string[] };
  expect(validateValue(importResult as ValueType, out)).toBeNull();
  pageCount = out.pages.length;
}, 60_000);

afterEach(() => h.dispose());

const head = (b: Uint8Array, n: number) => new TextDecoder().decode(b.subarray(0, n));

describe("[plugin-publish.typed-commands] import and export as typed commands", () => {
  it("lists four typed commands carrying the schema files' types", async () => {
    const mine = (await h.objects.commands()).filter((c) => c.owner === "media.paged.publish");
    expect(mine.map((c) => c.id).sort()).toEqual(
      [EXPORT_IDML_COMMAND, EXPORT_PAGED_COMMAND, EXPORT_PDF_COMMAND, IMPORT_IDML_COMMAND].sort(),
    );
    const pdf = mine.find((c) => c.id === EXPORT_PDF_COMMAND)!;
    expect(pdf.args).toEqual(EXPORT_PDF_ARGS_TYPE);
    expect(pdf.result).toEqual(EXPORT_RESULT_TYPE);
  });

  it("importIdml opens the document and answers its pages", async () => {
    expect(pageCount).toBeGreaterThan(0);
    expect(await h.objects.query("page")).toHaveLength(pageCount);
  });

  it("exportPdf with the defaults writes a PDF of every page in the ExportResult shape", async () => {
    expect(validateValue(EXPORT_PDF_ARGS_TYPE, { ...PDF_EXPORT_DEFAULTS })).toBeNull();
    const out = (await h.objects.invoke(EXPORT_PDF_COMMAND, pdfExportArgs())) as ExportResult;
    expect(validateValue(EXPORT_RESULT_TYPE, out)).toBeNull();
    expect(head(out.bytes, 5)).toBe("%PDF-");
    expect(out.mimeType).toBe("application/pdf");
    expect(out.fileName).toMatch(/\.pdf$/);
    expect(out.pageCount).toBe(pageCount);
  }, 60_000);

  it("exportPdf honours a page range and a title", async () => {
    const one = (await h.objects.invoke(EXPORT_PDF_COMMAND, pdfExportArgs({ pages: "1", title: "One page" }))) as ExportResult;
    expect(head(one.bytes, 5)).toBe("%PDF-");
    expect(one.pageCount).toBe(1);
    expect(new TextDecoder("latin1").decode(one.bytes)).toContain("One page");
    // PDF/X-4 without an output intent is refused before the engine runs.
    await expect(h.objects.invoke(EXPORT_PDF_COMMAND, pdfExportArgs({ standard: "pdfx4" }))).rejects.toThrow(/invalidValue/);
  }, 60_000);

  it("refuses args outside the schema, or a range the schema cannot carry — never coerced", async () => {
    await expect(h.objects.invoke(EXPORT_PDF_COMMAND, { pages: 3 })).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(EXPORT_PDF_COMMAND, pdfExportArgs({ standard: "pdf20" as never }))).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(EXPORT_PDF_COMMAND, { ...pdfExportArgs(), cropMarks: "yes" })).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(EXPORT_PDF_COMMAND, pdfExportArgs({ pages: "3-1" }))).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(EXPORT_PDF_COMMAND, pdfExportArgs({ bleed: [9, 9] }))).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(EXPORT_PDF_COMMAND, { downsamplePpi: 0 })).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(IMPORT_IDML_COMMAND, {})).rejects.toThrow(/invalidValue/);
    await expect(h.objects.invoke(IMPORT_IDML_COMMAND, { bytes: "x", name: "x" })).rejects.toThrow(/invalidValue/);
  });

  it("exportIdml and exportPaged answer the same shape; the .paged reopens", async () => {
    const idml = (await h.objects.invoke(EXPORT_IDML_COMMAND, {})) as ExportResult;
    expect(validateValue(EXPORT_RESULT_TYPE, idml)).toBeNull();
    expect(head(idml.bytes, 2)).toBe("PK");
    expect(idml.fileName).toMatch(/\.idml$/);
    expect(idml.pageCount).toBe(pageCount);
    const paged = (await h.objects.invoke(EXPORT_PAGED_COMMAND, {})) as ExportResult;
    expect(validateValue(EXPORT_RESULT_TYPE, paged)).toBeNull();
    expect(await h.load(paged.bytes)).toHaveLength(pageCount);
  }, 60_000);

  it("the File ▸ Export exporter shares the typed command's path", async () => {
    const [exporter] = h.exportersContributed();
    const r = await exporter!.export();
    expect(r).not.toBeNull();
    expect(head(r!.bytes, 2)).toBe("PK");
    expect(r!.fileName).toMatch(/\.idml$/);
  });
});

describe("[plugin-publish.typed-commands] page ranges", () => {
  it("maps 1-based ranges onto the engine's 0-based inclusive ones", () => {
    expect(parsePageRange("")).toEqual({ pageFrom: null, pageTo: null });
    expect(parsePageRange("all")).toEqual({ pageFrom: null, pageTo: null });
    expect(parsePageRange("3")).toEqual({ pageFrom: 2, pageTo: 2 });
    expect(parsePageRange("2 - 5")).toEqual({ pageFrom: 1, pageTo: 4 });
    expect(() => parsePageRange("0")).toThrow(/invalidValue/);
    expect(() => parsePageRange("a-b")).toThrow(/invalidValue/);
  });
});

describe("[plugin-publish.typed-commands] partial args: struct defaults and optional fields (SDK 0.2.43)", () => {
  type Fields = Record<string, { default?: unknown; optional?: true }>;
  const pdfFields = (exportPdfArgsFile as { fields: Fields }).fields;

  it("PDF_EXPORT_DEFAULTS is exactly the schema file's defaults; every other field is optional", () => {
    const defaults = Object.fromEntries(
      Object.entries(pdfFields).filter(([, f]) => "default" in f).map(([k, f]) => [k, f.default]),
    );
    expect(PDF_EXPORT_DEFAULTS).toEqual(defaults);
    for (const [k, f] of Object.entries(pdfFields)) expect("default" in f || f.optional === true, k).toBe(true);
    expect(applyDefaults(EXPORT_PDF_ARGS_TYPE, {})).toEqual(PDF_EXPORT_DEFAULTS);
    expect(validateValue(EXPORT_PDF_ARGS_TYPE, {})).toBeNull();
    // The other commands: linkBase optional, import's bytes required, name defaulted.
    expect((exportIdmlArgsFile as { fields: Fields }).fields.linkBase!.optional).toBe(true);
    const imp = (importIdmlArgsFile as { fields: Fields }).fields;
    expect("default" in imp.bytes! || imp.bytes!.optional).toBeFalsy();
    expect(imp.name!.default).toBe("document.idml");
  });

  it("an absent optional field is 'not set' on the wire, never a stand-in value", () => {
    const wire = pdfWireOptions(applyDefaults(EXPORT_PDF_ARGS_TYPE, {}) as PdfExportArgs);
    expect(wire).toMatchObject({
      standard: "pdf17",
      outputIntentProfile: null,
      outputCondition: null,
      pageFrom: null,
      pageTo: null,
      marksOffsetPt: null,
      bleedOverridePt: null,
      downsamplePpi: null,
      effectDpi: 150,
      title: null,
    });
    // A present marksOffset of 0 is a real offset, not "the engine's default".
    expect(pdfWireOptions({ ...PDF_EXPORT_DEFAULTS, marksOffset: 0 }).marksOffsetPt).toBe(0);
  });

  it("exportPdf with {} — the defaults fill in — writes every page", async () => {
    const out = (await h.objects.invoke(EXPORT_PDF_COMMAND, {})) as ExportResult;
    expect(head(out.bytes, 5)).toBe("%PDF-");
    expect(out.pageCount).toBe(pageCount);
  }, 60_000);

  it("exportPdf with only { pages, title }", async () => {
    const out = (await h.objects.invoke(EXPORT_PDF_COMMAND, { pages: "1", title: "Partial" })) as ExportResult;
    expect(out.pageCount).toBe(1);
    expect(new TextDecoder("latin1").decode(out.bytes)).toContain("Partial");
  }, 60_000);

  it("importIdml with only the bytes (name defaults)", async () => {
    const out = (await h.objects.invoke(IMPORT_IDML_COMMAND, { bytes: new Uint8Array(readFileSync(IDML)) })) as { pages: string[] };
    expect(out.pages).toHaveLength(pageCount);
  }, 60_000);
});
