/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! In-memory cache for parsed lexical scopes, trait supertraits, and associated type bounds.
//!
//! Eliminates repeated disk I/O and lexical re-parsing across completion queries in hot paths.

pub mod parse;
pub mod traits;
pub mod types;

#[cfg(test)]
pub mod tests;

pub use parse::*;
pub use traits::*;
pub use types::*;

use ridx_format::WorkspaceIndex;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::SystemTime;

type ScopeCacheMap = HashMap<String, (FileCacheKey, Arc<FileScopeData>)>;

static FILE_SCOPE_CACHE: OnceLock<RwLock<ScopeCacheMap>> = OnceLock::new();

fn file_scope_cache() -> &'static RwLock<ScopeCacheMap> {
    FILE_SCOPE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Retrieves or parses `FileScopeData` for a given file and optional overlay text.
pub fn get_or_parse_file_scopes(file: &str, overlay_text: Option<&str>) -> Option<Arc<FileScopeData>> {
    let key = if let Some(text) = overlay_text {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in text.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        FileCacheKey::Overlay { path: file.to_string(), full_hash: h }
    } else {
        let meta = Path::new(file).metadata().ok()?;
        FileCacheKey::Disk { path: file.to_string(), mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), len: meta.len() }
    };

    let cache = file_scope_cache();
    if let Ok(guard) = cache.read() {
        if let Some((cached_key, data)) = guard.get(file) {
            if *cached_key == key {
                return Some(Arc::clone(data));
            }
        }
    }

    let text_arc: Arc<str> = if let Some(text) = overlay_text { Arc::from(text) } else { Arc::from(std::fs::read_to_string(file).ok()?) };

    let parsed = Arc::new(parse_file_scopes(text_arc));

    if let Ok(mut guard) = cache.write() {
        if guard.len() >= 64 {
            guard.clear();
        }
        guard.insert(file.to_string(), (key, Arc::clone(&parsed)));
    }

    Some(parsed)
}

type FileTraitsCacheMap = HashMap<String, (FileCacheKey, u64, Arc<HashSet<String>>)>;

static FILE_TRAITS_CACHE: OnceLock<RwLock<FileTraitsCacheMap>> = OnceLock::new();

fn file_traits_cache() -> &'static RwLock<FileTraitsCacheMap> {
    FILE_TRAITS_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Retrieves or collects referenced traits in `file` using scope cache.
pub fn get_or_collect_file_referenced_traits(ws: &WorkspaceIndex, file: &str, overlay_text: Option<&str>) -> Arc<HashSet<String>> {
    let key = if let Some(text) = overlay_text {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in text.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        FileCacheKey::Overlay { path: file.to_string(), full_hash: h }
    } else if let Ok(meta) = Path::new(file).metadata() {
        FileCacheKey::Disk { path: file.to_string(), mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), len: meta.len() }
    } else {
        FileCacheKey::Disk { path: file.to_string(), mtime: SystemTime::UNIX_EPOCH, len: 0 }
    };

    let ws_fingerprint = {
        let mut h: u64 = 0xcbf29ce484222325;
        h ^= ws.readers().len() as u64;
        h = h.wrapping_mul(0x100000001b3);
        if let Some(mt) = ws.shard_mtime_for_file(file) {
            let nanos = mt.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
            h ^= nanos as u64;
            h = h.wrapping_mul(0x100000001b3);
            h ^= (nanos >> 64) as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        for idx in ws.candidate_reader_indices_for_file(file) {
            if let Some(r) = ws.readers().get(idx) {
                h ^= r.references().len() as u64;
                h = h.wrapping_mul(0x100000001b3);
                h ^= r.definitions().len() as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        h
    };
    let profile_active = is_profile_completion_enabled();
    let t_traits0 = profile_active.then(std::time::Instant::now);

    let cache = file_traits_cache();
    if let Ok(guard) = cache.read() {
        if let Some((cached_key, cached_fp, traits)) = guard.get(file) {
            if *cached_key == key && *cached_fp == ws_fingerprint {
                if let Some(t) = t_traits0 {
                    eprintln!("[RIDX_PROFILE_TRAITS] hit=true us={}", t.elapsed().as_micros());
                }
                return Arc::clone(traits);
            }
        }
    }

    let collected = Arc::new(collect_file_referenced_traits(ws, file));

    if let Ok(mut guard) = cache.write() {
        if guard.len() >= 64 {
            guard.clear();
        }
        guard.insert(file.to_string(), (key, ws_fingerprint, Arc::clone(&collected)));
    }

    if let Some(t) = t_traits0 {
        eprintln!("[RIDX_PROFILE_TRAITS] hit=false us={}", t.elapsed().as_micros());
    }

    collected
}

/// Checks whether completion latency breakdown profiling is active.
pub fn is_profile_completion_enabled() -> bool {
    static PROFILE: OnceLock<bool> = OnceLock::new();
    *PROFILE.get_or_init(|| std::env::var_os("RIDX_PROFILE_COMPLETION").is_some())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerefChainItem {
    pub target: String,
    pub is_trait: bool,
}

type DerefChainCacheMap = HashMap<(String, usize), (u64, Arc<[DerefChainItem]>)>;

static DEREF_CHAIN_CACHE: OnceLock<RwLock<DerefChainCacheMap>> = OnceLock::new();

fn deref_chain_cache() -> &'static RwLock<DerefChainCacheMap> {
    DEREF_CHAIN_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Computes a lightweight fingerprint of the workspace index state for deref resolution.
pub fn workspace_deref_fingerprint(ws: &WorkspaceIndex) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    h ^= ws.readers().len() as u64;
    h = h.wrapping_mul(0x100000001b3);
    if let Some(avid) = ws.active_variant_id() {
        for &b in avid.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    if let Some(cas) = ws.sysroot_cas_key() {
        for &b in cas.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    for r in ws.readers() {
        let hdr = r.header();
        h ^= hdr.defs_count as u64;
        h = h.wrapping_mul(0x100000001b3);
        h ^= hdr.derefs_count as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Retrieves or computes the autoderef chain for `owner` up to `max_depth` across workspace readers.
pub fn get_or_resolve_deref_chain(ws: &WorkspaceIndex, owner: &str, max_depth: usize) -> Arc<[DerefChainItem]> {
    let ws_fp = workspace_deref_fingerprint(ws);
    let cache = deref_chain_cache();

    if let Ok(guard) = cache.read() {
        if let Some((cached_fp, targets)) = guard.get(&(owner.to_string(), max_depth)) {
            if *cached_fp == ws_fp {
                return Arc::clone(targets);
            }
        }
    }

    let raw_targets = ws.resolve_deref_chain(owner, max_depth);
    let mut resolved = Vec::with_capacity(raw_targets.len());
    for target in raw_targets {
        let is_trait = ws.find_definition_by_path(target).is_some_and(|d| d.kind == ridx_format::DefKind::Trait);
        resolved.push(DerefChainItem { target: target.to_string(), is_trait });
    }
    let arc_resolved: Arc<[DerefChainItem]> = Arc::from(resolved);

    if let Ok(mut guard) = cache.write() {
        if guard.len() >= 512 {
            guard.clear();
        }
        guard.insert((owner.to_string(), max_depth), (ws_fp, Arc::clone(&arc_resolved)));
    }

    arc_resolved
}
