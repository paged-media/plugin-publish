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

//! IDML coverage inventory: which elements and attributes `idml-import`
//! reads and `idml-export` writes, found by walking their sources with `syn`
//! and merged with the hand-kept `manual.toml`.
//!
//! The output, `docs/reference/idml-coverage.json`, is committed and
//! byte-stable (no timestamps, no revisions); the
//! `idml_coverage_json_is_current` test fails when it drifts from the code.
//!
//! Each read and write also carries the package part it works on (seeded by
//! `[[part_entry]]`, carried down the call graph), and the rows are joined
//! with the vendored corpus census into part-aware shares
//! ([`parts`]). `docs/reference/idml-coverage-ratchet.json` holds the
//! baseline the `idml_coverage_ratchet` test keeps from falling.

pub mod parts;
mod scan;
mod types;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use syn::visit::Visit;

use scan::{FnId, Kind, Reader, Scan, Side, Tables, Walker};
use types::StructMap;

pub const IMPORT_SRC: &str = "crates/idml-import/src";
pub const EXPORT_SRC: &str = "crates/idml-export/src";
pub const MANUAL: &str = "tools/idml-coverage/manual.toml";
pub const CENSUS: &str = "tools/idml-coverage/census.json";
pub const OUTPUT: &str = "docs/reference/idml-coverage.json";
pub const RATCHET: &str = "docs/reference/idml-coverage-ratchet.json";

// ---------------------------------------------------------------- manual

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Manual {
    #[serde(default, rename = "fn")]
    fns: Vec<FnMap>,
    #[serde(default)]
    properties_kind: Vec<PropKind>,
    #[serde(default)]
    properties_child: Vec<PropChild>,
    #[serde(default)]
    read: Vec<ManualRead>,
    #[serde(default)]
    export: Vec<ManualExport>,
    #[serde(default)]
    ignore: Vec<Ignore>,
    #[serde(default)]
    part_entry: Vec<PartEntry>,
    #[serde(default)]
    unseen: Vec<parts::UnseenEntry>,
}

/// The package part(s) a function reads or writes; its callees inherit them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartEntry {
    file: String,
    name: String,
    parts: Vec<String>,
}

/// A function's element(s), replacing call-site inference.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FnMap {
    file: String,
    name: String,
    elements: Vec<String>,
}

/// `(N, b"Child")` arms: which element's `<Properties>` kind `N` is.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PropKind {
    file: String,
    kind: i64,
    element: String,
    #[serde(default)]
    rename: BTreeMap<String, String>,
}

/// Properties children read as text by a pending-property arm.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PropChild {
    file: String,
    item: String,
    children: Vec<String>,
    elements: Vec<String>,
    #[serde(default = "string_type")]
    coarse_type: String,
}

/// Reads `syn` cannot attribute (computed keys).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualRead {
    file: String,
    item: String,
    attributes: Vec<String>,
    /// Empty: the elements inferred for `item`.
    #[serde(default)]
    elements: Vec<String>,
    /// The literals that must still be in `item` (default: `attributes`) —
    /// for keys assembled from parts (`RuleAbove` + `Color`).
    #[serde(default)]
    verify: Vec<String>,
    #[serde(default = "string_type")]
    coarse_type: String,
    #[serde(default)]
    field: Option<String>,
}

/// Writes `syn` cannot attribute.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualExport {
    file: String,
    item: String,
    attributes: Vec<String>,
    elements: Vec<String>,
    #[serde(default)]
    properties_child: bool,
    /// Prepended to each attribute in the row (`TabList/ListItem/`).
    #[serde(default)]
    prefix: String,
}

/// Literals whose walk attribution is dropped: they are not reads or
/// writes at all, or a `[[read]]` / `[[export]]` attributes them instead.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ignore {
    file: String,
    item: String,
    literals: Vec<String>,
    #[allow(dead_code)]
    reason: String,
}

fn string_type() -> String {
    "string".to_string()
}

// ---------------------------------------------------------------- output

#[derive(Serialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Source {
    pub file: String,
    pub line: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub element: String,
    pub attribute: String,
    pub kind: &'static str,
    pub importer_field: Option<String>,
    pub coarse_type: Option<String>,
    pub imported: bool,
    pub exported: bool,
    /// The package parts the importer reads it from.
    pub import_parts: Vec<String>,
    /// The package parts the exporter writes it to.
    pub export_parts: Vec<String>,
    pub source: Source,
    pub export_source: Option<Source>,
}

#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct Unresolved {
    pub file: String,
    pub line: usize,
    pub side: &'static str,
    pub item: String,
    pub literal: String,
    pub reason: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Counts {
    rows: usize,
    elements: usize,
    attributes: usize,
    imported: usize,
    exported: usize,
    imported_and_exported: usize,
    import_only: usize,
    export_only: usize,
    properties_child: usize,
    importer_read_sites: usize,
    exporter_write_sites: usize,
    unresolved: usize,
    part_rows: usize,
    misplaced: usize,
    unseen: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Output<'a> {
    generated_from: BTreeMap<&'static str, &'static str>,
    census: &'a parts::CensusId,
    counts: Counts,
    metrics: &'a parts::Metrics,
    rows: &'a [Row],
    misplaced: &'a [parts::Misplaced],
    unseen: &'a [parts::Unseen],
    unresolved: Vec<Unresolved>,
}

// ---------------------------------------------------------------- sources

struct SourceFile {
    rel: String,
    ast: syn::File,
}

fn rs_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for e in entries {
            let p = e.map_err(|e| e.to_string())?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn load(root: &Path, rel_dir: &str) -> Result<Vec<SourceFile>, String> {
    rs_files(&root.join(rel_dir))?
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let ast = syn::parse_file(&text).map_err(|e| format!("{}: {e}", p.display()))?;
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            Ok(SourceFile { rel, ast })
        })
        .collect()
}

/// The `paged-model` sources the importer builds into, located through
/// `cargo metadata` (a pinned git dependency).
fn paged_model_src(root: &Path) -> Result<PathBuf, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--offline"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed (fetch dependencies first: `cargo fetch`): {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let manifest = v["packages"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["name"] == "paged-model"))
        .and_then(|p| p["manifest_path"].as_str())
        .ok_or("cargo metadata: no paged-model package")?;
    Ok(Path::new(manifest)
        .parent()
        .ok_or("paged-model manifest has no parent")?
        .join("src"))
}

// ---------------------------------------------------------------- pre-passes

struct FnBody {
    name: String,
    file: String,
    params: Vec<String>,
    body: syn::Expr,
    ret: Option<String>,
}

fn collect_fns(files: &[SourceFile]) -> Vec<FnBody> {
    struct V<'a> {
        file: &'a str,
        out: Vec<FnBody>,
    }
    fn is_test(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|a| {
            let t = scan::compact(a);
            t == "#[test]" || t.starts_with("#[cfg(test")
        })
    }
    impl V<'_> {
        fn push(&mut self, sig: &syn::Signature, block: &syn::Block) {
            self.out.push(FnBody {
                name: sig.ident.to_string(),
                file: self.file.to_string(),
                params: scan::fn_params(sig),
                body: syn::Expr::Block(syn::ExprBlock {
                    attrs: vec![],
                    label: None,
                    block: block.clone(),
                }),
                ret: match &sig.output {
                    syn::ReturnType::Type(_, t) => Some(scan::compact(&**t)),
                    syn::ReturnType::Default => None,
                },
            });
        }
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            if !is_test(&m.attrs) {
                syn::visit::visit_item_mod(self, m);
            }
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            if !is_test(&f.attrs) {
                self.push(&f.sig, &f.block);
                syn::visit::visit_item_fn(self, f);
            }
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            if !is_test(&f.attrs) {
                self.push(&f.sig, &f.block);
                syn::visit::visit_impl_item_fn(self, f);
            }
        }
    }
    let mut out = Vec::new();
    for f in files {
        let mut v = V {
            file: &f.rel,
            out: vec![],
        };
        v.visit_file(&f.ast);
        out.extend(v.out);
    }
    out
}

/// `attr` and its wrappers, then every function that forwards a key
/// parameter into one of them, to a fixpoint.
fn readers(fns: &[FnBody], structs: &StructMap) -> HashMap<String, Reader> {
    let mut r: HashMap<String, Reader> = HashMap::new();
    let base = |idx: Option<usize>, c: &str| Reader {
        key_idx: idx,
        coarse: c.to_string(),
    };
    r.insert("attr".into(), base(Some(1), "string"));
    r.insert("attr_unescaped".into(), base(Some(1), "string"));
    r.insert("parse_f".into(), base(Some(1), "number"));
    r.insert("parse_tint_attr".into(), base(Some(1), "tint"));
    r.insert("label_value".into(), base(None, "string"));
    loop {
        let mut changed = false;
        for f in fns.iter().filter(|f| !f.file.ends_with("/util.rs")) {
            if r.contains_key(&f.name) {
                continue;
            }
            for (i, p) in f.params.iter().enumerate() {
                if let Some(chain) = scan::find_param_read(&f.body, p, &r) {
                    let ret = f
                        .ret
                        .as_deref()
                        .and_then(|t| types::coarse_of_type(t, structs));
                    let coarse = match (chain, ret) {
                        (Some(c), _) if c != "parsed" => c.to_string(),
                        (_, Some(t)) => t,
                        (Some(c), None) => c.to_string(),
                        (None, None) => "string".to_string(),
                    };
                    r.insert(f.name.clone(), base(Some(i), &coarse));
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            return r;
        }
    }
}

/// Functions that push a `(key, value)` whose key is a parameter
/// (`push_opt_f32(out, k, v)`), to a fixpoint.
fn writers(fns: &[FnBody]) -> HashMap<String, usize> {
    let mut w: HashMap<String, usize> = HashMap::new();
    loop {
        let mut changed = false;
        for f in fns {
            if w.contains_key(&f.name) {
                continue;
            }
            for (i, p) in f.params.iter().enumerate() {
                if p != "self" && !p.is_empty() && scan::writes_param_key(&f.body, p, &w) {
                    w.insert(f.name.clone(), i);
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            return w;
        }
    }
}

// ---------------------------------------------------------------- resolve

pub(crate) fn fns_by_name(scan: &Scan) -> HashMap<String, Vec<FnId>> {
    let mut by_name: HashMap<String, Vec<FnId>> = HashMap::new();
    for f in &scan.fns {
        by_name.entry(f.1.clone()).or_default().push(f.clone());
    }
    by_name
}

pub(crate) fn resolve_callee<'a>(
    by_name: &'a HashMap<String, Vec<FnId>>,
    file: &str,
    callee: &str,
) -> Option<&'a FnId> {
    let cands = by_name.get(callee)?;
    cands
        .iter()
        .find(|(f, _)| f == file)
        .or(if cands.len() == 1 {
            cands.first()
        } else {
            None
        })
}

/// Each function's elements: the manual map, else what its call sites say,
/// to a fixpoint. Strong evidence — an emitting call's element literal, or a
/// local bound to an element passed in — beats weak evidence (the enclosing
/// arm, or the caller's own elements): a helper called both as
/// `emit_start_with_attrs(w, "ParagraphStyleRange", &paragraph_attrs(p))` and
/// from deep inside an `idPkg:Story` arm is a `ParagraphStyleRange` helper.
fn fn_elements(
    scan: &Scan,
    manual: &HashMap<FnId, Vec<String>>,
) -> HashMap<FnId, BTreeSet<String>> {
    let by_name = fns_by_name(scan);
    let mut strong: HashMap<FnId, BTreeSet<String>> = manual
        .iter()
        .map(|(k, v)| (k.clone(), v.iter().cloned().collect()))
        .collect();
    let mut weak: HashMap<FnId, BTreeSet<String>> = HashMap::new();
    let effective = |strong: &HashMap<FnId, BTreeSet<String>>,
                     weak: &HashMap<FnId, BTreeSet<String>>,
                     f: &FnId|
     -> BTreeSet<String> {
        match strong.get(f) {
            Some(s) if !s.is_empty() => s.clone(),
            _ => weak.get(f).cloned().unwrap_or_default(),
        }
    };
    loop {
        let mut changed = false;
        for c in &scan.calls {
            let Some(callee) = resolve_callee(&by_name, &c.caller.0, &c.callee) else {
                continue;
            };
            if manual.contains_key(callee) || *callee == c.caller {
                continue;
            }
            let bound: BTreeSet<String> = match &c.bound {
                Some(b) => [b.clone()].into(),
                None => scan
                    .bindings
                    .get(&c.caller)
                    .map(|b| {
                        c.arg_vars
                            .iter()
                            .filter_map(|v| b.get(v))
                            .flatten()
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            let (target, contrib) = if !bound.is_empty() {
                (&mut strong, bound)
            } else if !c.ctx.is_empty() {
                (&mut weak, c.ctx.iter().cloned().collect())
            } else if strong.get(&c.caller).is_some_and(|s| !s.is_empty()) {
                // Inheriting from a caller whose elements are strong is strong.
                let e = effective(&strong, &weak, &c.caller);
                (&mut strong, e)
            } else {
                let e = effective(&strong, &weak, &c.caller);
                (&mut weak, e)
            };
            let entry = target.entry(callee.clone()).or_default();
            for e in contrib {
                changed |= entry.insert(e);
            }
        }
        if !changed {
            let all: BTreeSet<FnId> = strong.keys().chain(weak.keys()).cloned().collect();
            return all
                .into_iter()
                .map(|f| {
                    let e = effective(&strong, &weak, &f);
                    (f, e)
                })
                .collect();
        }
    }
}

#[derive(Default)]
struct Acc {
    reads: Vec<(Source, Option<String>, String)>,
    writes: Vec<Source>,
    read_parts: BTreeSet<String>,
    write_parts: BTreeSet<String>,
}

fn type_rank(t: &str) -> u8 {
    match t {
        "string" => 0,
        "parsed" => 1,
        _ => 2,
    }
}

fn manual_line(
    scan: &Scan,
    file: &str,
    item: &str,
    literal: &str,
    what: &str,
) -> Result<usize, String> {
    scan.lits
        .iter()
        .filter(|l| l.func.0 == file && l.func.1 == item && l.literal == literal)
        .map(|l| l.line)
        .min()
        .ok_or_else(|| {
            format!("{MANUAL}: {what} names `{literal}` in {file}::{item}, which no longer has it")
        })
}

/// The inventory and its census join.
pub struct Analysis {
    /// `docs/reference/idml-coverage.json`, as it should be committed.
    pub json: String,
    pub joined: parts::Joined,
    /// How many reads and writes the tool could not attribute.
    pub unresolved: usize,
}

impl Analysis {
    /// The ratchet baseline this analysis would record.
    pub fn ratchet(&self) -> parts::Ratchet {
        parts::Ratchet::of(&self.joined, self.unresolved)
    }
}

/// Build the coverage JSON for the repository at `root`.
pub fn generate(root: &Path) -> Result<String, String> {
    analyse(root).map(|a| a.json)
}

/// Walk the sources, join them with the census and build the coverage JSON.
pub fn analyse(root: &Path) -> Result<Analysis, String> {
    let manual_text =
        std::fs::read_to_string(root.join(MANUAL)).map_err(|e| format!("{MANUAL}: {e}"))?;
    let manual: Manual = toml::from_str(&manual_text).map_err(|e| format!("{MANUAL}: {e}"))?;

    let import = load(root, IMPORT_SRC)?;
    let export = load(root, EXPORT_SRC)?;

    let mut structs = StructMap::default();
    for p in rs_files(&paged_model_src(root)?)? {
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let ast = syn::parse_file(&text).map_err(|e| format!("{}: {e}", p.display()))?;
        structs.add_file(&ast);
    }
    for f in &import {
        structs.add_file(&f.ast);
    }

    let import_fns = collect_fns(&import);
    let export_fns = collect_fns(&export);
    let readers = readers(&import_fns, &structs);
    let writers = writers(&export_fns);
    let no_writers = HashMap::new();
    let no_readers = HashMap::new();
    let prop_kinds: scan::PropKinds = manual
        .properties_kind
        .iter()
        .map(|p| {
            (
                (p.file.clone(), p.kind),
                (
                    p.element.clone(),
                    p.rename
                        .iter()
                        .map(|(a, b)| (a.clone(), b.clone()))
                        .collect(),
                ),
            )
        })
        .collect();
    let no_props = HashMap::new();

    let mut iscan = Scan::default();
    let itables = Tables {
        readers: &readers,
        writers: &no_writers,
        structs: &structs,
        prop_kinds: &prop_kinds,
    };
    for f in &import {
        Walker::new(Side::Import, f.rel.clone(), &itables, &mut iscan).visit_file(&f.ast);
    }
    let mut escan = Scan::default();
    let etables = Tables {
        readers: &no_readers,
        writers: &writers,
        structs: &structs,
        prop_kinds: &no_props,
    };
    for f in &export {
        Walker::new(Side::Export, f.rel.clone(), &etables, &mut escan).visit_file(&f.ast);
    }

    let manual_fns: HashMap<FnId, Vec<String>> = manual
        .fns
        .iter()
        .map(|m| ((m.file.clone(), m.name.clone()), m.elements.clone()))
        .collect();
    for m in &manual.fns {
        let id = (m.file.clone(), m.name.clone());
        if !iscan.fns.contains(&id) && !escan.fns.contains(&id) {
            return Err(format!(
                "{MANUAL}: fn {}::{} does not exist",
                m.file, m.name
            ));
        }
    }
    let iels = fn_elements(&iscan, &manual_fns);
    let eels = fn_elements(&escan, &manual_fns);

    // Package parts: the `[[part_entry]]` seeds, carried down each crate's
    // call graph.
    let mut iseeds: HashMap<FnId, BTreeSet<String>> = HashMap::new();
    let mut eseeds: HashMap<FnId, BTreeSet<String>> = HashMap::new();
    for m in &manual.part_entry {
        let id = (m.file.clone(), m.name.clone());
        let seeds = if iscan.fns.contains(&id) {
            &mut iseeds
        } else if escan.fns.contains(&id) {
            &mut eseeds
        } else {
            return Err(format!(
                "{MANUAL}: [[part_entry]] fn {}::{} does not exist",
                m.file, m.name
            ));
        };
        if let Some(p) = m
            .parts
            .iter()
            .find(|p| !parts::PART_CLASSES.contains(&p.as_str()))
        {
            return Err(format!(
                "{MANUAL}: [[part_entry]] {}::{} names `{p}`, not a part class ({})",
                m.file,
                m.name,
                parts::PART_CLASSES.join(", ")
            ));
        }
        if m.parts.is_empty()
            || seeds
                .insert(id, m.parts.iter().cloned().collect())
                .is_some()
        {
            return Err(format!(
                "{MANUAL}: [[part_entry]] {}::{} is empty or listed twice",
                m.file, m.name
            ));
        }
    }
    let iparts = parts::fn_parts(&iscan, &iseeds);
    let eparts = parts::fn_parts(&escan, &eseeds);
    let parts_of = |map: &HashMap<FnId, BTreeSet<String>>, f: &FnId| -> BTreeSet<String> {
        map.get(f).cloned().unwrap_or_default()
    };
    if std::env::var_os("IDML_COVERAGE_DEBUG").is_some() {
        let mut per: BTreeMap<FnId, (usize, usize)> = BTreeMap::new();
        for w in &escan.writes {
            per.entry(w.func.clone()).or_default().0 += 1;
        }
        for r in &iscan.reads {
            per.entry(r.func.clone()).or_default().1 += 1;
        }
        for (f, (w, r)) in per {
            let els = eels.get(&f).or_else(|| iels.get(&f));
            let parts = eparts.get(&f).or_else(|| iparts.get(&f));
            eprintln!(
                "{}::{} writes={w} reads={r} fn-elements={:?} parts={:?}",
                f.0, f.1, els, parts
            );
        }
    }

    let mut acc: BTreeMap<(String, String, Kind), Acc> = BTreeMap::new();
    let mut unresolved: BTreeSet<Unresolved> = BTreeSet::new();
    let mut read_sites = 0;
    let mut write_sites = 0;

    let ignored: BTreeSet<(String, String, String)> = manual
        .ignore
        .iter()
        .flat_map(|m| {
            m.literals
                .iter()
                .map(|l| (m.file.clone(), m.item.clone(), l.clone()))
        })
        .collect();
    let is_ignored = |file: &str, f: &FnId, lit: &str| {
        ignored.contains(&(file.to_string(), f.1.clone(), lit.to_string()))
    };

    for r in &iscan.reads {
        if is_ignored(&r.file, &r.func, &r.attribute) {
            continue;
        }
        read_sites += 1;
        let elements: BTreeSet<String> = if r.ctx.is_empty() {
            iels.get(&r.func).cloned().unwrap_or_default()
        } else {
            r.ctx.iter().cloned().collect()
        };
        if elements.is_empty() {
            unresolved.insert(Unresolved {
                file: r.file.clone(),
                line: r.line,
                side: "import",
                item: r.func.1.clone(),
                literal: r.attribute.clone(),
                reason: "read with no element context".into(),
            });
            continue;
        }
        let src = Source {
            file: r.file.clone(),
            line: r.line,
        };
        let rparts = parts_of(&iparts, &r.func);
        if rparts.is_empty() {
            unresolved.insert(Unresolved {
                file: r.file.clone(),
                line: r.line,
                side: "import",
                item: r.func.1.clone(),
                literal: r.attribute.clone(),
                reason: "read in a function no [[part_entry]] reaches".into(),
            });
        }
        for e in elements {
            let a = acc.entry((e, r.attribute.clone(), r.kind)).or_default();
            a.reads
                .push((src.clone(), r.field.clone(), r.coarse.clone()));
            a.read_parts.extend(rparts.iter().cloned());
        }
    }

    for w in &escan.writes {
        if is_ignored(&w.file, &w.func, &w.attribute) {
            continue;
        }
        write_sites += 1;
        let bound = w
            .sink
            .as_ref()
            .and_then(|s| escan.bindings.get(&w.func).and_then(|b| b.get(s)))
            .cloned()
            .unwrap_or_default();
        let elements: BTreeSet<String> = if !bound.is_empty() {
            bound
        } else if !w.ctx.is_empty() {
            w.ctx.iter().cloned().collect()
        } else {
            let e = eels.get(&w.func).cloned().unwrap_or_default();
            if std::env::var_os("IDML_COVERAGE_DEBUG").is_some() {
                eprintln!(
                    "FALLBACK {}:{} {} {} sink={:?} -> {:?}",
                    w.file, w.line, w.func.1, w.attribute, w.sink, e
                );
            }
            e
        };
        if elements.is_empty() {
            unresolved.insert(Unresolved {
                file: w.file.clone(),
                line: w.line,
                side: "export",
                item: w.func.1.clone(),
                literal: w.attribute.clone(),
                reason: "write with no element context".into(),
            });
            continue;
        }
        let src = Source {
            file: w.file.clone(),
            line: w.line,
        };
        let wparts = parts_of(&eparts, &w.func);
        if wparts.is_empty() {
            unresolved.insert(Unresolved {
                file: w.file.clone(),
                line: w.line,
                side: "export",
                item: w.func.1.clone(),
                literal: w.attribute.clone(),
                reason: "write in a function no [[part_entry]] reaches".into(),
            });
        }
        for e in elements {
            let a = acc.entry((e, w.attribute.clone(), Kind::Attr)).or_default();
            a.writes.push(src.clone());
            a.write_parts.extend(wparts.iter().cloned());
        }
    }

    // Manual entries: each literal must still be in the item it names.
    let mut covered: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut acknowledged: BTreeSet<(String, String)> = BTreeSet::new();
    for m in &manual.read {
        acknowledged.insert((m.file.clone(), m.item.clone()));
        let mut line = usize::MAX;
        for v in &m.verify {
            line = line.min(manual_line(&iscan, &m.file, &m.item, v, "[[read]]")?);
        }
        let elements: Vec<String> = if m.elements.is_empty() {
            iels.get(&(m.file.clone(), m.item.clone()))
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default()
        } else {
            m.elements.clone()
        };
        if elements.is_empty() {
            return Err(format!(
                "{MANUAL}: [[read]] in {}::{} names no elements and none are inferred",
                m.file, m.item
            ));
        }
        let rparts = parts_of(&iparts, &(m.file.clone(), m.item.clone()));
        for a in &m.attributes {
            let line = if m.verify.is_empty() {
                manual_line(&iscan, &m.file, &m.item, a, "[[read]]")?
            } else {
                line
            };
            covered.insert((m.file.clone(), m.item.clone(), a.clone()));
            for e in &elements {
                let r = acc.entry((e.clone(), a.clone(), Kind::Attr)).or_default();
                r.reads.push((
                    Source {
                        file: m.file.clone(),
                        line,
                    },
                    m.field.clone(),
                    m.coarse_type.clone(),
                ));
                r.read_parts.extend(rparts.iter().cloned());
            }
        }
    }
    for m in &manual.properties_child {
        let rparts = parts_of(&iparts, &(m.file.clone(), m.item.clone()));
        for c in &m.children {
            let line = manual_line(&iscan, &m.file, &m.item, c, "[[properties_child]]")?;
            for e in &m.elements {
                let r = acc
                    .entry((e.clone(), c.clone(), Kind::PropertiesChild))
                    .or_default();
                r.reads.push((
                    Source {
                        file: m.file.clone(),
                        line,
                    },
                    None,
                    m.coarse_type.clone(),
                ));
                r.read_parts.extend(rparts.iter().cloned());
            }
        }
    }
    for m in &manual.export {
        let kind = if m.properties_child {
            Kind::PropertiesChild
        } else {
            Kind::Attr
        };
        let wparts = parts_of(&eparts, &(m.file.clone(), m.item.clone()));
        for a in &m.attributes {
            let line = manual_line(&escan, &m.file, &m.item, a, "[[export]]")?;
            covered.insert((m.file.clone(), m.item.clone(), a.clone()));
            for e in &m.elements {
                let w = acc
                    .entry((e.clone(), format!("{}{a}", m.prefix), kind))
                    .or_default();
                w.writes.push(Source {
                    file: m.file.clone(),
                    line,
                });
                w.write_parts.extend(wparts.iter().cloned());
            }
        }
    }
    for m in &manual.ignore {
        for l in &m.literals {
            let scan = if m.file.starts_with(IMPORT_SRC) {
                &iscan
            } else {
                &escan
            };
            manual_line(scan, &m.file, &m.item, l, "[[ignore]]")?;
            covered.insert((m.file.clone(), m.item.clone(), l.clone()));
        }
    }
    // A manual `[[export]]` resolves the unattributed write it names.
    unresolved.retain(|u| !covered.contains(&(u.file.clone(), u.item.clone(), u.literal.clone())));

    for l in &iscan.loose {
        let key = (l.file.clone(), l.func.1.clone());
        let is_covered = covered.contains(&(l.file.clone(), l.func.1.clone(), l.literal.clone()))
            || (l.reason == "read with a computed key" && acknowledged.contains(&key));
        if !is_covered {
            unresolved.insert(Unresolved {
                file: l.file.clone(),
                line: l.line,
                side: "import",
                item: l.func.1.clone(),
                literal: l.literal.clone(),
                reason: l.reason.into(),
            });
        }
    }

    let rows: Vec<Row> = acc
        .into_iter()
        .map(|((element, attribute, kind), mut a)| {
            a.reads.sort();
            a.writes.sort();
            let importer_field = a.reads.iter().find_map(|(_, f, _)| f.clone());
            let coarse_type = a
                .reads
                .iter()
                .map(|(_, _, t)| t.clone())
                .max_by_key(|t| type_rank(t))
                .and_then(|best| {
                    // Among equally specific spellings, the first read's.
                    a.reads
                        .iter()
                        .map(|(_, _, t)| t.clone())
                        .find(|t| type_rank(t) == type_rank(&best))
                });
            let export_source = a.writes.first().cloned();
            let source = a
                .reads
                .first()
                .map(|(s, _, _)| s.clone())
                .or_else(|| export_source.clone())
                .expect("a row has a read or a write");
            Row {
                element,
                attribute,
                kind: kind.as_str(),
                importer_field,
                coarse_type,
                imported: !a.reads.is_empty(),
                exported: !a.writes.is_empty(),
                import_parts: a.read_parts.into_iter().collect(),
                export_parts: a.write_parts.into_iter().collect(),
                source,
                export_source,
            }
        })
        .collect();

    let census_text =
        std::fs::read_to_string(root.join(CENSUS)).map_err(|e| format!("{CENSUS}: {e}"))?;
    let joined = parts::join(&census_text, CENSUS, &rows, &manual.unseen, MANUAL)?;

    let count = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count();
    let counts = Counts {
        rows: rows.len(),
        elements: rows
            .iter()
            .map(|r| r.element.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        attributes: rows
            .iter()
            .map(|r| r.attribute.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        imported: count(&|r| r.imported),
        exported: count(&|r| r.exported),
        imported_and_exported: count(&|r| r.imported && r.exported),
        import_only: count(&|r| r.imported && !r.exported),
        export_only: count(&|r| !r.imported && r.exported),
        properties_child: count(&|r| r.kind == Kind::PropertiesChild.as_str()),
        importer_read_sites: read_sites,
        exporter_write_sites: write_sites,
        unresolved: unresolved.len(),
        part_rows: joined.part_rows.len(),
        misplaced: joined.misplaced.len(),
        unseen: joined.unseen.len(),
    };
    let generated_from = BTreeMap::from([
        ("importer", IMPORT_SRC),
        ("exporter", EXPORT_SRC),
        ("manual", MANUAL),
        ("census", CENSUS),
        ("tool", "tools/idml-coverage"),
        (
            "regenerate",
            "cargo run -p idml-coverage > docs/reference/idml-coverage.json",
        ),
        ("ratchet", "cargo run -p idml-coverage -- --ratchet"),
    ]);
    let unresolved_count = unresolved.len();
    let out = Output {
        generated_from,
        census: &joined.census,
        counts,
        metrics: &joined.metrics,
        rows: &rows,
        misplaced: &joined.misplaced,
        unseen: &joined.unseen,
        unresolved: unresolved.into_iter().collect(),
    };
    let json = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    // `partRows` last, one compact row per line: the census has thousands,
    // and a line each keeps the file readable and its diffs small.
    let Some(body) = json.strip_suffix("\n}") else {
        return Err("coverage JSON does not end in an object".into());
    };
    let mut json = format!("{body},\n  \"partRows\": [");
    for (i, r) in joined.part_rows.iter().enumerate() {
        json.push_str(if i == 0 { "\n    " } else { ",\n    " });
        json.push_str(&serde_json::to_string(r).map_err(|e| e.to_string())?);
    }
    json.push_str(if joined.part_rows.is_empty() {
        "]\n}\n"
    } else {
        "\n  ]\n}\n"
    });
    Ok(Analysis {
        json,
        joined,
        unresolved: unresolved_count,
    })
}

/// The repository root, from this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed `docs/reference/idml-coverage.json` must match what the
    /// importer and exporter sources say today.
    #[test]
    fn idml_coverage_json_is_current() {
        let root = repo_root();
        let generated = generate(&root).expect("generate the coverage inventory");
        let committed = std::fs::read_to_string(root.join(OUTPUT)).unwrap_or_default();
        assert!(
            committed == generated,
            "{OUTPUT} is stale — regenerate: \
             cargo run -p idml-coverage > {OUTPUT}"
        );
    }

    /// The committed `docs/reference/idml-coverage-ratchet.json` is a floor:
    /// no covered (part, element, attribute) key may drop, no part-aware
    /// share may fall, and unresolved, misplaced or unjustified unseen
    /// reads may not rise. A gain must be recorded too, so the floor moves up with it.
    #[test]
    fn idml_coverage_ratchet() {
        let root = repo_root();
        let now = analyse(&root).expect("analyse the inventory").ratchet();
        let text = std::fs::read_to_string(root.join(RATCHET)).unwrap_or_else(|e| {
            panic!("{RATCHET}: {e} — create it: cargo run -p idml-coverage -- --ratchet")
        });
        let base: parts::Ratchet =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{RATCHET}: {e}"));
        let diff = parts::compare(&base, &now);
        let mut msg = String::new();
        if !diff.regressions.is_empty() {
            msg.push_str(
                "regressions (fix them; a deliberate one is recorded with \
                          `cargo run -p idml-coverage -- --ratchet --allow-regression`):\n",
            );
            for r in &diff.regressions {
                msg.push_str(&format!("  - {r}\n"));
            }
        }
        if !diff.unrecorded.is_empty() {
            msg.push_str(
                "gains the baseline has not recorded (raise it: \
                          `cargo run -p idml-coverage -- --ratchet`):\n",
            );
            for r in &diff.unrecorded {
                msg.push_str(&format!("  - {r}\n"));
            }
        }
        assert!(diff.is_empty(), "{RATCHET}:\n{msg}");
    }
}
