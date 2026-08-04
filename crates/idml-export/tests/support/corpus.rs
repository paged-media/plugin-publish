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

//! Shared plumbing for the OPT-IN corpus lanes.
//!
//! `paged-media/corpus` is a private, gitignored sibling checkout (Git
//! LFS, and a 4.4 GB payload that is not even in the repo). Nothing here
//! may hard-fail on a machine that has no copy — CI has none. Every
//! caller is `#[ignore]`d as well, so the corpus lanes run only when
//! asked for:
//!
//! ```text
//! PAGED_IDML_CORPUS=1 cargo test -p idml-export -- --ignored --nocapture
//! ```
//!
//! `PAGED_IDML_CORPUS` may also name the corpus root directly, which is
//! how a checkout somewhere other than the default sibling path is used.
//! Mirrors plugin-image's `PAGED_PSD_ORACLE` lane in shape and posture.

#![allow(dead_code)]

use std::io::Read;
use std::path::PathBuf;

/// The corpus root, or `None` (with a printed reason) when this machine
/// has no usable copy. Callers `return` on `None`.
pub fn root() -> Option<PathBuf> {
    let Some(switch) = std::env::var_os("PAGED_IDML_CORPUS") else {
        eprintln!(
            "SKIP corpus lane: PAGED_IDML_CORPUS unset \
             (set it to 1, or to a corpus root, and run with --ignored)"
        );
        return None;
    };
    let switch = switch.to_string_lossy().into_owned();
    let path = if switch == "1" || switch.is_empty() {
        // The default sibling layout: <workspace>/../../../corpus, i.e.
        // ~/paged/corpus next to ~/paged/plugins/plugin-publish.
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../corpus")
    } else {
        PathBuf::from(switch)
    };
    if !path.is_dir() {
        eprintln!("SKIP corpus lane: {} is not a directory", path.display());
        return None;
    }
    Some(path)
}

/// Every `Spreads/*.xml` entry of an IDML package, as
/// `(entry name, decompressed bytes)`.
pub fn spreads(package: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let bytes = std::fs::read(package).expect("read package");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("valid zip");
    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
        .filter(|n| n.starts_with("Spreads/") && n.ends_with(".xml"))
        .collect();
    names
        .into_iter()
        .map(|name| {
            let mut body = Vec::new();
            zip.by_name(&name)
                .expect("named entry")
                .read_to_end(&mut body)
                .expect("read entry");
            (name, body)
        })
        .collect()
}
