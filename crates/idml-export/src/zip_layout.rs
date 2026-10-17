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

//! The ZIP layout a saved package is written in.
//!
//! Save-back is a byte splice: entries the model did not touch are copied
//! with their compressed bytes, and an unmutated package must come back
//! byte-identical. The container records around those bytes are written by
//! the `zip` crate, and their layout is part of that contract.
//!
//! The layout is the one zip 2.4 wrote. zip 9's raw copy carries more of the
//! source entry over than zip 2's did: the host system, the external
//! attributes, the extra fields and the comment. An InDesign package, made
//! on "MS-DOS" with no attributes, would then save with different
//! version-made-by and attribute bytes in every central record. So every
//! raw copy goes through [`copy_entry`], and every writer that uses it is
//! closed with [`finish`]:
//!
//! * [`copy_entry`] copies through `raw_copy_file_touch` with the timestamp
//!   and Unix mode zip 2 derived from the source. That call drops the
//!   source's extra fields, as zip 2 did.
//! * [`finish`] restores the central-directory fields zip 2 wrote for
//!   every entry and zip 9 can take from a source instead: host system
//!   Unix, attributes holding only the Unix mode (no MS-DOS attribute
//!   byte), no entry comment, the UTF-8 flag set by the name alone, and
//!   the version zip 2 needed for an MS-DOS directory entry. Entries
//!   written with `start_file` already have these values; the pass leaves
//!   them as they are.

use std::io::{Cursor, Read, Seek, Write};

use zip::read::ZipFile;
use zip::result::{ZipError, ZipResult};
use zip::write::ZipWriter;
use zip::{DateTime, System};

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EXTRA_ID: u16 = 0x0001;
const CENTRAL_LEN: usize = 46;
const EOCD_LEN: usize = 22;
const ZIP64_LOCATOR_LEN: usize = 20;
const UTF8_FLAG: u16 = 1 << 11;
const HOST_UNIX: u8 = 3;
const HOST_DOS: u8 = 0;
const DOS_DIRECTORY: u32 = 0x10;
const DOS_READ_ONLY: u32 = 0x01;
const METHOD_STORED: u16 = 0;
const VERSION_STORED: u16 = 10;
const VERSION_DIRECTORY: u16 = 20;

/// Copy `entry` (from `by_index_raw`) into `zip` without recompressing it,
/// with the timestamp and Unix mode zip 2 gave a raw copy.
pub fn copy_entry<W: Write + Seek, R: Read>(
    zip: &mut ZipWriter<W>,
    entry: ZipFile<'_, R>,
) -> ZipResult<()> {
    let modified = entry
        .last_modified()
        .filter(DateTime::is_valid)
        .unwrap_or_default();
    let mode = zip2_unix_mode(entry.system(), entry.external_attributes());
    zip.raw_copy_file_touch(entry, modified, Some(mode))
}

/// Finish `zip` and return its bytes in zip 2's layout (see the module docs).
pub fn finish(zip: ZipWriter<Cursor<Vec<u8>>>) -> ZipResult<Vec<u8>> {
    let mut bytes = zip.finish()?.into_inner();
    restore_central_directory(&mut bytes)?;
    Ok(bytes)
}

/// The permission bits zip 2 gave a raw copy: its reading of the source's
/// mode (Unix: the high half of the attributes; MS-DOS: 0o775 for a
/// directory, else 0o664, write bits off when read-only; any other host,
/// or no attributes: none), defaulting to 0o644.
fn zip2_unix_mode(system: System, attributes: u32) -> u32 {
    let mode = match system {
        _ if attributes == 0 => 0o644,
        System::Unix => attributes >> 16,
        System::Dos => {
            let mode = if attributes & DOS_DIRECTORY != 0 {
                0o775
            } else {
                0o664
            };
            if attributes & DOS_READ_ONLY != 0 {
                mode & 0o555
            } else {
                mode
            }
        }
        _ => 0o644,
    };
    mode & 0o777
}

fn invalid(what: &'static str) -> ZipError {
    ZipError::InvalidArchive(what.into())
}

fn u16_at(b: &[u8], at: usize) -> ZipResult<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| invalid("truncated record"))
}

fn u32_at(b: &[u8], at: usize) -> ZipResult<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| invalid("truncated record"))
}

fn u64_at(b: &[u8], at: usize) -> ZipResult<u64> {
    b.get(at..at + 8)
        .map(|s| u64::from_le_bytes(s.try_into().expect("8 bytes")))
        .ok_or_else(|| invalid("truncated record"))
}

fn usize_of(v: u64) -> ZipResult<usize> {
    usize::try_from(v).map_err(|_| invalid("offset out of range"))
}

/// Where the central directory sits and how many records it holds, read
/// from the end records of an archive `ZipWriter` just wrote (no archive
/// comment, so the end record is the last 22 bytes).
struct Directory {
    start: usize,
    size: usize,
    entries: u64,
    /// Offset of the ZIP64 end record, when the archive has one.
    zip64_end: Option<usize>,
}

fn directory(bytes: &[u8]) -> ZipResult<Directory> {
    let eocd = bytes
        .len()
        .checked_sub(EOCD_LEN)
        .ok_or_else(|| invalid("no end record"))?;
    if u32_at(bytes, eocd)? != EOCD_SIG || u16_at(bytes, eocd + 20)? != 0 {
        return Err(invalid("end record is not where ZipWriter puts it"));
    }
    let zip64_end = match eocd.checked_sub(ZIP64_LOCATOR_LEN) {
        Some(locator) if u32_at(bytes, locator)? == ZIP64_LOCATOR_SIG => {
            let end = usize_of(u64_at(bytes, locator + 8)?)?;
            if u32_at(bytes, end)? != ZIP64_EOCD_SIG {
                return Err(invalid("ZIP64 locator points at no end record"));
            }
            Some(end)
        }
        _ => None,
    };
    Ok(match zip64_end {
        Some(end) => Directory {
            start: usize_of(u64_at(bytes, end + 48)?)?,
            size: usize_of(u64_at(bytes, end + 40)?)?,
            entries: u64_at(bytes, end + 32)?,
            zip64_end,
        },
        None => Directory {
            start: u32_at(bytes, eocd + 16)? as usize,
            size: u32_at(bytes, eocd + 12)? as usize,
            entries: u64::from(u16_at(bytes, eocd + 10)?),
            zip64_end,
        },
    })
}

/// The local header offset of a central record: the 32-bit field, or the
/// ZIP64 extra field's value when that field holds the sentinel.
fn local_offset(record: &[u8], name_len: usize, extra_len: usize) -> ZipResult<usize> {
    let offset = u32_at(record, 42)?;
    if offset != u32::MAX {
        return Ok(offset as usize);
    }
    let extra = record
        .get(CENTRAL_LEN + name_len..CENTRAL_LEN + name_len + extra_len)
        .ok_or_else(|| invalid("truncated extra field"))?;
    let mut at = 0;
    while at + 4 <= extra.len() {
        let id = u16_at(extra, at)?;
        let len = u16_at(extra, at + 2)? as usize;
        if id == ZIP64_EXTRA_ID {
            // Values appear in a fixed order, each only when its 32-bit
            // field holds the sentinel: sizes first, then the offset.
            let mut value = at + 4;
            if u32_at(record, 24)? == u32::MAX {
                value += 8;
            }
            if u32_at(record, 20)? == u32::MAX {
                value += 8;
            }
            return usize_of(u64_at(extra, value)?);
        }
        at += 4 + len;
    }
    Err(invalid("offset sentinel without a ZIP64 extra field"))
}

fn restore_central_directory(bytes: &mut Vec<u8>) -> ZipResult<()> {
    let dir = directory(bytes)?;
    let end = dir.start + dir.size;
    let mut central = Vec::with_capacity(dir.size);
    let mut at = dir.start;
    for _ in 0..dir.entries {
        if at + CENTRAL_LEN > end || u32_at(bytes, at)? != CENTRAL_SIG {
            return Err(invalid("central record expected"));
        }
        let name_len = u16_at(bytes, at + 28)? as usize;
        let extra_len = u16_at(bytes, at + 30)? as usize;
        let comment_len = u16_at(bytes, at + 32)? as usize;
        let kept = CENTRAL_LEN + name_len + extra_len;
        let next = at + kept + comment_len;
        if next > end {
            return Err(invalid("central record runs past the directory"));
        }
        let mut record = bytes[at..at + kept].to_vec();
        let name = &record[CENTRAL_LEN..CENTRAL_LEN + name_len];

        let host = record[5];
        let attributes = u32_at(&record, 38)?;
        let mut version_needed = u16_at(&record, 6)?;
        // zip 9 counts an MS-DOS directory bit toward the version needed;
        // zip 2 gave the copy a file mode, so a stored entry needed 1.0.
        if host == HOST_DOS
            && attributes & DOS_DIRECTORY != 0
            && u16_at(&record, 10)? == METHOD_STORED
            && version_needed == VERSION_DIRECTORY
        {
            version_needed = VERSION_STORED;
        }
        let utf8 = std::str::from_utf8(name).is_ok() && !name.is_ascii();
        let flags = (u16_at(&record, 8)? & !UTF8_FLAG) | if utf8 { UTF8_FLAG } else { 0 };

        record[4] = version_needed as u8;
        record[5] = HOST_UNIX;
        record[6..8].copy_from_slice(&version_needed.to_le_bytes());
        record[8..10].copy_from_slice(&flags.to_le_bytes());
        record[32..34].copy_from_slice(&0u16.to_le_bytes());
        record[38..42].copy_from_slice(&(attributes & 0xFFFF_0000).to_le_bytes());

        // The local header repeats the version needed and the flags.
        let local = local_offset(&record, name_len, extra_len)?;
        if u32_at(bytes, local)? != LOCAL_SIG {
            return Err(invalid("central record points at no local header"));
        }
        bytes[local + 4..local + 6].copy_from_slice(&version_needed.to_le_bytes());
        bytes[local + 6..local + 8].copy_from_slice(&flags.to_le_bytes());

        central.extend_from_slice(&record);
        at = next;
    }
    if at != end {
        return Err(invalid("central directory size disagrees with its records"));
    }

    let shrink = dir.size - central.len();
    let tail = bytes.split_off(end);
    bytes.truncate(dir.start);
    bytes.extend_from_slice(&central);
    bytes.extend_from_slice(&tail);
    if shrink > 0 {
        // Dropped comments shorten the directory; the end records follow
        // it, so their size fields and the ZIP64 end record's offset move.
        let eocd = bytes.len() - EOCD_LEN;
        let size = u32_at(bytes, eocd + 12)?;
        if size != u32::MAX {
            let size = size - shrink as u32;
            bytes[eocd + 12..eocd + 16].copy_from_slice(&size.to_le_bytes());
        }
        if let Some(old_end) = dir.zip64_end {
            let zip64_end = old_end - shrink;
            let size = (dir.size - shrink) as u64;
            bytes[zip64_end + 40..zip64_end + 48].copy_from_slice(&size.to_le_bytes());
            let locator = eocd - ZIP64_LOCATOR_LEN;
            bytes[locator + 8..locator + 16].copy_from_slice(&(zip64_end as u64).to_le_bytes());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    #[test]
    fn zip2_modes() {
        assert_eq!(zip2_unix_mode(System::Dos, 0), 0o644);
        assert_eq!(zip2_unix_mode(System::Dos, 0x20), 0o664);
        assert_eq!(zip2_unix_mode(System::Dos, 0x21), 0o444);
        assert_eq!(zip2_unix_mode(System::Dos, 0x10), 0o775);
        assert_eq!(zip2_unix_mode(System::Unix, 0o100_640 << 16), 0o640);
        assert_eq!(zip2_unix_mode(System::Unix, 0o104_755 << 16), 0o755);
        assert_eq!(zip2_unix_mode(System::WindowsNTFS, 0o100_600 << 16), 0o644);
    }

    /// A package `start_file` wrote is already in zip 2's layout; the pass
    /// leaves every byte where it is.
    #[test]
    fn start_file_output_is_left_alone() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file("mimetype", stored).unwrap();
        zip.write_all(b"application/vnd.adobe.indesign-idml-package")
            .unwrap();
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("designmap.xml", deflated).unwrap();
        zip.write_all(b"<Document/>").unwrap();
        let plain = zip.finish().unwrap().into_inner();
        let mut restored = plain.clone();
        restore_central_directory(&mut restored).unwrap();
        assert_eq!(restored, plain);
    }
}
