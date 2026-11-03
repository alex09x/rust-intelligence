/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-workspace O(1) exact short-name symbol index for low-latency code actions and quickfixes.

use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

type CacheKey = (u64, u64, usize);

struct ExactSymbolCache {
    by_name: HashMap<String, Vec<(u32, u32)>>,
}

static EXACT_SYMBOL_CACHE: OnceLock<Mutex<HashMap<CacheKey, Arc<ExactSymbolCache>>>> = OnceLock::new();

impl ExactSymbolCache {
    fn get(ws: &WorkspaceIndex) -> Arc<Self> {
        let key = (ws.instance_id, ws.generation, ws.readers.len());
        let mutex = EXACT_SYMBOL_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(existing) = mutex.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return Arc::clone(existing);
        }
        let built = Arc::new(Self::build(ws));
        let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|&(inst, gen, _), _| inst != ws.instance_id || gen == ws.generation);
        guard.insert(key, Arc::clone(&built));
        built
    }

    fn build(ws: &WorkspaceIndex) -> Self {
        let mut by_name: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
        for (ri, reader) in ws.readers.iter().enumerate() {
            for (di, def) in reader.definitions().iter().enumerate() {
                if def.kind == DefKind::TyParam as u8 {
                    continue;
                }
                let raw_name = reader.get_str(def.name_offset());
                let bare = raw_name.strip_prefix("r#").unwrap_or(raw_name);
                if !bare.is_empty() {
                    by_name.entry(bare.to_string()).or_default().push((ri as u32, di as u32));
                }
            }
        }
        Self { by_name }
    }
}

impl WorkspaceIndex {
    /// O(1) exact bare-identifier lookup across all indexed definitions (including dependencies).
    pub fn exact_symbols_by_name<'a>(&'a self, ident: &str) -> Vec<WorkspaceSymbol<'a>> {
        self.exact_symbols_filtered(ident, |_| true)
    }

    /// O(1) exact bare-identifier lookup across local (non-dependency) workspace definitions.
    pub fn exact_local_symbols_by_name<'a>(&'a self, ident: &str) -> Vec<WorkspaceSymbol<'a>> {
        self.exact_symbols_filtered(ident, |ri| !self.reader_is_dependency.get(ri).copied().unwrap_or(false))
    }

    /// O(1) exact bare-identifier lookup filtered by shard reader index.
    pub fn exact_symbols_filtered<'a, F>(&'a self, ident: &str, mut filter: F) -> Vec<WorkspaceSymbol<'a>>
    where
        F: FnMut(usize) -> bool,
    {
        let bare = ident.strip_prefix("r#").unwrap_or(ident);
        if bare.is_empty() {
            return Vec::new();
        }
        let cache = ExactSymbolCache::get(self);
        let Some(entries) = cache.by_name.get(bare) else {
            return Vec::new();
        };
        let mut matches = Vec::with_capacity(entries.len());
        let mut seen = HashSet::new();
        for &(ri_u32, di_u32) in entries {
            let ri = ri_u32 as usize;
            if !filter(ri) {
                continue;
            }
            let Some(reader) = self.readers.get(ri) else { continue };
            let Some(def) = reader.definitions().get(di_u32 as usize) else { continue };
            let name = reader.get_str(def.name_offset());
            let path = reader.get_str(def.path_offset());
            let span = def.span();
            let file = reader.get_file(span.file_id());
            if !seen.insert((path, file, span.start_line(), span.start_col())) {
                continue;
            }
            let signature = if def.sig_offset() != 0 { Some(reader.get_str(def.sig_offset())) } else { None };
            let source_root = self.source_roots.get(ri).and_then(|r| r.as_deref()).or(self.workspace_root.as_deref());
            let unit_key = self.unit_keys.get(ri).and_then(|u| u.as_deref());
            matches.push(WorkspaceSymbol {
                crate_name: reader.crate_name(),
                name,
                path,
                kind: DefKind::from(def.kind),
                file,
                span,
                signature,
                is_macro_expansion: def.is_macro_expansion(),
                source_root,
                unit_key,
            });
        }
        matches
    }
}
