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

pub use super::cache::CompactImpl;
use super::cache::{intern_path, intern_str};
use super::def_cache::{resolve_def, CachedDef};
use super::fallback_cache::FallbackCache;
use super::scan::{get_impl_line, parse_impl_line};
use super::types::parse_fallback_impl_path;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

const BLOCKING_IMPL: u32 = u32::MAX;

pub struct ImplTable {
    pub(crate) impls: Vec<CompactImpl>,
    pub(crate) names: Vec<Box<str>>,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) file_ids: HashMap<Box<str>, u32>,
    pub(crate) by_file_line: Vec<(u64, u32, u32)>,
    pub(crate) by_target_short: HashMap<Box<str>, Vec<u32>>,
    pub(crate) by_trait_short: HashMap<Box<str>, Vec<u32>>,
    pub(crate) trait_defs: RwLock<HashMap<Box<str>, Option<CachedDef>>>,
    pub(crate) target_defs: RwLock<HashMap<Box<str>, Option<CachedDef>>>,
    pub(crate) fallback_cache: FallbackCache,
    pub(crate) type_defs: Vec<super::type_index::TypeDefEntry>,
}

struct PendingImpl {
    file_id: u32,
    start_line: u32,
    reader_idx: u32,
    path_offset: u32,
    orig_idx: u32,
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
        let mut pending = Vec::new();

        for (reader_idx, reader) in ws.readers().iter().enumerate() {
            let defs = reader.definitions();
            let source_root = ws.source_root_at(reader_idx);
            for def in defs {
                if def.kind != DefKind::Impl as u8 {
                    continue;
                }
                let raw_file = reader.get_file(def.span.file_id);
                let full_path = match source_root {
                    Some(root) => ridx_format::join_resolving_overlap(root, Path::new(raw_file)),
                    None => PathBuf::from(raw_file),
                };
                let file_id = intern_path(&mut files, &mut file_to_id, &full_path);
                pending.push(PendingImpl {
                    file_id,
                    start_line: def.span.start_line(),
                    reader_idx: reader_idx as u32,
                    path_offset: def.path_offset,
                    orig_idx: pending.len() as u32,
                });
            }
        }

        pending.sort_unstable_by_key(|p| p.file_id);

        let mut resolved = Vec::with_capacity(pending.len());
        let mut extra_impl_lines = Vec::new();
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

            if let Some(lines) = &lines_opt {
                for (l_idx, line_str) in lines.iter().enumerate() {
                    let trimmed = line_str.trim_start();
                    if trimmed.starts_with("impl") {
                        let line_1based = (l_idx + 1) as u32;
                        let parsed = get_impl_line(lines, l_idx).and_then(|f| {
                            let (tr, tgt) = parse_impl_line(&f);
                            Some((tr?, tgt?))
                        });
                        extra_impl_lines.push((file_id, line_1based, parsed));
                    }
                }
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
            impls.push(CompactImpl { trait_id, target_id, file_id, line: start_line });

            let target_short = target_base.rsplit("::").next().unwrap_or(target_base);
            let trait_short = trait_base.rsplit("::").next().unwrap_or(trait_base);

            by_target_short.entry(target_short.into()).or_default().push(impl_idx);
            by_trait_short.entry(trait_short.into()).or_default().push(impl_idx);
        }

        let mut by_file_line: Vec<(u64, u32, u32)> = Vec::with_capacity(extra_impl_lines.len());

        for (file_id, line, parsed) in extra_impl_lines {
            let key = ((file_id as u64) << 32) | (line as u64);
            if let Some((tr_name, tgt_name)) = parsed {
                let tr_base = tr_name.split('<').next().unwrap_or(&tr_name).trim();
                let tgt_base = tgt_name.split('<').next().unwrap_or(&tgt_name).trim();
                if !tr_base.is_empty() {
                    let trait_id = intern_str(&mut names, &mut name_to_id, tr_base);
                    let target_id = intern_str(&mut names, &mut name_to_id, tgt_base);
                    by_file_line.push((key, trait_id, target_id));
                    continue;
                }
            }
            by_file_line.push((key, BLOCKING_IMPL, BLOCKING_IMPL));
        }

        by_file_line.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        by_file_line.dedup_by_key(|e| e.0);

        let mut file_ids = HashMap::with_capacity(files.len());
        for (idx, f) in files.iter().enumerate() {
            if let Some(s) = f.to_str() {
                file_ids.insert(s.into(), idx as u32);
            }
        }
        let fallback_cache = FallbackCache::new();

        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        // SAFETY: malloc_trim is an extern C function provided by glibc's allocator.
        // Calling it with pad = 0 instructs glibc to release free top-of-heap memory
        // to the OS. It has no undefined behavior or side effects on live Rust allocations.
        unsafe {
            extern "C" {
                fn malloc_trim(pad: usize) -> i32;
            }
            malloc_trim(0);
        }

        let type_defs = super::type_index::build_type_index(ws);

        Self {
            impls,
            names,
            files,
            file_ids,
            by_file_line,
            by_target_short,
            by_trait_short,
            trait_defs: RwLock::new(HashMap::new()),
            target_defs: RwLock::new(HashMap::new()),
            fallback_cache,
            type_defs,
        }
    }

    pub fn find_supertypes(&self, ws: &WorkspaceIndex, query_name: &str, seen: &mut HashSet<String>, results: &mut Vec<Value>) {
        let name_base = query_name.split('<').next().unwrap_or(query_name).trim();
        let short = name_base.rsplit("::").next().unwrap_or(name_base);

        if let Some(candidate_indices) = self.by_target_short.get(short) {
            for &idx in candidate_indices {
                let imp = &self.impls[idx as usize];
                let target_str = &self.names[imp.target_id as usize];
                if target_str.as_ref() == name_base || target_str.ends_with(&format!("::{name_base}")) {
                    let trait_str = &self.names[imp.trait_id as usize];
                    if !seen.insert(trait_str.to_string()) {
                        continue;
                    }
                    let def_val = self.resolve_trait_def(ws, trait_str, &self.files[imp.file_id as usize], imp.line);
                    results.push(def_val);
                }
            }
        }
    }

    pub fn find_subtypes(&self, ws: &WorkspaceIndex, query_name: &str, seen: &mut HashSet<String>, results: &mut Vec<Value>) {
        let name_base = query_name.split('<').next().unwrap_or(query_name).trim();
        let short = name_base.rsplit("::").next().unwrap_or(name_base);

        if let Some(candidate_indices) = self.by_trait_short.get(short) {
            for &idx in candidate_indices {
                let imp = &self.impls[idx as usize];
                let trait_str = &self.names[imp.trait_id as usize];
                if trait_str.as_ref() == name_base || trait_str.ends_with(&format!("::{name_base}")) {
                    let target_str = &self.names[imp.target_id as usize];
                    if !seen.insert(target_str.to_string()) {
                        continue;
                    }
                    let def_val = self.resolve_target_def(ws, target_str, &self.files[imp.file_id as usize], imp.line);
                    results.push(def_val);
                }
            }
        }
    }

    pub fn find_type_def<'a>(&self, ws: &'a WorkspaceIndex, query: &str, expected_mask: u8) -> Option<ridx_format::WorkspaceSymbol<'a>> {
        super::type_index::find_in_type_index(&self.type_defs, ws, query, expected_mask)
    }

    pub(crate) fn resolve_trait_def(&self, ws: &WorkspaceIndex, tr: &str, file: &Path, line: u32) -> Value {
        resolve_def(&self.trait_defs, tr, 11, file, line, || self.find_type_def(ws, tr, super::type_index::KIND_TRAIT).map(|t| super::def_cache::make_cached_def(&t, 11)))
    }

    pub(crate) fn resolve_target_def(&self, ws: &WorkspaceIndex, tgt: &str, file: &Path, line: u32) -> Value {
        resolve_def(&self.target_defs, tgt, 23, file, line, || {
            self.find_type_def(ws, tgt, super::type_index::KIND_STRUCT | super::type_index::KIND_ENUM)
                .map(|t| super::def_cache::make_cached_def(&t, crate::lsp::protocol::lsp_symbol_kind(t.kind)))
        })
    }

    pub fn find_impl_by_file_line(&self, file_path: &str, line: u32) -> Option<(&str, &str)> {
        let fid = self.file_ids.get(file_path)?;
        let max_offset = 4.min(line.saturating_sub(1));
        for offset in 0..=max_offset {
            let target_line = line - offset;
            let key = ((*fid as u64) << 32) | (target_line as u64);
            if let Ok(idx) = self.by_file_line.binary_search_by_key(&key, |e| e.0) {
                let entry = self.by_file_line[idx];
                if entry.1 == BLOCKING_IMPL {
                    return None;
                }
                let tr = &self.names[entry.1 as usize];
                let tgt = &self.names[entry.2 as usize];
                return Some((tr, tgt));
            }
        }
        None
    }

    pub fn get_or_parse_fallback_line(&self, file_path: &str, line_1based: u32, fallback: (Option<String>, Option<String>)) -> (Option<String>, Option<String>) {
        if let Some((tr, tgt)) = self.find_impl_by_file_line(file_path, line_1based) {
            self.fallback_cache.record_hit();
            return (Some(tr.to_string()), Some(tgt.to_string()));
        }
        if self.file_ids.contains_key(file_path) {
            self.fallback_cache.record_hit();
            return fallback;
        }
        self.fallback_cache.get_or_parse(file_path, line_1based, fallback)
    }

    pub fn get_fallback_results(&self, name: &str, is_supertype: bool) -> Option<Vec<Value>> {
        self.fallback_cache.get_fallback_results(name, is_supertype)
    }

    pub fn insert_fallback_results(&self, name: &str, is_supertype: bool, results: Vec<Value>) {
        self.fallback_cache.insert_fallback_results(name, is_supertype, results);
    }
}

pub use super::cache::{clear_impl_table_cache, get_or_build_impl_table};
