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
use super::manifest::VariantManifest;
use super::WorkspaceIndex;
use crate::reader::IndexReader;
use std::collections::HashMap;
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

    pub(super) fn reload_active_variant_if_switched(&mut self) -> io::Result<bool> {
        // 0. Check if active variant manifest was updated or switched
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

            if let Ok(content) = fs::read(mf) {
                let content_hash = crate::format::normalized_source_hash(&content);
                let manifest_unchanged = current_manifest_mtime == self.manifest_mtime
                    && current_manifest_len == self.manifest_len
                    && (self.manifest_ino.is_none() || current_manifest_ino == self.manifest_ino)
                    && Some(content_hash) == self.manifest_hash;

                if !manifest_unchanged {
                    self.manifest_mtime = current_manifest_mtime;
                    self.manifest_len = current_manifest_len;
                    self.manifest_ino = current_manifest_ino;
                    self.manifest_hash = Some(content_hash);
                    if let Ok(manifest) = serde_json::from_slice::<VariantManifest>(&content) {
                        self.manifest_generation = Some(manifest.generation);
                        if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!(
                                    "Format version mismatch in reloaded manifest {:?}: format_version {} != current {}",
                                    mf, manifest.format_version, crate::format::CURRENT_FORMAT_VERSION
                                ),
                            ));
                        }
                        if manifest.extractor_version.as_deref()
                            != Some(crate::format::EXTRACTOR_VERSION)
                        {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!(
                                    "Extractor version mismatch in reloaded manifest {:?}: extractor_version {:?} != current {:?}",
                                    mf, manifest.extractor_version, crate::format::EXTRACTOR_VERSION
                                ),
                            ));
                        }

                        let base_dir = mf.parent().unwrap_or_else(|| Path::new("."));
                        if Some(&manifest.active_variant_id) != self.active_variant_id.as_ref() {
                            let expected_variant_dir =
                                format!("variants/{}", manifest.active_variant_id);
                            let vdir_str = manifest.variant_dir.trim();
                            if !manifest.active_variant_id.is_empty()
                                && vdir_str == expected_variant_dir
                            {
                                let new_vdir = base_dir.join(&expected_variant_dir);
                                if new_vdir != base_dir && new_vdir.is_dir() {
                                    // Active variant changed! Atomically switch reader set
                                    let mut new_files = Vec::new();
                                    collect_ridx_files(&new_vdir, &mut new_files)?;
                                    new_files.sort();

                                    let mut new_readers = Vec::with_capacity(new_files.len());
                                    let mut new_paths = Vec::with_capacity(new_files.len());
                                    let mut new_mtimes = Vec::with_capacity(new_files.len());
                                    let mut new_source_roots = Vec::with_capacity(new_files.len());
                                    let mut new_unit_keys = Vec::with_capacity(new_files.len());
                                    let mut new_target_kinds = Vec::with_capacity(new_files.len());
                                    let mut new_reader_generations =
                                        Vec::with_capacity(new_files.len());
                                    let mut new_reader_dependencies =
                                        Vec::with_capacity(new_files.len());
                                    let mut new_reader_extern_units =
                                        Vec::with_capacity(new_files.len());
                                    let mut new_reader_file_hashes =
                                        Vec::with_capacity(new_files.len());
                                    let mut new_reader_is_dependency =
                                        Vec::with_capacity(new_files.len());
                                    let mut new_meta_paths = Vec::with_capacity(new_files.len());
                                    let mut new_meta_mtimes = Vec::with_capacity(new_files.len());

                                    for fp in new_files {
                                        if let Ok(r) = IndexReader::open(&fp) {
                                            let mt =
                                                fs::metadata(&fp).and_then(|m| m.modified()).ok();
                                            let embedded = r.embedded_file_hashes();
                                            let meta = Self::load_file_metadata(
                                                &fp,
                                                &new_vdir,
                                                r.crate_name(),
                                                r.unit_key(),
                                                r.generation(),
                                                &embedded,
                                            );
                                            let is_dep = meta.source_root.is_some();
                                            new_readers.push(r);
                                            new_paths.push(fp);
                                            new_mtimes.push(mt);
                                            new_source_roots.push(meta.source_root);
                                            new_unit_keys.push(meta.unit_key);
                                            new_target_kinds.push(meta.target_kind);
                                            new_reader_generations.push(meta.generation);
                                            new_reader_dependencies.push(meta.dependencies);
                                            new_reader_extern_units.push(meta.extern_units);
                                            new_reader_file_hashes.push(meta.file_hashes);
                                            new_reader_is_dependency.push(is_dep);
                                            new_meta_paths.push(meta.meta_path);
                                            new_meta_mtimes.push(meta.meta_mtime);
                                        }
                                    }

                                    // Load dependency shards for new variant if any
                                    let cache_dir = crate::format::resolve_cache_dir();
                                    let mut cache_resolver =
                                        super::cache_resolver::CacheObjectResolver::new(cache_dir);
                                    for dep in &manifest.dependency_shards {
                                        let cand = cache_resolver.resolve(dep);
                                        if let Some(dep_file) = cand {
                                            let canon_cand = fs::canonicalize(&dep_file)
                                                .unwrap_or_else(|_| dep_file.clone());
                                            let existing_pos = new_paths
                                                .iter()
                                                .position(|existing| {
                                                    fs::canonicalize(existing)
                                                        .unwrap_or_else(|_| existing.clone())
                                                        == canon_cand
                                                })
                                                .or_else(|| {
                                                    new_unit_keys.iter().position(|existing_uk| {
                                                        existing_uk.as_deref()
                                                            == Some(&dep.unit_key)
                                                    })
                                                });
                                            if let Some(idx) = existing_pos {
                                                new_reader_is_dependency[idx] = true;
                                                if new_source_roots[idx].is_none() {
                                                    new_source_roots[idx] =
                                                        dep.source_root.as_ref().map(PathBuf::from);
                                                }
                                                if new_target_kinds[idx].is_none()
                                                    && dep.target_kind.is_some()
                                                {
                                                    new_target_kinds[idx] = dep.target_kind;
                                                }
                                                if new_reader_generations[idx].is_none() {
                                                    new_reader_generations[idx] =
                                                        dep.generation.clone();
                                                }
                                                if new_reader_dependencies[idx].is_empty()
                                                    && !dep.dependencies.is_empty()
                                                {
                                                    new_reader_dependencies[idx] =
                                                        dep.dependencies.clone();
                                                }
                                                if new_reader_extern_units[idx].is_empty()
                                                    && !dep.extern_units.is_empty()
                                                {
                                                    new_reader_extern_units[idx] =
                                                        dep.extern_units.clone();
                                                }
                                                if !dep.file_hashes.is_empty() {
                                                    let embedded =
                                                        new_readers[idx].embedded_file_hashes();
                                                    let mut combined = dep.file_hashes.clone();
                                                    let sroot_path =
                                                        dep.source_root.as_deref().map(Path::new);
                                                    crate::format::merge_embedded_hashes_authoritative(
                                                        &mut combined,
                                                        &embedded,
                                                        sroot_path,
                                                    );
                                                    new_reader_file_hashes[idx] = combined;
                                                }
                                                continue;
                                            }

                                            if let Ok(r) = IndexReader::open(&dep_file) {
                                                if !dep.name.is_empty()
                                                    && r.crate_name() != dep.name
                                                {
                                                    continue;
                                                }
                                                if !dep.version.is_empty()
                                                    && !r.crate_version().is_empty()
                                                    && r.crate_version() != dep.version
                                                {
                                                    continue;
                                                }
                                                if !dep.unit_key.is_empty()
                                                    && !r.unit_key().is_empty()
                                                    && dep.unit_key != r.unit_key()
                                                {
                                                    let dep_gen =
                                                        dep.generation.as_deref().unwrap_or("");
                                                    let gen_matches = if !dep_gen.is_empty() {
                                                        dep_gen == r.generation()
                                                    } else if !dep.file_hashes.is_empty()
                                                        && !r.generation().is_empty()
                                                    {
                                                        let computed_digest =
                                                            crate::format::compute_source_snapshot_digest(
                                                                &dep.file_hashes,
                                                            );
                                                        let computed_gen =
                                                            format!("{:016x}", computed_digest);
                                                        computed_gen == r.generation()
                                                    } else {
                                                        false
                                                    };
                                                    if !gen_matches {
                                                        continue;
                                                    }
                                                }
                                                let mt = fs::metadata(&dep_file)
                                                    .and_then(|m| m.modified())
                                                    .ok();
                                                let embedded = r.embedded_file_hashes();
                                                let reader_gen = r.generation();
                                                let mut gen_valid = true;
                                                if !reader_gen.is_empty() {
                                                    let dep_gen =
                                                        dep.generation.as_deref().unwrap_or("");
                                                    if !dep_gen.is_empty() {
                                                        if dep_gen != reader_gen {
                                                            gen_valid = false;
                                                        }
                                                    } else if !dep.file_hashes.is_empty() {
                                                        let computed_digest = crate::format::compute_source_snapshot_digest(
                                                            &dep.file_hashes,
                                                        );
                                                        let computed_gen =
                                                            format!("{:016x}", computed_digest);
                                                        if computed_gen != reader_gen {
                                                            gen_valid = false;
                                                        }
                                                    }
                                                }
                                                let gen = if !reader_gen.is_empty() {
                                                    Some(reader_gen.to_string())
                                                } else if gen_valid {
                                                    dep.generation.clone()
                                                } else {
                                                    None
                                                };
                                                new_readers.push(r);
                                                new_paths.push(dep_file);
                                                new_mtimes.push(mt);
                                                new_source_roots.push(
                                                    dep.source_root.as_ref().map(PathBuf::from),
                                                );
                                                new_unit_keys.push(Some(dep.unit_key.clone()));
                                                new_target_kinds.push(dep.target_kind);
                                                new_reader_generations.push(gen);
                                                let deps = if gen_valid {
                                                    dep.dependencies.clone()
                                                } else {
                                                    Vec::new()
                                                };
                                                let ext_units = if gen_valid {
                                                    dep.extern_units.clone()
                                                } else {
                                                    HashMap::new()
                                                };
                                                new_reader_dependencies.push(deps);
                                                new_reader_extern_units.push(ext_units);
                                                let mut combined_hashes = if gen_valid {
                                                    dep.file_hashes.clone()
                                                } else {
                                                    HashMap::new()
                                                };
                                                let sroot_path =
                                                    dep.source_root.as_deref().map(Path::new);
                                                crate::format::merge_embedded_hashes_authoritative(
                                                    &mut combined_hashes,
                                                    &embedded,
                                                    sroot_path,
                                                );
                                                new_reader_file_hashes.push(combined_hashes);
                                                new_reader_is_dependency.push(true);
                                                new_meta_paths.push(None);
                                                new_meta_mtimes.push(None);
                                            }
                                        }
                                    }

                                    if !new_readers.is_empty() {
                                        self.readers = new_readers;
                                        self.paths = new_paths;
                                        self.mtimes = new_mtimes;
                                        self.source_roots = new_source_roots;
                                        self.unit_keys = new_unit_keys;
                                        self.target_kinds = new_target_kinds;
                                        self.reader_generations = new_reader_generations;
                                        self.reader_dependencies = new_reader_dependencies;
                                        self.reader_extern_units = new_reader_extern_units;
                                        self.reader_file_hashes = new_reader_file_hashes;
                                        self.reader_is_dependency = new_reader_is_dependency;
                                        self.meta_paths = new_meta_paths;
                                        self.meta_mtimes = new_meta_mtimes;
                                        self.root_dir = Some(new_vdir);
                                        self.active_variant_id = Some(manifest.active_variant_id);
                                        for idx in 0..self.readers.len() {
                                            self.record_reloaded_reader_files(idx);
                                        }
                                        self.rebuild_unit_match();
                                        self.rebuild_path_index();
                                        return Ok(true);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(false)
    }

    /// Checks timestamps of indexed files and reloads any that have changed.
    /// Also scans directory for newly generated .ridx files and removes deleted ones.
    /// If an active variant manifest exists and was switched, atomically reloads the new variant's reader set.
    /// Returns true if any index was updated, added, removed, or switched.
    pub fn reload_if_modified(&mut self) -> io::Result<bool> {
        let mut changed = false;
        self.last_reloaded_files.clear();

        // 0. Check if active variant manifest was updated or switched
        if self.reload_active_variant_if_switched()? {
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
            let current_meta_mtime = self
                .meta_paths
                .get(i)
                .and_then(|p| p.as_ref())
                .and_then(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
            let index_changed = current_mtime != self.mtimes[i];
            let meta_changed = current_meta_mtime != self.meta_mtimes.get(i).copied().flatten();

            if index_changed {
                if let Ok(new_reader) = IndexReader::open(&p) {
                    self.readers[i] = new_reader;
                    self.mtimes[i] = current_mtime;
                    let meta_dir = self
                        .root_dir
                        .clone()
                        .or_else(|| p.parent().map(PathBuf::from));
                    if let Some(dir) = meta_dir {
                        let embedded = self.readers[i].embedded_file_hashes();
                        let meta = Self::load_file_metadata(
                            &p,
                            &dir,
                            self.readers[i].crate_name(),
                            self.readers[i].unit_key(),
                            self.readers[i].generation(),
                            &embedded,
                        );
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
                let meta_dir = self
                    .root_dir
                    .clone()
                    .or_else(|| p.parent().map(PathBuf::from));
                if let Some(dir) = meta_dir {
                    let embedded = self.readers[i].embedded_file_hashes();
                    let meta = Self::load_file_metadata(
                        &p,
                        &dir,
                        self.readers[i].crate_name(),
                        self.readers[i].unit_key(),
                        self.readers[i].generation(),
                        &embedded,
                    );
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
            let existing_set: std::collections::HashSet<PathBuf> = self
                .paths
                .iter()
                .map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
                .collect();

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
                        let meta = Self::load_file_metadata(
                            &file_path,
                            &dir,
                            reader.crate_name(),
                            reader.unit_key(),
                            reader.generation(),
                            &embedded,
                        );
                        if let Some(ref uk) = meta.unit_key {
                            if self
                                .unit_keys
                                .iter()
                                .any(|existing_uk| existing_uk.as_deref() == Some(uk))
                            {
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
            if self
                .last_reloaded_files
                .contains(&canon.to_string_lossy().to_string())
            {
                return true;
            }
        }
        self.last_reloaded_files
            .iter()
            .any(|rf| rf == file_path || rf.ends_with(file_path) || file_path.ends_with(rf))
    }

    fn record_reloaded_reader_files(&mut self, idx: usize) {
        if let Some(reader) = self.readers.get(idx) {
            let source_root = self.source_roots.get(idx).and_then(|r| r.as_ref());
            let header = reader.header();
            for fid in 0..header.files_count {
                let f = reader.get_file(fid);
                if !f.is_empty() {
                    let full = if let Some(root) = source_root {
                        crate::format::join_resolving_overlap(root, Path::new(f))
                    } else {
                        PathBuf::from(f)
                    };
                    self.last_reloaded_files
                        .insert(full.to_string_lossy().to_string());
                    if let Ok(canon) = full.canonicalize() {
                        self.last_reloaded_files
                            .insert(canon.to_string_lossy().to_string());
                    }
                    self.last_reloaded_files.insert(f.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
