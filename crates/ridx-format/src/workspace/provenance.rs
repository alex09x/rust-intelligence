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

    /// The reader's root as routing resolves it: a relative recorded root (`source_root: "shop"`) lies under the workspace
    /// root, never under the process's working directory (review of bbed0a7e).
    fn rooted_reader_root(&self, idx: usize) -> Option<PathBuf> {
        let root = self.reader_root(idx)?;
        Some(match self.workspace_root.as_deref() {
            Some(ws) if root.is_relative() => ws.join(root),
            _ => root.to_path_buf(),
        })
    }

    /// The names the sidecar of reader `idx` may record one file under: its path as given, its canonical path, and its
    /// path relative to the reader's root, then to the workspace root (from the given and the canonical spellings of
    /// both), with that relative path joined back onto the root as it is spelled. All of them name `file` only: never a
    /// union of alternative resolutions, which may be other files (review of c94616ac).
    fn names_of_one_file(&self, idx: usize, file: &Path) -> Vec<String> {
        let canonical = crate::canonicalize_best_effort(file);
        let mut names = vec![file.display().to_string(), canonical.display().to_string()];
        for root in [self.rooted_reader_root(idx), self.workspace_root.clone()].into_iter().flatten() {
            let canonical_root = crate::canonicalize_best_effort(&root);
            for rel in [file.strip_prefix(&root).ok(), canonical.strip_prefix(&canonical_root).ok()].into_iter().flatten() {
                names.push(rel.display().to_string());
                names.push(root.join(rel).display().to_string());
            }
        }
        let mut seen = std::collections::HashSet::new();
        names.retain(|n| seen.insert(n.clone()));
        names
    }

    /// The one file `query` names for reader `idx`, which path routing stores as `key` (`None`: not routed). An absolute
    /// query is that file as spelled, whatever form routing stores it in (relative to the workspace or to the reader's
    /// root: review of 4d8906dc). A relative query is the workspace's, else the reader root's: the first of them whose
    /// file routing stored as `key`, as spelled or, as routing canonicalizes a candidate, through its canonical path
    /// against the canonical roots (`W/shop/alias -> W/shop/src`: `alias/gen.rs` is `src/gen.rs` of `W/shop`, review of
    /// a0634347); an absolute `key` is that file itself. The file keeps the query's spelling.
    fn file_of_query(&self, idx: usize, query: &str, key: Option<&str>) -> PathBuf {
        let q = Path::new(query);
        if q.is_absolute() {
            return q.to_path_buf();
        }
        if let Some(k) = key.filter(|k| Path::new(k).is_absolute()) {
            return PathBuf::from(k);
        }
        let anchors: Vec<PathBuf> = [self.workspace_root.clone(), self.rooted_reader_root(idx)].into_iter().flatten().collect();
        let files: Vec<PathBuf> = anchors.iter().map(|a| a.join(q)).collect();
        let Some(k) = key else { return files.first().cloned().unwrap_or_else(|| q.to_path_buf()) };
        let is_key = |file: &Path, roots: &[PathBuf]| roots.iter().any(|a| file.strip_prefix(a).ok().and_then(|r| r.to_str()) == Some(k));
        if let Some(f) = files.iter().find(|f| is_key(f, &anchors)) {
            return f.clone();
        }
        let canonical: Vec<PathBuf> = anchors.iter().map(|a| crate::canonicalize_best_effort(a)).collect();
        files.iter().find(|f| is_key(&crate::canonicalize_best_effort(f), &canonical)).or(files.first()).cloned().unwrap_or_else(|| q.to_path_buf())
    }

    /// The hash the sidecar of reader `idx` recorded for the file `query` names (routing stores it as `key`) that has no
    /// entry in its shard, unless the unit only read the file through a dependency: the sidecar loaded with the reader
    /// lists its own sources (`own_sources`) and the file is not among them. Both come from the same bytes, kept in
    /// memory: a sidecar rewritten on disk since never changes the answer (review of 2f7a84e3). The hash and the
    /// ownership are looked up under the names of that one file: the query as spelled, the stored key, and the
    /// absolute, canonical and relative names of the file. Without `own_sources` the unit counts as holding the file: a
    /// doubt is a disagreement, never a coverage (review of 9f9c7bba).
    fn sidecar_only_hash(&self, idx: usize, query: &str, key: Option<&str>) -> Option<u64> {
        let hashes = self.reader_file_hashes.get(idx)?;
        let file = self.file_of_query(idx, query, key);
        let mut names = self.names_of_one_file(idx, &file);
        names.extend(key.filter(|k| !names.iter().any(|n| n == k)).map(str::to_string));
        let h = names.iter().find_map(|n| hashes.get(n.as_str())).copied().filter(|&h| h != 0)?;
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
                Some(fid) => Some(reader.file_source_hash(fid)).filter(|&h| h != 0).or_else(|| self.sidecar_only_hash(idx, file_path, target.as_deref())),
                None => self.sidecar_only_hash(idx, file_path, target.as_deref()),
            };
            match (h, seen) {
                (Some(h), Some(s)) if h != s => return false,
                (Some(h), None) => seen = Some(h),
                _ => {}
            }
        }
        true
    }

    /// Every file of reader `idx`'s loaded source snapshot (the sidecar's, a non-Rust `include!` input too) with the
    /// hash the compiler read it with, as the snapshot spells its path.
    pub fn reader_source_snapshot(&self, idx: usize) -> impl Iterator<Item = (&str, u64)> {
        self.reader_file_hashes.get(idx).into_iter().flatten().map(|(f, &h)| (f.as_str(), h))
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
