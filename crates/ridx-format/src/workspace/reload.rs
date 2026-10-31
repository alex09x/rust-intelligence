/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Incremental reload, modification detection, and variant reconciliation.

use super::collect_ridx_files;
use super::WorkspaceIndex;
use crate::reader::IndexReader;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

impl WorkspaceIndex {
    /// Fast sub-microsecond check: verifies if ridx_manifest.json mtime/size/hash has changed.
    /// If manifest is unchanged, returns Ok(false) immediately with zero shard filesystem syscalls.
    /// If manifest was updated or switched, triggers full shard reload and returns Ok(true).
    pub fn reload_manifest_if_modified(&mut self) -> io::Result<bool> {
        if self.manifest_path.is_none() {
            if let Some(ref dir) = self.root_dir {
                let cand = dir.join("ridx_manifest.json");
                if cand.is_file() {
                    self.manifest_path = Some(cand);
                }
            }
        }

        if let Some(ref mf) = self.manifest_path {
            let meta = fs::metadata(mf).ok();
            let current_manifest_mtime = meta.as_ref().and_then(|m| m.modified().ok());
            let current_manifest_len = meta.as_ref().map(|m| m.len());
            #[cfg(unix)]
            let current_manifest_ino = meta.as_ref().map(|m| {
                use std::os::unix::fs::MetadataExt;
                m.ino()
            });
            #[cfg(not(unix))]
            let current_manifest_ino: Option<u64> = None;

            if current_manifest_mtime == self.manifest_mtime
                && current_manifest_len == self.manifest_len
                && (self.manifest_ino.is_none() || current_manifest_ino == self.manifest_ino)
            {
                return Ok(false);
            }
            // Manifest changed: run full reload and shard reconciliation
            return self.reload_if_modified();
        }

        self.reload_if_modified()
    }

    /// Checks timestamps of indexed files and reloads any that have changed.
    /// Also scans directory for newly generated .ridx files and removes deleted ones.
    /// If an active variant manifest exists and was switched, atomically reloads the new variant's reader set.
    /// Returns true if any index was updated, added, removed, or switched.
    pub fn reload_if_modified(&mut self) -> io::Result<bool> {
        self.reload_if_modified_with_cache_dir(crate::format::resolve_cache_dir())
    }

    /// Checks timestamps of indexed files and reloads any that have changed, using an explicit cache directory.
    pub fn reload_if_modified_with_cache_dir(&mut self, cache_dir: impl AsRef<Path>) -> io::Result<bool> {
        let mut changed = false;
        self.last_reloaded_files.clear();

        // 0. Check if active variant manifest was updated or switched
        if self.reload_active_variant_if_switched(cache_dir.as_ref().to_path_buf())? {
            return Ok(true);
        }

        // 1. Reconcile existing files: remove deleted, reload modified
        let mut i = 0;
        while i < self.paths.len() {
            let p = self.paths[i].clone();
            if !p.exists() {
                self.paths.remove(i);
                self.readers.remove(i);
                self.mtimes.remove(i);
                if i < self.source_roots.len() {
                    self.source_roots.remove(i);
                }
                if i < self.unit_keys.len() {
                    self.unit_keys.remove(i);
                }
                if i < self.target_kinds.len() {
                    self.target_kinds.remove(i);
                }
                if i < self.reader_generations.len() {
                    self.reader_generations.remove(i);
                }
                if i < self.reader_dependencies.len() {
                    self.reader_dependencies.remove(i);
                }
                if i < self.reader_extern_units.len() {
                    self.reader_extern_units.remove(i);
                }
                if i < self.reader_file_hashes.len() {
                    self.reader_file_hashes.remove(i);
                }
                if i < self.reader_is_dependency.len() {
                    self.reader_is_dependency.remove(i);
                }
                if i < self.meta_paths.len() {
                    self.meta_paths.remove(i);
                }
                if i < self.meta_mtimes.len() {
                    self.meta_mtimes.remove(i);
                }
                changed = true;
                continue;
            }

            let current_mtime = fs::metadata(&p).and_then(|m| m.modified()).ok();
            let current_meta_mtime = self.meta_paths.get(i).and_then(|p| p.as_ref()).and_then(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
            let index_changed = current_mtime != self.mtimes[i];
            let meta_changed = current_meta_mtime != self.meta_mtimes.get(i).copied().flatten();

            if index_changed {
                if let Ok(new_reader) = IndexReader::open(&p) {
                    self.readers[i] = new_reader;
                    self.mtimes[i] = current_mtime;
                    let meta_dir = self.root_dir.clone().or_else(|| p.parent().map(PathBuf::from));
                    if let Some(dir) = meta_dir {
                        let embedded = self.readers[i].embedded_file_hashes();
                        let meta = Self::load_file_metadata(&p, &dir, self.readers[i].crate_name(), self.readers[i].unit_key(), self.readers[i].generation(), &embedded);
                        if let Some(sr) = self.source_roots.get_mut(i) {
                            *sr = meta.source_root;
                        }
                        if let Some(uk) = self.unit_keys.get_mut(i) {
                            *uk = meta.unit_key;
                        }
                        if let Some(tk) = self.target_kinds.get_mut(i) {
                            *tk = meta.target_kind;
                        }
                        if let Some(g) = self.reader_generations.get_mut(i) {
                            *g = meta.generation;
                        }
                        if let Some(dp) = self.reader_dependencies.get_mut(i) {
                            *dp = meta.dependencies;
                        }
                        if let Some(eu) = self.reader_extern_units.get_mut(i) {
                            *eu = meta.extern_units;
                        }
                        if let Some(fh) = self.reader_file_hashes.get_mut(i) {
                            *fh = meta.file_hashes;
                        }
                        if let Some(mp) = self.meta_paths.get_mut(i) {
                            *mp = meta.meta_path;
                        }
                        if let Some(mm) = self.meta_mtimes.get_mut(i) {
                            *mm = meta.meta_mtime;
                        }
                    }
                    self.record_reloaded_reader_files(i);
                    changed = true;
                }
            } else if meta_changed {
                let meta_dir = self.root_dir.clone().or_else(|| p.parent().map(PathBuf::from));
                if let Some(dir) = meta_dir {
                    let embedded = self.readers[i].embedded_file_hashes();
                    let meta = Self::load_file_metadata(&p, &dir, self.readers[i].crate_name(), self.readers[i].unit_key(), self.readers[i].generation(), &embedded);
                    if let Some(sr) = self.source_roots.get_mut(i) {
                        *sr = meta.source_root;
                    }
                    if let Some(uk) = self.unit_keys.get_mut(i) {
                        *uk = meta.unit_key;
                    }
                    if let Some(tk) = self.target_kinds.get_mut(i) {
                        *tk = meta.target_kind;
                    }
                    if let Some(g) = self.reader_generations.get_mut(i) {
                        *g = meta.generation;
                    }
                    if let Some(dp) = self.reader_dependencies.get_mut(i) {
                        *dp = meta.dependencies;
                    }
                    if let Some(eu) = self.reader_extern_units.get_mut(i) {
                        *eu = meta.extern_units;
                    }
                    if let Some(fh) = self.reader_file_hashes.get_mut(i) {
                        *fh = meta.file_hashes;
                    }
                    if let Some(mp) = self.meta_paths.get_mut(i) {
                        *mp = meta.meta_path;
                    }
                    if let Some(mm) = self.meta_mtimes.get_mut(i) {
                        *mm = meta.meta_mtime;
                    }
                    self.record_reloaded_reader_files(i);
                    changed = true;
                }
            }
            i += 1;
        }

        // 2. If loaded from directory, check for new .ridx files
        if let Some(dir) = self.root_dir.clone() {
            let existing_set: std::collections::HashSet<PathBuf> = self.paths.iter().map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.clone())).collect();

            let mut all_files = Vec::new();
            if dir.is_dir() {
                collect_ridx_files(&dir, &mut all_files)?;
            }

            for file_path in all_files {
                let canon_fp = fs::canonicalize(&file_path).unwrap_or_else(|_| file_path.clone());
                if !existing_set.contains(&canon_fp) {
                    if let Ok(reader) = IndexReader::open(&file_path) {
                        let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                        let embedded = reader.embedded_file_hashes();
                        let meta = Self::load_file_metadata(&file_path, &dir, reader.crate_name(), reader.unit_key(), reader.generation(), &embedded);
                        if let Some(ref uk) = meta.unit_key {
                            if self.unit_keys.iter().any(|existing_uk| existing_uk.as_deref() == Some(uk)) {
                                continue;
                            }
                        }
                        let is_dep = meta.source_root.is_some();
                        self.readers.push(reader);
                        self.paths.push(file_path);
                        self.mtimes.push(mtime);
                        self.source_roots.push(meta.source_root);
                        self.unit_keys.push(meta.unit_key);
                        self.target_kinds.push(meta.target_kind);
                        self.reader_generations.push(meta.generation);
                        self.reader_dependencies.push(meta.dependencies);
                        self.reader_extern_units.push(meta.extern_units);
                        self.reader_file_hashes.push(meta.file_hashes);
                        self.reader_is_dependency.push(is_dep);
                        self.meta_paths.push(meta.meta_path);
                        self.meta_mtimes.push(meta.meta_mtime);
                        let new_idx = self.readers.len() - 1;
                        self.record_reloaded_reader_files(new_idx);
                        changed = true;
                    }
                }
            }
        }

        if changed {
            self.rebuild_unit_match();
            self.rebuild_path_index();
        }
        Ok(changed)
    }

    /// Checks if a file path belongs to any shard that was newly reloaded or published in the last reload.
    pub fn is_file_reloaded(&self, file_path: &str) -> bool {
        if self.last_reloaded_files.contains(file_path) {
            return true;
        }
        let p = Path::new(file_path);
        if let Ok(canon) = p.canonicalize() {
            if self.last_reloaded_files.contains(&canon.to_string_lossy().to_string()) {
                return true;
            }
        }
        self.last_reloaded_files.iter().any(|rf| rf == file_path || rf.ends_with(file_path) || file_path.ends_with(rf))
    }

    pub(super) fn record_reloaded_reader_files(&mut self, idx: usize) {
        if let Some(reader) = self.readers.get(idx) {
            let source_root = self.source_roots.get(idx).and_then(|r| r.as_ref());
            let header = reader.header();
            for fid in 0..header.files_count {
                let f = reader.get_file(fid);
                if !f.is_empty() {
                    let full = if let Some(root) = source_root { crate::format::join_resolving_overlap(root, Path::new(f)) } else { PathBuf::from(f) };
                    self.last_reloaded_files.insert(full.to_string_lossy().to_string());
                    if let Ok(canon) = full.canonicalize() {
                        self.last_reloaded_files.insert(canon.to_string_lossy().to_string());
                    }
                    self.last_reloaded_files.insert(f.to_string());
                }
            }
        }
    }
}

mod variant;

#[cfg(test)]
mod tests;
