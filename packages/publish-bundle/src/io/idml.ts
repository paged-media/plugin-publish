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

// ADR-022 Phase 4 — paged.publish IDML interchange (the plugin-contributed
// importer + exporter that surface the engine's EXISTING IDML path through
// the plugin registry).
//
// Under ADR-022 Option A the engine (canvas-wasm) still performs the actual
// IDML parse/write via the out-of-repo adapter crates; this bundle does NOT
// re-implement IDML. It only routes the file/registry flow to the engine:
//
//   · The IMPORTER claims `.idml`: it hands the opened bytes to the isolate-
//     safe native-document door (`host.nativeDocument.open`), which loads the
//     package as the active document (the engine owns the IDML→native parse).
//     Capability-gated on `document.openNative@1`.
//
//   · The EXPORTER produces `.idml`: it reuses the engine's existing IDML
//     serializer through the `host.editor` escape hatch (`client.send`
//     `{ kind: "exportIdml" }` → `idmlExported`). This escape hatch is used
//     DELIBERATELY as a transitional bridge (see exportIdml below).

import type {
  BundleHost,
  Disposable,
  ImportRequest,
  ExportResult,
} from "@paged-media/plugin-api";

export const IDML_IMPORTER_ID = "media.paged.publish.importer.idml";
export const IDML_EXPORTER_ID = "media.paged.publish.exporter.idml";
export const IDML_MIME = "application/vnd.adobe.indesign-idml-package";

// ---------------------------------------------------------- importer

/**
 * Import an opened `.idml` file by routing its bytes to the native-document
 * door — the engine performs the IDML→native parse and replaces the active
 * document (ADR-022 Option A: the engine owns the conversion; this bundle only
 * surfaces the registry entry). Capability-gated on `document.openNative@1`;
 * when the door isn't wired we warn and return rather than throw. Loading is
 * destructive, so a failure fails LOUD in the log (never a throw past the host
 * — the mutate-never-throws convention).
 */
async function importIdml(host: BundleHost, file: ImportRequest): Promise<void> {
  if (!host.supports("document.openNative@1")) {
    host.log.warn(
      `${IDML_IMPORTER_ID}: host predates document.openNative@1 — ${file.name} not opened`,
    );
    return;
  }
  try {
    await host.nativeDocument.open(file.bytes);
    host.log.info(`${IDML_IMPORTER_ID}: opened ${file.name} as the active document`);
  } catch (err) {
    host.log.error(
      `${IDML_IMPORTER_ID}: failed to open ${file.name} — ${String(err)}`,
    );
  }
}

// ---------------------------------------------------------- exporter

/**
 * Export the active document to IDML by reusing the engine's EXISTING IDML
 * serializer through the `host.editor` escape hatch (`client.send`
 * `{ kind: "exportIdml" }`). The engine owns the native→IDML conversion under
 * ADR-022 Option A, so the escape hatch is used DELIBERATELY here as a
 * transitional bridge — ADR-022 Phase 5 retires it once a first-class export
 * door (or the adapter-wasm path) lands. Returns null when the engine reports
 * anything other than a successful `idmlExported` reply (nothing to save).
 */
async function exportIdml(host: BundleHost): Promise<ExportResult | null> {
  const reply = await host.editor.client.send({ kind: "exportIdml", payload: {} });
  if (reply.kind !== "idmlExported") return null;
  return {
    bytes: Uint8Array.from(reply.payload.idmlBytes),
    fileName: "document.idml",
  };
}

// ------------------------------------------------------- registration

/** Register both the IDML importer and exporter through the K-2 doors,
 *  capability-gated (degrades honestly when a host predates the door).
 *  Returns a Disposable dropping both. */
export function contributeIdmlIo(host: BundleHost): Disposable {
  const disposers: Disposable[] = [];
  if (host.supports("contribute.importer@1")) {
    disposers.push(
      host.contribute.importer({
        id: IDML_IMPORTER_ID,
        title: "IDML package",
        extensions: [".idml"],
        mimeTypes: [IDML_MIME],
        import: (file) => importIdml(host, file),
      }),
    );
  } else {
    host.log.warn(
      `${IDML_IMPORTER_ID}: host predates contribute.importer@1 — not registered`,
    );
  }
  if (host.supports("contribute.exporter@1")) {
    disposers.push(
      host.contribute.exporter({
        id: IDML_EXPORTER_ID,
        title: "IDML package",
        extension: ".idml",
        mimeType: IDML_MIME,
        export: () => exportIdml(host),
      }),
    );
  } else {
    host.log.warn(
      `${IDML_EXPORTER_ID}: host predates contribute.exporter@1 — not registered`,
    );
  }
  return {
    dispose() {
      for (const d of disposers) d.dispose();
    },
  };
}
