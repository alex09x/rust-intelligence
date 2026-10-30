/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Global generation-keyed cache for workspace ImplTable instances.

use super::impl_table::ImplTable;
use ridx_format::WorkspaceIndex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Copy, Debug)]
pub struct CompactImpl {
    pub trait_id: u32,
    pub target_id: u32,
    pub file_id: u32,
    pub line: u32,
}

pub(crate) fn intern_str(names: &mut Vec<Box<str>>, map: &mut HashMap<Box<str>, u32>, s: &str) -> u32 {
    if let Some(&id) = map.get(s) {
        return id;
    }
    let id = names.len() as u32;
    names.push(s.into());
    map.insert(s.into(), id);
    id
}

pub(crate) fn intern_path(files: &mut Vec<PathBuf>, map: &mut HashMap<PathBuf, u32>, p: &Path) -> u32 {
    if let Some(&id) = map.get(p) {
        return id;
    }
    let id = files.len() as u32;
    files.push(p.to_path_buf());
    map.insert(p.to_path_buf(), id);
    id
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
