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

//! Struct field types from the model sources, so an untyped
//! `attr(e, b"X").and_then(|s| s.parse().ok())` in a struct literal gets the
//! coarse type of the field it fills.

use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};

use crate::scan::compact;

#[derive(Default)]
pub struct StructMap {
    /// struct → field → compact type text; `None` when two same-named
    /// structs disagree about the field.
    fields: BTreeMap<String, BTreeMap<String, Option<String>>>,
    enums: BTreeSet<String>,
}

impl StructMap {
    pub fn add_file(&mut self, file: &syn::File) {
        struct V<'a>(&'a mut StructMap);
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
                if let syn::Fields::Named(n) = &s.fields {
                    let entry = self.0.fields.entry(s.ident.to_string()).or_default();
                    for f in &n.named {
                        let Some(id) = &f.ident else { continue };
                        let ty = compact(&f.ty);
                        entry
                            .entry(id.to_string())
                            .and_modify(|old| {
                                if old.as_deref() != Some(ty.as_str()) {
                                    *old = None;
                                }
                            })
                            .or_insert_with(|| Some(ty.clone()));
                    }
                }
                visit::visit_item_struct(self, s);
            }
            fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
                self.0.enums.insert(e.ident.to_string());
                visit::visit_item_enum(self, e);
            }
        }
        V(self).visit_file(file);
    }

    pub fn field_type(&self, s: &str, f: &str) -> Option<&str> {
        self.fields.get(s)?.get(f)?.as_deref()
    }

    fn is_enum(&self, name: &str) -> bool {
        self.enums.contains(name)
    }
}

/// The coarse type of a (compact) Rust type spelling, `None` when it says
/// nothing useful.
pub fn coarse_of_type(t: &str, structs: &StructMap) -> Option<String> {
    let mut t = t.trim_start_matches('&');
    while let Some(inner) = t.strip_prefix("Option<").and_then(|r| r.strip_suffix('>')) {
        t = inner;
    }
    let last = t.rsplit("::").next().unwrap_or(t);
    let c = match last {
        "f32" | "f64" | "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize"
        | "isize" => "number",
        "bool" => "bool",
        "String" | "str" => "string",
        "Bounds" => "bounds",
        _ if last.starts_with('[') || last.starts_with("Vec<") => "list",
        _ if structs.is_enum(last) => "enum",
        _ => return None,
    };
    Some(c.to_string())
}
