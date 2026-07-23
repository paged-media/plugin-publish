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

// @paged-media/pdf — the paged.pdf plugin bundle. Phase 0: image-only PDF
// import (raster → inline-image IDML). Phase 1: editable reconstruction
// (pdf.js text → Document IR → the pdf-import wasm mapper → native .paged),
// with the image path as the fallback.

export { pdfBundle, activate } from "./activate";
export { contributePdfIo, PDF_IMPORTER_ID, PDF_MIME } from "./io/pdf";
export { rasterizePdf, renderPageToPng, ensureWorker } from "./raster";
export { buildIdmlFromRasters } from "./idml-fallback";
export type { PdfPageRaster } from "./idml-fallback";
// Phase 1 surface.
export { reconstructPdf } from "./reconstruct";
export type { ReconstructPdfOptions } from "./reconstruct";
export { loadPdfMapper, _resetPdfMapperCache } from "./engine-loader";
export type { PdfMapper, LoadMapperOptions } from "./engine-loader";
export {
  itemsToParagraphs,
  groupLines,
  lineToParagraph,
  textBBox,
  textCharCount,
  DEFAULT_OPTIONS,
} from "./extract";
export type { PositionedItem, ReconstructOptions } from "./extract";
export type {
  DocumentIr,
  PageIr,
  FrameIr,
  TextFrameIr,
  ImageFrameIr,
  ParagraphIr,
  RunIr,
} from "./ir";
