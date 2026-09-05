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

//! Opt-in probe: `PAGED_DIFF_PACKAGE=<path>.idml` — open the package,
//! export it unmutated, and report the first entry that is not
//! byte-identical with 300 bytes of context. A developer's diff tool for
//! "which part churns on an unmutated save", nothing more; skipped
//! unless the variable is set.

#[path = "support/pkg.rs"]
mod pkg;

#[test]
fn unmutated_export_of_the_named_package_is_entry_identical() {
    let Some(path) = std::env::var_os("PAGED_DIFF_PACKAGE") else {
        eprintln!("SKIP: PAGED_DIFF_PACKAGE unset");
        return;
    };
    let source = std::fs::read(&path).expect("read package");
    let doc = pkg::open(&source);
    let out = idml_export::write_idml(&doc, &source).expect("write");
    pkg::assert_same_entries(&source, &out);
}
