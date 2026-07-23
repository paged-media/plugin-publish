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

// paged.pdf — the `.pdf` importer.
//
// Mirrors the publish-bundle IDML importer's shape: a `.pdf` importer
// registered through the public contribution surface, routed to the
// native-document door. Two paths, best-first:
//
//   Phase 1 (editable) — reconstruct the PDF into an editable native document:
//     pdf.js text extraction → the reconstruct heuristics → a Document IR →
//     the `pdf-import` wasm mapper (`loadPdfMapper`) → `.paged` bytes. Text
//     pages come in as editable text frames; image/scanned pages keep a raster
//     background (per-page confidence gate). Used when the wasm mapper loads.
//
//   Phase 0 (image fallback) — rasterize every page and wrap the PNGs in a
//     minimal inline-image IDML (`rasterizePdf` + `buildIdmlFromRasters`).
//     Used when the mapper can't load or the reconstruction throws, so a PDF
//     always opens *something* real.
//
// Either way the engine owns the final parse via `host.nativeDocument.open`.
// Capability-gated on `document.openNative@1`; a failure fails LOUD in the log
// rather than throwing past the host (the mutate-never-throws convention).

import type {
  BundleHost,
  Disposable,
  ImportRequest,
} from "@paged-media/plugin-api";

import { rasterizePdf } from "../raster";
import { buildIdmlFromRasters } from "../idml-fallback";
import { reconstructPdf } from "../reconstruct";
import { loadPdfMapper } from "../engine-loader";

export const PDF_IMPORTER_ID = "media.paged.pdf.importer.pdf";
export const PDF_MIME = "application/pdf";

// ---------------------------------------------------------- importer

/**
 * Import an opened `.pdf` file: rasterize its pages, wrap them in a minimal
 * IDML package, and hand that to the native-document door (the engine owns
 * the IDML→native parse + inline-image resolve). Capability-gated on
 * `document.openNative@1`; when the door isn't wired we warn and return
 * rather than throw. Loading is destructive, so a failure fails LOUD in the
 * log (never a throw past the host).
 */
async function importPdf(host: BundleHost, file: ImportRequest): Promise<void> {
  if (!host.supports("document.openNative@1")) {
    host.log.warn(
      `${PDF_IMPORTER_ID}: host predates document.openNative@1 — ${file.name} not opened`,
    );
    return;
  }
  try {
    // Phase 1 — try the editable reconstruction first (wasm mapper present).
    const editable = await tryEditableImport(host, file.bytes);
    if (editable) {
      await host.nativeDocument.open(editable.paged);
      host.log.info(
        `${PDF_IMPORTER_ID}: opened ${file.name} as an editable document (${editable.pageCount} page(s))`,
      );
      return;
    }
    // Phase 0 — image-only fallback (mapper unavailable or reconstruction failed).
    const pages = await rasterizePdf(file.bytes);
    const idml = await buildIdmlFromRasters(pages);
    await host.nativeDocument.open(idml);
    host.log.info(
      `${PDF_IMPORTER_ID}: opened ${file.name} as ${pages.length} image page(s) (image fallback)`,
    );
  } catch (err) {
    host.log.error(
      `${PDF_IMPORTER_ID}: failed to open ${file.name} — ${String(err)}`,
    );
  }
}

/** Attempt the Phase 1 editable path: reconstruct → wasm map → `.paged`.
 *  Returns `null` (never throws) when the wasm mapper isn't loaded or the
 *  reconstruction/mapping fails, so the caller cleanly falls back to Phase 0. */
async function tryEditableImport(
  host: BundleHost,
  bytes: Uint8Array,
): Promise<{ paged: Uint8Array; pageCount: number } | null> {
  const mapper = await loadPdfMapper(host);
  if (!mapper) return null;
  try {
    const ir = await reconstructPdf(bytes);
    const paged = mapper.mapIrToPaged(ir);
    if (!paged) return null;
    return { paged, pageCount: ir.pages.length };
  } catch (err) {
    host.log.warn(
      `${PDF_IMPORTER_ID}: editable reconstruction failed (${String(err)}) — image fallback`,
    );
    return null;
  }
}

// ------------------------------------------------------- registration

/** Register the PDF importer through the K-2 door, capability-gated
 *  (degrades honestly when a host predates the door). Returns a Disposable
 *  dropping it. */
export function contributePdfIo(host: BundleHost): Disposable {
  const disposers: Disposable[] = [];
  if (host.supports("contribute.importer@1")) {
    disposers.push(
      host.contribute.importer({
        id: PDF_IMPORTER_ID,
        title: "PDF",
        extensions: [".pdf"],
        mimeTypes: [PDF_MIME],
        import: (file) => importPdf(host, file),
      }),
    );
  } else {
    host.log.warn(
      `${PDF_IMPORTER_ID}: host predates contribute.importer@1 — not registered`,
    );
  }
  return {
    dispose() {
      for (const d of disposers) d.dispose();
    },
  };
}
