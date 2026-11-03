/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Precomputed O(1) return-type -> factory-function lookup index for unindexed Cargo target rename.

use super::rename_parse::has_self_receiver;
use super::WorkspaceIndex;
use crate::format::DefKind;
use std::collections::{HashMap, HashSet};

/// Compact pre-indexed function or associated function whose signature return type mentions a workspace nominal type.
#[derive(Debug, Clone)]
pub struct FactoryFnCandidate {
    pub reader_crate: String,
    pub fn_name: String,
    /// `None` for free `DefKind::Fn`; `Some(fty_short)` for `DefKind::AssocFn` on a non-UFCS parent path.
    pub parent_short: Option<String>,
    /// True when `AssocFn` has a `self` receiver and is not a standard conversion/clone trait method.
    pub dot_eligible: bool,
    /// True when `dot_eligible` holds and every `AssocFn` with `fn_name` in the same shard returns this nominal type.
    pub all_shard_assoc_return_owner: bool,
}

#[inline]
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn extract_return_nominal_idents<'a>(sig: &'a str, known_types: &HashSet<String>) -> Vec<&'a str> {
    let Some((_, ret)) = sig.split_once("->") else {
        return Vec::new();
    };
    let bytes = ret.as_bytes();
    let mut out: Vec<&'a str> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if (b.is_ascii_alphabetic() || b == b'_') && (i == 0 || bytes[i - 1] != b'\'') {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let tok = &ret[start..i];
            if known_types.contains(tok) && !out.contains(&tok) {
                out.push(tok);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Builds a workspace-wide map `return_type_short -> Vec<FactoryFnCandidate>` in a single pass per shard.
pub fn build_factory_return_index(ws: &WorkspaceIndex, type_def_crates_by_name: &HashMap<String, HashSet<String>>) -> HashMap<String, Vec<FactoryFnCandidate>> {
    let known_types: HashSet<String> = type_def_crates_by_name.keys().cloned().collect();
    let mut out: HashMap<String, Vec<FactoryFnCandidate>> = HashMap::new();
    if known_types.is_empty() {
        return out;
    }

    for (ri, reader) in ws.readers.iter().enumerate() {
        if ws.is_reader_external(ri) {
            continue;
        }
        let rc = reader.crate_name();
        let defs = reader.definitions();

        // Pass 1: intersect return-type sets per `AssocFn` short name within this shard so we never rescan `defs`.
        let mut common_assoc_ret: HashMap<&str, Vec<&str>> = HashMap::new();
        for d in defs {
            if DefKind::from(d.kind) != DefKind::AssocFn || d.sig_offset() == 0 {
                continue;
            }
            let fn_name = reader.get_str(d.name_offset());
            let sig = reader.get_str(d.sig_offset());
            let ret_tys = extract_return_nominal_idents(sig, &known_types);
            common_assoc_ret.entry(fn_name).and_modify(|cur| cur.retain(|t| ret_tys.contains(t))).or_insert(ret_tys);
        }

        // Pass 2: record compact `FactoryFnCandidate` entries keyed by returned nominal short name.
        for d in defs {
            let dk = DefKind::from(d.kind);
            if d.sig_offset() == 0 || !matches!(dk, DefKind::Fn | DefKind::AssocFn) {
                continue;
            }
            let fn_name = reader.get_str(d.name_offset());
            let sig = reader.get_str(d.sig_offset());
            let ret_tys = extract_return_nominal_idents(sig, &known_types);
            if ret_tys.is_empty() {
                continue;
            }
            if dk == DefKind::Fn {
                for &ret_ty in &ret_tys {
                    out.entry(ret_ty.to_string()).or_default().push(FactoryFnCandidate {
                        reader_crate: rc.to_string(),
                        fn_name: fn_name.to_string(),
                        parent_short: None,
                        dot_eligible: false,
                        all_shard_assoc_return_owner: false,
                    });
                }
            } else if let Some((parent, _)) = reader.get_str(d.path_offset()).rsplit_once("::") {
                if parent.trim_start().starts_with('<') {
                    continue;
                }
                let fty_short = parent.trim().split('<').next().unwrap_or(parent).rsplit("::").next().unwrap_or("");
                let dot_eligible = has_self_receiver(sig) && !matches!(fn_name, "clone" | "default" | "into" | "from" | "as_ref" | "as_mut" | "deref" | "borrow");
                if fty_short.is_empty() && !dot_eligible {
                    continue;
                }
                for &ret_ty in &ret_tys {
                    let all_same = dot_eligible && common_assoc_ret.get(fn_name).is_some_and(|v| v.contains(&ret_ty));
                    out.entry(ret_ty.to_string()).or_default().push(FactoryFnCandidate {
                        reader_crate: rc.to_string(),
                        fn_name: fn_name.to_string(),
                        parent_short: Some(fty_short.to_string()),
                        dot_eligible,
                        all_shard_assoc_return_owner: all_same,
                    });
                }
            }
        }
    }

    out
}
