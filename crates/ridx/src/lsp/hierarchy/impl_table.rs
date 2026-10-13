/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! In-memory compact impl index and definition cache for ultra-fast type hierarchy.

use super::scan::{get_impl_line, parse_impl_line};
use super::types::parse_fallback_impl_path;
use crate::lsp::protocol::{lsp_symbol_kind, path_to_uri};
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Copy, Debug)]
pub struct CompactImpl {
    pub trait_id: u32,
    pub target_id: u32,
    pub file_id: u32,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct CachedDef {
    pub uri: String,
    pub range: Value,
    pub kind: u32,
}

impl CachedDef {
    fn to_json(&self, name: &str) -> Value {
        json!({
            "name": name,
            "kind": self.kind,
            "uri": self.uri,
            "range": self.range,
            "selectionRange": self.range,
        })
    }
}

pub struct ImplTable {
    pub(crate) impls: Vec<CompactImpl>,
    pub(crate) names: Vec<Box<str>>,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) by_target_short: HashMap<Box<str>, Vec<u32>>,
    pub(crate) by_trait_short: HashMap<Box<str>, Vec<u32>>,
    pub(crate) trait_defs: RwLock<HashMap<Box<str>, Option<CachedDef>>>,
    pub(crate) target_defs: RwLock<HashMap<Box<str>, Option<CachedDef>>>,
}

fn intern_str(names: &mut Vec<Box<str>>, map: &mut HashMap<Box<str>, u32>, s: &str) -> u32 {
    if let Some(&id) = map.get(s) {
        return id;
    }
    let id = names.len() as u32;
    let b: Box<str> = s.into();
    map.insert(b.clone(), id);
    names.push(b);
    id
}

fn intern_path(files: &mut Vec<PathBuf>, map: &mut HashMap<PathBuf, u32>, p: &Path) -> u32 {
    if let Some(&id) = map.get(p) {
        return id;
    }
    let id = files.len() as u32;
    let pb = p.to_path_buf();
    map.insert(pb.clone(), id);
    files.push(pb);
    id
}

impl ImplTable {
    pub fn build(ws: &WorkspaceIndex) -> Self {
        let mut names = Vec::new();
        let mut name_to_id = HashMap::new();
        let mut files = Vec::new();
        let mut file_to_id = HashMap::new();
        let mut impls = Vec::new();
        let mut by_target_short: HashMap<Box<str>, Vec<u32>> = HashMap::new();
        let mut by_trait_short: HashMap<Box<str>, Vec<u32>> = HashMap::new();

        struct PendingImpl {
            file_id: u32,
            start_line: u32,
            reader_idx: u32,
            path_offset: u32,
            orig_idx: u32,
        }

        let mut pending = Vec::new();

        for (reader_idx, reader) in ws.readers().iter().enumerate() {
            let defs = reader.definitions();
            let source_root = ws.source_root_at(reader_idx);
            for def in defs {
                if def.kind != DefKind::Impl as u8 {
                    continue;
                }
                let raw_file = reader.get_file(def.span.file_id);
                let full_path = if let Some(root) = source_root {
                    ridx_format::join_resolving_overlap(root, Path::new(raw_file))
                } else {
                    PathBuf::from(raw_file)
                };
                let file_id = intern_path(&mut files, &mut file_to_id, &full_path);

                let orig_idx = pending.len() as u32;
                pending.push(PendingImpl {
                    file_id,
                    start_line: def.span.start_line(),
                    reader_idx: reader_idx as u32,
                    path_offset: def.path_offset,
                    orig_idx,
                });
            }
        }

        pending.sort_unstable_by_key(|p| p.file_id);

        let mut resolved = Vec::with_capacity(pending.len());
        let mut i = 0;
        while i < pending.len() {
            let file_id = pending[i].file_id;
            let start = i;
            while i < pending.len() && pending[i].file_id == file_id {
                i += 1;
            }
            let chunk = &pending[start..i];
            let file_path = &files[file_id as usize];
            let content_opt = std::fs::read_to_string(file_path).ok();
            let lines_opt: Option<Vec<&str>> = content_opt.as_ref().map(|c| c.lines().collect());

            for p in chunk {
                let l_idx = p.start_line.saturating_sub(1) as usize;
                let parsed = lines_opt
                    .as_ref()
                    .and_then(|lines| get_impl_line(lines, l_idx).map(|l| parse_impl_line(&l)))
                    .or_else(|| {
                        let r = &ws.readers()[p.reader_idx as usize];
                        let raw_path = r.get_str(p.path_offset);
                        let fallback = parse_fallback_impl_path(raw_path);
                        if fallback.0.is_some() && fallback.1.is_some() {
                            Some(fallback)
                        } else {
                            None
                        }
                    })
                    .unwrap_or((None, None));
                resolved.push((p.orig_idx, p.file_id, p.start_line, parsed.0, parsed.1));
            }
        }
        drop(pending);

        resolved.sort_unstable_by_key(|r| r.0);

        for (_orig_idx, file_id, start_line, trait_name_opt, target_name_opt) in resolved {
            let (Some(trait_name), Some(target_name)) = (trait_name_opt, target_name_opt) else {
                continue;
            };

            let trait_base = trait_name.split('<').next().unwrap_or(&trait_name).trim();
            let target_base = target_name.split('<').next().unwrap_or(&target_name).trim();

            if trait_base.is_empty() {
                continue;
            }

            let trait_id = intern_str(&mut names, &mut name_to_id, trait_base);
            let target_id = intern_str(&mut names, &mut name_to_id, target_base);

            let impl_idx = impls.len() as u32;
            impls.push(CompactImpl {
                trait_id,
                target_id,
                file_id,
                line: start_line,
            });

            let target_short = target_base.rsplit("::").next().unwrap_or(target_base);
            let trait_short = trait_base.rsplit("::").next().unwrap_or(trait_base);

            by_target_short
                .entry(target_short.into())
                .or_default()
                .push(impl_idx);
            by_trait_short
                .entry(trait_short.into())
                .or_default()
                .push(impl_idx);
        }

        #[cfg(target_os = "linux")]
        unsafe {
            extern "C" {
                fn malloc_trim(pad: usize) -> i32;
            }
            malloc_trim(0);
        }

        Self {
            impls,
            names,
            files,
            by_target_short,
            by_trait_short,
            trait_defs: RwLock::new(HashMap::new()),
            target_defs: RwLock::new(HashMap::new()),
        }
    }

    pub fn find_supertypes(
        &self,
        ws: &WorkspaceIndex,
        query_name: &str,
        seen: &mut HashSet<String>,
        results: &mut Vec<Value>,
    ) {
        let name_base = query_name.split('<').next().unwrap_or(query_name).trim();
        let short = name_base.rsplit("::").next().unwrap_or(name_base);

        if let Some(candidate_indices) = self.by_target_short.get(short) {
            for &idx in candidate_indices {
                let imp = &self.impls[idx as usize];
                let target_str = &self.names[imp.target_id as usize];
                if target_str.as_ref() == name_base
                    || target_str.ends_with(&format!("::{name_base}"))
                {
                    let trait_str = &self.names[imp.trait_id as usize];
                    if !seen.insert(trait_str.to_string()) {
                        continue;
                    }
                    let def_val = self.resolve_trait_def(
                        ws,
                        trait_str,
                        &self.files[imp.file_id as usize],
                        imp.line,
                    );
                    results.push(def_val);
                }
            }
        }
    }

    pub fn find_subtypes(
        &self,
        ws: &WorkspaceIndex,
        query_name: &str,
        seen: &mut HashSet<String>,
        results: &mut Vec<Value>,
    ) {
        let name_base = query_name.split('<').next().unwrap_or(query_name).trim();
        let short = name_base.rsplit("::").next().unwrap_or(name_base);

        if let Some(candidate_indices) = self.by_trait_short.get(short) {
            for &idx in candidate_indices {
                let imp = &self.impls[idx as usize];
                let trait_str = &self.names[imp.trait_id as usize];
                if trait_str.as_ref() == name_base
                    || trait_str.ends_with(&format!("::{name_base}"))
                {
                    let target_str = &self.names[imp.target_id as usize];
                    if !seen.insert(target_str.to_string()) {
                        continue;
                    }
                    let def_val = self.resolve_target_def(
                        ws,
                        target_str,
                        &self.files[imp.file_id as usize],
                        imp.line,
                    );
                    results.push(def_val);
                }
            }
        }
    }

    fn resolve_def<F>(
        cache: &RwLock<HashMap<Box<str>, Option<CachedDef>>>,
        name: &str,
        kind: u32,
        file: &Path,
        line: u32,
        find_fn: F,
    ) -> Value
    where
        F: FnOnce() -> Option<CachedDef>,
    {
        if let Ok(guard) = cache.read() {
            if let Some(cached) = guard.get(name) {
                return match cached {
                    Some(cd) => cd.to_json(name),
                    None => fallback_json(name, kind, file, line),
                };
            }
        }
        let opt_def = find_fn();
        let result = match &opt_def {
            Some(cd) => cd.to_json(name),
            None => fallback_json(name, kind, file, line),
        };
        if let Ok(mut guard) = cache.write() {
            guard.insert(name.into(), opt_def);
        }
        result
    }

    fn resolve_trait_def(&self, ws: &WorkspaceIndex, tr: &str, file: &Path, line: u32) -> Value {
        Self::resolve_def(&self.trait_defs, tr, 11, file, line, || find_first_trait_def(ws, tr))
    }

    fn resolve_target_def(&self, ws: &WorkspaceIndex, tgt: &str, file: &Path, line: u32) -> Value {
        Self::resolve_def(&self.target_defs, tgt, 23, file, line, || find_first_target_def(ws, tgt))
    }
}

fn fallback_json(name: &str, kind: u32, file_path: &Path, impl_line: u32) -> Value {
    let uri = path_to_uri(file_path.to_str().unwrap_or(""));
    let r = json!({
        "start": { "line": impl_line.saturating_sub(1), "character": 0 },
        "end": { "line": impl_line.saturating_sub(1), "character": 0 }
    });
    json!({ "name": name, "kind": kind, "uri": uri, "range": r, "selectionRange": r })
}

fn make_cached_def(sym: &ridx_format::WorkspaceSymbol, kind: u32) -> CachedDef {
    let p = sym.resolved_path();
    let s = p.to_str().unwrap_or(sym.file);
    let range = json!({
        "start": { "line": sym.span.start_line().saturating_sub(1), "character": sym.span.start_col().saturating_sub(1) },
        "end": { "line": sym.span.end_line().saturating_sub(1), "character": sym.span.end_col().saturating_sub(1) }
    });
    CachedDef { uri: path_to_uri(s), range, kind }
}

fn find_first_trait_def(ws: &WorkspaceIndex, trait_base: &str) -> Option<CachedDef> {
    ws.find_first_symbol(trait_base, |d| d.name == trait_base && d.kind == DefKind::Trait)
        .map(|t| make_cached_def(&t, 11))
}

fn find_first_target_def(ws: &WorkspaceIndex, target_base: &str) -> Option<CachedDef> {
    ws.find_first_symbol(target_base, |d| {
        d.name == target_base && matches!(d.kind, DefKind::Struct | DefKind::Enum)
    })
    .map(|t| make_cached_def(&t, lsp_symbol_kind(t.kind)))
}

type CacheMap = HashMap<u64, (u64, Arc<ImplTable>)>;
static IMPL_TABLE_CACHE: OnceLock<RwLock<CacheMap>> = OnceLock::new();

fn get_cache() -> &'static RwLock<CacheMap> {
    IMPL_TABLE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn get_or_build_impl_table(ws: &WorkspaceIndex) -> Arc<ImplTable> {
    let key = ws.instance_id();
    let gen = ws.generation();

    if let Ok(guard) = get_cache().read() {
        if let Some((cached_gen, table)) = guard.get(&key) {
            if *cached_gen == gen {
                return Arc::clone(table);
            }
        }
    }

    let new_table = Arc::new(ImplTable::build(ws));
    if let Ok(mut guard) = get_cache().write() {
        if guard.len() > 16 {
            guard.clear();
        }
        guard.insert(key, (gen, Arc::clone(&new_table)));
    }
    new_table
}

pub fn clear_impl_table_cache() {
    if let Some(cache) = IMPL_TABLE_CACHE.get() {
        if let Ok(mut guard) = cache.write() {
            guard.clear();
        }
    }
}
