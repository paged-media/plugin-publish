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

// @paged-media/pdf — the paged.pdf plugin bundle (Phase 0): image-only PDF
// import routed through the engine's IDML + inline-image path.

export { pdfBundle, activate } from "./activate";
export { contributePdfIo, PDF_IMPORTER_ID, PDF_MIME } from "./io/pdf";
export { rasterizePdf } from "./raster";
export { buildIdmlFromRasters } from "./idml-fallback";
export type { PdfPageRaster } from "./idml-fallback";
