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
                                            let already_loaded =
                                                new_paths.iter().any(|existing| {
                                                    fs::canonicalize(existing)
                                                        .unwrap_or_else(|_| existing.clone())
                                                        == canon_cand
                                                }) || new_unit_keys.iter().any(|existing_uk| {
                                                    existing_uk.as_deref() == Some(&dep.unit_key)
                                                });
                                            if already_loaded {
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
                                                let mut combined_hashes = embedded;
                                                if gen_valid {
                                                    for (k, v) in &dep.file_hashes {
                                                        combined_hashes
                                                            .entry(k.clone())
                                                            .or_insert(*v);
                                                    }
                                                }
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
