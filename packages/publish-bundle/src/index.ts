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

// @paged-media/publish — the paged.publish plugin bundle (ADR-022 Phase 4):
// IDML import (native-document door) + IDML export (engine escape hatch).

export { publishBundle, activate } from "./activate";
export {
  contributeIdmlIo,
  IDML_IMPORTER_ID,
  IDML_EXPORTER_ID,
  IDML_MIME,
} from "./io/idml";
