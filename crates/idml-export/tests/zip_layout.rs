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

//! The container layout of a saved package (see `idml_export::zip_layout`).
//!
//! Both expectations were measured with zip 2.4.2, the version these
//! packages were written with before zip 9. They pin the bytes, so a later
//! `zip` upgrade that changes what a save writes fails here.

use std::io::{Cursor, Write};

use idml_export::zip_layout;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/{name}")).expect("fixture")
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Every entry of `source.zip` raw-copied, then one part added: the
/// result is the archive zip 2.4.2 wrote for the same calls.
///
/// The source holds what zip 9 would otherwise carry over from a source
/// entry: MS-DOS and NTFS hosts, MS-DOS attribute bytes (archive,
/// read-only, directory), a Unix setuid mode, extra fields, ASCII and
/// non-ASCII entry comments, InDesign's UTF-8 and deflate-option flags, a
/// non-ASCII name, a 60-second timestamp and a zero date.
#[test]
fn raw_copies_keep_zip2_layout() {
    let source = fixture("zip-layout/source.zip");
    let mut src = ZipArchive::new(Cursor::new(&source)).expect("source");
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let entry = src.by_index_raw(i).expect("entry");
        zip_layout::copy_entry(&mut zip, entry).expect("copy");
    }
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("Resources/Added.xml", deflated)
        .expect("start");
    zip.write_all(b"<Added/>").expect("write");
    let out = zip_layout::finish(zip).expect("finish");

    assert_eq!(out, fixture("zip-layout/zip2-copy.zip"));
}

/// An unmutated InDesign package saves to the bytes zip 2.4.2 wrote for
/// it (31,603 bytes, FNV-1a `6684cff85755977a`). InDesign marks every
/// entry as made on MS-DOS with no attributes; the save keeps zip 2's
/// Unix host and 0o644 mode, and every entry's compressed bytes.
#[test]
fn unmutated_indesign_package_saves_as_with_zip2() {
    let source = fixture("indesign-20.0.1-navigation.idml");
    let doc = idml_import::import_idml_doc(&source).expect("open");
    let out = idml_export::write_idml(&doc, &source).expect("write");

    assert_eq!(out.len(), 31_603);
    assert_eq!(fnv1a(&out), 0x6684_cff8_5755_977a);

    let mut zip = ZipArchive::new(Cursor::new(&out)).expect("saved package");
    let first = zip.by_index(0).expect("first entry");
    assert_eq!(first.name().expect("name"), "mimetype");
    assert_eq!(first.compression(), CompressionMethod::Stored);
}
