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
    /// Checks timestamps of indexed files and reloads any that have changed.
    /// Also scans directory for newly generated .ridx files and removes deleted ones.
    /// If an active variant manifest exists and was switched, atomically reloads the new variant's reader set.
    /// Returns true if any index was updated, added, removed, or switched.
    pub fn reload_if_modified(&mut self) -> io::Result<bool> {
        let mut changed = false;
        self.last_reloaded_files.clear();

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
            let current_manifest_mtime = fs::metadata(mf).and_then(|m| m.modified()).ok();
            if current_manifest_mtime != self.manifest_mtime {
                self.manifest_mtime = current_manifest_mtime;
                if let Ok(content) = fs::read(mf) {
                    if let Ok(manifest) = serde_json::from_slice::<VariantManifest>(&content) {
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
                                            let (
                                                sroot,
                                                ukey,
                                                gen,
                                                deps,
                                                ext_units,
                                                file_hashes,
                                                mpath,
                                                mmtime,
                                            ) = Self::load_file_metadata(
                                                &fp,
                                                &new_vdir,
                                                r.crate_name(),
                                                r.unit_key(),
                                                r.generation(),
                                                &embedded,
                                            );
                                            let is_dep = sroot.is_some();
                                            new_readers.push(r);
                                            new_paths.push(fp);
                                            new_mtimes.push(mt);
                                            new_source_roots.push(sroot);
                                            new_unit_keys.push(ukey);
                                            new_reader_generations.push(gen);
                                            new_reader_dependencies.push(deps);
                                            new_reader_extern_units.push(ext_units);
                                            new_reader_file_hashes.push(file_hashes);
                                            new_reader_is_dependency.push(is_dep);
                                            new_meta_paths.push(mpath);
                                            new_meta_mtimes.push(mmtime);
                                        }
                                    }

                                    // Load dependency shards for new variant if any
                                    let cache_dir = crate::format::resolve_cache_dir();
                                    for dep in &manifest.dependency_shards {
                                        let obj_path = PathBuf::from(&dep.object_path);
                                        let cand = if obj_path.is_file() {
                                            Some(obj_path)
                                        } else {
                                            let fallback = crate::format::cache_object_path(
                                                &cache_dir,
                                                &dep.unit_key,
                                            );
                                            if fallback.is_file() {
                                                Some(fallback)
                                            } else {
                                                None
                                            }
                                        };
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
                                                if !dep.unit_key.is_empty()
                                                    && !r.unit_key().is_empty()
                                                    && dep.unit_key != r.unit_key()
                                                {
                                                    continue;
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
                                        return Ok(true);
                                    }
                                }
                            }
                        }
                    }
                }
            }
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
                        let (sroot, ukey, gen, deps, ext_units, file_hashes, mpath, mmtime) =
                            Self::load_file_metadata(
                                &p,
                                &dir,
                                self.readers[i].crate_name(),
                                self.readers[i].unit_key(),
                                self.readers[i].generation(),
                                &embedded,
                            );
                        if let Some(sr) = self.source_roots.get_mut(i) {
                            *sr = sroot;
                        }
                        if let Some(uk) = self.unit_keys.get_mut(i) {
                            *uk = ukey;
                        }
                        if let Some(g) = self.reader_generations.get_mut(i) {
                            *g = gen;
                        }
                        if let Some(dp) = self.reader_dependencies.get_mut(i) {
                            *dp = deps;
                        }
                        if let Some(eu) = self.reader_extern_units.get_mut(i) {
                            *eu = ext_units;
                        }
                        if let Some(fh) = self.reader_file_hashes.get_mut(i) {
                            *fh = file_hashes;
                        }
                        if let Some(mp) = self.meta_paths.get_mut(i) {
                            *mp = mpath;
                        }
                        if let Some(mm) = self.meta_mtimes.get_mut(i) {
                            *mm = mmtime;
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
                    let (sroot, ukey, gen, deps, ext_units, file_hashes, mpath, mmtime) =
                        Self::load_file_metadata(
                            &p,
                            &dir,
                            self.readers[i].crate_name(),
                            self.readers[i].unit_key(),
                            self.readers[i].generation(),
                            &embedded,
                        );
                    if let Some(sr) = self.source_roots.get_mut(i) {
                        *sr = sroot;
                    }
                    if let Some(uk) = self.unit_keys.get_mut(i) {
                        *uk = ukey;
                    }
                    if let Some(g) = self.reader_generations.get_mut(i) {
                        *g = gen;
                    }
                    if let Some(dp) = self.reader_dependencies.get_mut(i) {
                        *dp = deps;
                    }
                    if let Some(eu) = self.reader_extern_units.get_mut(i) {
                        *eu = ext_units;
                    }
                    if let Some(fh) = self.reader_file_hashes.get_mut(i) {
                        *fh = file_hashes;
                    }
                    if let Some(mp) = self.meta_paths.get_mut(i) {
                        *mp = mpath;
                    }
                    if let Some(mm) = self.meta_mtimes.get_mut(i) {
                        *mm = mmtime;
                    }
                    self.record_reloaded_reader_files(i);
                    changed = true;
                }
            }
            i += 1;
        }

        // 2. If loaded from directory, check for new .ridx files
        if let Some(dir) = self.root_dir.clone() {
            let mut existing_set = std::collections::HashSet::new();
            for p in &self.paths {
                existing_set.insert(p.clone());
            }

            let mut all_files = Vec::new();
            if dir.is_dir() {
                collect_ridx_files(&dir, &mut all_files)?;
            }

            for file_path in all_files {
                let canon_fp = fs::canonicalize(&file_path).unwrap_or_else(|_| file_path.clone());
                let already_present = self.paths.iter().any(|existing| {
                    fs::canonicalize(existing).unwrap_or_else(|_| existing.clone()) == canon_fp
                });
                if !existing_set.contains(&file_path) && !already_present {
                    if let Ok(reader) = IndexReader::open(&file_path) {
                        let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                        let embedded = reader.embedded_file_hashes();
                        let (sroot, ukey, gen, deps, ext_units, file_hashes, mpath, mmtime) =
                            Self::load_file_metadata(
                                &file_path,
                                &dir,
                                reader.crate_name(),
                                reader.unit_key(),
                                reader.generation(),
                                &embedded,
                            );
                        if let Some(ref uk) = ukey {
                            if self
                                .unit_keys
                                .iter()
                                .any(|existing_uk| existing_uk.as_deref() == Some(uk))
                            {
                                continue;
                            }
                        }
                        let is_dep = sroot.is_some();
                        self.readers.push(reader);
                        self.paths.push(file_path);
                        self.mtimes.push(mtime);
                        self.source_roots.push(sroot);
                        self.unit_keys.push(ukey);
                        self.reader_generations.push(gen);
                        self.reader_dependencies.push(deps);
                        self.reader_extern_units.push(ext_units);
                        self.reader_file_hashes.push(file_hashes);
                        self.reader_is_dependency.push(is_dep);
                        self.meta_paths.push(mpath);
                        self.meta_mtimes.push(mmtime);
                        let new_idx = self.readers.len() - 1;
                        self.record_reloaded_reader_files(new_idx);
                        changed = true;
                    }
                }
            }
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
mod tests {
    use super::*;
    use crate::format::DefKind;
    use crate::writer::{IndexWriter, RawDefinition};
    use std::fs::{self, File};

    struct TestDir(PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn new_test_dir(prefix: &str) -> TestDir {
        static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TestDir(path)
    }

    fn write_test_index(path: &Path, crate_name: &str) {
        let mut writer = IndexWriter::new(crate_name, "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "test_fn".to_string(),
            path: format!("{crate_name}::test_fn"),
            signature: Some("fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 2,
            end_col: 2,
        });
        let mut f = File::create(path).unwrap();
        writer.write_to(&mut f).unwrap();
    }

    #[test]
    fn test_reload_format_and_extractor_version_errors() {
        let test_dir = new_test_dir("ridx_reload_err");
        let idx = test_dir.0.join("sample.ridx");
        write_test_index(&idx, "sample");

        let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();

        let mf = test_dir.0.join("ridx_manifest.json");
        let manifest_data = serde_json::json!({
            "version": "2.0",
            "format_version": 999,
            "active_variant_id": "var1",
            "variant_dir": "variants/var1",
            "workspace_root": test_dir.0.display().to_string(),
        });
        fs::write(&mf, serde_json::to_string(&manifest_data).unwrap()).unwrap();

        let err = ws.reload_if_modified();
        assert!(err.is_err());
        assert!(err
            .unwrap_err()
            .to_string()
            .contains("Format version mismatch"));

        // Extractor version mismatch
        let manifest_data_ext = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": "non_matching_v99",
            "active_variant_id": "var1",
            "variant_dir": "variants/var1",
            "workspace_root": test_dir.0.display().to_string(),
        });
        fs::write(&mf, serde_json::to_string(&manifest_data_ext).unwrap()).unwrap();
        ws.manifest_mtime = None; // Reset cached mtime to force re-reading manifest

        let err2 = ws.reload_if_modified();
        assert!(err2.is_err());
        assert!(err2
            .unwrap_err()
            .to_string()
            .contains("Extractor version mismatch"));
    }

    #[test]
    fn test_reload_file_modifications_and_deletions() {
        let test_dir = new_test_dir("ridx_reload_mod");
        let idx1 = test_dir.0.join("crate1.ridx");
        write_test_index(&idx1, "crate1");

        let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        assert_eq!(ws.readers.len(), 1);

        // Immediate reload without changes returns false
        assert!(!ws.reload_if_modified().unwrap());

        // Overwrite file
        std::thread::sleep(std::time::Duration::from_millis(50));
        write_test_index(&idx1, "crate1");
        assert!(ws.reload_if_modified().unwrap());
        assert!(ws.is_file_reloaded("src/lib.rs"));

        // Add new file
        let idx2 = test_dir.0.join("crate2.ridx");
        write_test_index(&idx2, "crate2");
        assert!(ws.reload_if_modified().unwrap());
        assert_eq!(ws.readers.len(), 2);

        // Remove file
        fs::remove_file(&idx2).unwrap();
        assert!(ws.reload_if_modified().unwrap());
        assert_eq!(ws.readers.len(), 1);
    }

    #[test]
    fn test_reload_active_variant_switch() {
        let test_dir = new_test_dir("ridx_reload_switch");
        let v1_dir = test_dir.0.join("variants/var1");
        fs::create_dir_all(&v1_dir).unwrap();
        write_test_index(&v1_dir.join("crate1.ridx"), "crate1");

        let v2_dir = test_dir.0.join("variants/var2");
        fs::create_dir_all(&v2_dir).unwrap();
        write_test_index(&v2_dir.join("crate2.ridx"), "crate2");

        let mf = test_dir.0.join("ridx_manifest.json");
        let manifest_data = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
            "active_variant_id": "var1",
            "variant_dir": "variants/var1",
            "workspace_root": test_dir.0.display().to_string(),
        });
        fs::write(&mf, serde_json::to_string(&manifest_data).unwrap()).unwrap();

        let mut ws = WorkspaceIndex::open(&v1_dir).unwrap();
        ws.manifest_path = Some(mf.clone());
        ws.active_variant_id = Some("var1".to_string());

        // Switch to var2 with dependency shard
        let dep_file = test_dir.0.join("dep_crate.ridx");
        write_test_index(&dep_file, "dep_crate");

        std::thread::sleep(std::time::Duration::from_millis(50));
        let manifest_data2 = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
            "active_variant_id": "var2",
            "variant_dir": "variants/var2",
            "workspace_root": test_dir.0.display().to_string(),
            "dependency_shards": [{
                "name": "dep_crate",
                "version": "0.1.0",
                "unit_key": "dep_crate:0.1.0",
                "generation": "1234567812345678",
                "object_path": dep_file.display().to_string(),
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": {}
            }]
        });
        fs::write(&mf, serde_json::to_string(&manifest_data2).unwrap()).unwrap();

        let res = ws.reload_if_modified().unwrap();
        assert!(res);
        assert_eq!(ws.active_variant_id.as_deref(), Some("var2"));
        assert_eq!(ws.readers.len(), 2);
    }

    #[test]
    fn test_reload_with_metadata_sidecar() {
        let test_dir = new_test_dir("ridx_reload_sidecar");
        let idx = test_dir.0.join("crate1.ridx");
        write_test_index(&idx, "crate1");

        let deps_dir = test_dir.0.join(".deps");
        fs::create_dir_all(&deps_dir).unwrap();
        let meta_file = deps_dir.join("crate1.json");
        let meta_data = serde_json::json!({
            "name": "crate1",
            "is_dependency": false,
            "source_root": test_dir.0.display().to_string(),
            "object_path": idx.display().to_string(),
            "dependencies": ["dep_a"],
            "extern_units": ["ext_u1"]
        });
        fs::write(&meta_file, serde_json::to_string(&meta_data).unwrap()).unwrap();

        let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        assert_eq!(ws.readers.len(), 1);

        // Update sidecar metadata
        std::thread::sleep(std::time::Duration::from_millis(50));
        let meta_data2 = serde_json::json!({
            "name": "crate1",
            "is_dependency": false,
            "source_root": test_dir.0.display().to_string(),
            "object_path": idx.display().to_string(),
            "dependencies": ["dep_a", "dep_b"],
            "extern_units": ["ext_u1"]
        });
        fs::write(&meta_file, serde_json::to_string(&meta_data2).unwrap()).unwrap();

        let res = ws.reload_if_modified().unwrap();
        assert!(res);
    }

    #[test]
    fn test_reload_new_file_discovery_and_is_reloaded() {
        let test_dir = new_test_dir("ridx_reload_new_file");
        let idx1 = test_dir.0.join("crate1.ridx");
        write_test_index(&idx1, "crate1");

        let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        assert_eq!(ws.readers.len(), 1);

        // Before adding new index:
        assert!(!ws.is_file_reloaded("src/lib.rs"));

        // Add crate2.ridx to directory
        std::thread::sleep(std::time::Duration::from_millis(50));
        let idx2 = test_dir.0.join("crate2.ridx");
        write_test_index(&idx2, "crate2");

        let changed = ws.reload_if_modified().unwrap();
        assert!(changed);
        assert_eq!(ws.readers.len(), 2);
        assert!(ws.is_file_reloaded("src/lib.rs"));
    }

    #[test]
    fn test_reload_variant_dependency_dedup_and_generation_mismatch() {
        let test_dir = new_test_dir("ridx_reload_dedup");
        let v1_dir = test_dir.0.join("variants/v1");
        fs::create_dir_all(&v1_dir).unwrap();
        let idx1 = v1_dir.join("main.ridx");
        write_test_index(&idx1, "main");

        let dep_file = test_dir.0.join("dep.ridx");
        let mut w_dep = IndexWriter::new("dep", "0.1.0");
        w_dep.set_unit_key("dep_key_shared");
        w_dep.set_generation("gen_alpha");
        let mut f_dep = File::create(&dep_file).unwrap();
        w_dep.write_to(&mut f_dep).unwrap();
        drop(f_dep);

        let mf = test_dir.0.join("ridx_manifest.json");
        let manifest_v1 = serde_json::json!({
            "version": "2.0",
            "format_version": crate::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::EXTRACTOR_VERSION,
            "active_variant_id": "v1",
            "variant_dir": "variants/v1",
            "workspace_root": test_dir.0.display().to_string(),
            "shards": [{ "name": "main", "version": "0.1.0", "file": "main.ridx", "is_dependency": false }]
        });
        fs::write(&mf, serde_json::to_string(&manifest_v1).unwrap()).unwrap();

        let mut ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        assert_eq!(ws.readers.len(), 1);

        let v2_dir = test_dir.0.join("variants/v2");
        fs::create_dir_all(&v2_dir).unwrap();
        let idx2 = v2_dir.join("main.ridx");
        write_test_index(&idx2, "main");

        let dep_file2 = test_dir.0.join("dep_mismatch.ridx");
        let mut w_dep2 = IndexWriter::new("dep2", "0.1.0");
        w_dep2.set_unit_key("dep2_key");
        w_dep2.set_generation("gen_actual");
        let mut f_dep2 = File::create(&dep_file2).unwrap();
        w_dep2.write_to(&mut f_dep2).unwrap();
        drop(f_dep2);

        std::thread::sleep(std::time::Duration::from_millis(50));
        let manifest_v2 = serde_json::json!({
            "version": "2.0",
            "format_version": crate::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::EXTRACTOR_VERSION,
            "active_variant_id": "v2",
            "variant_dir": "variants/v2",
            "workspace_root": test_dir.0.display().to_string(),
            "shards": [{ "name": "main", "version": "0.2.0", "file": "main.ridx", "is_dependency": false }],
            "dependency_shards": [
                {
                    "name": "dep",
                    "version": "0.1.0",
                    "unit_key": "dep_key_shared",
                    "object_path": dep_file.display().to_string(),
                    "source_root": test_dir.0.display().to_string(),
                    "is_dependency": true,
                    "generation": "gen_alpha",
                    "dependencies": ["extra_dep"],
                    "extern_units": { "core": "core_unit_key" },
                    "file_hashes": { "src/lib.rs": 1234 }
                },
                {
                    "name": "dep2",
                    "version": "0.1.0",
                    "unit_key": "dep2_key",
                    "object_path": dep_file2.display().to_string(),
                    "source_root": test_dir.0.display().to_string(),
                    "is_dependency": true,
                    "generation": null,
                    "dependencies": ["unreached"],
                    "extern_units": {},
                    "file_hashes": { "src/different.rs": 9999 }
                },
                {
                    "name": "dep_nonexistent",
                    "version": "0.1.0",
                    "unit_key": "dep_nonexistent_key",
                    "object_path": "nonexistent_obj_path.ridx",
                    "source_root": null,
                    "is_dependency": true,
                    "generation": null,
                    "dependencies": [],
                    "extern_units": {},
                    "file_hashes": {}
                }
            ]
        });
        fs::write(&mf, serde_json::to_string(&manifest_v2).unwrap()).unwrap();

        let reloaded = ws.reload_if_modified().unwrap();
        assert!(reloaded);
        assert_eq!(ws.active_variant_id.as_deref(), Some("v2"));

        let canon_file = test_dir.0.join("src/lib.rs");
        let _ = fs::create_dir_all(canon_file.parent().unwrap());
        let _ = fs::write(&canon_file, b"content");
        // Test non-canonical path that resolves to canonical in last_reloaded_files (line 522)
        let non_canon = format!("{}/./src/lib.rs", test_dir.0.display());
        assert!(ws.is_file_reloaded(&non_canon));
        assert!(ws.is_file_reloaded(canon_file.to_str().unwrap()));
        assert!(ws.is_file_reloaded("src/lib.rs"));
        assert!(ws.is_file_reloaded("some_prefix/src/lib.rs"));
        assert!(!ws.is_file_reloaded("other_unrelated_file.rs"));

        // Test shard deletion reconciliation (lines 306-330)
        let prev_count = ws.crate_count();
        let _ = fs::remove_file(&dep_file2);
        let reloaded_after_delete = ws.reload_if_modified().unwrap();
        assert!(reloaded_after_delete);
        assert_eq!(ws.crate_count(), prev_count - 1);

        // Test empty new variant directory fallback (lines 297-302)
        let empty_v3_dir = test_dir.0.join("variants/v3");
        let _ = fs::create_dir_all(&empty_v3_dir);
        let manifest_v3 = serde_json::json!({
            "version": "2.0",
            "format_version": 2,
            "extractor_version": 1,
            "active_variant_id": "v3",
            "variant_dir": "variants/v3",
            "base_out_dir": test_dir.0.display().to_string(),
            "workspace_root": test_dir.0.display().to_string(),
            "manifest_path": "",
            "profile": "dev",
            "timestamp_epoch_secs": 0,
            "total_crates": 0,
            "total_defs": 0,
            "total_refs": 0,
            "total_types": 0,
            "dependency_shards": [],
            "crates": []
        });
        fs::write(&mf, serde_json::to_string(&manifest_v3).unwrap()).unwrap();
        // Since v3 has no readers, it falls through lines 297-302
        let _ = ws.reload_if_modified();

        // Test meta_changed branch (lines 401-443): sidecar changed without .ridx file mtime changing
        let sidecar_p = test_dir.0.join("main.json");
        let sidecar_info = serde_json::json!({
            "name": "main",
            "version": "0.2.0",
            "unit_key": "main_unit",
            "generation": "gen_alpha",
            "object_path": idx2.display().to_string(),
            "source_root": test_dir.0.display().to_string(),
            "is_dependency": false,
            "dependencies": ["dep_updated"],
            "extern_units": {},
            "file_hashes": { "src/lib.rs": 12345 }
        });
        fs::write(&sidecar_p, serde_json::to_string(&sidecar_info).unwrap()).unwrap();
        // Set meta_paths and past meta_mtime so meta_changed evaluates to true
        if let Some(mp) = ws.meta_paths.get_mut(0) {
            *mp = Some(sidecar_p.clone());
        }
        if let Some(mtime) = ws.meta_mtimes.get_mut(0) {
            *mtime = Some(std::time::SystemTime::UNIX_EPOCH);
        }
        let reloaded_meta = ws.reload_if_modified().unwrap();
        assert!(reloaded_meta);
    }

    #[test]
    fn test_reload_additional_coverage() {
        let _lock = crate::workspace::ENV_LOCK.lock().unwrap();
        let temp = new_test_dir("reload_cov");
        let idx_path = temp.0.join("test.ridx");
        let mut w = IndexWriter::new("reload_cov_pkg", "0.1.0");
        w.set_unit_key("cov_uk");
        w.set_generation("gen_cov");
        w.get_or_insert_file("src/lib.rs");
        let mut f = File::create(&idx_path).unwrap();
        w.write_to(&mut f).unwrap();
        drop(f);

        let mut ws = WorkspaceIndex::open(&idx_path).unwrap();

        // 1. is_file_reloaded with canonical path matching (line 522)
        // and record_reloaded_reader_files with source_root (line 538)
        ws.source_roots = vec![Some(temp.0.clone())];
        ws.record_reloaded_reader_files(0);
        let test_src = temp.0.join("src/lib.rs");
        let _ = fs::create_dir_all(test_src.parent().unwrap());
        let _ = fs::write(&test_src, b"fn foo() {}");
        assert!(ws.is_file_reloaded(&test_src.to_string_lossy()));

        // 2. Discover new file whose unit_key is already present (line 485)
        let dup_path = temp.0.join("dup.ridx");
        let mut w_dup = IndexWriter::new("dup_pkg", "0.1.0");
        w_dup.set_unit_key("cov_uk"); // same unit key as idx_path
        let mut f_dup = File::create(&dup_path).unwrap();
        w_dup.write_to(&mut f_dup).unwrap();
        drop(f_dup);

        ws.root_dir = Some(temp.0.clone());
        let changed = ws.reload_if_modified().unwrap();
        // Since dup_pkg has unit_key == "cov_uk" which is already in ws.unit_keys, it is skipped (line 485)
        assert!(!changed);

        // 3. Variant reload with cache fallback (line 141), empty extern_units replaced (lines 180-182),
        // unit_key mismatch (line 205), and computed digest mismatch (line 229)
        let cache_dir = temp.0.join("cache");
        let _ = fs::create_dir_all(&cache_dir);
        std::env::set_var("RIDX_CACHE_DIR", cache_dir.display().to_string());
        let cached_obj = crate::format::cache_object_path(&cache_dir, "cache_reload_uk");
        let _ = fs::create_dir_all(cached_obj.parent().unwrap());
        let mut w_cached = IndexWriter::new("cached_reload_dep", "0.1.0");
        w_cached.set_unit_key("cache_reload_uk");
        w_cached.set_generation("gen_cached_target");
        let mut fc = File::create(&cached_obj).unwrap();
        w_cached.write_to(&mut fc).unwrap();
        drop(fc);

        let cached_mismatch_obj = crate::format::cache_object_path(&cache_dir, "cache_mismatch_uk");
        let _ = fs::create_dir_all(cached_mismatch_obj.parent().unwrap());
        let mut w_mismatch = IndexWriter::new("cached_mismatch_dep", "0.1.0");
        w_mismatch.set_unit_key("cache_mismatch_uk");
        w_mismatch.set_generation("gen_reader_actual");
        let mut fm = File::create(&cached_mismatch_obj).unwrap();
        w_mismatch.write_to(&mut fm).unwrap();
        drop(fm);

        let no_gen_obj = crate::format::cache_object_path(&cache_dir, "no_gen_uk");
        let _ = fs::create_dir_all(no_gen_obj.parent().unwrap());
        let mut w_nogen = IndexWriter::new("no_gen_dep", "0.1.0");
        w_nogen.set_unit_key("no_gen_uk");
        let mut fng = File::create(&no_gen_obj).unwrap();
        w_nogen.write_to(&mut fng).unwrap();
        drop(fng);

        let v_new_dir = temp.0.join("variants/v_cov");
        let _ = fs::create_dir_all(&v_new_dir);
        let v_new_idx = v_new_dir.join("cov.ridx");
        let mut w_new = IndexWriter::new("v_cov_pkg", "0.1.0");
        w_new.set_unit_key("cov_uk");
        w_new.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "test_fn".to_string(),
            path: "v_cov_pkg::test_fn".to_string(),
            signature: Some("fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 2,
            end_col: 2,
        });
        let mut fn_idx = File::create(&v_new_idx).unwrap();
        w_new.write_to(&mut fn_idx).unwrap();
        drop(fn_idx);

        let mut ext_u = HashMap::new();
        ext_u.insert("dep_alias".to_string(), "dep_unit".to_string());

        let mf_cov = serde_json::json!({
            "version": "2.0",
            "format_version": 2,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
            "active_variant_id": "v_cov",
            "variant_dir": "variants/v_cov",
            "base_out_dir": temp.0.display().to_string(),
            "workspace_root": temp.0.display().to_string(),
            "manifest_path": "",
            "profile": "dev",
            "timestamp_epoch_secs": 0,
            "total_crates": 1,
            "total_defs": 0,
            "total_refs": 0,
            "total_types": 0,
            "dependency_shards": [
                {
                    "name": "v_cov_pkg",
                    "version": "0.1.0",
                    "unit_key": "cov_uk",
                    "generation": null,
                    "object_path": v_new_idx.display().to_string(),
                    "source_root": temp.0.display().to_string(),
                    "is_dependency": false,
                    "dependencies": ["dep_cov_1"],
                    "extern_units": ext_u, // hits lines 180-182
                    "file_hashes": { "src/lib.rs": 12345 }
                },
                {
                    "name": "cached_reload_dep",
                    "version": "0.1.0",
                    "unit_key": "cache_reload_uk",
                    "generation": null,
                    "object_path": "/nonexistent/object.ridx", // hits line 141 (fallback to cache)
                    "source_root": null,
                    "is_dependency": true,
                    "dependencies": [],
                    "extern_units": {},
                    "file_hashes": { "src/lib.rs": 99999 } // hits line 229 (computed != gen_cached_target)
                },
                {
                    "name": "cached_mismatch_dep",
                    "version": "0.1.0",
                    "unit_key": "cache_mismatch_uk",
                    "generation": "gen_different_from_reader", // hits line 228
                    "object_path": "/nonexistent/object.ridx",
                    "source_root": null,
                    "is_dependency": true,
                    "dependencies": [],
                    "extern_units": {},
                    "file_hashes": {}
                },
                {
                    "name": "no_gen_dep",
                    "version": "0.1.0",
                    "unit_key": "no_gen_uk",
                    "generation": "dep_gen_fallback", // hits line 244
                    "object_path": "/nonexistent/object.ridx",
                    "source_root": null,
                    "is_dependency": true,
                    "dependencies": [],
                    "extern_units": {},
                    "file_hashes": {}
                },
                {
                    "name": "mismatch_key_dep",
                    "version": "0.1.0",
                    "unit_key": "expected_key_wrong", // hits line 205 (unit_key != r.unit_key)
                    "generation": null,
                    "object_path": idx_path.display().to_string(),
                    "source_root": null,
                    "is_dependency": true,
                    "dependencies": [],
                    "extern_units": {},
                    "file_hashes": {}
                }
            ],
            "crates": []
        });

        let mf_path = temp.0.join("ridx_manifest.json");
        fs::write(&mf_path, serde_json::to_string(&mf_cov).unwrap()).unwrap();
        ws.manifest_path = Some(mf_path);
        ws.manifest_mtime = Some(std::time::SystemTime::UNIX_EPOCH);
        let reloaded_v = ws.reload_if_modified().unwrap();
        assert!(reloaded_v);

        // Test is_file_reloaded (lines 535-538)
        assert!(ws.is_file_reloaded("src/lib.rs"));
        assert!(ws.is_file_reloaded("lib.rs"));
        assert!(!ws.is_file_reloaded("nonexistent/path.rs"));

        let _ = fs::remove_file(&cached_obj);
        let _ = fs::remove_file(&cached_mismatch_obj);
        let _ = fs::remove_file(&no_gen_obj);
        std::env::remove_var("RIDX_CACHE_DIR");
    }
}
