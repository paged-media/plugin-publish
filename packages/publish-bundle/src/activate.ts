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

// @paged-media/publish — the paged.publish plugin bundle (ADR-022 Phase 4).
//
// Registration happens HERE, through the public contribution surface: an IDML
// importer (routed to `host.nativeDocument.open` — the engine owns the parse)
// and an IDML exporter (reusing the engine's existing serializer through the
// `host.editor` escape hatch, transitional per ADR-022 Phase 5). The host
// tracks both registrations; disposing the bundle removes them cleanly.

import { defineBundle } from "@paged-media/plugin-sdk";
import type {
  BundleHandle,
  BundleHost,
  PluginManifest,
} from "@paged-media/plugin-api";

import manifestJson from "../manifest.json";

import { contributeIdmlIo } from "./io/idml";

export function activate(host: BundleHost): BundleHandle {
  // ADR-022 Phase 4 — the IDML importer + exporter, capability-gated. Both
  // surface the engine's EXISTING IDML path (native-document door for import,
  // the `host.editor` escape hatch for export) through the plugin registry.
  const idmlIoSub = contributeIdmlIo(host);
  host.log.info(
    `activated — IDML importer + exporter (apiVersion ${manifestJson.apiVersion})`,
  );
  // The IO registrations are allocated outside a facade-tracked registration,
  // so dispose the handle here.
  return {
    dispose() {
      idmlIoSub.dispose();
    },
  };
}

export const publishBundle = defineBundle({
  manifest: manifestJson as PluginManifest,
  activate,
});
