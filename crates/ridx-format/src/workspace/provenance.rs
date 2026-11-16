/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Source provenance, hash resolution, and shard metadata loading.

mod metadata;
#[cfg(test)]
mod tests;

use super::target_kind;
use super::WorkspaceIndex;
#[cfg(test)]
use crate::paths::normalize_path;
use std::fs;
use std::path::{Path, PathBuf};

/// Checks whether a path contains a `ParentDir` (`..`) that pops across a symlink component,
/// meaning lexical normalization would diverge from the true filesystem target.
pub(crate) fn path_traverses_symlink_parent(path: &Path) -> bool {
    let mut current = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::Normal(seg) => {
                current.push(seg);
            }
            std::path::Component::ParentDir => {
                if let Ok(meta) = fs::symlink_metadata(&current) {
                    if meta.file_type().is_symlink() {
                        return true;
                    }
                }
                current.pop();
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                current.push(c);
            }
            std::path::Component::CurDir => {}
        }
    }
    false
}

/// Matches an entry path from an index shard or sidecar map against a requested file path.
/// Resolves real filesystem identity for both entry and request before any lexical collapse,
/// guaranteeing that lexical collapse never overrides known unequal canonical paths.
#[cfg(test)]
pub(crate) fn file_entry_matches(entry_str: &str, reader_origin: Option<&Path>, req_path: &Path, req_canon: Option<&Path>) -> bool {
    if entry_str.is_empty() {
        return false;
    }

    // 0. Fast literal string match before any filesystem syscalls
    if entry_str == req_path.to_str().unwrap_or("") {
        return true;
    }

    let entry_p = Path::new(entry_str);

    // 1. Resolve original uncollapsed entry path under reader's origin
    let entry_orig = if entry_p.is_absolute() {
        entry_p.to_path_buf()
    } else if let Some(orig) = reader_origin {
        orig.join(entry_p)
    } else {
        entry_p.to_path_buf()
    };

    // 1b. Fast path equality before filesystem syscalls
    if entry_orig == req_path {
        return true;
    }

    // 2. Resolve canonical identities on the filesystem before any lexical collapse
    use super::path_index_tests::oracle::cached_canonicalize;
    let entry_canon = cached_canonicalize(&entry_orig);
    let effective_req_canon = req_canon.map(Path::to_path_buf).or_else(|| req_path.is_absolute().then(|| cached_canonicalize(req_path))?);

    // 3. Authoritative filesystem identity comparison
    if let (Some(ref c_entry), Some(ref c_req)) = (&entry_canon, &effective_req_canon) {
        if c_entry == c_req {
            return true;
        }
        // When both filesystem identities are known and unequal,
        // lexical collapse MUST NOT override known unequal canonical paths.
        return false;
    }

    // 4. Literal string match (only when canonical comparison did not rule it out)
    if entry_str == req_path.to_str().unwrap_or("") {
        return true;
    }

    // 5. Lexical fallback: only allowed where it cannot reinterpret a symlink traversal
    if path_traverses_symlink_parent(&entry_orig)
        || path_traverses_symlink_parent(req_path)
        || reader_origin.is_some_and(|orig| path_traverses_symlink_parent(&orig.join(req_path)))
    {
        return false;
    }

    let norm_entry = normalize_path(&entry_orig);

    if req_path.is_absolute() {
        let norm_req = normalize_path(req_path);
        if norm_entry == norm_req {
            return true;
        }
    } else {
        // Requested path is relative (e.g. "src/lib.rs")
        let norm_req = normalize_path(req_path);
        if norm_entry.is_relative() {
            if norm_entry == norm_req {
                return true;
            }
        } else if let Some(orig) = reader_origin {
            let norm_orig = normalize_path(orig);
            if let Ok(rel) = norm_entry.strip_prefix(&norm_orig) {
                if rel == norm_req || normalize_path(rel) == norm_req {
                    return true;
                }
            }
            if let (Some(ref c_entry), Some(c_orig)) = (&entry_canon, cached_canonicalize(orig)) {
                if let Ok(rel) = c_entry.strip_prefix(&c_orig) {
                    if rel == norm_req || normalize_path(rel) == norm_req {
                        return true;
                    }
                }
            }
        }
    }
    false
}

impl WorkspaceIndex {
    /// Returns the file modification timestamp of the index shard containing the given source file.
    pub fn shard_mtime_for_file(&self, file_path: &str) -> Option<std::time::SystemTime> {
        // Fast path: query candidate readers prioritized for this file using in-memory indices
        if self.path_index.may_match_fast_path(self, file_path) {
            for idx in self.candidate_reader_indices_for_file(file_path) {
                if let Some(target_file) = self.target_file_for_reader(idx, file_path) {
                    let reader = &self.readers[idx];
                    if reader.find_file_id(target_file.as_ref()).is_some() {
                        return self.mtimes.get(idx).copied().flatten();
                    }
                    if let Some(hashes) = self.reader_file_hashes.get(idx) {
                        if hashes.contains_key(target_file.as_ref()) {
                            return self.mtimes.get(idx).copied().flatten();
                        }
                    }
                }
                if let Some(hashes) = self.reader_file_hashes.get(idx) {
                    if hashes.contains_key(file_path) {
                        return self.mtimes.get(idx).copied().flatten();
                    }
                }
            }
        }
        self.path_index.shard_mtime_slow(self, file_path)
    }

    /// Whether every compilation unit recording a source hash for `file_path` recorded the same one. A failed build of
    /// one unit (`lib test` with an error in `mod tests`) leaves that unit's data from the text before while the units
    /// that built record the new text: no single text then matches all of the file's data.
    pub fn units_agree_on_source(&self, file_path: &str) -> bool {
        let mut seen: Option<u64> = None;
        for idx in self.candidate_reader_indices_for_file(file_path) {
            let reader = &self.readers[idx];
            let target = self.target_file_for_reader(idx, file_path);
            let name = target.as_ref().map_or(file_path, |t| t.as_ref());
            let from_reader = reader.find_file_id(name).map(|fid| reader.file_source_hash(fid)).filter(|&h| h != 0);
            let h = from_reader.or_else(|| self.reader_file_hashes.get(idx).and_then(|m| m.get(name).or_else(|| m.get(file_path))).copied().filter(|&h| h != 0));
            match (h, seen) {
                (Some(h), Some(s)) if h != s => return false,
                (Some(h), None) => seen = Some(h),
                _ => {}
            }
        }
        true
    }

    /// Returns the compiled source hash (from rustc compilation provenance) for a file if available.
    pub fn source_hash_for_file(&self, file_path: &str) -> Option<u64> {
        // Fast path: query candidate readers prioritized for this file using in-memory indices
        if self.path_index.may_match_fast_path(self, file_path) {
            for idx in self.candidate_reader_indices_for_file(file_path) {
                if let Some(target_file) = self.target_file_for_reader(idx, file_path) {
                    let reader = &self.readers[idx];
                    if let Some(fid) = reader.find_file_id(target_file.as_ref()) {
                        let h = reader.file_source_hash(fid);
                        if h != 0 {
                            return Some(h);
                        }
                    }
                    if let Some(hashes) = self.reader_file_hashes.get(idx) {
                        if let Some(&h) = hashes.get(target_file.as_ref()) {
                            if h != 0 {
                                return Some(h);
                            }
                        }
                    }
                }
                if let Some(hashes) = self.reader_file_hashes.get(idx) {
                    if let Some(&h) = hashes.get(file_path) {
                        if h != 0 {
                            return Some(h);
                        }
                    }
                }
            }
        }
        self.path_index.source_hash_slow(self, file_path)
    }
}
