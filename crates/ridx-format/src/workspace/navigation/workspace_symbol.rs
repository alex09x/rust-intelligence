/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

fn contains_ascii_ci(haystack: &[u8], needle_lower: &[u8]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if haystack.len() < needle_lower.len() {
        return false;
    }
    let first = needle_lower[0];
    let first_upper = first.to_ascii_uppercase();
    let max_start = haystack.len() - needle_lower.len();
    for i in 0..=max_start {
        let b = haystack[i];
        if (b == first || b == first_upper) && haystack[i + 1..i + needle_lower.len()].eq_ignore_ascii_case(&needle_lower[1..]) {
            return true;
        }
    }
    false
}

fn is_fuzzy_subseq(name: &str, query: &str) -> bool {
    if !(2..=12).contains(&query.len()) || name.len() < query.len() || !name.is_ascii() || !query.is_ascii() {
        return false;
    }
    let has_upper = query.bytes().any(|b| b.is_ascii_uppercase());
    let mut nb = name.bytes();
    for qb in query.bytes() {
        let found = if has_upper && qb.is_ascii_uppercase() { nb.any(|c| c == qb) } else { nb.any(|c| c.eq_ignore_ascii_case(&qb)) };
        if !found {
            return false;
        }
    }
    true
}

struct CachedWsDefs {
    crate_roots: Vec<u32>,
    type_defs: Vec<(u32, u32)>,
    other_defs: Vec<(u32, u32)>,
}

fn get_or_build_ws_defs(ws: &WorkspaceIndex, has_local: bool) -> std::sync::Arc<CachedWsDefs> {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex, OnceLock};
    type CacheKey = (usize, usize, usize, bool);
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Arc<CachedWsDefs>>>> = OnceLock::new();
    let first_ptr = ws.readers.first().map(|r| r.definitions().as_ptr() as usize).unwrap_or(0);
    let last_ptr = ws.readers.last().map(|r| r.definitions().as_ptr() as usize).unwrap_or(0);
    let key = (ws.readers.len(), first_ptr, last_ptr, has_local);
    let mutex = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = mutex.lock() {
        if let Some(found) = guard.get(&key) {
            return Arc::clone(found);
        }
    }
    let mut crate_roots = Vec::new();
    let mut type_defs = Vec::new();
    let mut other_defs = Vec::new();
    let mut seen = HashSet::new();
    for (i, reader) in ws.readers.iter().enumerate() {
        if has_local && ws.reader_is_dependency.get(i).copied().unwrap_or(false) {
            continue;
        }
        let crate_name = reader.crate_name();
        let root_file = reader.get_file(0);
        if !crate_name.is_empty() && !root_file.is_empty() && seen.insert((crate_name, root_file, 1u32, 1u32)) {
            crate_roots.push(i as u32);
        }
        for (d_idx, def) in reader.definitions().iter().enumerate() {
            let kind = DefKind::from(def.kind);
            if is_excluded_ws_kind(kind) {
                continue;
            }
            let name = reader.get_str(def.name_offset());
            if name.is_empty() {
                continue;
            }
            let path = reader.get_str(def.path_offset());
            let span = def.span();
            let file = reader.get_file(span.file_id());
            if !seen.insert((path, file, span.start_line(), span.start_col())) {
                continue;
            }
            if is_type_or_mod_ws_kind(kind) {
                type_defs.push((i as u32, d_idx as u32));
            } else {
                other_defs.push((i as u32, d_idx as u32));
            }
        }
    }
    let built = Arc::new(CachedWsDefs { crate_roots, type_defs, other_defs });
    if let Ok(mut guard) = mutex.lock() {
        if guard.len() >= 16 {
            guard.clear();
        }
        guard.insert(key, Arc::clone(&built));
    }
    built
}

fn is_excluded_ws_kind(kind: DefKind) -> bool {
    matches!(
        kind,
        DefKind::TyParam
            | DefKind::LifetimeParam
            | DefKind::ConstParam
            | DefKind::Local
            | DefKind::Field
            | DefKind::Use
            | DefKind::ExternCrate
            | DefKind::Impl
            | DefKind::Closure
            | DefKind::AnonConst
            | DefKind::InlineConst
            | DefKind::StructCtor
            | DefKind::VariantCtor
    )
}

fn is_type_or_mod_ws_kind(kind: DefKind) -> bool {
    matches!(
        kind,
        DefKind::Mod
            | DefKind::Struct
            | DefKind::Union
            | DefKind::Enum
            | DefKind::Trait
            | DefKind::TyAlias
            | DefKind::ForeignTy
            | DefKind::TraitAlias
            | DefKind::AssocTy
            | DefKind::Const
            | DefKind::Static
            | DefKind::AssocConst
            | DefKind::Macro
            | DefKind::ForeignMod
            | DefKind::OpaqueTy
    )
}

impl WorkspaceIndex {
    /// Fast workspace symbol search prioritizing local (non-dependency) crates with substring and fuzzy matching.
    pub fn search_workspace_symbols<'a>(&'a self, query: &str, limit: usize) -> Vec<WorkspaceSymbol<'a>> {
        let q_lower = query.to_ascii_lowercase();
        let q_bytes = q_lower.as_bytes();
        let has_local = self.reader_is_dependency.iter().any(|&d| !d);
        let multi_reader = self.readers.len() > 1;
        let mut results: Vec<WorkspaceSymbol<'a>> = Vec::new();
        if self.readers.len() > 4 && !query.is_empty() && !query.contains("::") {
            let cached = get_or_build_ws_defs(self, has_local);
            let fuzzy_ok = (2..=12).contains(&query.len());
            for pass in 0..4u8 {
                if results.len() >= limit || ((pass == 1 || pass == 3) && !fuzzy_ok) || (pass == 3 && !results.is_empty()) {
                    continue;
                }
                if pass <= 1 {
                    for &r_idx in &cached.crate_roots {
                        let i = r_idx as usize;
                        let reader = &self.readers[i];
                        let crate_name = reader.crate_name();
                        let m = if pass == 0 {
                            contains_ascii_ci(crate_name.as_bytes(), q_bytes)
                        } else {
                            !contains_ascii_ci(crate_name.as_bytes(), q_bytes) && is_fuzzy_subseq(crate_name, query)
                        };
                        if m {
                            let root_file = reader.get_file(0);
                            let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
                            let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
                            results.push(WorkspaceSymbol {
                                crate_name,
                                name: crate_name,
                                path: crate_name,
                                kind: DefKind::Mod,
                                file: root_file,
                                span: crate::format::Span::new(0, 1, 1, 1, crate_name.len() as u32 + 1),
                                signature: None,
                                is_macro_expansion: false,
                                source_root,
                                unit_key,
                            });
                            if results.len() >= limit {
                                return results;
                            }
                        }
                    }
                }
                let list = if pass <= 1 { &cached.type_defs } else { &cached.other_defs };
                for &(r_idx, d_idx) in list {
                    let i = r_idx as usize;
                    let reader = &self.readers[i];
                    let def = &reader.definitions()[d_idx as usize];
                    let name = reader.get_str(def.name_offset());
                    let m = if pass == 0 || pass == 2 {
                        contains_ascii_ci(name.as_bytes(), q_bytes)
                    } else {
                        !contains_ascii_ci(name.as_bytes(), q_bytes) && is_fuzzy_subseq(name, query)
                    };
                    if !m {
                        continue;
                    }
                    let span = def.span();
                    results.push(WorkspaceSymbol {
                        crate_name: reader.crate_name(),
                        name,
                        path: reader.get_str(def.path_offset()),
                        kind: DefKind::from(def.kind),
                        file: reader.get_file(span.file_id()),
                        span,
                        signature: if def.sig_offset() != 0 { Some(reader.get_str(def.sig_offset())) } else { None },
                        is_macro_expansion: def.is_macro_expansion(),
                        source_root: self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref()),
                        unit_key: self.unit_keys.get(i).and_then(|u| u.as_deref()),
                    });
                    if results.len() >= limit {
                        return results;
                    }
                }
            }
            return results;
        }
        let mut seen = std::collections::HashSet::new();

        for pass in 0..4u8 {
            if results.len() >= limit {
                break;
            }
            if pass > 0 && query.is_empty() {
                break;
            }
            if (pass == 1 || pass == 3) && query.len() < 2 {
                continue;
            }
            if pass == 3 && !results.is_empty() {
                break;
            }
            for (i, reader) in self.readers.iter().enumerate() {
                if has_local && self.reader_is_dependency.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let crate_name = reader.crate_name();
                let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
                let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());

                if multi_reader && (pass == 0 || pass == 1) && !crate_name.is_empty() {
                    let crate_matched = if pass == 0 { contains_ascii_ci(crate_name.as_bytes(), q_bytes) } else { is_fuzzy_subseq(crate_name, query) };
                    if crate_matched {
                        let root_file = reader.get_file(0);
                        if !root_file.is_empty() && seen.insert((crate_name, root_file, 1u32, 1u32)) {
                            results.push(WorkspaceSymbol {
                                crate_name,
                                name: crate_name,
                                path: crate_name,
                                kind: DefKind::Mod,
                                file: root_file,
                                span: crate::format::Span::new(0, 1, 1, 1, crate_name.len() as u32 + 1),
                                signature: None,
                                is_macro_expansion: false,
                                source_root,
                                unit_key,
                            });
                            if results.len() >= limit {
                                return results;
                            }
                        }
                    }
                }

                for def in reader.definitions() {
                    let kind = DefKind::from(def.kind);
                    if is_excluded_ws_kind(kind) {
                        continue;
                    }
                    if !query.is_empty() {
                        let is_type = is_type_or_mod_ws_kind(kind);
                        if (pass <= 1 && !is_type) || (pass >= 2 && is_type) {
                            continue;
                        }
                    }
                    let name = reader.get_str(def.name_offset());
                    if name.is_empty() {
                        continue;
                    }
                    let path = reader.get_str(def.path_offset());
                    let matched = if query.is_empty() {
                        true
                    } else if pass == 0 || pass == 2 {
                        contains_ascii_ci(name.as_bytes(), q_bytes)
                            || ((query.contains("::") || (!multi_reader && crate_name.eq_ignore_ascii_case(query))) && contains_ascii_ci(path.as_bytes(), q_bytes))
                    } else {
                        is_fuzzy_subseq(name, query)
                    };
                    if !matched {
                        continue;
                    }
                    let span = def.span();
                    let file = reader.get_file(span.file_id());
                    if !seen.insert((path, file, span.start_line(), span.start_col())) {
                        continue;
                    }
                    let signature = if def.sig_offset() != 0 { Some(reader.get_str(def.sig_offset())) } else { None };
                    results.push(WorkspaceSymbol { crate_name, name, path, kind, file, span, signature, is_macro_expansion: def.is_macro_expansion(), source_root, unit_key });
                    if results.len() >= limit {
                        return results;
                    }
                }
            }
        }
        results
    }
}
