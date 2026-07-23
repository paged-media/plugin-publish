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

// @paged-media/pdf — the paged.pdf plugin bundle (Phase 0: image-only PDF
// import, zero Rust/wasm).
//
// Registration happens HERE, through the public contribution surface: a PDF
// importer that rasterizes pages (pdf.js) and wraps them in a minimal IDML
// package routed to `host.nativeDocument.open` (the engine owns the parse +
// inline-image path). The host tracks the registration; disposing the bundle
// removes it cleanly.

import { defineBundle } from "@paged-media/plugin-sdk";
import type {
  BundleHandle,
  BundleHost,
  PluginManifest,
} from "@paged-media/plugin-api";

import manifestJson from "../manifest.json";

import { contributePdfIo } from "./io/pdf";

export function activate(host: BundleHost): BundleHandle {
  // Phase 0 — the PDF importer, capability-gated. Surfaces the engine's
  // EXISTING IDML/inline-image path (via the native-document door) through
  // the plugin registry.
  const pdfIoSub = contributePdfIo(host);
  host.log.info(
    `activated — PDF importer (apiVersion ${manifestJson.apiVersion})`,
  );
  // The IO registration is allocated outside a facade-tracked registration,
  // so dispose the handle here.
  return {
    dispose() {
      pdfIoSub.dispose();
    },
  };
}

export const pdfBundle = defineBundle({
  manifest: manifestJson as PluginManifest,
  activate,
});
