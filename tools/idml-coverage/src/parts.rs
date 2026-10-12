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

//! Package parts and the census join.
//!
//! An IDML package spreads one document over several XML parts, and InDesign
//! writes each element to a particular one: `<ColorGroup>` to `designmap.xml`,
//! `<FootnoteOption>` to `Resources/Preferences.xml`. Reading an element from
//! the wrong part reads nothing in InDesign's files. So the inventory records
//! which part each importer and exporter function works on, seeded by hand
//! (`[[part_entry]]` in `manual.toml`) and carried down the call graph, and
//! joins the result with the vendored corpus census (`census.json`): how many
//! documents carry each (part, element, attribute), and in how many of them
//! the value departs from the corpus mode.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::scan::{FnId, Scan};
use crate::Row;

/// The part classes of an IDML package, spelled as the census spells them.
pub const PART_CLASSES: &[&str] = &[
    "designmap",
    "META-INF",
    "MasterSpreads",
    "Spreads",
    "Stories",
    "XML",
    "Resources/Styles",
    "Resources/Graphic",
    "Resources/Fonts",
    "Resources/Preferences",
    "Other",
];

/// The census schema this tool reads (it needs per-attribute parts).
pub const CENSUS_SCHEMA: &str = "paged.idml-census/2";

// ---------------------------------------------------------------- call graph

/// Each function's parts: its `[[part_entry]]` seed plus the union of its
/// callers' parts, to a fixpoint. Seeds add rather than replace, because one
/// function can be handed several parts: `parse_conditions` reads the
/// designmap for `import_idml_archive` and Styles.xml for `parse_stylesheet`.
pub fn fn_parts(
    scan: &Scan,
    seeds: &HashMap<FnId, BTreeSet<String>>,
) -> HashMap<FnId, BTreeSet<String>> {
    let by_name = crate::fns_by_name(scan);
    let mut parts: HashMap<FnId, BTreeSet<String>> = seeds.clone();
    loop {
        let mut changed = false;
        for c in &scan.calls {
            let Some(callee) = crate::resolve_callee(&by_name, &c.caller.0, &c.callee) else {
                continue;
            };
            if *callee == c.caller {
                continue;
            }
            let Some(from) = parts.get(&c.caller).cloned() else {
                continue;
            };
            let entry = parts.entry(callee.clone()).or_default();
            for p in from {
                changed |= entry.insert(p);
            }
        }
        if !changed {
            return parts;
        }
    }
}

// ---------------------------------------------------------------- census

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Census {
    schema: String,
    generated_at: String,
    totals: CensusTotals,
    elements: Vec<CensusElement>,
}

#[derive(Deserialize)]
struct CensusTotals {
    documents: u64,
}

#[derive(Deserialize)]
struct CensusElement {
    element: String,
    attributes: BTreeMap<String, CensusAttr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CensusAttr {
    kinds: BTreeMap<String, u64>,
    parts: BTreeMap<String, u64>,
    #[serde(default)]
    kind_parts: BTreeMap<String, BTreeMap<String, u64>>,
    non_modal_docs: u64,
    non_modal_parts: BTreeMap<String, u64>,
}

/// Where the census came from: enough to tell two vendored copies apart.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CensusId {
    pub file: String,
    pub schema: String,
    pub generated_at: String,
    pub sha256: String,
    pub documents: u64,
}

/// One census row: how many documents carry `element@attribute` (of `kind`)
/// in `part`, and in how many of those the value is not the corpus mode.
struct CensusRow {
    part: String,
    element: String,
    attribute: String,
    kind: &'static str,
    doc_count: u64,
    non_modal: u64,
}

fn coverage_kind(census_kind: &str) -> Option<&'static str> {
    match census_kind {
        "attribute" => Some("attr"),
        "properties-child" => Some("properties-child"),
        _ => None,
    }
}

/// The census rows, keyed (part, element, attribute, kind), from the
/// vendored census JSON.
fn census_rows(text: &str, file: &str) -> Result<(CensusId, Vec<CensusRow>), String> {
    use sha2::{Digest, Sha256};
    let census: Census = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
    if census.schema != CENSUS_SCHEMA {
        return Err(format!(
            "{file}: schema {} — this tool reads {CENSUS_SCHEMA} (per-attribute parts)",
            census.schema
        ));
    }
    let sha256 = Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let id = CensusId {
        file: file.to_string(),
        schema: census.schema.clone(),
        generated_at: census.generated_at.clone(),
        sha256,
        documents: census.totals.documents,
    };
    let mut rows = Vec::new();
    for e in &census.elements {
        for (a, ca) in &e.attributes {
            for k in ca.kinds.keys() {
                let Some(kind) = coverage_kind(k) else {
                    return Err(format!("{file}: {}@{a}: unknown kind {k}", e.element));
                };
                let parts = if ca.kinds.len() > 1 {
                    ca.kind_parts.get(k).ok_or_else(|| {
                        format!("{file}: {}@{a}: two kinds but no kindParts", e.element)
                    })?
                } else {
                    &ca.parts
                };
                for (p, &n) in parts {
                    if !PART_CLASSES.contains(&p.as_str()) {
                        return Err(format!("{file}: {}@{a}: unknown part {p}", e.element));
                    }
                    let nm = ca.non_modal_parts.get(p).copied().unwrap_or(0);
                    rows.push(CensusRow {
                        part: p.clone(),
                        element: e.element.clone(),
                        attribute: a.clone(),
                        kind,
                        doc_count: n,
                        non_modal: nm.min(n).min(ca.non_modal_docs),
                    });
                }
            }
        }
    }
    Ok((id, rows))
}

// ---------------------------------------------------------------- the join

/// `[[unseen]]` in `manual.toml`: why the importer reads something no
/// document in the census carries.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnseenEntry {
    /// `Element@Attribute`, any kind.
    #[serde(default)]
    pub keys: Vec<String>,
    /// Every unseen read on these elements.
    #[serde(default)]
    pub elements: Vec<String>,
    pub reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartRow {
    pub part: String,
    pub element: String,
    pub attribute: String,
    pub kind: &'static str,
    pub doc_count: u64,
    pub non_modal_docs: u64,
    pub imported: bool,
    pub exported: bool,
    /// Read, but only from other parts.
    pub read_elsewhere: bool,
}

impl PartRow {
    pub fn key(&self) -> String {
        format!(
            "{}|{}@{}|{}",
            self.part, self.element, self.attribute, self.kind
        )
    }

    /// The work-order rank: documents whose value departs from the mode,
    /// times documents that carry the attribute in this part. The first
    /// factor is what reading the attribute changes if the engine's default
    /// is the corpus mode; the second hedges for a default that is not.
    pub fn impact(&self) -> u64 {
        self.non_modal_docs * self.doc_count
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Misplaced {
    pub element: String,
    pub attribute: String,
    pub kind: &'static str,
    pub read_parts: Vec<String>,
    pub census_parts: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unseen {
    pub element: String,
    pub attribute: String,
    pub kind: &'static str,
    pub reason: Option<String>,
}

/// A share of the census: what is covered over what there is.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Ratio {
    pub covered: u64,
    pub total: u64,
    pub value: f64,
}

impl Ratio {
    fn new(covered: u64, total: u64) -> Self {
        let value = if total == 0 {
            0.0
        } else {
            (covered as f64 / total as f64 * 1e6).round() / 1e6
        };
        Ratio {
            covered,
            total,
            value,
        }
    }

    /// `self` against `base`, exactly (cross-multiplied).
    fn cmp(&self, base: &Ratio) -> std::cmp::Ordering {
        (u128::from(self.covered) * u128::from(base.total))
            .cmp(&(u128::from(base.covered) * u128::from(self.total)))
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    /// Σ documents over (part, element, attribute, kind) rows the importer
    /// reads in that part / Σ documents over every census row.
    pub weighted_import: Ratio,
    /// The same, weighted by documents whose value departs from the mode.
    pub impact_import: Ratio,
    /// As `weightedImport`, for the exporter's writes.
    pub weighted_export: Ratio,
    /// (element, attribute, kind) read, but from none of the parts the
    /// census finds it in.
    pub misplaced_reads: usize,
    /// (element, attribute, kind) read that no census document carries.
    pub unseen_reads: usize,
    /// Unseen reads with no `[[unseen]]` justification.
    pub unjustified_unseen_reads: usize,
}

pub struct Joined {
    pub census: CensusId,
    pub part_rows: Vec<PartRow>,
    pub metrics: Metrics,
    pub misplaced: Vec<Misplaced>,
    pub unseen: Vec<Unseen>,
}

/// Join the inventory rows with the census.
pub fn join(
    census_text: &str,
    census_file: &str,
    rows: &[Row],
    unseen_entries: &[UnseenEntry],
    manual_file: &str,
) -> Result<Joined, String> {
    let (census, crows) = census_rows(census_text, census_file)?;

    type Key = (String, String, &'static str);
    let mut census_parts: BTreeMap<Key, BTreeSet<String>> = BTreeMap::new();
    for r in &crows {
        census_parts
            .entry((r.element.clone(), r.attribute.clone(), r.kind))
            .or_default()
            .insert(r.part.clone());
    }
    let coverage: BTreeMap<Key, &Row> = rows
        .iter()
        .map(|r| ((r.element.clone(), r.attribute.clone(), r.kind), r))
        .collect();

    let mut part_rows = Vec::with_capacity(crows.len());
    let (mut docs, mut docs_read, mut docs_written) = (0u64, 0u64, 0u64);
    let (mut nm, mut nm_read) = (0u64, 0u64);
    for c in crows {
        let cov = coverage.get(&(c.element.clone(), c.attribute.clone(), c.kind));
        let imported = cov.is_some_and(|r| r.import_parts.contains(&c.part));
        let exported = cov.is_some_and(|r| r.export_parts.contains(&c.part));
        let read_elsewhere = !imported && cov.is_some_and(|r| r.imported);
        docs += c.doc_count;
        nm += c.non_modal;
        if imported {
            docs_read += c.doc_count;
            nm_read += c.non_modal;
        }
        if exported {
            docs_written += c.doc_count;
        }
        part_rows.push(PartRow {
            part: c.part,
            element: c.element,
            attribute: c.attribute,
            kind: c.kind,
            doc_count: c.doc_count,
            non_modal_docs: c.non_modal,
            imported,
            exported,
            read_elsewhere,
        });
    }

    let mut misplaced = Vec::new();
    let mut unseen = Vec::new();
    let mut matched_keys: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut matched_elements: BTreeSet<(usize, String)> = BTreeSet::new();
    for r in rows.iter().filter(|r| r.imported) {
        let key = (r.element.clone(), r.attribute.clone(), r.kind);
        match census_parts.get(&key) {
            Some(cp) => {
                if !r.import_parts.iter().any(|p| cp.contains(p)) {
                    misplaced.push(Misplaced {
                        element: r.element.clone(),
                        attribute: r.attribute.clone(),
                        kind: r.kind,
                        read_parts: r.import_parts.clone(),
                        census_parts: cp.iter().cloned().collect(),
                    });
                }
            }
            None => {
                let ea = format!("{}@{}", r.element, r.attribute);
                let mut reason = None;
                for (i, u) in unseen_entries.iter().enumerate() {
                    let by_key = u.keys.contains(&ea);
                    let by_element = u.elements.contains(&r.element);
                    if by_key {
                        matched_keys.insert((i, ea.clone()));
                    }
                    if by_element {
                        matched_elements.insert((i, r.element.clone()));
                    }
                    if (by_key || by_element) && reason.is_none() {
                        reason = Some(u.reason.clone());
                    }
                }
                unseen.push(Unseen {
                    element: r.element.clone(),
                    attribute: r.attribute.clone(),
                    kind: r.kind,
                    reason,
                });
            }
        }
    }
    // Every `[[unseen]]` name must still be an unseen read.
    for (i, u) in unseen_entries.iter().enumerate() {
        for k in &u.keys {
            if !matched_keys.contains(&(i, k.clone())) {
                return Err(format!(
                    "{manual_file}: [[unseen]] names {k}, which is no longer an unseen read \
                     (the census has it, or the importer no longer reads it)"
                ));
            }
        }
        for e in &u.elements {
            if !matched_elements.contains(&(i, e.clone())) {
                return Err(format!(
                    "{manual_file}: [[unseen]] names element {e}, which has no unseen read left"
                ));
            }
        }
    }

    let metrics = Metrics {
        weighted_import: Ratio::new(docs_read, docs),
        impact_import: Ratio::new(nm_read, nm),
        weighted_export: Ratio::new(docs_written, docs),
        misplaced_reads: misplaced.len(),
        unseen_reads: unseen.len(),
        unjustified_unseen_reads: unseen.iter().filter(|u| u.reason.is_none()).count(),
    };
    Ok(Joined {
        census,
        part_rows,
        metrics,
        misplaced,
        unseen,
    })
}

// ---------------------------------------------------------------- ratchet

/// The committed baseline: `docs/reference/idml-coverage-ratchet.json`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Ratchet {
    pub census: CensusId,
    pub metrics: Metrics,
    /// `part|Element@Attribute|kind` the importer reads in that part.
    pub covered_import: Vec<String>,
    /// The same for the exporter's writes.
    pub covered_export: Vec<String>,
    /// Reads and writes the tool could not attribute (`unresolved`).
    pub unresolved: usize,
}

impl Ratchet {
    pub fn of(j: &Joined, unresolved: usize) -> Self {
        let keys = |f: &dyn Fn(&PartRow) -> bool| -> Vec<String> {
            let set: BTreeSet<String> = j
                .part_rows
                .iter()
                .filter(|r| f(r))
                .map(PartRow::key)
                .collect();
            set.into_iter().collect()
        };
        Ratchet {
            census: j.census.clone(),
            metrics: j.metrics.clone(),
            covered_import: keys(&|r| r.imported),
            covered_export: keys(&|r| r.exported),
            unresolved,
        }
    }
}

/// What differs between a baseline and now. `regressions` are what the
/// ratchet exists to stop; `unrecorded` are gains (or neutral changes) the
/// baseline has not taken in yet.
#[derive(Default)]
pub struct Diff {
    pub regressions: Vec<String>,
    pub unrecorded: Vec<String>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.regressions.is_empty() && self.unrecorded.is_empty()
    }
}

fn list(keys: &[String]) -> String {
    const SHOW: usize = 12;
    let mut s = keys
        .iter()
        .take(SHOW)
        .map(|k| format!("\n      {k}"))
        .collect::<String>();
    if keys.len() > SHOW {
        s.push_str(&format!("\n      … and {} more", keys.len() - SHOW));
    }
    s
}

pub fn compare(base: &Ratchet, now: &Ratchet) -> Diff {
    let mut d = Diff::default();
    if base.census != now.census {
        d.regressions.push(format!(
            "the census changed ({} {} → {} {}); every share is measured against it",
            base.census.generated_at,
            &base.census.sha256[..12.min(base.census.sha256.len())],
            now.census.generated_at,
            &now.census.sha256[..12.min(now.census.sha256.len())],
        ));
    }
    for (what, b, n) in [
        ("import", &base.covered_import, &now.covered_import),
        ("export", &base.covered_export, &now.covered_export),
    ] {
        let bs: BTreeSet<&String> = b.iter().collect();
        let ns: BTreeSet<&String> = n.iter().collect();
        let dropped: Vec<String> = bs.difference(&ns).map(|s| (*s).clone()).collect();
        let gained: Vec<String> = ns.difference(&bs).map(|s| (*s).clone()).collect();
        if !dropped.is_empty() {
            d.regressions.push(format!(
                "{} covered {what} key(s) dropped:{}",
                dropped.len(),
                list(&dropped)
            ));
        }
        if !gained.is_empty() {
            d.unrecorded.push(format!(
                "{} newly covered {what} key(s):{}",
                gained.len(),
                list(&gained)
            ));
        }
    }
    let (bm, nm) = (&base.metrics, &now.metrics);
    for (name, b, n) in [
        ("weightedImport", bm.weighted_import, nm.weighted_import),
        ("impactImport", bm.impact_import, nm.impact_import),
        ("weightedExport", bm.weighted_export, nm.weighted_export),
    ] {
        match n.cmp(&b) {
            std::cmp::Ordering::Less => d
                .regressions
                .push(format!("{name} fell: {} → {}", b.value, n.value)),
            std::cmp::Ordering::Greater => d
                .unrecorded
                .push(format!("{name} rose: {} → {}", b.value, n.value)),
            std::cmp::Ordering::Equal => {}
        }
    }
    for (name, b, n) in [
        ("unresolved", base.unresolved, now.unresolved),
        ("misplacedReads", bm.misplaced_reads, nm.misplaced_reads),
        (
            "unjustifiedUnseenReads",
            bm.unjustified_unseen_reads,
            nm.unjustified_unseen_reads,
        ),
    ] {
        if n > b {
            d.regressions.push(format!("{name} rose: {b} → {n}"));
        } else if n < b {
            d.unrecorded.push(format!("{name} fell: {b} → {n}"));
        }
    }
    if bm.unseen_reads != nm.unseen_reads {
        d.unrecorded.push(format!(
            "unseenReads changed: {} → {}",
            bm.unseen_reads, nm.unseen_reads
        ));
    }
    d
}

/// The uncovered part rows, highest impact first.
pub fn queue(part_rows: &[PartRow]) -> Vec<&PartRow> {
    let mut q: Vec<&PartRow> = part_rows.iter().filter(|r| !r.imported).collect();
    q.sort_by(|a, b| {
        b.impact()
            .cmp(&a.impact())
            .then(b.doc_count.cmp(&a.doc_count))
            .then_with(|| a.key().cmp(&b.key()))
    });
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ratchet(import: &[&str], covered: u64, misplaced: usize) -> Ratchet {
        let r = |c| Ratio::new(c, 100);
        Ratchet {
            census: CensusId {
                file: "census.json".into(),
                schema: CENSUS_SCHEMA.into(),
                generated_at: "2026-10-12".into(),
                sha256: "00".into(),
                documents: 1,
            },
            metrics: Metrics {
                weighted_import: r(covered),
                impact_import: r(covered),
                weighted_export: r(0),
                misplaced_reads: misplaced,
                unseen_reads: 0,
                unjustified_unseen_reads: 0,
            },
            covered_import: import.iter().map(|s| s.to_string()).collect(),
            covered_export: vec![],
            unresolved: 0,
        }
    }

    #[test]
    fn ratchet_compare_classifies_drops_as_regressions_and_gains_as_unrecorded() {
        let base = ratchet(&["a", "b"], 10, 3);
        assert!(compare(&base, &base).is_empty());

        let dropped = compare(&base, &ratchet(&["a"], 9, 3));
        assert_eq!(dropped.regressions.len(), 3, "a key and two shares");
        assert!(dropped.unrecorded.is_empty());

        let gained = compare(&base, &ratchet(&["a", "b", "c"], 11, 2));
        assert!(gained.regressions.is_empty());
        assert_eq!(gained.unrecorded.len(), 4, "a key, two shares, misplaced");

        let misplaced = compare(&base, &ratchet(&["a", "b"], 10, 4));
        assert_eq!(misplaced.regressions.len(), 1);
    }

    #[test]
    fn queue_ranks_uncovered_rows_by_impact() {
        let row = |a: &str, docs, nm, imported| PartRow {
            part: "designmap".into(),
            element: "E".into(),
            attribute: a.into(),
            kind: "attr",
            doc_count: docs,
            non_modal_docs: nm,
            imported,
            exported: false,
            read_elsewhere: false,
        };
        let rows = [
            row("Wide", 300, 3, false),
            row("Read", 300, 300, true),
            row("Varied", 40, 40, false),
            row("Rare", 2, 2, false),
        ];
        let q: Vec<&str> = queue(&rows).iter().map(|r| r.attribute.as_str()).collect();
        assert_eq!(q, ["Varied", "Wide", "Rare"]);
    }
}
