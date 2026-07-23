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

// paged.pdf — Phase 0 PDF importer (image-only, zero Rust/wasm).
//
// Mirrors the publish-bundle IDML importer's shape: a `.pdf` importer
// registered through the public contribution surface, routed to the
// native-document door. The DIFFERENCE is the conversion step — a PDF is not
// IDML, so before handing bytes to `host.nativeDocument.open` this importer:
//
//   1. rasterizes every page to a PNG with pdf.js (`rasterizePdf`), then
//   2. wraps the page PNGs in a minimal IDML package — one inline-image
//      `<Rectangle>` per page (`buildIdmlFromRasters`).
//
// The engine's EXISTING IDML importer + inline-image path then loads it as
// the active document. Capability-gated on `document.openNative@1`; a failure
// fails LOUD in the log rather than throwing past the host (the
// mutate-never-throws convention).

import type {
  BundleHost,
  Disposable,
  ImportRequest,
} from "@paged-media/plugin-api";

import { rasterizePdf } from "../raster";
import { buildIdmlFromRasters } from "../idml-fallback";

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
    const pages = await rasterizePdf(file.bytes);
    const idml = await buildIdmlFromRasters(pages);
    await host.nativeDocument.open(idml);
    host.log.info(
      `${PDF_IMPORTER_ID}: opened ${file.name} as ${pages.length} image page(s)`,
    );
  } catch (err) {
    host.log.error(
      `${PDF_IMPORTER_ID}: failed to open ${file.name} — ${String(err)}`,
    );
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
