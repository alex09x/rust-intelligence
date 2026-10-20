/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Reference oracle implementations for `PathIndex` equivalence tests.

use crate::paths::normalize_path;
use crate::workspace::provenance::file_entry_matches;
use crate::workspace::WorkspaceIndex;
use std::path::{Path, PathBuf};

/// Reference oracle reproducing the legacy `shard_mtime_for_file` implementation verbatim.
pub(crate) fn shard_mtime_for_file_oracle(
    ws: &WorkspaceIndex,
    file_path: &str,
) -> Option<std::time::SystemTime> {
    for idx in ws.candidate_reader_indices_for_file(file_path) {
        if let Some(target_file) = ws.target_file_for_reader(idx, file_path) {
            if ws.readers[idx].find_file_id(target_file.as_ref()).is_some() {
                return ws.mtimes.get(idx).copied().flatten();
            }
            if ws
                .reader_file_hashes
                .get(idx)
                .is_some_and(|h| h.contains_key(target_file.as_ref()))
            {
                return ws.mtimes.get(idx).copied().flatten();
            }
        }
        if ws
            .reader_file_hashes
            .get(idx)
            .is_some_and(|h| h.contains_key(file_path))
        {
            return ws.mtimes.get(idx).copied().flatten();
        }
    }

    let p = Path::new(file_path);
    let (ws_abs, req_canon, ws_canon, candidate_indices) = legacy_candidates(ws, p);
    for &i in &candidate_indices {
        let reader = &ws.readers[i];
        let origin = reader_origin(ws, i);
        for fid in 0..reader.files_count() {
            let entry_str = reader.get_file(fid);
            if entry_str.is_empty() {
                continue;
            }
            if file_entry_matches(entry_str, origin, p, req_canon.as_deref())
                || ws_abs
                    .as_ref()
                    .is_some_and(|a| file_entry_matches(entry_str, origin, a, ws_canon.as_deref()))
            {
                return ws.mtimes.get(i).copied().flatten();
            }
        }
    }
    for &i in &candidate_indices {
        let origin = reader_origin(ws, i);
        if let Some(map) = ws.reader_file_hashes.get(i) {
            for k in map.keys() {
                if file_entry_matches(k, origin, p, req_canon.as_deref())
                    || ws_abs
                        .as_ref()
                        .is_some_and(|a| file_entry_matches(k, origin, a, ws_canon.as_deref()))
                {
                    return ws.mtimes.get(i).copied().flatten();
                }
            }
        }
    }
    None
}

/// Reference oracle reproducing the legacy `source_hash_for_file` implementation verbatim.
pub(crate) fn source_hash_for_file_oracle(ws: &WorkspaceIndex, file_path: &str) -> Option<u64> {
    for idx in ws.candidate_reader_indices_for_file(file_path) {
        if let Some(target_file) = ws.target_file_for_reader(idx, file_path) {
            if let Some(fid) = ws.readers[idx].find_file_id(target_file.as_ref()) {
                let h = ws.readers[idx].file_source_hash(fid);
                if h != 0 {
                    return Some(h);
                }
            }
            if let Some(&h) = ws
                .reader_file_hashes
                .get(idx)
                .and_then(|m| m.get(target_file.as_ref()))
            {
                if h != 0 {
                    return Some(h);
                }
            }
        }
        if let Some(&h) = ws
            .reader_file_hashes
            .get(idx)
            .and_then(|m| m.get(file_path))
        {
            if h != 0 {
                return Some(h);
            }
        }
    }

    let p = Path::new(file_path);
    let (ws_abs, req_canon, ws_canon, candidate_indices) = legacy_candidates(ws, p);
    for &i in &candidate_indices {
        let reader = &ws.readers[i];
        let origin = reader_origin(ws, i);
        for fid in 0..reader.files_count() {
            let entry_str = reader.get_file(fid);
            let h = reader.file_source_hash(fid);
            if h == 0 || entry_str.is_empty() {
                continue;
            }
            if file_entry_matches(entry_str, origin, p, req_canon.as_deref())
                || ws_abs
                    .as_ref()
                    .is_some_and(|a| file_entry_matches(entry_str, origin, a, ws_canon.as_deref()))
            {
                return Some(h);
            }
        }
    }
    for &i in &candidate_indices {
        let origin = reader_origin(ws, i);
        if let Some(map) = ws.reader_file_hashes.get(i) {
            for (k, &h) in map {
                if h == 0 || k.is_empty() {
                    continue;
                }
                if file_entry_matches(k, origin, p, req_canon.as_deref())
                    || ws_abs
                        .as_ref()
                        .is_some_and(|a| file_entry_matches(k, origin, a, ws_canon.as_deref()))
                {
                    return Some(h);
                }
            }
        }
    }
    None
}

fn reader_origin(ws: &WorkspaceIndex, i: usize) -> Option<&Path> {
    ws.source_roots
        .get(i)
        .and_then(|r| r.as_deref())
        .or(ws.workspace_root.as_deref())
        .or(ws.root_dir.as_deref())
}

pub(crate) fn cached_canonicalize(p: &Path) -> Option<PathBuf> {
    // Unit tests mutate the filesystem under temp_dir(), whereas corpus directories are static.
    if p.starts_with(std::env::temp_dir()) {
        return p.canonicalize().ok();
    }
    static CACHE: std::sync::OnceLock<
        std::sync::RwLock<std::collections::HashMap<PathBuf, Option<PathBuf>>>,
    > = std::sync::OnceLock::new();
    let lock = CACHE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    if let Some(res) = lock.read().unwrap().get(p) {
        return res.clone();
    }
    let res = p.canonicalize().ok();
    lock.write().unwrap().insert(p.to_path_buf(), res.clone());
    res
}

fn legacy_candidates(
    ws: &WorkspaceIndex,
    p: &Path,
) -> (
    Option<PathBuf>,
    Option<PathBuf>,
    Option<PathBuf>,
    Vec<usize>,
) {
    let explicit_origin = ws
        .workspace_root
        .as_deref()
        .or(ws.root_dir.as_deref())
        .or_else(|| (ws.readers.len() == 1).then(|| ws.source_roots.first()?.as_deref())?);

    let (ws_abs, req_canon) = if p.is_absolute() {
        (None, cached_canonicalize(p))
    } else if let Some(origin) = explicit_origin {
        let abs = origin.join(p);
        let canon = cached_canonicalize(&abs);
        (Some(abs), canon)
    } else {
        (None, None)
    };
    let ws_canon = req_canon.clone();

    let mut candidate_indices: Vec<usize> = Vec::with_capacity(ws.readers.len());
    if let Some(ref c_p) = req_canon {
        for i in 0..ws.readers.len() {
            if let Some(Some(ref root)) = ws.source_roots.get(i) {
                if cached_canonicalize(root).is_some_and(|cr| c_p.starts_with(&cr))
                    && !candidate_indices.contains(&i)
                {
                    candidate_indices.push(i);
                }
            }
        }
    }
    if let Some(ref abs) = ws_abs {
        let norm_abs = normalize_path(abs);
        for i in 0..ws.readers.len() {
            if let Some(Some(ref root)) = ws.source_roots.get(i) {
                if norm_abs.starts_with(normalize_path(root)) && !candidate_indices.contains(&i) {
                    candidate_indices.push(i);
                }
            }
        }
    } else if p.is_absolute() {
        let norm_p = normalize_path(p);
        for i in 0..ws.readers.len() {
            if let Some(Some(ref root)) = ws.source_roots.get(i) {
                if norm_p.starts_with(normalize_path(root)) && !candidate_indices.contains(&i) {
                    candidate_indices.push(i);
                }
            }
        }
    }
    for i in 0..ws.readers.len() {
        let is_dep = ws.reader_is_dependency.get(i).copied().unwrap_or(false)
            || ws.source_roots.get(i).and_then(|r| r.as_ref()).is_some();
        if !is_dep && !candidate_indices.contains(&i) {
            candidate_indices.push(i);
        }
    }
    for i in 0..ws.readers.len() {
        if !candidate_indices.contains(&i) {
            candidate_indices.push(i);
        }
    }
    (ws_abs, req_canon, ws_canon, candidate_indices)
}
