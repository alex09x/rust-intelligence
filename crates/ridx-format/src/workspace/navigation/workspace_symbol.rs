/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::workspace_symbol_use::{import_target_kind, use_is_pub};
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

/// A symbol rust-analyzer's search covers: a workspace definition `(reader, definition)` or a crate root
/// `(reader, CRATE_ROOT)`, with the kind it is listed under (an import takes the kind of what it imports).
#[derive(Clone, Copy)]
struct SymEntry {
    reader: u32,
    def: u32,
    kind: DefKind,
}

struct CachedWsDefs {
    /// Every symbol, dependencies' crate roots included, ordered by lowercased name.
    all_by_name: Vec<SymEntry>,
    /// The type-like subset rust-analyzer searches first (its default `only_types` search kind).
    types_by_name: Vec<SymEntry>,
}

/// Marks a crate root entry.
const CRATE_ROOT: u32 = u32::MAX;

fn entry_name(ws: &WorkspaceIndex, e: SymEntry) -> &str {
    let reader = &ws.readers[e.reader as usize];
    if e.def == CRATE_ROOT {
        reader.crate_name()
    } else {
        reader.get_str(reader.definitions()[e.def as usize].name_offset())
    }
}

/// What rust-analyzer's `only_types` search returns: types, traits, type aliases and crates, not modules.
fn is_ra_type_entry(e: &SymEntry) -> bool {
    e.def == CRATE_ROOT
        || matches!(e.kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait | DefKind::TraitAlias | DefKind::TyAlias | DefKind::AssocTy | DefKind::ForeignTy)
}

fn get_or_build_ws_defs(ws: &WorkspaceIndex, has_local: bool) -> std::sync::Arc<CachedWsDefs> {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex, OnceLock};
    type CacheKey = (u64, bool);
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, Arc<CachedWsDefs>>>> = OnceLock::new();
    // Every reader's definition table (address and length) identifies the workspace: entries index into them.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for r in &ws.readers {
        std::hash::Hash::hash(&(r.definitions().as_ptr() as usize, r.definitions().len(), r.crate_name()), &mut hasher);
    }
    let key = (std::hash::Hasher::finish(&hasher), has_local);
    let mutex = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = mutex.lock() {
        if let Some(found) = guard.get(&key) {
            return Arc::clone(found);
        }
    }
    let mut by_name: Vec<SymEntry> = Vec::new();
    let mut seen = HashSet::new();
    let mut sources: HashMap<&str, Option<String>> = HashMap::new();
    for (i, reader) in ws.readers.iter().enumerate() {
        if has_local && ws.reader_is_dependency.get(i).copied().unwrap_or(false) {
            continue;
        }
        for (d_idx, def) in reader.definitions().iter().enumerate() {
            let kind = DefKind::from(def.kind);
            let name = reader.get_str(def.name_offset());
            // rust-analyzer lists re-exports (`pub use a::B;`) and renaming imports (`use a::B as C;`) as symbols,
            // not plain private imports, globs or `self`.
            let target = if def.sig_offset() != 0 { reader.get_str(def.sig_offset()) } else { "" };
            let renamed = target.rsplit("::").next().is_some_and(|last| !last.is_empty() && last != name);
            let is_named_import = kind == DefKind::Use && !matches!(name, "*" | "_" | "self") && {
                let span = def.span();
                let file = reader.get_file(span.file_id());
                let read = || ws.read_source_file(file).or_else(|| ws.resolve_path_for_crate(reader.crate_name(), file).to_str().and_then(|s| ws.read_source_file(s)));
                renamed || sources.entry(file).or_insert_with(read).as_deref().is_some_and(|src| use_is_pub(src, span.start_line(), span.start_col()))
            };
            if (is_excluded_ws_kind(kind) && !is_named_import) || name.is_empty() {
                continue;
            }
            let path = reader.get_str(def.path_offset());
            let span = def.span();
            // rustc's test descriptor: a `const` named like its `#[test]` function, spanning the attribute, or sharing
            // the function's path (a `const` and a `fn` never share one otherwise) where a macro wrote both.
            let width = span.end_col().saturating_sub(span.start_col()) as usize;
            let spans_attr = span.start_line() == span.end_line() && width != name.chars().count() && width != name.chars().count() + 2;
            if kind == DefKind::Const && (spans_attr || reader.has_multiple_definitions_at_path(path)) {
                continue;
            }
            // rust-analyzer collects items declared in the bodies of module-level functions, not in method bodies.
            if path.rsplit_once("::").and_then(|(parent, _)| reader.find_definition_by_path(parent)).is_some_and(|p| DefKind::from(p.kind) == DefKind::AssocFn) {
                continue;
            }
            let file = reader.get_file(span.file_id());
            if !seen.insert((path, file, span.start_line(), span.start_col())) {
                continue;
            }
            let kind = if is_named_import && !target.is_empty() { import_target_kind(ws, reader, target) } else { kind };
            by_name.push(SymEntry { reader: i as u32, def: d_idx as u32, kind });
        }
    }
    let mut crates_seen = HashSet::new();
    for (i, reader) in ws.readers.iter().enumerate() {
        let (crate_name, root_file) = (reader.crate_name(), reader.get_file(0));
        if !crate_name.is_empty() && !root_file.is_empty() && crates_seen.insert((crate_name, root_file)) {
            by_name.push(SymEntry { reader: i as u32, def: CRATE_ROOT, kind: DefKind::Mod });
        }
    }
    // Stable sort: equal lowercased names keep index order. Lowercasing on the fly keeps no name copies alive.
    by_name.sort_by(|&a, &b| entry_name(ws, a).chars().flat_map(char::to_lowercase).cmp(entry_name(ws, b).chars().flat_map(char::to_lowercase)));
    let types_by_name = by_name.iter().copied().filter(is_ra_type_entry).collect();
    let built = Arc::new(CachedWsDefs { all_by_name: by_name, types_by_name });
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

/// rust-analyzer's symbol match: the query is a subsequence of the name, compared case-insensitively except for the
/// query's own uppercase letters when it has any.
fn ra_fuzzy_match(name: &str, query: &str, case_sensitive: bool) -> bool {
    if name.len() < query.len() {
        return false;
    }
    // Smart case: with an uppercase letter in the query its uppercase letters must match exactly; lowercase ones
    // always match either case (`oolVisito` finds `TagContentOtherFieldVisitor`, `Option` misses `option`).
    if name.is_ascii() && query.is_ascii() {
        let mut nb = name.bytes();
        return query.bytes().all(|q| nb.any(|n| if case_sensitive && q.is_ascii_uppercase() { n == q } else { n.eq_ignore_ascii_case(&q) }));
    }
    let mut name_chars = name.chars();
    query.chars().all(|q| name_chars.any(|n| if case_sensitive && q.is_uppercase() { n == q } else { n.to_lowercase().eq(q.to_lowercase()) }))
}

impl WorkspaceIndex {
    /// Workspace symbols as rust-analyzer lists them: items and imports whose name contains the query as a smart-case
    /// subsequence, plus crates (dependencies included) whose name does, ordered by lowercased name and cut at
    /// `limit`; types first, every kind only when no type matches.
    fn search_workspace_symbols_like_ra<'a>(&'a self, query: &str, limit: usize) -> Vec<WorkspaceSymbol<'a>> {
        let has_local = self.reader_is_dependency.iter().any(|&d| !d);
        let cached = get_or_build_ws_defs(self, has_local);
        // rust-analyzer searches types first and all symbols only when no type matches.
        let types = self.collect_ra_matches(&cached.types_by_name, query, limit);
        if !types.is_empty() {
            return types;
        }
        self.collect_ra_matches(&cached.all_by_name, query, limit)
    }

    fn collect_ra_matches<'a>(&'a self, entries: &[SymEntry], query: &str, limit: usize) -> Vec<WorkspaceSymbol<'a>> {
        let mut out = Vec::new();
        // One item compiled into several units (lib, lib test, bin) is one symbol: dedupe on the file it resolves to.
        let mut seen = std::collections::HashSet::new();
        let case_sensitive = query.chars().any(char::is_uppercase);
        // rust-analyzer leaves `__`-prefixed names (macro internals such as `__DeserializeWith`) out unless asked for.
        let skip_dunder = !query.starts_with("__");
        for &entry in entries {
            let name = entry_name(self, entry);
            if (skip_dunder && name.starts_with("__")) || !ra_fuzzy_match(name, query, case_sensitive) {
                continue;
            }
            let symbol = self.ws_symbol_at(entry);
            if !seen.insert((symbol.resolved_path(), symbol.span.start_line(), symbol.span.start_col())) {
                continue;
            }
            out.push(symbol);
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    fn ws_symbol_at(&self, entry: SymEntry) -> WorkspaceSymbol<'_> {
        let i = entry.reader as usize;
        let reader = &self.readers[i];
        let source_root = self.source_roots.get(i).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
        let unit_key = self.unit_keys.get(i).and_then(|u| u.as_deref());
        let crate_name = reader.crate_name();
        if entry.def == CRATE_ROOT {
            let span = crate::format::Span::new(0, 1, 1, 1, crate_name.len() as u32 + 1);
            return WorkspaceSymbol {
                crate_name,
                name: crate_name,
                path: crate_name,
                kind: DefKind::Mod,
                file: reader.get_file(0),
                span,
                signature: None,
                is_macro_expansion: false,
                source_root,
                unit_key,
            };
        }
        let def = &reader.definitions()[entry.def as usize];
        let span = def.span();
        WorkspaceSymbol {
            crate_name,
            name: reader.get_str(def.name_offset()),
            path: reader.get_str(def.path_offset()),
            kind: entry.kind,
            file: reader.get_file(span.file_id()),
            span,
            signature: if def.sig_offset() != 0 { Some(reader.get_str(def.sig_offset())) } else { None },
            is_macro_expansion: def.is_macro_expansion(),
            source_root,
            unit_key,
        }
    }

    /// Workspace symbol search. A plain query follows rust-analyzer (see `search_workspace_symbols_like_ra`); an empty
    /// query or a path query (`a::b`) keeps the local-first substring search.
    pub fn search_workspace_symbols<'a>(&'a self, query: &str, limit: usize) -> Vec<WorkspaceSymbol<'a>> {
        if !query.is_empty() && !query.contains("::") {
            return self.search_workspace_symbols_like_ra(query, limit);
        }
        let q_lower = query.to_ascii_lowercase();
        let q_bytes = q_lower.as_bytes();
        let has_local = self.reader_is_dependency.iter().any(|&d| !d);
        let multi_reader = self.readers.len() > 1;
        let mut results: Vec<WorkspaceSymbol<'a>> = Vec::new();
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

#[cfg(test)]
#[path = "workspace_symbol_tests.rs"]
mod tests;
