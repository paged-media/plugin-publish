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

// paged.pdf TYPED COMMAND (plugin-sdk ADR 323): `media.paged.pdf.importPdf`
// opens a PDF with validated options, so the Node CLI and scripts choose the
// editable or the image path, the page cap and the raster resolution — the
// importer (File ▸ Open) runs the same code with `PDF_IMPORT_DEFAULTS`.
// A PDF has no editable model of its own once opened (it becomes a native
// document), so the plugin contributes no object kinds.

import type { BundleHost, ObjectModelHandle, TypedCommandContribution, ValueType } from "@paged-media/plugin-api";

import importPdfArgs from "../schema/import-pdf.args.json";
import importPdfResult from "../schema/import-pdf.result.json";

import { openPdf, type PdfImportOptions } from "./io/pdf";

export const IMPORT_PDF_COMMAND = "media.paged.pdf.importPdf";
export const IMPORT_PDF_ARGS_TYPE = importPdfArgs as ValueType;
export const IMPORT_PDF_RESULT_TYPE = importPdfResult as ValueType;

export interface PdfImportArgs extends PdfImportOptions {
  bytes: Uint8Array;
  name: string;
}

/** The door that replaces the active document: the native one (the
 *  editor), else the documents door (headless hosts, the Node CLI). */
function openDoor(host: BundleHost, name: string): ((bytes: Uint8Array) => Promise<void>) | null {
  if (host.supports("document.openNative@1")) return (bytes) => host.nativeDocument.open(bytes);
  if (host.supports("documents.open@1")) {
    return async (bytes) => {
      const r = await host.documents.open(bytes, { name });
      if (!r.opened) throw new Error(`failed: the host declined to open ${name}`);
    };
  }
  return null;
}

export function contributePdfCommands(host: BundleHost): ObjectModelHandle | null {
  if (!host.supports("contribute.objectModel@1")) return null;
  const importPdf: TypedCommandContribution<PdfImportArgs, { mode: "editable" | "image"; pageCount: number }> = {
    id: IMPORT_PDF_COMMAND,
    title: "Open PDF",
    args: IMPORT_PDF_ARGS_TYPE,
    result: IMPORT_PDF_RESULT_TYPE,
    async handler(_ctx, args) {
      const open = openDoor(host, args.name);
      if (!open) throw new Error("failed: this host cannot open a document");
      return openPdf(host, args.bytes, { mode: args.mode, maxPages: args.maxPages, dpi: args.dpi }, open);
    },
  };
  return host.contribute.objectModel({ kinds: [], commands: [importPdf as TypedCommandContribution] });
}
