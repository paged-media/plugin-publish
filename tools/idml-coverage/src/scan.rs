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

//! The `syn` walk over one crate's sources.
//!
//! The walk records what it can see locally — attribute reads and writes with
//! the element context of the arm or `if` they sit in, every call between
//! functions, and the literals it could not attribute. Resolving a function's
//! element from its call sites happens afterwards, over the whole crate
//! (`crate::resolve`).

use std::collections::{BTreeSet, HashMap, HashSet};

use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{BinOp, Expr, ExprCall, ExprMethodCall, Lit, Pat, Token};

use crate::types::{coarse_of_type, StructMap};

/// Which crate a scan walks: the importer reads attributes, the exporter
/// writes them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Import,
    Export,
}

/// A function (or const) by file and name. Nested fns get their own id.
pub type FnId = (String, String);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Kind {
    Attr,
    PropertiesChild,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Attr => "attr",
            Kind::PropertiesChild => "properties-child",
        }
    }
}

/// A key-taking reader: `attr(e, key)` and everything built on it.
#[derive(Clone, Debug)]
pub struct Reader {
    /// Index of the key argument; `None` for `label_value(e)`, which always
    /// reads `Value`.
    pub key_idx: Option<usize>,
    pub coarse: String,
}

#[derive(Clone, Debug)]
pub struct RawRead {
    pub file: String,
    pub line: usize,
    pub func: FnId,
    pub ctx: Vec<String>,
    pub attribute: String,
    pub kind: Kind,
    pub field: Option<String>,
    pub coarse: String,
}

#[derive(Clone, Debug)]
pub struct RawWrite {
    pub file: String,
    pub line: usize,
    pub func: FnId,
    pub ctx: Vec<String>,
    pub attribute: String,
    /// The local the attribute is pushed into, when there is one.
    pub sink: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RawCall {
    pub caller: FnId,
    pub callee: String,
    pub ctx: Vec<String>,
    /// Locals passed as `&V` / `&mut V` / `V`.
    pub arg_vars: Vec<String>,
    /// The element literal of an emitting call (`emit_start_with_attrs(w,
    /// "TextFrame", &frame_attrs(f))` binds `frame_attrs` to `TextFrame`).
    pub bound: Option<String>,
}

/// Something the walk saw but could not attribute.
#[derive(Clone, Debug)]
pub struct Loose {
    pub file: String,
    pub line: usize,
    pub func: FnId,
    pub literal: String,
    pub reason: &'static str,
}

/// A literal occurrence, by item — what `manual.toml` entries are checked
/// against (and take their line from).
#[derive(Clone, Debug)]
pub struct LitSite {
    pub func: FnId,
    pub literal: String,
    pub line: usize,
}

#[derive(Default)]
pub struct Scan {
    pub reads: Vec<RawRead>,
    pub writes: Vec<RawWrite>,
    pub calls: Vec<RawCall>,
    pub loose: Vec<Loose>,
    pub lits: Vec<LitSite>,
    /// Per fn: local → the elements it is emitted as.
    pub bindings: HashMap<FnId, HashMap<String, BTreeSet<String>>>,
    pub fns: BTreeSet<FnId>,
}

/// `(file, N)` → `(element, child renames)` for `(N, b"Child")` arms.
pub type PropKinds = HashMap<(String, i64), (String, HashMap<String, String>)>;

pub struct Tables<'a> {
    pub readers: &'a HashMap<String, Reader>,
    pub writers: &'a HashMap<String, usize>,
    pub structs: &'a StructMap,
    pub prop_kinds: &'a PropKinds,
}

pub fn line_of(span: proc_macro2::Span) -> usize {
    span.start().line
}

fn pos_of(span: proc_macro2::Span) -> (usize, usize) {
    let s = span.start();
    (s.line, s.column)
}

pub fn is_attr_shaped(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(f) if f.is_ascii_uppercase()) && c.all(|c| c.is_ascii_alphanumeric())
}

pub fn is_element_shaped(s: &str) -> bool {
    let local = s.strip_prefix("idPkg:").unwrap_or(s);
    is_attr_shaped(local)
}

fn is_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let t = a.to_token_stream().to_string().replace(' ', "");
        t == "#[test]" || t.starts_with("#[cfg(test") || t.starts_with("#[cfg(all(test")
    })
}

/// Token text with the whitespace `to_string` inserts removed.
pub fn compact(t: &impl ToTokens) -> String {
    t.to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// The last path segment of a call's callee, `crate::util::attr` → `attr`.
pub fn callee_name(c: &ExprCall) -> Option<String> {
    match &*c.func {
        Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

fn callee_path(c: &ExprCall) -> Vec<String> {
    match &*c.func {
        Expr::Path(p) => p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect(),
        _ => vec![],
    }
}

fn strip(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(p) => strip(&p.expr),
        Expr::Reference(r) => strip(&r.expr),
        Expr::Group(g) => strip(&g.expr),
        _ => e,
    }
}

fn path_ident(e: &Expr) -> Option<String> {
    match strip(e) {
        Expr::Path(p) if p.path.segments.len() == 1 && p.qself.is_none() => {
            Some(p.path.segments[0].ident.to_string())
        }
        _ => None,
    }
}

pub fn byte_lit(e: &Expr) -> Option<(String, proc_macro2::Span)> {
    match strip(e) {
        Expr::Lit(l) => match &l.lit {
            Lit::ByteStr(b) => Some((String::from_utf8_lossy(&b.value()).into_owned(), b.span())),
            _ => None,
        },
        _ => None,
    }
}

fn str_lit(e: &Expr) -> Option<(String, proc_macro2::Span)> {
    match strip(e) {
        Expr::Lit(l) => match &l.lit {
            Lit::Str(s) => Some((s.value(), s.span())),
            _ => None,
        },
        _ => None,
    }
}

/// Peel `?`, parens, `&` and method calls off a chain: the innermost
/// receiver, and the methods applied to it, innermost first.
pub fn flatten(mut e: &Expr) -> (&Expr, Vec<&ExprMethodCall>) {
    let mut methods = Vec::new();
    loop {
        match e {
            Expr::MethodCall(m) => {
                methods.push(m);
                e = &m.receiver;
            }
            Expr::Try(t) => e = &t.expr,
            Expr::Paren(p) => e = &p.expr,
            Expr::Reference(r) => e = &r.expr,
            Expr::Group(g) => e = &g.expr,
            _ => break,
        }
    }
    methods.reverse();
    (e, methods)
}

fn chain_text(methods: &[&ExprMethodCall]) -> String {
    let mut s = String::new();
    for m in methods {
        s.push('.');
        s.push_str(&m.method.to_string());
        if let Some(t) = &m.turbofish {
            s.push_str(&compact(t));
        }
        s.push('(');
        for a in &m.args {
            s.push_str(&compact(a));
            s.push(',');
        }
        s.push(')');
    }
    s
}

const NUMBER_TYPES: [&str; 12] = [
    "f32", "f64", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "isize",
];

/// The coarse type a conversion spells, from compact token text. `None`
/// when the text converts nothing (the value stays a string).
pub fn coarse_of_text(t: &str) -> Option<&'static str> {
    if t.contains("parse_tint") {
        return Some("tint");
    }
    if t.contains("parse_bounds") {
        return Some("bounds");
    }
    if t.contains("parse_matrix") {
        return Some("matrix");
    }
    if t.contains("parse_insets")
        || t.contains("parse_color_value")
        || t.contains("split_")
        || t.contains(".split(")
    {
        return Some("list");
    }
    if t.contains("parse::<bool>") || t.contains("==\"true\"") {
        return Some("bool");
    }
    if NUMBER_TYPES
        .iter()
        .any(|n| t.contains(&format!("parse::<{n}>")))
    {
        return Some("number");
    }
    if t.contains("::from_") || t.contains("from_str") {
        return Some("enum");
    }
    if t.contains("parse()") || t.contains(".parse") {
        // `.parse().ok()).unwrap_or(0.0)`: the fallback names the type.
        if let Some(rest) = t.split("unwrap_or(").nth(1) {
            if rest.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
                return Some("number");
            }
            if rest.starts_with("true") || rest.starts_with("false") {
                return Some("bool");
            }
        }
        return Some("parsed");
    }
    None
}

/// The coarse type a wrapping call imposes on its argument.
fn hint_of_callee(name: &str) -> Option<&'static str> {
    match name {
        "parse_tint" => Some("tint"),
        n if n.starts_with("parse_bounds") => Some("bounds"),
        n if n.starts_with("parse_matrix") => Some("matrix"),
        "parse_insets" => Some("list"),
        "from_idml" => Some("enum"),
        "parse_ppi_x" => Some("number"),
        _ => None,
    }
}

fn is_key_expr(e: &Expr) -> bool {
    let mut e = strip(e);
    loop {
        match e {
            Expr::Unary(u) => e = strip(&u.expr),
            Expr::MethodCall(m) if m.method == "as_ref" || m.method == "as_slice" => {
                e = strip(&m.receiver)
            }
            _ => break,
        }
    }
    matches!(path_ident(e).as_deref(), Some("k" | "key"))
}

fn mentions_name(e: &impl ToTokens) -> bool {
    e.to_token_stream().into_iter().any(|t| {
        fn walk(t: proc_macro2::TokenTree) -> bool {
            match t {
                proc_macro2::TokenTree::Ident(i) => i == "name",
                proc_macro2::TokenTree::Group(g) => g.stream().into_iter().any(walk),
                _ => false,
            }
        }
        walk(t)
    })
}

fn pat_str_lits(p: &Pat) -> Vec<String> {
    match p {
        Pat::Lit(l) => match &l.lit {
            Lit::Str(s) => vec![s.value()],
            _ => vec![],
        },
        Pat::Or(o) => {
            let all: Vec<_> = o.cases.iter().map(pat_str_lits).collect();
            if all.iter().any(|v| v.is_empty()) {
                vec![]
            } else {
                all.into_iter().flatten().collect()
            }
        }
        _ => vec![],
    }
}

fn pat_byte_lits(p: &Pat) -> Vec<(String, proc_macro2::Span)> {
    match p {
        Pat::Lit(l) => match &l.lit {
            Lit::ByteStr(b) => vec![(String::from_utf8_lossy(&b.value()).into_owned(), b.span())],
            _ => vec![],
        },
        Pat::Or(o) => {
            let all: Vec<_> = o.cases.iter().map(pat_byte_lits).collect();
            if all.iter().any(|v| v.is_empty()) {
                vec![]
            } else {
                all.into_iter().flatten().collect()
            }
        }
        Pat::Paren(p) => pat_byte_lits(&p.pat),
        _ => vec![],
    }
}

/// `(N, b"Child")` — the importer's Properties-child dispatch.
fn pat_prop_tuple(p: &Pat) -> Option<(i64, String, proc_macro2::Span)> {
    let Pat::Tuple(t) = p else { return None };
    if t.elems.len() != 2 {
        return None;
    }
    let n = match &t.elems[0] {
        Pat::Lit(l) => match &l.lit {
            Lit::Int(i) => i.base10_parse::<i64>().ok()?,
            _ => return None,
        },
        _ => return None,
    };
    let (name, span) = pat_byte_lits(&t.elems[1]).into_iter().next()?;
    Some((n, name, span))
}

/// Elements an `if` condition or arm guard narrows to:
/// `e.name().as_ref() == b"X"` and `matches!(e.name()…, b"A" | b"B")`.
fn name_cmp_elements(e: &Expr) -> Vec<String> {
    match e {
        Expr::Binary(b) => match b.op {
            BinOp::Eq(_) => {
                let (l, r) = (&*b.left, &*b.right);
                for (lit, other) in [(r, l), (l, r)] {
                    if let Some((s, _)) = byte_lit(lit) {
                        if is_element_shaped(&s) && mentions_name(other) {
                            return vec![s];
                        }
                    }
                }
                vec![]
            }
            BinOp::And(_) | BinOp::Or(_) => {
                let mut v = name_cmp_elements(&b.left);
                v.extend(name_cmp_elements(&b.right));
                v
            }
            _ => vec![],
        },
        Expr::Paren(p) => name_cmp_elements(&p.expr),
        Expr::Macro(m) if m.mac.path.is_ident("matches") => {
            let parsed = m.mac.parse_body_with(|input: syn::parse::ParseStream| {
                let scrutinee: Expr = input.parse()?;
                let _: Token![,] = input.parse()?;
                let pat = Pat::parse_multi_with_leading_vert(input)?;
                let _: proc_macro2::TokenStream = input.parse()?;
                Ok((scrutinee, pat))
            });
            match parsed {
                Ok((s, p)) if mentions_name(&s) => pat_byte_lits(&p)
                    .into_iter()
                    .map(|(s, _)| s)
                    .filter(|s| is_element_shaped(s))
                    .collect(),
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

/// The member path an assignment writes: `p.span_columns.count = …` →
/// `span_columns.count`.
fn assign_field(lhs: &Expr) -> Option<String> {
    let mut parts = Vec::new();
    let mut e = lhs;
    loop {
        match e {
            Expr::Field(f) => {
                if let syn::Member::Named(n) = &f.member {
                    parts.push(n.to_string());
                }
                e = &f.base;
            }
            Expr::Index(i) => e = &i.expr,
            Expr::Unary(u) => e = &u.expr,
            Expr::Paren(p) => e = &p.expr,
            _ => break,
        }
    }
    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    Some(parts.join("."))
}

fn first_assign_field(body: &Expr) -> Option<String> {
    struct F(Option<String>);
    impl<'ast> Visit<'ast> for F {
        fn visit_expr_assign(&mut self, a: &'ast syn::ExprAssign) {
            if self.0.is_none() {
                self.0 = assign_field(&a.left);
            }
        }
    }
    let mut f = F(None);
    f.visit_expr(body);
    f.0
}

fn closure_params(c: &syn::ExprClosure) -> Vec<String> {
    c.inputs
        .iter()
        .map(|p| match p {
            Pat::Ident(i) => i.ident.to_string(),
            Pat::Type(t) => match &*t.pat {
                Pat::Ident(i) => i.ident.to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        })
        .collect()
}

pub fn fn_params(sig: &syn::Signature) -> Vec<String> {
    sig.inputs
        .iter()
        .map(|a| match a {
            syn::FnArg::Typed(t) => match &*t.pat {
                Pat::Ident(i) => i.ident.to_string(),
                _ => String::new(),
            },
            syn::FnArg::Receiver(_) => "self".to_string(),
        })
        .collect()
}

/// Find, in `body`, a reader call whose key argument is the local `param`;
/// return the coarse type its chain converts to (`None` when the chain is a
/// plain string read).
pub fn find_param_read(
    body: &Expr,
    param: &str,
    readers: &HashMap<String, Reader>,
) -> Option<Option<&'static str>> {
    struct F<'a> {
        param: &'a str,
        readers: &'a HashMap<String, Reader>,
        found: Option<Option<&'static str>>,
        hint: Vec<&'static str>,
    }
    impl F<'_> {
        fn check(&mut self, base: &Expr, methods: &[&ExprMethodCall]) {
            if self.found.is_some() {
                return;
            }
            let Expr::Call(c) = base else { return };
            let Some(name) = callee_name(c) else { return };
            let Some(r) = self.readers.get(&name) else {
                return;
            };
            let Some(idx) = r.key_idx else { return };
            if c.args.iter().nth(idx).and_then(path_ident).as_deref() == Some(self.param) {
                let t = coarse_of_text(&chain_text(methods))
                    .or_else(|| self.hint.last().copied())
                    .or(match r.coarse.as_str() {
                        "string" => None,
                        "number" => Some("number"),
                        "tint" => Some("tint"),
                        "bool" => Some("bool"),
                        _ => None,
                    });
                self.found = Some(t);
            }
        }
    }
    impl<'ast> Visit<'ast> for F<'_> {
        fn visit_expr_method_call(&mut self, m: &'ast ExprMethodCall) {
            let (base, methods) = flatten_m(m);
            self.check(base, &methods);
            visit::visit_expr_method_call(self, m);
        }
        fn visit_expr_call(&mut self, c: &'ast ExprCall) {
            let pushed = callee_name(c)
                .and_then(|n| hint_of_callee(&n))
                .map(|h| self.hint.push(h))
                .is_some();
            self.check(&Expr::Call(c.clone()), &[]);
            visit::visit_expr_call(self, c);
            if pushed {
                self.hint.pop();
            }
        }
    }
    let mut f = F {
        param,
        readers,
        found: None,
        hint: vec![],
    };
    f.visit_expr(body);
    f.found
}

fn flatten_m(m: &ExprMethodCall) -> (&Expr, Vec<&ExprMethodCall>) {
    let (base, mut rest) = flatten(&m.receiver);
    rest.push(m);
    (base, rest)
}

/// Does `body` push a tuple whose key is the local `param`, or hand `param`
/// to a known writer at its key slot?
pub fn writes_param_key(body: &Expr, param: &str, writers: &HashMap<String, usize>) -> bool {
    struct F<'a> {
        param: &'a str,
        writers: &'a HashMap<String, usize>,
        found: bool,
    }
    impl<'ast> Visit<'ast> for F<'_> {
        fn visit_expr_tuple(&mut self, t: &'ast syn::ExprTuple) {
            if t.elems.len() >= 2 && path_ident(&t.elems[0]).as_deref() == Some(self.param) {
                self.found = true;
            }
            visit::visit_expr_tuple(self, t);
        }
        fn visit_expr_call(&mut self, c: &'ast ExprCall) {
            if let Some(idx) = callee_name(c).and_then(|n| self.writers.get(&n).copied()) {
                if c.args.iter().nth(idx).and_then(path_ident).as_deref() == Some(self.param) {
                    self.found = true;
                }
            }
            visit::visit_expr_call(self, c);
        }
    }
    let mut f = F {
        param,
        writers,
        found: false,
    };
    f.visit_expr(body);
    f.found
}

#[derive(Default)]
struct FnState {
    func: Option<FnId>,
    params: Vec<String>,
    ctx: Vec<Vec<String>>,
    field: Vec<(Option<String>, String)>,
    let_ty: Vec<Option<String>>,
    hint: Vec<&'static str>,
    closures: HashMap<String, Reader>,
    closure_params: Vec<Vec<String>>,
    sinks: Vec<String>,
    /// `let` instances per local name: a name reused for a second `let` in
    /// the same fn is a different local (`attrs` per effect block).
    instances: HashMap<String, usize>,
    next_instance: usize,
}

pub struct Walker<'a> {
    side: Side,
    file: String,
    t: &'a Tables<'a>,
    pub out: &'a mut Scan,
    st: FnState,
    consumed: HashSet<(usize, usize)>,
}

impl<'a> Walker<'a> {
    pub fn new(side: Side, file: String, t: &'a Tables<'a>, out: &'a mut Scan) -> Self {
        Walker {
            side,
            file,
            t,
            out,
            st: FnState::default(),
            consumed: HashSet::new(),
        }
    }

    fn func(&self) -> FnId {
        self.st
            .func
            .clone()
            .unwrap_or_else(|| (self.file.clone(), "<module>".to_string()))
    }

    fn ctx(&self) -> Vec<String> {
        self.st.ctx.last().cloned().unwrap_or_default()
    }

    fn enter(&mut self, name: String, params: Vec<String>) -> FnState {
        let id = (self.file.clone(), name);
        self.out.fns.insert(id.clone());
        std::mem::replace(
            &mut self.st,
            FnState {
                func: Some(id),
                params,
                ..FnState::default()
            },
        )
    }

    fn lit_site(&mut self, literal: &str, span: proc_macro2::Span) {
        self.out.lits.push(LitSite {
            func: self.func(),
            literal: literal.to_string(),
            line: line_of(span),
        });
    }

    fn loose(&mut self, literal: String, span: proc_macro2::Span, reason: &'static str) {
        self.out.loose.push(Loose {
            file: self.file.clone(),
            line: line_of(span),
            func: self.func(),
            literal,
            reason,
        });
    }

    fn reader_for(&self, c: &ExprCall) -> Option<Reader> {
        let name = callee_name(c)?;
        if let Some(r) = self.st.closures.get(&name) {
            return Some(r.clone());
        }
        self.t.readers.get(&name).cloned()
    }

    fn is_local_param(&self, ident: &str) -> bool {
        self.st.params.iter().any(|p| p == ident)
            || self
                .st
                .closure_params
                .iter()
                .any(|ps| ps.iter().any(|p| p == ident))
    }

    /// Record the read whose call is `base`, converted by `methods`.
    fn try_read(&mut self, base: &Expr, methods: &[&ExprMethodCall]) {
        let Expr::Call(c) = base else { return };
        let Some(r) = self.reader_for(c) else { return };
        let (attribute, span) = match r.key_idx {
            None => match c.args.first() {
                Some(_) => ("Value".to_string(), c.paren_token.span.open()),
                None => return,
            },
            Some(idx) => match c.args.iter().nth(idx) {
                Some(k) => match byte_lit(k) {
                    Some(l) => l,
                    None => {
                        // A key parameter is a helper's business (its
                        // callers are read); anything else is computed.
                        let id = path_ident(k);
                        if !id.as_deref().is_some_and(|id| self.is_local_param(id)) {
                            let span = c.paren_token.span.open();
                            if self.consumed.insert(pos_of(span)) {
                                let what = id.unwrap_or_else(|| compact(k));
                                self.loose(what, span, "read with a computed key");
                            }
                        }
                        return;
                    }
                },
                None => return,
            },
        };
        if !self.consumed.insert(pos_of(span)) {
            return;
        }
        self.lit_site(&attribute, span);
        let chain = coarse_of_text(&chain_text(methods));
        let coarse = self.resolve_coarse(chain, &r);
        let field = self.st.field.last().map(|(s, f)| match s {
            Some(s) => format!("{s}.{f}"),
            None => f.clone(),
        });
        self.out.reads.push(RawRead {
            file: self.file.clone(),
            line: line_of(span),
            func: self.func(),
            ctx: self.ctx(),
            attribute,
            kind: Kind::Attr,
            field,
            coarse,
        });
        // `attr(e, b"A").or_else(|| attr(e, b"B"))…`: the fallback is the
        // same value, converted by what follows it.
        for (i, m) in methods.iter().enumerate() {
            if m.method == "or_else" || m.method == "or" {
                if let Some(arg) = m.args.first() {
                    let inner = match strip(arg) {
                        Expr::Closure(cl) => &*cl.body,
                        other => other,
                    };
                    let (b, mut ms) = flatten(inner);
                    ms.extend(methods[i + 1..].iter().copied());
                    self.try_read(b, &ms);
                }
            }
        }
    }

    fn resolve_coarse(&self, chain: Option<&'static str>, r: &Reader) -> String {
        let mut c = match chain {
            Some(c) => c.to_string(),
            None => match self.st.hint.last() {
                Some(h) => h.to_string(),
                None => r.coarse.clone(),
            },
        };
        if c == "parsed" {
            if r.coarse != "string" && r.coarse != "parsed" {
                return r.coarse.clone();
            }
            if let Some((Some(s), f)) = self.st.field.last() {
                if let Some(t) = self.t.structs.field_type(s, f) {
                    if let Some(k) = coarse_of_type(t, self.t.structs) {
                        return k;
                    }
                }
            }
            if let Some(Some(t)) = self.st.let_ty.last() {
                if let Some(k) = coarse_of_type(t, self.t.structs) {
                    return k;
                }
            }
            c = "parsed".to_string();
        }
        c
    }

    fn write(&mut self, attribute: String, span: proc_macro2::Span, sink: Option<String>) {
        self.lit_site(&attribute, span);
        self.out.writes.push(RawWrite {
            file: self.file.clone(),
            line: line_of(span),
            func: self.func(),
            ctx: self.ctx(),
            attribute,
            sink,
        });
    }

    /// The current `let` instance of a local, `attrs` → `attrs#3`.
    fn inst(&self, var: &str) -> String {
        format!("{var}#{}", self.st.instances.get(var).copied().unwrap_or(0))
    }

    fn bind(&mut self, var: String, element: String) {
        let var = self.inst(&var);
        let f = self.func();
        self.out
            .bindings
            .entry(f)
            .or_default()
            .entry(var)
            .or_default()
            .insert(element);
    }

    fn arg_var(e: &Expr) -> Option<String> {
        path_ident(e)
    }

    fn mut_ref_var(e: &Expr) -> Option<String> {
        match e {
            Expr::Reference(r) if r.mutability.is_some() => path_ident(&r.expr),
            _ => None,
        }
    }
}

impl<'ast> Visit<'ast> for Walker<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if is_test_attr(&m.attrs) {
            return;
        }
        visit::visit_item_mod(self, m);
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if is_test_attr(&f.attrs) {
            return;
        }
        let saved = self.enter(f.sig.ident.to_string(), fn_params(&f.sig));
        visit::visit_item_fn(self, f);
        self.st = saved;
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if is_test_attr(&f.attrs) {
            return;
        }
        let saved = self.enter(f.sig.ident.to_string(), fn_params(&f.sig));
        visit::visit_impl_item_fn(self, f);
        self.st = saved;
    }

    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        let saved = self.enter(c.ident.to_string(), vec![]);
        visit::visit_item_const(self, c);
        self.st = saved;
    }

    fn visit_item_static(&mut self, c: &'ast syn::ItemStatic) {
        let saved = self.enter(c.ident.to_string(), vec![]);
        visit::visit_item_static(self, c);
        self.st = saved;
    }

    // Patterns carry no reads; arm patterns are handled in `visit_expr_match`.
    fn visit_pat(&mut self, _: &'ast Pat) {}

    fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
        self.visit_expr(&m.expr);
        let key_scrutinee = is_key_expr(&m.expr);
        for arm in &m.arms {
            let lits = pat_byte_lits(&arm.pat);
            let mut pushed = false;
            if !lits.is_empty() {
                for (l, s) in &lits {
                    self.lit_site(l, *s);
                }
                if key_scrutinee && self.side == Side::Export {
                    for (l, s) in lits.iter().filter(|(l, _)| is_attr_shaped(l)) {
                        self.write(l.clone(), *s, None);
                    }
                } else if lits.iter().all(|(l, _)| is_element_shaped(l)) {
                    let mut names: Vec<String> = lits.iter().map(|(l, _)| l.clone()).collect();
                    names.dedup();
                    self.st.ctx.push(names);
                    pushed = true;
                }
            } else if self.side == Side::Export
                && !key_scrutinee
                && !pat_str_lits(&arm.pat).is_empty()
            {
                // `match pref { "PageNumberVariablePreference" => … }`.
                let names = pat_str_lits(&arm.pat);
                if names.iter().all(|n| is_element_shaped(n)) {
                    self.st.ctx.push(names);
                    pushed = true;
                }
            } else if let Some((n, child, span)) = pat_prop_tuple(&arm.pat) {
                self.lit_site(&child, span);
                if self.side == Side::Import {
                    match self.t.prop_kinds.get(&(self.file.clone(), n)) {
                        Some((element, renames)) => {
                            let attribute = renames.get(&child).cloned().unwrap_or(child);
                            let coarse = coarse_of_text(&compact(&*arm.body))
                                .filter(|c| *c != "parsed")
                                .unwrap_or("string")
                                .to_string();
                            self.out.reads.push(RawRead {
                                file: self.file.clone(),
                                line: line_of(span),
                                func: self.func(),
                                ctx: vec![element.clone()],
                                attribute,
                                kind: Kind::PropertiesChild,
                                field: first_assign_field(&arm.body),
                                coarse,
                            });
                        }
                        None => {
                            self.loose(child, span, "properties-child arm with an unmapped kind")
                        }
                    }
                }
            }
            if let Some((_, g)) = &arm.guard {
                if !pushed {
                    let els = name_cmp_elements(g);
                    if !els.is_empty() {
                        self.st.ctx.push(els);
                        pushed = true;
                    }
                }
                self.visit_expr(g);
            }
            self.visit_expr(&arm.body);
            if pushed {
                self.st.ctx.pop();
            }
        }
    }

    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        self.visit_expr(&i.cond);
        let els = name_cmp_elements(&i.cond);
        let pushed = !els.is_empty();
        if pushed {
            self.st.ctx.push(els);
        }
        self.visit_block(&i.then_branch);
        if pushed {
            self.st.ctx.pop();
        }
        if let Some((_, e)) = &i.else_branch {
            self.visit_expr(e);
        }
    }

    fn visit_expr_binary(&mut self, b: &'ast syn::ExprBinary) {
        if matches!(b.op, BinOp::Eq(_) | BinOp::Ne(_)) {
            for (lit, other) in [(&*b.right, &*b.left), (&*b.left, &*b.right)] {
                if let Some((s, span)) = byte_lit(lit) {
                    self.consumed.insert(pos_of(span));
                    self.lit_site(&s, span);
                    if self.side == Side::Export
                        && matches!(b.op, BinOp::Eq(_))
                        && is_key_expr(other)
                        && is_attr_shaped(&s)
                    {
                        self.write(s, span, None);
                    }
                }
            }
        }
        visit::visit_expr_binary(self, b);
    }

    fn visit_expr_struct(&mut self, s: &'ast syn::ExprStruct) {
        let name = s.path.segments.last().map(|s| s.ident.to_string());
        for f in &s.fields {
            match &f.member {
                syn::Member::Named(id) => {
                    self.st.field.push((name.clone(), id.to_string()));
                    self.visit_expr(&f.expr);
                    self.st.field.pop();
                }
                syn::Member::Unnamed(_) => self.visit_expr(&f.expr),
            }
        }
        if let Some(r) = &s.rest {
            self.visit_expr(r);
        }
    }

    fn visit_expr_assign(&mut self, a: &'ast syn::ExprAssign) {
        match assign_field(&a.left) {
            Some(f) => {
                self.st.field.push((None, f));
                self.visit_expr(&a.right);
                self.st.field.pop();
            }
            None => self.visit_expr(&a.right),
        }
        self.visit_expr(&a.left);
    }

    fn visit_expr_closure(&mut self, c: &'ast syn::ExprClosure) {
        self.st.closure_params.push(closure_params(c));
        visit::visit_expr_closure(self, c);
        self.st.closure_params.pop();
    }

    fn visit_local(&mut self, l: &'ast syn::Local) {
        let (name, ty) = match &l.pat {
            Pat::Ident(i) => (Some(i.ident.to_string()), None),
            Pat::Type(t) => match &*t.pat {
                Pat::Ident(i) => (Some(i.ident.to_string()), Some(compact(&*t.ty))),
                _ => (None, Some(compact(&*t.ty))),
            },
            _ => (None, None),
        };
        let Some(init) = &l.init else { return };
        if let (Some(n), Expr::Closure(cl)) = (&name, strip(&init.expr)) {
            // `let f = |k: &[u8]| attr(e, k)…` — a local reader.
            let params = closure_params(cl);
            for (i, p) in params.iter().enumerate() {
                if let Some(chain) = find_param_read(&cl.body, p, self.t.readers) {
                    let ret = match &cl.output {
                        syn::ReturnType::Type(_, t) => {
                            coarse_of_type(&compact(&**t), self.t.structs)
                        }
                        syn::ReturnType::Default => None,
                    };
                    let coarse = match (chain, ret) {
                        (Some(c), _) if c != "parsed" => c.to_string(),
                        (_, Some(r)) => r,
                        (Some(c), None) => c.to_string(),
                        (None, None) => "string".to_string(),
                    };
                    self.st.closures.insert(
                        n.clone(),
                        Reader {
                            key_idx: Some(i),
                            coarse,
                        },
                    );
                }
            }
        }
        if let Some(n) = &name {
            self.st.next_instance += 1;
            let id = self.st.next_instance;
            self.st.instances.insert(n.clone(), id);
        }
        if self.side == Side::Export {
            if let Some(n) = &name {
                let (base, _) = flatten(&init.expr);
                if let Expr::Call(c) = base {
                    let p = callee_path(c);
                    if p.ends_with(&["BytesStart".to_string(), "new".to_string()]) {
                        if let Some((e, _)) = c.args.first().and_then(str_lit) {
                            if is_element_shaped(&e) {
                                self.bind(n.clone(), e);
                            }
                        }
                    }
                }
            }
        }
        self.st.let_ty.push(ty);
        let sink = name.is_some();
        if let Some(n) = name {
            let n = self.inst(&n);
            self.st.sinks.push(n);
        }
        self.visit_expr(&init.expr);
        if let Some((_, d)) = &init.diverge {
            self.visit_expr(d);
        }
        if sink {
            self.st.sinks.pop();
        }
        self.st.let_ty.pop();
    }

    fn visit_expr_method_call(&mut self, m: &'ast ExprMethodCall) {
        if self.side == Side::Import {
            let (base, methods) = flatten_m(m);
            self.try_read(base, &methods);
            visit::visit_expr_method_call(self, m);
            return;
        }
        let method = m.method.to_string();
        let sink = if matches!(
            method.as_str(),
            "push" | "push_attribute" | "extend" | "insert"
        ) {
            path_ident(&m.receiver).map(|v| self.inst(&v))
        } else {
            None
        };
        self.visit_expr(&m.receiver);
        if let Some(s) = &sink {
            self.st.sinks.push(s.clone());
        }
        for a in &m.args {
            self.visit_expr(a);
        }
        if sink.is_some() {
            self.st.sinks.pop();
        }
    }

    fn visit_expr_call(&mut self, c: &'ast ExprCall) {
        let name = callee_name(c).unwrap_or_default();
        // The locals handed in, plus the local the result flows into
        // (`let attrs = paragraph_style_attrs(s)`, `attrs.extend(f(t))`).
        let arg_vars: Vec<String> = c
            .args
            .iter()
            .filter_map(Self::arg_var)
            .map(|v| self.inst(&v))
            .chain(self.st.sinks.last().cloned())
            .collect();
        match self.side {
            Side::Import => {
                self.try_read(&Expr::Call(c.clone()), &[]);
                let hint = hint_of_callee(&name);
                if let Some(h) = hint {
                    self.st.hint.push(h);
                }
                self.out.calls.push(RawCall {
                    caller: self.func(),
                    callee: name,
                    ctx: self.ctx(),
                    arg_vars,
                    bound: None,
                });
                for a in &c.args {
                    self.visit_expr(a);
                }
                if hint.is_some() {
                    self.st.hint.pop();
                }
            }
            Side::Export => {
                let sink = c
                    .args
                    .iter()
                    .find_map(Self::mut_ref_var)
                    .map(|v| self.inst(&v));
                let writer_idx = self.t.writers.get(&name).copied();
                if let Some(idx) = writer_idx {
                    if let Some((k, span)) = c.args.iter().nth(idx).and_then(str_lit) {
                        if is_attr_shaped(&k) {
                            self.write(k, span, sink.clone());
                        }
                    }
                }
                // An emitting call: an element literal beside the attributes.
                let element = if writer_idx.is_none() {
                    c.args
                        .iter()
                        .filter_map(|a| match a {
                            Expr::Lit(_) => str_lit(a),
                            _ => None,
                        })
                        .map(|(s, _)| s)
                        .find(|s| is_element_shaped(s))
                        .filter(|_| {
                            c.args.iter().any(|a| {
                                matches!(a, Expr::Reference(r) if r.mutability.is_none())
                                    && !matches!(strip(a), Expr::Lit(_))
                            })
                        })
                } else {
                    None
                };
                let mut bound_fn_calls = Vec::new();
                if let Some(e) = &element {
                    for a in &c.args {
                        if matches!(a, Expr::Reference(r) if r.mutability.is_none()) {
                            match strip(a) {
                                Expr::Path(_) => {
                                    if let Some(v) = path_ident(a) {
                                        self.bind(v, e.clone());
                                    }
                                }
                                Expr::Call(inner) => {
                                    if let Some(n) = callee_name(inner) {
                                        bound_fn_calls.push(n);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // `write_new_box_item(w, "Rectangle", …, &paint)`: the callee
                // itself emits the element it is handed.
                self.out.calls.push(RawCall {
                    caller: self.func(),
                    callee: name,
                    ctx: self.ctx(),
                    arg_vars,
                    bound: element.clone(),
                });
                for n in bound_fn_calls {
                    self.out.calls.push(RawCall {
                        caller: self.func(),
                        callee: n,
                        ctx: vec![],
                        arg_vars: vec![],
                        bound: element.clone(),
                    });
                }
                if let Some(s) = &sink {
                    self.st.sinks.push(s.clone());
                }
                for a in &c.args {
                    let direct =
                        element.is_some() && matches!(strip(a), Expr::Array(_) | Expr::Macro(_));
                    if direct {
                        self.st.ctx.push(vec![element.clone().unwrap_or_default()]);
                    }
                    self.visit_expr(a);
                    if direct {
                        self.st.ctx.pop();
                    }
                }
                if sink.is_some() {
                    self.st.sinks.pop();
                }
            }
        }
    }

    fn visit_expr_tuple(&mut self, t: &'ast syn::ExprTuple) {
        if self.side == Side::Export && t.elems.len() >= 2 {
            if let Some((k, span)) = str_lit(&t.elems[0]) {
                if is_attr_shaped(&k) {
                    let sink = self.st.sinks.last().cloned();
                    self.write(k, span, sink);
                }
            }
        }
        visit::visit_expr_tuple(self, t);
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if m.path.is_ident("vec") {
            if let Ok(items) = m.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
                for e in &items {
                    self.visit_expr(e);
                }
            }
        } else if m.path.is_ident("matches") {
            let parsed = m.parse_body_with(|input: syn::parse::ParseStream| {
                let scrutinee: Expr = input.parse()?;
                let _: proc_macro2::TokenStream = input.parse()?;
                Ok(scrutinee)
            });
            if let Ok(e) = parsed {
                self.visit_expr(&e);
            }
        }
    }

    fn visit_expr_lit(&mut self, l: &'ast syn::ExprLit) {
        match &l.lit {
            Lit::ByteStr(b) => {
                let s = String::from_utf8_lossy(&b.value()).into_owned();
                self.lit_site(&s, b.span());
                if self.side == Side::Import
                    && is_attr_shaped(&s)
                    && !self.consumed.contains(&pos_of(b.span()))
                {
                    self.loose(s, b.span(), "byte literal not attributed to a read");
                }
            }
            Lit::Str(s) => {
                self.lit_site(&s.value(), s.span());
            }
            _ => {}
        }
    }
}
