/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Workspace index opening and initial shard/dependency loading.

use super::{cache_resolver, collect_ridx_files, next_workspace_instance_id};
use super::{PathIndex, UnitMatchIndex, WorkspaceIndex};
use crate::reader::IndexReader;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

impl WorkspaceIndex {
    /// Opens a single .ridx file or scans a directory containing .ridx files.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_with_cache_dir(path, crate::format::resolve_cache_dir())
    }

    /// Opens a single .ridx file or directory using an explicit global cache directory.
    pub fn open_with_cache_dir(path: impl AsRef<Path>, cache_dir: impl AsRef<Path>) -> io::Result<Self> {
        let p = path.as_ref();
        let cache_dir = cache_dir.as_ref().to_path_buf();

        if p.is_file() {
            let mtime = fs::metadata(p).and_then(|m| m.modified()).ok();
            let reader = IndexReader::open(p)?;
            let embedded = reader.embedded_file_hashes();
            let parent = p.parent().unwrap_or_else(|| Path::new(""));
            let meta = Self::load_file_metadata(p, parent, reader.crate_name(), reader.unit_key(), reader.generation(), &embedded);
            let is_dep = meta.source_root.is_some() && meta.target_kind.map(|k| k.is_lib()).unwrap_or(true);
            let ws_root = p.parent().and_then(|parent| if parent.ends_with("target/ridx") { parent.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()) } else { None });
            let readers = vec![reader];
            let paths = vec![p.to_path_buf()];
            let mtimes = vec![mtime];
            let source_roots = vec![meta.source_root];
            let unit_keys = vec![meta.unit_key];
            let target_kinds = vec![meta.target_kind];
            let reader_generations = vec![meta.generation];
            let reader_dependencies = vec![meta.dependencies];
            let reader_extern_units = vec![meta.extern_units];
            let reader_file_hashes = vec![meta.file_hashes];
            let own_sources: HashMap<PathBuf, HashSet<String>> = meta.own_sources.map(|o| (p.to_path_buf(), o)).into_iter().collect();
            let reader_is_dependency = vec![is_dep];
            let meta_paths = vec![meta.meta_path];
            let meta_mtimes = vec![meta.meta_mtime];

            let unit_match = UnitMatchIndex::build_with_deps(&readers, &unit_keys, &reader_extern_units, &reader_dependencies);
            let mut ws = Self {
                readers,
                paths,
                mtimes,
                source_roots,
                unit_keys,
                unit_match,
                path_index: PathIndex::default(),
                target_kinds,
                reader_generations,
                reader_dependencies,
                reader_extern_units,
                reader_file_hashes,
                own_sources,
                reader_is_dependency,
                meta_paths,
                meta_mtimes,
                last_reloaded_files: HashSet::new(),
                root_dir: None,
                workspace_root: ws_root,
                manifest_path: None,
                manifest_mtime: None,
                manifest_len: None,
                manifest_hash: None,
                manifest_generation: None,
                manifest_ino: None,
                active_variant_id: None,
                sysroot_cas_key: None,
                instance_id: next_workspace_instance_id(),
                generation: 1,
            };
            ws.path_index = PathIndex::build(&ws);
            return Ok(ws);
        }

        // If directory, check target/ridx subfolder first if it exists
        let search_dir = if p.is_dir() && p.join("target/ridx").is_dir() { p.join("target/ridx") } else { p.to_path_buf() };

        // Check if an active build variant manifest exists
        let (scan_dir, manifest_info) = Self::resolve_open_manifest(&search_dir)?;

        let mut ridx_files = Vec::new();
        if scan_dir.is_dir() {
            collect_ridx_files(&scan_dir, &mut ridx_files)?;
        }

        if ridx_files.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("No .ridx index files found in {:?}", p)));
        }

        // Sort files for deterministic order
        ridx_files.sort();

        let mut readers = Vec::with_capacity(ridx_files.len());
        let mut paths = Vec::with_capacity(ridx_files.len());
        let mut mtimes = Vec::with_capacity(ridx_files.len());
        let mut source_roots = Vec::with_capacity(ridx_files.len());
        let mut unit_keys = Vec::with_capacity(ridx_files.len());
        let mut target_kinds = Vec::with_capacity(ridx_files.len());
        let mut reader_generations = Vec::with_capacity(ridx_files.len());
        let mut reader_dependencies = Vec::with_capacity(ridx_files.len());
        let mut reader_extern_units = Vec::with_capacity(ridx_files.len());
        let mut reader_file_hashes = Vec::with_capacity(ridx_files.len());
        let mut reader_is_dependency = Vec::with_capacity(ridx_files.len());
        let mut meta_paths = Vec::with_capacity(ridx_files.len());
        let mut meta_mtimes = Vec::with_capacity(ridx_files.len());
        let mut own_sources: HashMap<PathBuf, HashSet<String>> = HashMap::new();

        for file_path in ridx_files {
            match IndexReader::open(&file_path) {
                Ok(reader) => {
                    let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                    let embedded = reader.embedded_file_hashes();
                    let meta = Self::load_file_metadata(&file_path, &scan_dir, reader.crate_name(), reader.unit_key(), reader.generation(), &embedded);
                    let is_dep = meta.is_dependency;
                    if let Some(own) = meta.own_sources {
                        own_sources.insert(file_path.clone(), own);
                    }
                    readers.push(reader);
                    paths.push(file_path);
                    mtimes.push(mtime);
                    source_roots.push(meta.source_root);
                    unit_keys.push(meta.unit_key);
                    target_kinds.push(meta.target_kind);
                    reader_generations.push(meta.generation);
                    reader_dependencies.push(meta.dependencies);
                    reader_extern_units.push(meta.extern_units);
                    reader_file_hashes.push(meta.file_hashes);
                    reader_is_dependency.push(is_dep);
                    meta_paths.push(meta.meta_path);
                    meta_mtimes.push(meta.meta_mtime);
                }
                Err(e) => {
                    eprintln!("Warning: skipping invalid index {:?}: {e}", file_path);
                }
            }
        }

        // Load external dependency shards from global cache if specified in manifest
        if let Some(ref info) = manifest_info {
            let mut cache_resolver = cache_resolver::CacheObjectResolver::new(cache_dir);
            let mut canon_path_to_idx: HashMap<PathBuf, usize> = HashMap::with_capacity(paths.len() + info.dependency_shards.len());
            for (i, existing) in paths.iter().enumerate() {
                let canon = fs::canonicalize(existing).unwrap_or_else(|_| existing.clone());
                canon_path_to_idx.entry(canon).or_insert(i);
            }
            let mut unit_key_to_idx: HashMap<String, usize> = HashMap::with_capacity(unit_keys.len() + info.dependency_shards.len());
            for (i, uk_opt) in unit_keys.iter().enumerate() {
                if let Some(ref uk) = uk_opt {
                    unit_key_to_idx.entry(uk.clone()).or_insert(i);
                }
            }
            for dep in &info.dependency_shards {
                let cand = cache_resolver.resolve(dep);

                if let Some(dep_file) = cand {
                    let canon_cand = fs::canonicalize(&dep_file).unwrap_or_else(|_| dep_file.clone());
                    let existing_pos = canon_path_to_idx.get(&canon_cand).copied().or_else(|| unit_key_to_idx.get(&dep.unit_key).copied());

                    if let Some(idx) = existing_pos {
                        reader_is_dependency[idx] = true;
                        if source_roots[idx].is_none() {
                            source_roots[idx] = dep.source_root.as_ref().map(PathBuf::from);
                        }
                        if target_kinds[idx].is_none() && dep.target_kind.is_some() {
                            target_kinds[idx] = dep.target_kind;
                        }
                        if reader_generations[idx].is_none() {
                            reader_generations[idx] = dep.generation.clone();
                        }
                        if reader_dependencies[idx].is_empty() && !dep.dependencies.is_empty() {
                            reader_dependencies[idx] = dep.dependencies.clone();
                        }
                        if reader_extern_units[idx].is_empty() && !dep.extern_units.is_empty() {
                            reader_extern_units[idx] = dep.extern_units.clone();
                        }
                        if !dep.file_hashes.is_empty() {
                            let embedded = readers[idx].embedded_file_hashes();
                            let mut combined = dep.file_hashes.clone();
                            let sroot_path = dep.source_root.as_deref().map(Path::new);
                            crate::format::merge_embedded_hashes_authoritative(&mut combined, &embedded, sroot_path);
                            reader_file_hashes[idx] = combined;
                        }
                        continue;
                    }

                    if let Ok(reader) = IndexReader::open(&dep_file) {
                        if !dep.name.is_empty() && reader.crate_name() != dep.name {
                            continue;
                        }
                        if !dep.version.is_empty() && !reader.crate_version().is_empty() && reader.crate_version() != dep.version {
                            continue;
                        }
                        if !dep.unit_key.is_empty() && !reader.unit_key().is_empty() && dep.unit_key != reader.unit_key() {
                            let dep_gen = dep.generation.as_deref().unwrap_or("");
                            let gen_matches = if !dep_gen.is_empty() {
                                dep_gen == reader.generation()
                            } else if !dep.file_hashes.is_empty() && !reader.generation().is_empty() {
                                let computed_digest = crate::format::compute_source_snapshot_digest(&dep.file_hashes);
                                let computed_gen = format!("{:016x}", computed_digest);
                                computed_gen == reader.generation()
                            } else {
                                false
                            };
                            if !gen_matches {
                                continue;
                            }
                        }
                        let embedded = reader.embedded_file_hashes();
                        let reader_gen = reader.generation();
                        let mut gen_valid = true;
                        if !reader_gen.is_empty() {
                            let dep_gen = dep.generation.as_deref().unwrap_or("");
                            if !dep_gen.is_empty() {
                                if dep_gen != reader_gen {
                                    gen_valid = false;
                                }
                            } else if !dep.file_hashes.is_empty() {
                                let computed_digest = crate::format::compute_source_snapshot_digest(&dep.file_hashes);
                                let computed_gen = format!("{:016x}", computed_digest);
                                if computed_gen != reader_gen {
                                    gen_valid = false;
                                }
                            }
                        }
                        let gen = if !reader_gen.is_empty() { Some(reader_gen.to_string()) } else { dep.generation.clone() };
                        let mtime = fs::metadata(&dep_file).and_then(|m| m.modified()).ok();
                        let new_idx = paths.len();
                        canon_path_to_idx.entry(canon_cand).or_insert(new_idx);
                        unit_key_to_idx.entry(dep.unit_key.clone()).or_insert(new_idx);
                        readers.push(reader);
                        paths.push(dep_file);
                        mtimes.push(mtime);
                        source_roots.push(dep.source_root.as_ref().map(PathBuf::from));
                        unit_keys.push(Some(dep.unit_key.clone()));
                        target_kinds.push(dep.target_kind);
                        reader_generations.push(gen);
                        let deps = if gen_valid { dep.dependencies.clone() } else { Vec::new() };
                        let ext_units = if gen_valid { dep.extern_units.clone() } else { HashMap::new() };
                        reader_dependencies.push(deps);
                        reader_extern_units.push(ext_units);
                        let mut combined_hashes = if gen_valid { dep.file_hashes.clone() } else { HashMap::new() };
                        let sroot_path = dep.source_root.as_deref().map(Path::new);
                        crate::format::merge_embedded_hashes_authoritative(&mut combined_hashes, &embedded, sroot_path);
                        reader_file_hashes.push(combined_hashes);
                        reader_is_dependency.push(true);
                        meta_paths.push(None);
                        meta_mtimes.push(None);
                    }
                }
            }
        }

        if readers.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Failed to open any valid .ridx index files in {:?}", p)));
        }

        let (manifest_path, manifest_mtime, manifest_len, manifest_hash, manifest_generation, manifest_ino, active_variant_id, manifest_ws_root, sysroot_cas_key) =
            match manifest_info {
                Some(info) => {
                    (Some(info.path), info.mtime, info.len, info.hash, info.generation, info.ino, Some(info.active_variant_id), info.workspace_root, info.sysroot_cas_key)
                }
                None => (None, None, None, None, None, None, None, None, None),
            };
        let workspace_root =
            manifest_ws_root.or_else(
                || {
                    if scan_dir.ends_with("target/ridx") {
                        scan_dir.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf())
                    } else {
                        Some(p.to_path_buf())
                    }
                },
            );

        let unit_match = UnitMatchIndex::build_with_deps(&readers, &unit_keys, &reader_extern_units, &reader_dependencies);
        let mut ws = Self {
            readers,
            paths,
            mtimes,
            source_roots,
            unit_keys,
            unit_match,
            path_index: PathIndex::default(),
            target_kinds,
            reader_generations,
            reader_dependencies,
            reader_extern_units,
            reader_file_hashes,
            own_sources,
            reader_is_dependency,
            meta_paths,
            meta_mtimes,
            last_reloaded_files: HashSet::new(),
            root_dir: Some(scan_dir),
            workspace_root,
            manifest_path,
            manifest_mtime,
            manifest_len,
            manifest_hash,
            manifest_generation,
            manifest_ino,
            active_variant_id,
            sysroot_cas_key: sysroot_cas_key.clone(),
            instance_id: next_workspace_instance_id(),
            generation: 1,
        };
        if let Some(cas_key) = sysroot_cas_key {
            ws.attach_sysroot_shards_for_key(&cas_key);
        }
        ws.generation = 1;
        ws.path_index = PathIndex::build(&ws);
        Ok(ws)
    }
}
