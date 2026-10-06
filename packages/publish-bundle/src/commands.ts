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

// paged.publish TYPED COMMANDS (plugin-sdk ADR 323): import and export as
// commands whose arguments and results carry value types, so the Node CLI
// (`paged export --format pdf …`), Boa scripts and a generated options form
// call them with validated args — `host.objects.invoke` checks the args
// against the schema before the handler runs, and never coerces.
//
//   media.paged.publish.exportPdf    PDF options          → ExportResult
//   media.paged.publish.exportIdml   { linkBase? }        → ExportResult
//   media.paged.publish.exportPaged  {}                   → ExportResult
//   media.paged.publish.importIdml   { bytes, name? }     → { pages }
//
// Every exporter answers ONE result shape (`schema/export-result.json`), so
// a caller writes `bytes` to `fileName` and reports `diagnostics` without
// knowing the format. The plugin has no editable state of its own, so it
// contributes no object kinds.
//
// Struct fields are `optional` or carry a `default` (SDK 0.2.43,
// `StructField`): `host.objects.invoke` fills the defaults before the
// handler runs, so a caller passes only what it means (`{}` exports every
// page with the defaults); an absent optional field means "not set", never a
// stand-in value. `PDF_EXPORT_DEFAULTS` is read from the schema file.

import type {
  BundleHost,
  ObjectModelHandle,
  TypedCommandContribution,
  ValueType,
} from "@paged-media/plugin-api";

import exportResult from "../schema/export-result.json";
import exportPdfArgs from "../schema/export-pdf.args.json";
import exportIdmlArgs from "../schema/export-idml.args.json";
import exportPagedArgs from "../schema/export-paged.args.json";
import importIdmlArgs from "../schema/import-idml.args.json";
import importResult from "../schema/import-result.json";

export const EXPORT_PDF_COMMAND = "media.paged.publish.exportPdf";
export const EXPORT_IDML_COMMAND = "media.paged.publish.exportIdml";
export const EXPORT_PAGED_COMMAND = "media.paged.publish.exportPaged";
export const IMPORT_IDML_COMMAND = "media.paged.publish.importIdml";

export const EXPORT_RESULT_TYPE = exportResult as ValueType;
export const EXPORT_PDF_ARGS_TYPE = exportPdfArgs as ValueType;

/** The engine's `exportPdfBegin` options (wire `ExportPdfWireOptions`,
 *  which plugin-api does not re-export). */
export interface PdfWireOptions {
  standard: string | null;
  outputIntentProfile: string | null;
  outputCondition: string | null;
  colorPolicy: string | null;
  pageFrom: number | null;
  pageTo: number | null;
  cropMarks: boolean;
  registrationMarks: boolean;
  colorBars: boolean;
  pageInfo: boolean;
  marksOffsetPt: number | null;
  bleedOverridePt: [number, number, number, number] | null;
  downsamplePpi: number | null;
  effectDpi: number | null;
  restrictedFontPolicy: string | null;
  title: string | null;
}

/** One exporter diagnostic; `page` is 1-based, 0 = the whole document. */
export interface ExportDiagnostic {
  severity: "error" | "warning" | "info";
  code: string;
  message: string;
  page: number;
}

/** What every exporter command answers (`schema/export-result.json`). */
export interface ExportResult {
  bytes: Uint8Array;
  fileName: string;
  mimeType: string;
  pageCount: number;
  diagnostics: ExportDiagnostic[];
}

/** `media.paged.publish.exportPdf`'s args (`schema/export-pdf.args.json`)
 *  as the handler sees them: `host.objects.invoke` has filled every
 *  defaulted field; an OPTIONAL field is absent when not given, which means
 *  "not set" (the engine's or the document's own value) — never a stand-in. */
export interface PdfExportArgs {
  /** `pdf17` (PDF 1.7) or `pdfx4` (PDF/X-4; needs `outputIntentProfile`). */
  standard: "pdf17" | "pdfx4";
  /** 1-based page range: `"all"`, `"3"`, `"2-5"`; absent = every page. */
  pages?: string;
  /** Output-intent profile name; absent = the working CMYK space. */
  outputIntentProfile?: string;
  /** Output condition text for the OutputIntent; absent = none. */
  outputCondition?: string;
  colorPolicy: "preserveNumbers" | "convertToDestination";
  cropMarks: boolean;
  registrationMarks: boolean;
  colorBars: boolean;
  pageInfo: boolean;
  /** Printer's-marks offset from the trim, pt (0–72); absent = the engine's default. */
  marksOffset?: number;
  /** [top, inside, bottom, outside] pt; absent = the document's bleed. */
  bleed?: number[];
  /** Resample images above this effective ppi (1–2400); absent = never. */
  downsamplePpi?: number;
  /** Raster resolution of effect soft masks, ppi. */
  effectDpi: number;
  restrictedFontPolicy: "outline" | "fail";
  /** Document title (Info / XMP); absent = none. */
  title?: string;
}

/** The fields the schema DEFAULTS (every other field is optional). */
export type PdfExportDefaults = Pick<
  PdfExportArgs,
  "standard" | "colorPolicy" | "cropMarks" | "registrationMarks" | "colorBars" | "pageInfo" | "effectDpi" | "restrictedFontPolicy"
>;

/** The schema's struct-field defaults, read from `schema/export-pdf.args.json`
 *  (the ONE source the manifest, the CLI's options form and
 *  `host.objects.invoke` all see). */
export const PDF_EXPORT_DEFAULTS: Readonly<PdfExportDefaults> = Object.freeze(
  Object.fromEntries(
    Object.entries((exportPdfArgs as { fields: Record<string, { default?: unknown }> }).fields)
      .filter(([, f]) => "default" in f)
      .map(([k, f]) => [k, f.default]),
  ) as unknown as PdfExportDefaults,
);

/** The defaults with `overrides` applied: a complete, valid args record
 *  (what `invoke` hands the handler for the same partial args). */
export function pdfExportArgs(overrides: Partial<PdfExportArgs> = {}): PdfExportArgs {
  return { ...PDF_EXPORT_DEFAULTS, ...overrides, ...(overrides.bleed ? { bleed: [...overrides.bleed] } : {}) };
}

/** A refusal the registry reports as `invalidValue: …` (the code prefix
 *  `host.objects.invoke` uses for every refusal). */
function invalid(reason: string): Error {
  return new Error(`invalidValue: ${reason}`);
}

/** Parse a 1-based page range into the engine's 0-based inclusive one. */
export function parsePageRange(pages: string): { pageFrom: number | null; pageTo: number | null } {
  const t = pages.trim().toLowerCase();
  if (t === "" || t === "all") return { pageFrom: null, pageTo: null };
  const m = /^(\d+)(?:\s*-\s*(\d+))?$/.exec(t);
  if (!m) throw invalid(`pages "${pages}" is not "all", "n" or "a-b"`);
  const from = Number(m[1]);
  const to = m[2] === undefined ? from : Number(m[2]);
  if (from < 1 || to < from) throw invalid(`pages "${pages}" is not an ascending 1-based range`);
  return { pageFrom: from - 1, pageTo: to - 1 };
}

/** The checked args as the engine's wire options. Ranges a struct field
 *  cannot carry (ADR 132 puts `range` on rows) are checked here. */
export function pdfWireOptions(args: PdfExportArgs): PdfWireOptions {
  const { bleed, marksOffset, downsamplePpi } = args;
  if (bleed !== undefined && bleed.length !== 4) throw invalid("bleed is [top, inside, bottom, outside]");
  if (bleed?.some((b) => b < 0 || b > 432)) throw invalid("bleed values are 0–432 pt");
  if (marksOffset !== undefined && (marksOffset < 0 || marksOffset > 72)) throw invalid("marksOffset is 0–72 pt");
  if (downsamplePpi !== undefined && (downsamplePpi < 1 || downsamplePpi > 2400)) {
    throw invalid("downsamplePpi is 1–2400 (absent = never resample)");
  }
  if (args.effectDpi < 36 || args.effectDpi > 1200) throw invalid("effectDpi is 36–1200");
  const text = (s: string | undefined): string | null => (s === undefined || s === "" ? null : s);
  const outputIntentProfile = text(args.outputIntentProfile);
  if (args.standard === "pdfx4" && outputIntentProfile === null) {
    throw invalid("pdfx4 needs an outputIntentProfile (a registered profile name)");
  }
  return {
    standard: args.standard,
    outputIntentProfile,
    outputCondition: text(args.outputCondition),
    colorPolicy: args.colorPolicy,
    ...parsePageRange(args.pages ?? ""),
    cropMarks: args.cropMarks,
    registrationMarks: args.registrationMarks,
    colorBars: args.colorBars,
    pageInfo: args.pageInfo,
    marksOffsetPt: marksOffset ?? null,
    bleedOverridePt: bleed ? (bleed as [number, number, number, number]) : null,
    downsamplePpi: downsamplePpi ?? null,
    effectDpi: args.effectDpi,
    restrictedFontPolicy: args.restrictedFontPolicy,
    title: text(args.title),
  };
}

function severityOf(s: string): ExportDiagnostic["severity"] {
  return s === "error" || s === "warning" ? s : "info";
}

/** The document's name for an output file, from the engine's meta. */
async function baseName(host: BundleHost): Promise<string> {
  try {
    const meta = (await host.document.meta()) as { documentName?: string | null; name?: string | null };
    const n = (meta.documentName ?? meta.name ?? "").trim();
    return n === "" ? "document" : n.replace(/\.(idml|paged|pdf)$/i, "");
  } catch {
    return "document";
  }
}

/** Export the active document to PDF through the engine's session door
 *  (begin → one page per call → finish), cancelling on failure. */
export async function exportPdf(host: BundleHost, args: PdfExportArgs): Promise<ExportResult> {
  const options = pdfWireOptions(args);
  const client = host.editor.client;
  const begun = await client.send({ kind: "exportPdfBegin", payload: { options } });
  if (begun.kind === "exportPdfFailed") throw new Error(`failed: ${begun.payload.error}`);
  if (begun.kind !== "exportPdfBegun") throw new Error(`failed: unexpected reply ${begun.kind}`);
  const { session, pageCount } = begun.payload;
  try {
    for (let i = 0; i < pageCount; i++) {
      const r = await client.send({ kind: "exportPdfPage", payload: { session } });
      if (r.kind === "exportPdfFailed") throw new Error(`failed: ${r.payload.error}`);
    }
    const done = await client.send({ kind: "exportPdfFinish", payload: { session } });
    if (done.kind === "exportPdfFailed") throw new Error(`failed: ${done.payload.error}`);
    if (done.kind !== "pdfExported") throw new Error(`failed: unexpected reply ${done.kind}`);
    const findings = done.payload.findings ?? [];
    const diagnostics: ExportDiagnostic[] = [
      ...findings.map((f) => ({
        severity: severityOf(f.severity),
        code: f.code,
        message: f.message,
        page: f.pageIndex == null ? 0 : f.pageIndex + 1,
      })),
      // The flat list repeats findings as text; keep only what it adds.
      ...done.payload.diagnostics
        .filter((d) => !findings.some((f) => d.includes(f.message)))
        .map((message) => ({ severity: "info" as const, code: "pdf", message, page: 0 })),
    ];
    return {
      bytes: Uint8Array.from(done.payload.pdfBytes),
      fileName: `${await baseName(host)}.pdf`,
      mimeType: "application/pdf",
      pageCount,
      diagnostics,
    };
  } catch (err) {
    await client.send({ kind: "exportPdfCancel", payload: { session } }).catch(() => undefined);
    throw err;
  }
}

/** Export the active document to IDML through the engine's writer. Every
 *  loss the engine reports becomes a warning (the exporter used to drop
 *  them). */
export async function exportIdmlResult(host: BundleHost, linkBase?: string): Promise<ExportResult> {
  const reply = await host.editor.client.send({
    kind: "exportIdml",
    payload: { linkBase: linkBase === undefined || linkBase === "" ? null : linkBase },
  });
  if (reply.kind === "exportIdmlFailed") throw new Error(`failed: ${reply.payload.error}`);
  if (reply.kind !== "idmlExported") throw new Error(`failed: unexpected reply ${reply.kind}`);
  const pages = await host.document.collection("pages");
  return {
    bytes: Uint8Array.from(reply.payload.idmlBytes),
    fileName: `${await baseName(host)}.idml`,
    mimeType: "application/vnd.adobe.indesign-idml-package",
    pageCount: pages.length,
    diagnostics: (reply.payload.lost ?? []).map((message) => ({ severity: "warning" as const, code: "idml.lost", message, page: 0 })),
  };
}

/** Export the active document as a `.paged` container. */
async function exportPagedResult(host: BundleHost): Promise<ExportResult> {
  const bytes = host.supports("documents.exportPaged@1")
    ? await host.documents.exportPaged()
    : await (async () => {
        const reply = await host.editor.client.send({ kind: "exportPaged", payload: {} });
        if (reply.kind === "pagedPartFailed") throw new Error(`failed: ${reply.payload.error}`);
        if (reply.kind !== "pagedExported") throw new Error(`failed: unexpected reply ${reply.kind}`);
        return Uint8Array.from(reply.payload.bytes);
      })();
  const pages = await host.document.collection("pages");
  return { bytes, fileName: `${await baseName(host)}.paged`, mimeType: "application/zip", pageCount: pages.length, diagnostics: [] };
}

/** Open IDML bytes as the active document: the native-document door (the
 *  editor), else the documents door (headless hosts, the Node CLI). */
export async function openIdml(host: BundleHost, bytes: Uint8Array, name: string): Promise<string[]> {
  if (host.supports("document.openNative@1")) {
    await host.nativeDocument.open(bytes);
  } else if (host.supports("documents.open@1")) {
    const r = await host.documents.open(bytes, { name });
    if (!r.opened) throw new Error(`failed: the host declined to open ${name}`);
  } else {
    throw new Error("failed: this host cannot open a document");
  }
  return (await host.document.collection<{ selfId: string }>("pages")).map((p) => p.selfId);
}

/** Register the typed commands. `null` on a host without the door. */
export function contributePublishCommands(host: BundleHost): ObjectModelHandle | null {
  if (!host.supports("contribute.objectModel@1")) return null;
  const commands: TypedCommandContribution[] = [
    {
      id: EXPORT_PDF_COMMAND,
      title: "Export PDF",
      args: EXPORT_PDF_ARGS_TYPE,
      result: EXPORT_RESULT_TYPE,
      handler: (_ctx, args) => exportPdf(host, args as PdfExportArgs),
    },
    {
      id: EXPORT_IDML_COMMAND,
      title: "Export IDML",
      args: exportIdmlArgs as ValueType,
      result: EXPORT_RESULT_TYPE,
      handler: (_ctx, args) => exportIdmlResult(host, (args as { linkBase?: string }).linkBase),
    },
    {
      id: EXPORT_PAGED_COMMAND,
      title: "Save as .paged",
      args: exportPagedArgs as ValueType,
      result: EXPORT_RESULT_TYPE,
      handler: () => exportPagedResult(host),
    },
    {
      id: IMPORT_IDML_COMMAND,
      title: "Open IDML",
      args: importIdmlArgs as ValueType,
      result: importResult as ValueType,
      async handler(_ctx, args) {
        const { bytes, name } = args as { bytes: Uint8Array; name: string };
        const pages = await openIdml(host, bytes, name);
        return { pages: pages.map((id) => `page:${id}`) };
      },
    },
  ];
  return host.contribute.objectModel({ kinds: [], commands });
}
