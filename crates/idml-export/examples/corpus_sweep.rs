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

//! Measure the save-back's BYTE-IDENTITY gap across a corpus of IDML
//! packages: parse every `Spreads/*.xml`, re-serialise it with no
//! mutation at all, and count the entries whose bytes came back
//! different. An unmutated round-trip is supposed to be byte-identical,
//! so every gap is a defect — and the two numbers that matter are the
//! gap COUNT and the total byte GROWTH (growth means duplicated
//! content, which is the severe kind).
//!
//! Diagnostic tooling, in the tradition of idml-import's `dump_markers`
//! / `dump_anchored`. The regression GUARDS live in
//! `tests/nested_group_roundtrip.rs` and `tests/path_contour_roundtrip.rs`
//! (opt-in `PAGED_IDML_CORPUS` lanes); this is what you run to find the
//! next one.
//!
//! ```text
//! cargo run --release --example corpus_sweep                 # sibling corpus
//! cargo run --release --example corpus_sweep -- /path/to/corpus
//! SWEEP_VERBOSE=1 …    # one line per gap, with the size delta
//! SWEEP_FILTER=catalog …    # only packages whose path contains this
//! SWEEP_DUMP=/tmp/out …     # write .before/.after for every gap
//! ```
//!
//! A panic in the rewrite is caught and reported per entry rather than
//! ending the sweep — the wasm worker builds with `panic = abort`, so a
//! panic there is a killed save, and the count belongs in the summary.

use std::io::Read;

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| {
        // The default sibling layout: ~/paged/corpus next to
        // ~/paged/plugins/plugin-publish.
        format!("{}/../../../../corpus", env!("CARGO_MANIFEST_DIR"))
    });
    let verbose = std::env::var("SWEEP_VERBOSE").is_ok();
    let filter = std::env::var("SWEEP_FILTER").unwrap_or_default();
    let mut packages = Vec::new();
    collect(std::path::Path::new(&root), &mut packages);
    packages.sort();

    let mut n_pkgs = 0usize;
    let mut n_entries = 0usize;
    let mut n_gaps = 0usize;
    let mut n_panics = 0usize;
    let mut n_pkg_with_gaps = 0usize;
    let mut growth_total: i64 = 0;

    for p in &packages {
        let ps = p.to_string_lossy().to_string();
        if !filter.is_empty() && !ps.contains(&filter) {
            continue;
        }
        let Ok(bytes) = std::fs::read(p) else {
            continue;
        };
        let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(&bytes)) else {
            continue;
        };
        n_pkgs += 1;
        let mut pkg_gaps = 0usize;
        let names: Vec<String> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
            .filter(|n| n.starts_with("Spreads/") && n.ends_with(".xml"))
            .collect();
        for name in names {
            let mut buf = Vec::new();
            {
                let Ok(mut e) = zip.by_name(&name) else {
                    continue;
                };
                if e.read_to_end(&mut buf).is_err() {
                    continue;
                }
            }
            n_entries += 1;
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let spread = idml_import::parse_spread(&buf)?;
                let out = idml_export::rewrite::rewrite_spread(&buf, &spread)?;
                Ok::<_, Box<dyn std::error::Error>>(out)
            }));
            match res {
                Err(_) => {
                    n_panics += 1;
                    println!("PANIC {ps}#{name}");
                }
                Ok(Err(e)) => println!("ERR   {ps}#{name}: {e}"),
                Ok(Ok(out)) => {
                    if out != buf {
                        if let Ok(dir) = std::env::var("SWEEP_DUMP") {
                            let stem = name.replace('/', "_");
                            let _ = std::fs::write(format!("{dir}/{stem}.before"), &buf);
                            let _ = std::fs::write(format!("{dir}/{stem}.after"), &out);
                        }
                        n_gaps += 1;
                        pkg_gaps += 1;
                        growth_total += out.len() as i64 - buf.len() as i64;
                        if verbose {
                            println!(
                                "GAP   {ps}#{name} {} -> {} ({:+})",
                                buf.len(),
                                out.len(),
                                out.len() as i64 - buf.len() as i64
                            );
                        }
                    }
                }
            }
        }
        if pkg_gaps > 0 {
            n_pkg_with_gaps += 1;
        }
    }

    println!("--------------------------------------------------");
    println!("packages scanned      : {n_pkgs}");
    println!("packages with gaps    : {n_pkg_with_gaps}");
    println!("spread entries scanned: {n_entries}");
    println!("byte-identity gaps    : {n_gaps}");
    println!("panics                : {n_panics}");
    println!("total byte growth     : {growth_total:+}");
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        // The corpus symlinks a shared `Document fonts` directory into
        // every pack; following those would walk the same tree 61 times.
        if p.is_symlink() {
            continue;
        }
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "idml") {
            out.push(p);
        }
    }
}
