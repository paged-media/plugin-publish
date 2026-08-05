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
//! packages: re-serialise every entry the writer TRANSFORMS with no
//! mutation at all, and count the entries whose bytes came back
//! different. An unmutated round-trip is supposed to be byte-identical,
//! so every gap is a defect — and the two numbers that matter are the
//! gap COUNT and the total byte GROWTH (growth means duplicated
//! content, which is the severe kind).
//!
//! # Coverage is part of the measurement
//!
//! This sweep used to look at `Spreads/*.xml` and nothing else, which
//! made its headline number an answer to a question nobody asked: 823 of
//! the corpus's 12,668 entries. The story lane — 10,539 entries, 85% of
//! the package by count — carried the same defects and was never in the
//! denominator. A number that silently excludes lanes is worse than no
//! number, so the sweep now measures **every entry the writer touches**
//! and PRINTS, at the end, an inventory of every entry it does not —
//! with the reason. Adding a transformer to the writer without adding a
//! lane here should be visible as an entry sitting in the "carried
//! through verbatim" list that no longer is.
//!
//! The lanes, and what each corresponds to in [`idml_export::write_idml`]:
//!
//! | lane          | entries              | writer path                          |
//! |---------------|----------------------|--------------------------------------|
//! | `spread`      | `Spreads/*.xml`      | `rewrite::rewrite_spread`            |
//! | `story`       | `Stories/*.xml`      | `rewrite::rewrite_story`             |
//! | `graphic`     | `Resources/Graphic.xml` | `resources::patch_graphic`        |
//! | `styles`      | `Resources/Styles.xml`  | `resources::patch_styles`         |
//! | `master`      | `MasterSpreads/*.xml`| *(LATENT — see below)*               |
//! | `package`     | whole `.idml`        | `import_idml_doc` + `write_idml`     |
//!
//! `master` is the honest odd one out: master spreads ARE parsed (with
//! the same `parse_spread`, into `Document::master_spreads`) but
//! `write_idml` iterates `doc.spreads` only, so they are copied verbatim
//! today and a mutated master is silently dropped on save. The lane
//! measures the rewrite the writer WOULD use, so the latent gap is
//! visible before that hole is closed rather than after. It is reported
//! separately and excluded from the shipping total.
//!
//! `package` is the ground truth — the real save path, per-entry — and
//! necessarily double-counts the part lanes; it is reported separately
//! for that reason. The part lanes are the diagnostics that say WHERE.
//!
//! Diagnostic tooling, in the tradition of idml-import's `dump_markers`
//! / `dump_anchored`. The regression GUARDS live in
//! `tests/nested_group_roundtrip.rs`, `tests/path_contour_roundtrip.rs`
//! and `tests/story_precision.rs` (opt-in `PAGED_IDML_CORPUS` lanes);
//! this is what you run to find the next one.
//!
//! ```text
//! cargo run --release --example corpus_sweep                 # sibling corpus
//! cargo run --release --example corpus_sweep -- /path/to/corpus
//! SWEEP_VERBOSE=1 …    # one line per gap, with the size delta
//! SWEEP_FILTER=catalog …    # only packages whose path contains this
//! SWEEP_LANES=story,spread …  # only these lanes (default: all)
//! SWEEP_DUMP=/tmp/out …     # write .before/.after for every gap
//! ```
//!
//! A panic in the rewrite is caught and reported per entry rather than
//! ending the sweep — the wasm worker builds with `panic = abort`, so a
//! panic there is a killed save, and the count belongs in the summary.

use std::collections::BTreeMap;
use std::io::Read;

/// One measured lane's running totals.
#[derive(Default)]
struct Lane {
    entries: usize,
    gaps: usize,
    /// Rewrites whose output is NOT WELL-FORMED XML. A killed save —
    /// strictly worse than a byte-identity gap and, until this column
    /// existed, entirely invisible: the sweep only ever compared byte
    /// LENGTHS and contents, so a scrambled but same-ish-sized entry
    /// counted as one more "gap" among hundreds.
    malformed: usize,
    panics: usize,
    errors: usize,
    growth: i64,
    packages_with_gaps: std::collections::BTreeSet<String>,
}

impl Lane {
    /// Record one entry's outcome. `res` is the rewrite result, already
    /// caught: `Err(())` for a panic, `Ok(Err(_))` for a reported error.
    fn record(
        &mut self,
        pkg: &str,
        name: &str,
        original: &[u8],
        res: Result<Result<Vec<u8>, String>, ()>,
        opts: &Opts,
    ) {
        self.entries += 1;
        match res {
            Err(()) => {
                self.panics += 1;
                println!("PANIC {pkg}#{name}");
            }
            Ok(Err(e)) => {
                self.errors += 1;
                println!("ERR   {pkg}#{name}: {e}");
            }
            Ok(Ok(out)) => {
                if let Some(e) = not_well_formed(&out) {
                    self.malformed += 1;
                    println!("MALFORMED {pkg}#{name}: {e}");
                }
                if out != original {
                    self.gaps += 1;
                    self.packages_with_gaps.insert(pkg.to_string());
                    self.growth += out.len() as i64 - original.len() as i64;
                    if let Some(dir) = &opts.dump {
                        let stem = format!("{}_{}", sanitize(pkg), name.replace('/', "_"));
                        let _ = std::fs::write(format!("{dir}/{stem}.before"), original);
                        let _ = std::fs::write(format!("{dir}/{stem}.after"), &out);
                    }
                    if opts.verbose {
                        println!(
                            "GAP   {pkg}#{name} {} -> {} ({:+})",
                            original.len(),
                            out.len(),
                            out.len() as i64 - original.len() as i64
                        );
                    }
                }
            }
        }
    }
}

/// `Some(reason)` when `xml` is not well-formed. Runs the document
/// through a checking reader (mismatched / unclosed end tags are hard
/// errors) — the cheapest possible stand-in for "InDesign can still open
/// this".
fn not_well_formed(xml: &[u8]) -> Option<String> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let config = reader.config_mut();
    config.check_end_names = true;
    config.expand_empty_elements = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Eof) => return None,
            Ok(_) => buf.clear(),
            Err(e) => return Some(e.to_string()),
        }
    }
}

/// Last two path components, `/` → `_` — enough to tell two packages
/// apart in a dump directory without spelling the whole corpus root.
fn sanitize(pkg: &str) -> String {
    let parts: Vec<&str> = pkg.rsplit('/').take(2).collect();
    parts
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("_")
        .replace(['.', ' '], "_")
}

struct Opts {
    verbose: bool,
    dump: Option<String>,
    lanes: Vec<String>,
}

impl Opts {
    fn wants(&self, lane: &str) -> bool {
        self.lanes.is_empty() || self.lanes.iter().any(|l| l == lane)
    }
}

/// Catch a panic in a rewrite and normalise the error to a `String` so
/// every lane records the same shape.
fn attempt<F>(f: F) -> Result<Result<Vec<u8>, String>, ()>
where
    F: FnOnce() -> Result<Vec<u8>, String>,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| ())
}

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| {
        // The default sibling layout: ~/paged/corpus next to
        // ~/paged/plugins/plugin-publish.
        format!("{}/../../../../corpus", env!("CARGO_MANIFEST_DIR"))
    });
    let opts = Opts {
        verbose: std::env::var("SWEEP_VERBOSE").is_ok(),
        dump: std::env::var("SWEEP_DUMP").ok(),
        lanes: std::env::var("SWEEP_LANES")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    };
    let filter = std::env::var("SWEEP_FILTER").unwrap_or_default();
    let mut packages = Vec::new();
    collect(std::path::Path::new(&root), &mut packages);
    packages.sort();

    let mut n_pkgs = 0usize;
    let mut lanes: BTreeMap<&'static str, Lane> = BTreeMap::new();
    // Every entry the sweep did NOT put through a transformer, counted by
    // path (a `Stories/Story_u123.xml` collapses to `Stories/*.xml`). This
    // is the coverage report: what the headline number excludes.
    let mut unmeasured: BTreeMap<String, usize> = BTreeMap::new();
    let mut measured: BTreeMap<String, usize> = BTreeMap::new();
    // The whole-package lane's own counters (entries there are diffed
    // inside the saved archive, not rewritten one at a time).
    let mut pkg_scanned = 0usize;
    let mut pkg_identical = 0usize;
    let mut pkg_failed = 0usize;
    let mut pkg_entry_gaps: BTreeMap<String, usize> = BTreeMap::new();

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
        let names: Vec<String> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();

        for name in &names {
            let bucket = bucket_of(name);
            let lane = lane_for(name);
            match lane {
                Some(l) if opts.wants(l) => *measured.entry(bucket).or_default() += 1,
                _ => *unmeasured.entry(bucket).or_default() += 1,
            }
            let Some(lane) = lane else { continue };
            if !opts.wants(lane) {
                continue;
            }
            let mut buf = Vec::new();
            {
                let Ok(mut e) = zip.by_name(name) else {
                    continue;
                };
                if e.read_to_end(&mut buf).is_err() {
                    continue;
                }
            }
            let res = attempt(|| match lane {
                // A `<MasterSpread>` has the same schema as a `<Spread>`
                // and parses with the same parser (see
                // `idml_import::import_idml_archive`).
                "spread" | "master" => {
                    let spread = idml_import::parse_spread(&buf).map_err(|e| e.to_string())?;
                    idml_export::rewrite::rewrite_spread(&buf, &spread).map_err(|e| e.to_string())
                }
                "story" => {
                    let story = idml_import::parse_story(&buf).map_err(|e| e.to_string())?;
                    idml_export::rewrite::rewrite_story(&buf, &story).map_err(|e| e.to_string())
                }
                "graphic" => {
                    let palette = idml_import::parse_graphic(&buf).map_err(|e| e.to_string())?;
                    idml_export::resources::patch_graphic(&buf, &palette).map_err(|e| e.to_string())
                }
                "styles" => {
                    let styles = idml_import::parse_stylesheet(&buf).map_err(|e| e.to_string())?;
                    idml_export::resources::patch_styles(&buf, &styles).map_err(|e| e.to_string())
                }
                other => unreachable!("unhandled lane {other}"),
            });
            lanes
                .entry(lane)
                .or_default()
                .record(&ps, name, &buf, res, &opts);
        }

        // The whole-package lane: the REAL save path, end to end. Every
        // per-part lane above is a diagnostic for this one.
        if opts.wants("package") {
            pkg_scanned += 1;
            let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let doc = idml_import::import_idml_doc(&bytes).map_err(|e| e.to_string())?;
                idml_export::write_idml(&doc, &bytes).map_err(|e| e.to_string())
            }));
            match saved {
                Err(_) => {
                    pkg_failed += 1;
                    println!("PANIC {ps} (whole-package save)");
                }
                Ok(Err(e)) => {
                    pkg_failed += 1;
                    println!("ERR   {ps} (whole-package save): {e}");
                }
                Ok(Ok(out)) => {
                    let diffs = entry_diffs(&bytes, &out);
                    if diffs.is_empty() {
                        pkg_identical += 1;
                    } else {
                        for d in &diffs {
                            *pkg_entry_gaps.entry(bucket_of(d)).or_default() += 1;
                        }
                        if opts.verbose {
                            println!("PKGGAP {ps}: {} entries differ", diffs.len());
                        }
                    }
                }
            }
        }
    }

    println!("--------------------------------------------------");
    println!("packages scanned      : {n_pkgs}");
    println!();
    println!("PART LANES (the writer's per-entry transformers)");
    println!(
        "{:<10} {:>8} {:>7} {:>10} {:>7} {:>7} {:>6} {:>13}",
        "lane", "entries", "gaps", "MALFORMED", "panics", "errors", "pkgs", "byte growth"
    );
    let mut tot = Lane::default();
    for (name, lane) in &lanes {
        println!(
            "{:<10} {:>8} {:>7} {:>10} {:>7} {:>7} {:>6} {:>+13}",
            name,
            lane.entries,
            lane.gaps,
            lane.malformed,
            lane.panics,
            lane.errors,
            lane.packages_with_gaps.len(),
            lane.growth
        );
        // `master` is LATENT — the writer never emits it (see the module
        // docs), so it does not belong in the shipping total.
        if *name != "master" {
            tot.entries += lane.entries;
            tot.gaps += lane.gaps;
            tot.malformed += lane.malformed;
            tot.panics += lane.panics;
            tot.errors += lane.errors;
            tot.growth += lane.growth;
            for p in &lane.packages_with_gaps {
                tot.packages_with_gaps.insert(p.clone());
            }
        }
    }
    println!(
        "{:<10} {:>8} {:>7} {:>10} {:>7} {:>7} {:>6} {:>+13}",
        "TOTAL*",
        tot.entries,
        tot.gaps,
        tot.malformed,
        tot.panics,
        tot.errors,
        tot.packages_with_gaps.len(),
        tot.growth
    );
    println!("* excludes the LATENT `master` lane (never written today)");

    if opts.wants("package") {
        println!();
        println!("WHOLE-PACKAGE LANE (import_idml_doc + write_idml, the real save path)");
        println!("packages saved        : {pkg_scanned}");
        println!("byte-identical        : {pkg_identical}");
        println!(
            "with differing entries: {}",
            pkg_scanned - pkg_identical - pkg_failed
        );
        println!("failed (panic/error)  : {pkg_failed}");
        if !pkg_entry_gaps.is_empty() {
            println!("differing entries by kind:");
            for (k, v) in &pkg_entry_gaps {
                println!("  {v:>6}  {k}");
            }
        }
    }

    println!();
    println!("COVERAGE — entries the sweep PUT THROUGH a transformer");
    for (k, v) in &measured {
        println!("  {v:>6}  {k}");
    }
    println!("COVERAGE — entries the sweep did NOT measure");
    for (k, v) in &unmeasured {
        println!("  {v:>6}  {k:<26} {}", why_unmeasured(k));
    }
}

/// Which lane owns an entry, or `None` when the writer never transforms
/// it (it is `raw_copy_file`d verbatim, so byte-identity is structural).
fn lane_for(name: &str) -> Option<&'static str> {
    if name.starts_with("Spreads/") && name.ends_with(".xml") {
        return Some("spread");
    }
    if name.starts_with("MasterSpreads/") && name.ends_with(".xml") {
        return Some("master");
    }
    if name.starts_with("Stories/") && name.ends_with(".xml") {
        return Some("story");
    }
    match name {
        "Resources/Graphic.xml" => Some("graphic"),
        "Resources/Styles.xml" => Some("styles"),
        _ => None,
    }
}

/// Collapse an entry path to its reporting bucket
/// (`Stories/Story_u123.xml` → `Stories/*.xml`).
fn bucket_of(name: &str) -> String {
    match name {
        "Resources/Graphic.xml" | "Resources/Styles.xml" => name.to_string(),
        _ => match name.split_once('/') {
            Some((dir, rest)) => match rest.rsplit_once('.') {
                Some((_, ext)) => format!("{dir}/*.{ext}"),
                None => format!("{dir}/*"),
            },
            None => name.to_string(),
        },
    }
}

/// Why a bucket is not measured — the honest half of the coverage
/// report. "verbatim" means `write_idml` `raw_copy_file`s it, so the
/// bytes cannot change; "no lane" would mean a transformer exists that
/// this sweep forgot.
fn why_unmeasured(bucket: &str) -> &'static str {
    match bucket {
        // A bucket that HAS a lane can only land here when the run asked
        // for a subset (`SWEEP_LANES`) — never because the sweep forgot.
        "Spreads/*.xml"
        | "MasterSpreads/*.xml"
        | "Stories/*.xml"
        | "Resources/Graphic.xml"
        | "Resources/Styles.xml" => "lane deselected via SWEEP_LANES",
        "designmap.xml" => "verbatim unless a part is MINTED (emit::patch_designmap)",
        "Resources/Fonts.xml" | "Resources/Preferences.xml" => "verbatim — no transformer",
        "XML/*.xml" => "verbatim — BackingStory/Tags/Mapping are not parsed at all",
        "META-INF/*.xml" => "verbatim — container/metadata are not parsed at all",
        "mimetype" => "verbatim + stored first (the ZIP rule)",
        _ => "verbatim — no transformer",
    }
}

/// Entry names whose DECOMPRESSED bytes differ between the source
/// package and the saved one (plus any entry that appeared or vanished).
fn entry_diffs(original: &[u8], saved: &[u8]) -> Vec<String> {
    fn entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
            return out;
        };
        let names: Vec<String> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        for name in names {
            let mut buf = Vec::new();
            if let Ok(mut e) = zip.by_name(&name) {
                if e.read_to_end(&mut buf).is_ok() {
                    out.insert(name, buf);
                }
            }
        }
        out
    }
    let a = entries(original);
    let b = entries(saved);
    let mut out = Vec::new();
    for (name, body) in &a {
        match b.get(name) {
            Some(other) if other == body => {}
            _ => out.push(name.clone()),
        }
    }
    for name in b.keys() {
        if !a.contains_key(name) {
            out.push(name.clone());
        }
    }
    out
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
