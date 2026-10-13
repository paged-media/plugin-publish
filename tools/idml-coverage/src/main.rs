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

//! Print the IDML coverage inventory. Regenerate the committed file with:
//!
//! ```sh
//! cargo run -p idml-coverage > docs/reference/idml-coverage.json
//! ```
//!
//! `--ratchet` rewrites the baseline (`docs/reference/idml-coverage-ratchet.json`)
//! and refuses a regression unless `--allow-regression` is given.
//! `--queue N` prints the N uncovered (part, element, attribute) rows with the
//! highest impact: documents departing from the mode × documents carrying it.

use idml_coverage::parts::{self, Metrics, Ratchet};

fn main() {
    if let Err(e) = run() {
        eprintln!("idml-coverage: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut ratchet = false;
    let mut allow_regression = false;
    let mut queue: Option<usize> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--ratchet" => ratchet = true,
            "--allow-regression" => allow_regression = true,
            "--queue" => {
                let n = it.next().ok_or("--queue needs a row count")?;
                queue = Some(n.parse().map_err(|_| format!("--queue {n}: not a count"))?);
            }
            _ => {
                return Err(format!(
                    "unknown argument {a} (use --ratchet [--allow-regression] or --queue N)"
                ))
            }
        }
    }

    let root = idml_coverage::repo_root();
    let analysis = idml_coverage::analyse(&root)?;
    let metrics = &analysis.joined.metrics;
    let unresolved = analysis.unresolved;

    if let Some(n) = queue {
        print_metrics(metrics, unresolved, &mut std::io::stdout());
        println!();
        print_queue(&analysis.joined.part_rows, n);
        return Ok(());
    }
    if ratchet {
        let now = analysis.ratchet();
        let path = root.join(idml_coverage::RATCHET);
        if let Ok(text) = std::fs::read_to_string(&path) {
            let base: Ratchet = serde_json::from_str(&text)
                .map_err(|e| format!("{}: {e}", idml_coverage::RATCHET))?;
            let diff = parts::compare(&base, &now);
            if !diff.regressions.is_empty() && !allow_regression {
                for r in &diff.regressions {
                    eprintln!("  regression: {r}");
                }
                return Err(
                    "the baseline only moves up; pass --allow-regression to record a \
                     deliberate regression (a census refresh, say)"
                        .into(),
                );
            }
            for c in diff.regressions.iter().chain(&diff.unrecorded) {
                eprintln!("  {c}");
            }
        }
        let mut json = serde_json::to_string_pretty(&now).map_err(|e| e.to_string())?;
        json.push('\n');
        std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        print_metrics(metrics, unresolved, &mut std::io::stderr());
        eprintln!("wrote {}", idml_coverage::RATCHET);
        return Ok(());
    }
    print!("{}", analysis.json);
    print_metrics(metrics, unresolved, &mut std::io::stderr());
    Ok(())
}

fn print_metrics(m: &Metrics, unresolved: usize, out: &mut dyn std::io::Write) {
    let pct = |r: &parts::Ratio| format!("{:.2} % ({} / {})", r.value * 100.0, r.covered, r.total);
    let _ = writeln!(out, "weightedImport  {}", pct(&m.weighted_import));
    let _ = writeln!(out, "impactImport    {}", pct(&m.impact_import));
    let _ = writeln!(out, "weightedExport  {}", pct(&m.weighted_export));
    let _ = writeln!(out, "misplacedReads  {}", m.misplaced_reads);
    let _ = writeln!(
        out,
        "unseenReads     {} ({} unjustified)",
        m.unseen_reads, m.unjustified_unseen_reads
    );
    let _ = writeln!(out, "unresolved      {unresolved}");
}

fn print_queue(rows: &[parts::PartRow], n: usize) {
    let q = parts::queue(rows);
    println!(
        "{} uncovered (part, element, attribute) rows; impact = nonModalDocs × docCount",
        q.len()
    );
    println!(
        "{:>4}  {:>7}  {:>8}  {:>4}  {:<22}  {:<50}  note",
        "rank", "impact", "nonModal", "docs", "part", "element@attribute"
    );
    for (i, r) in q.iter().take(n).enumerate() {
        let name = if r.kind == "properties-child" {
            format!("{}@{} (Properties)", r.element, r.attribute)
        } else {
            format!("{}@{}", r.element, r.attribute)
        };
        println!(
            "{:>4}  {:>7}  {:>8}  {:>4}  {:<22}  {:<50}  {}",
            i + 1,
            r.impact(),
            r.non_modal_docs,
            r.doc_count,
            r.part,
            name,
            if r.read_elsewhere {
                "read, but from another part"
            } else {
                "not read"
            }
        );
    }
}
