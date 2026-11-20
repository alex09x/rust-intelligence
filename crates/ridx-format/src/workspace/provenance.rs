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

    /// Records the `own_sources` of the sidecar just loaded for the shard at `path`, or forgets them when it has none.
    pub(crate) fn set_own_sources(&mut self, path: &Path, own: Option<std::collections::HashSet<String>>) {
        match own {
            Some(own) => self.own_sources.insert(path.to_path_buf(), own),
            None => self.own_sources.remove(path),
        };
    }

    /// The hash the sidecar of reader `idx` recorded for the file `key` (as the reader names it; `query` as asked) that
    /// has no entry in its shard, unless the unit only read the file through a dependency: the sidecar loaded with the
    /// reader lists its own sources (`own_sources`) and the file is not among them. Both come from the same bytes,
    /// kept in memory: a sidecar rewritten on disk since never changes the answer (review of 2f7a84e3). A relative key
    /// resolves against the reader's own root, as path routing does, else the workspace root. Without `own_sources`
    /// the unit counts as holding the file: a doubt is a disagreement, never a coverage (review of 9f9c7bba).
    fn sidecar_only_hash(&self, idx: usize, key: &str, query: &str) -> Option<u64> {
        let hashes = self.reader_file_hashes.get(idx)?;
        let mut candidates: Vec<PathBuf> = Vec::new();
        for name in [key, query] {
            let path = Path::new(name);
            if path.is_absolute() {
                candidates.push(path.to_path_buf());
            } else {
                candidates.extend(self.reader_root(idx).map(|r| r.join(path)));
                candidates.extend(self.workspace_root.as_ref().map(|r| r.join(path)));
            }
        }
        let mut names: Vec<String> = candidates.iter().flat_map(|c| [c.display().to_string(), crate::canonicalize_best_effort(c).display().to_string()]).collect();
        // The sidecar names the reader's files relative to its root.
        if let Some(root) = self.reader_root(idx) {
            let relative: Vec<String> = candidates.iter().filter_map(|c| c.strip_prefix(root).ok()).map(|r| r.display().to_string()).collect();
            names.extend(relative);
        }
        let h = std::iter::once(key).chain(names.iter().map(String::as_str)).find_map(|n| hashes.get(n)).copied().filter(|&h| h != 0)?;
        let not_own = self.paths.get(idx).and_then(|p| self.own_sources.get(p)).is_some_and(|own| !names.iter().any(|n| own.contains(n)));
        (!not_own).then_some(h)
    }

    /// Whether every compilation unit recording a source hash for `file_path` recorded the same one. A failed build of
    /// one unit (`lib test` with an error in `mod tests`) leaves that unit's data from the text before while the units
    /// that built record the new text: no single text then matches all of the file's data.
    /// Every unit, not the candidates navigation ranks (those prefer the innermost source root): an integration test
    /// of an outer package including the file with `#[path]` holds it too (review of 503071cb). Dependencies hold no
    /// workspace file.
    pub fn units_agree_on_source(&self, file_path: &str) -> bool {
        let mut seen: Option<u64> = None;
        for idx in (0..self.readers.len()).filter(|&i| !self.reader_is_dependency.get(i).copied().unwrap_or(false)) {
            // Only the units the file belongs to: another crate's `src/lib.rs` is another file.
            // A file with an entry in the reader's shard was compiled by it; one with only a hash in its sidecar (brought
            // in with `include!`, or read through a dependency's metadata) is weighed by `sidecar_only_hash`, whether
            // asked for by an absolute or a relative path (reviews of 3650f8e3 and 9f9c7bba).
            let target = self.target_file_for_reader(idx, file_path);
            let key = target.as_deref().unwrap_or(file_path);
            let reader = &self.readers[idx];
            let h = match reader.find_file_id(key) {
                Some(fid) => Some(reader.file_source_hash(fid)).filter(|&h| h != 0).or_else(|| self.sidecar_only_hash(idx, key, file_path)),
                None => self.sidecar_only_hash(idx, key, file_path),
            };
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
