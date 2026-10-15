/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Multi-crate workspace index support for Rust Intelligence.
//!
//! Provides unified zero-copy queries across all crates in a cargo workspace.

pub mod cache_resolver;
pub mod file_module;
pub mod manifest;
pub mod navigation;
pub mod path_index;
pub mod prelude;
pub mod provenance;
pub mod qualifier;
pub mod reload;
pub mod std_map;
pub mod sysroot;
pub mod target_kind;
pub mod unit_match;

#[cfg(test)]
mod cache_resolver_tests;
#[cfg(test)]
mod file_module_tests;
#[cfg(test)]
mod path_index_tests;
#[cfg(test)]
mod routing_tests;
#[cfg(test)]
mod sibling_tests;
#[cfg(test)]
mod sysroot_tests;
#[cfg(test)]
mod target_kind_routing_tests;
#[cfg(test)]
mod unit_match_tests;

pub use manifest::*;
pub use path_index::PathIndex;
pub use prelude::is_prelude_trait;
pub use target_kind::{classify_target, classify_target_with_manifest, TargetKind};
pub use unit_match::UnitMatchIndex;

use crate::reader::IndexReader;
use memmap2::Mmap;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static WORKSPACE_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_workspace_instance_id() -> u64 {
    WORKSPACE_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Unified multi-crate index for a cargo workspace or multi-index directory.
pub struct WorkspaceIndex {
    pub(crate) readers: Vec<IndexReader<Mmap>>,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) mtimes: Vec<Option<std::time::SystemTime>>,
    pub(crate) source_roots: Vec<Option<PathBuf>>,
    pub(crate) unit_keys: Vec<Option<String>>,
    pub(crate) unit_match: UnitMatchIndex,
    pub(crate) path_index: PathIndex,
    pub(crate) target_kinds: Vec<Option<TargetKind>>,
    pub(crate) reader_generations: Vec<Option<String>>,
    pub(crate) reader_dependencies: Vec<Vec<String>>,
    pub(crate) reader_extern_units: Vec<HashMap<String, String>>,
    pub(crate) reader_file_hashes: Vec<HashMap<String, u64>>,
    pub(crate) reader_is_dependency: Vec<bool>,
    pub(crate) meta_paths: Vec<Option<PathBuf>>,
    pub(crate) meta_mtimes: Vec<Option<std::time::SystemTime>>,
    pub(crate) last_reloaded_files: HashSet<String>,
    pub(crate) root_dir: Option<PathBuf>,
    pub(crate) workspace_root: Option<PathBuf>,
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) manifest_mtime: Option<std::time::SystemTime>,
    pub(crate) manifest_len: Option<u64>,
    pub(crate) manifest_hash: Option<u64>,
    pub(crate) manifest_generation: Option<u64>,
    pub(crate) manifest_ino: Option<u64>,
    pub(crate) active_variant_id: Option<String>,
    pub(crate) sysroot_cas_key: Option<String>,
    pub(crate) instance_id: u64,
    pub(crate) generation: u64,
}

impl std::fmt::Debug for WorkspaceIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceIndex")
            .field("readers_count", &self.readers.len())
            .field("workspace_root", &self.workspace_root)
            .finish()
    }
}

impl WorkspaceIndex {
    /// Opens a single .ridx file or scans a directory containing .ridx files.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let p = path.as_ref();

        if p.is_file() {
            let mtime = fs::metadata(p).and_then(|m| m.modified()).ok();
            let reader = IndexReader::open(p)?;
            let embedded = reader.embedded_file_hashes();
            let parent = p.parent().unwrap_or_else(|| Path::new(""));
            let meta = Self::load_file_metadata(
                p,
                parent,
                reader.crate_name(),
                reader.unit_key(),
                reader.generation(),
                &embedded,
            );
            let is_dep =
                meta.source_root.is_some() && meta.target_kind.map(|k| k.is_lib()).unwrap_or(true);
            let ws_root = p.parent().and_then(|parent| {
                if parent.ends_with("target/ridx") {
                    parent
                        .parent()
                        .and_then(|p| p.parent())
                        .map(|p| p.to_path_buf())
                } else {
                    None
                }
            });
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
            let reader_is_dependency = vec![is_dep];
            let meta_paths = vec![meta.meta_path];
            let meta_mtimes = vec![meta.meta_mtime];

            let unit_match = UnitMatchIndex::build(&readers, &unit_keys, &reader_extern_units);
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
        let search_dir = if p.is_dir() && p.join("target/ridx").is_dir() {
            p.join("target/ridx")
        } else {
            p.to_path_buf()
        };

        // Check if an active build variant manifest exists
        let manifest_file = search_dir.join("ridx_manifest.json");
        let (scan_dir, manifest_info) = if manifest_file.is_file() {
            let content = fs::read(&manifest_file)?;
            let manifest = serde_json::from_slice::<VariantManifest>(&content).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to parse variant manifest {:?}: {e}", manifest_file),
                )
            })?;

            let mtime = fs::metadata(&manifest_file).and_then(|m| m.modified()).ok();
            if !manifest.active_variant_id.is_empty() {
                let expected_variant_dir = format!("variants/{}", manifest.active_variant_id);
                let vdir_str = manifest.variant_dir.trim();
                let vpath = Path::new(vdir_str);
                if vdir_str.is_empty()
                    || vpath.is_absolute()
                    || vpath
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
                    || vpath == Path::new(".")
                    || vdir_str != expected_variant_dir
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Active variant manifest {:?} specifies variant_dir {:?} which does not match active_variant_id {:?} (expected {:?})",
                            manifest_file, manifest.variant_dir, manifest.active_variant_id, expected_variant_dir
                        ),
                    ));
                }

                let vdir = search_dir.join(&expected_variant_dir);
                if vdir == search_dir || !vdir.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!(
                            "Active variant directory {:?} specified in {:?} does not exist. Run 'ridx index' to build this variant.",
                            vdir, manifest_file
                        ),
                    ));
                }

                if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Format version mismatch in manifest {:?}: format_version {} != current {}",
                            manifest_file, manifest.format_version, crate::format::CURRENT_FORMAT_VERSION
                        ),
                    ));
                }

                match manifest.extractor_version.as_deref() {
                    Some(ver) if ver == crate::format::EXTRACTOR_VERSION => {}
                    Some(ver) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Extractor version mismatch in manifest {:?}: extractor_version {} != current {}",
                                manifest_file, ver, crate::format::EXTRACTOR_VERSION
                            ),
                        ));
                    }
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Extractor version missing in manifest {:?}", manifest_file),
                        ));
                    }
                }

                (
                    vdir,
                    Some((
                        manifest_file,
                        mtime,
                        manifest.active_variant_id,
                        manifest.dependency_shards,
                        if !manifest.workspace_root.is_empty() {
                            Some(PathBuf::from(manifest.workspace_root))
                        } else {
                            None
                        },
                        manifest.sysroot_cas_key,
                    )),
                )
            } else {
                // Legacy manifest without active_variant_id: scan search_dir directly
                (search_dir.clone(), None)
            }
        } else if search_dir.join("variant_manifest.json").is_file() {
            let v_manifest_file = search_dir.join("variant_manifest.json");
            let content = fs::read(&v_manifest_file)?;
            let manifest = serde_json::from_slice::<VariantManifest>(&content).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Failed to parse variant manifest {:?}: {e}",
                        v_manifest_file
                    ),
                )
            })?;
            let mtime = fs::metadata(&v_manifest_file)
                .and_then(|m| m.modified())
                .ok();
            if manifest.format_version != crate::format::CURRENT_FORMAT_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Format version mismatch in manifest {:?}: format_version {} != current {}",
                        v_manifest_file,
                        manifest.format_version,
                        crate::format::CURRENT_FORMAT_VERSION
                    ),
                ));
            }
            match manifest.extractor_version.as_deref() {
                Some(ver) if ver == crate::format::EXTRACTOR_VERSION => {}
                Some(ver) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Extractor version mismatch in manifest {:?}: extractor_version {} != current {}",
                            v_manifest_file, ver, crate::format::EXTRACTOR_VERSION
                        ),
                    ));
                }
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Extractor version missing in manifest {:?}",
                            v_manifest_file
                        ),
                    ));
                }
            }
            (
                search_dir.clone(),
                Some((
                    v_manifest_file,
                    mtime,
                    manifest.active_variant_id,
                    manifest.dependency_shards,
                    if !manifest.workspace_root.is_empty() {
                        Some(PathBuf::from(manifest.workspace_root))
                    } else {
                        None
                    },
                    manifest.sysroot_cas_key,
                )),
            )
        } else {
            (search_dir.clone(), None)
        };

        let mut ridx_files = Vec::new();
        if scan_dir.is_dir() {
            collect_ridx_files(&scan_dir, &mut ridx_files)?;
        }

        if ridx_files.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("No .ridx index files found in {:?}", p),
            ));
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

        for file_path in ridx_files {
            match IndexReader::open(&file_path) {
                Ok(reader) => {
                    let mtime = fs::metadata(&file_path).and_then(|m| m.modified()).ok();
                    let embedded = reader.embedded_file_hashes();
                    let meta = Self::load_file_metadata(
                        &file_path,
                        &scan_dir,
                        reader.crate_name(),
                        reader.unit_key(),
                        reader.generation(),
                        &embedded,
                    );
                    let is_dep = meta.is_dependency;
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
        if let Some((_, _, _, ref dep_shards, _, _)) = manifest_info {
            let cache_dir = crate::format::resolve_cache_dir();
            let mut cache_resolver = cache_resolver::CacheObjectResolver::new(cache_dir);
            for dep in dep_shards {
                let cand = cache_resolver.resolve(dep);

                if let Some(dep_file) = cand {
                    let canon_cand =
                        fs::canonicalize(&dep_file).unwrap_or_else(|_| dep_file.clone());
                    let existing_pos = paths
                        .iter()
                        .position(|existing| {
                            fs::canonicalize(existing).unwrap_or_else(|_| existing.clone())
                                == canon_cand
                        })
                        .or_else(|| {
                            unit_keys.iter().position(|existing_uk| {
                                existing_uk.as_deref() == Some(&dep.unit_key)
                            })
                        });

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
                            crate::format::merge_embedded_hashes_authoritative(
                                &mut combined,
                                &embedded,
                                sroot_path,
                            );
                            reader_file_hashes[idx] = combined;
                        }
                        continue;
                    }

                    if let Ok(reader) = IndexReader::open(&dep_file) {
                        if !dep.name.is_empty() && reader.crate_name() != dep.name {
                            continue;
                        }
                        if !dep.version.is_empty()
                            && !reader.crate_version().is_empty()
                            && reader.crate_version() != dep.version
                        {
                            continue;
                        }
                        if !dep.unit_key.is_empty()
                            && !reader.unit_key().is_empty()
                            && dep.unit_key != reader.unit_key()
                        {
                            let dep_gen = dep.generation.as_deref().unwrap_or("");
                            let gen_matches = if !dep_gen.is_empty() {
                                dep_gen == reader.generation()
                            } else if !dep.file_hashes.is_empty() && !reader.generation().is_empty()
                            {
                                let computed_digest =
                                    crate::format::compute_source_snapshot_digest(&dep.file_hashes);
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
                                let computed_digest =
                                    crate::format::compute_source_snapshot_digest(&dep.file_hashes);
                                let computed_gen = format!("{:016x}", computed_digest);
                                if computed_gen != reader_gen {
                                    gen_valid = false;
                                }
                            }
                        }
                        let gen = if !reader_gen.is_empty() {
                            Some(reader_gen.to_string())
                        } else {
                            dep.generation.clone()
                        };
                        let mtime = fs::metadata(&dep_file).and_then(|m| m.modified()).ok();
                        readers.push(reader);
                        paths.push(dep_file);
                        mtimes.push(mtime);
                        source_roots.push(dep.source_root.as_ref().map(PathBuf::from));
                        unit_keys.push(Some(dep.unit_key.clone()));
                        target_kinds.push(dep.target_kind);
                        reader_generations.push(gen);
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
                        reader_dependencies.push(deps);
                        reader_extern_units.push(ext_units);
                        let mut combined_hashes = if gen_valid {
                            dep.file_hashes.clone()
                        } else {
                            HashMap::new()
                        };
                        let sroot_path = dep.source_root.as_deref().map(Path::new);
                        crate::format::merge_embedded_hashes_authoritative(
                            &mut combined_hashes,
                            &embedded,
                            sroot_path,
                        );
                        reader_file_hashes.push(combined_hashes);
                        reader_is_dependency.push(true);
                        meta_paths.push(None);
                        meta_mtimes.push(None);
                    }
                }
            }
        }

        if readers.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to open any valid .ridx index files in {:?}", p),
            ));
        }

        let (manifest_path, manifest_mtime, active_variant_id, manifest_ws_root, sysroot_cas_key) =
            match manifest_info {
                Some((mp, mm, avid, _, ws_r, cas_k)) => (Some(mp), mm, Some(avid), ws_r, cas_k),
                None => (None, None, None, None, None),
            };
        let workspace_root = manifest_ws_root.or_else(|| {
            if scan_dir.ends_with("target/ridx") {
                scan_dir
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
            } else {
                Some(p.to_path_buf())
            }
        });

        let unit_match = UnitMatchIndex::build(&readers, &unit_keys, &reader_extern_units);
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
            reader_is_dependency,
            meta_paths,
            meta_mtimes,
            last_reloaded_files: HashSet::new(),
            root_dir: Some(scan_dir),
            workspace_root,
            manifest_path,
            manifest_mtime,
            manifest_len: None,
            manifest_hash: None,
            manifest_generation: None,
            manifest_ino: None,
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

    /// Auto-discovers .ridx indices starting from current directory up to root or target/ridx.
    pub fn auto_discover() -> io::Result<Self> {
        if let Ok(dir) = std::env::var("RIDX_INDEX_DIR") {
            let p = PathBuf::from(dir);
            if p.exists() {
                return Self::open(p);
            }
        }

        let cur = std::env::current_dir()?;
        let mut candidates = vec![cur.join("target/ridx"), cur.join("../target/ridx")];
        let mut nearest_pkg = None;
        let mut enclosing_ws = None;
        for anc in cur.ancestors() {
            let cargo_toml = anc.join("Cargo.toml");
            if cargo_toml.is_file() {
                if nearest_pkg.is_none() {
                    nearest_pkg = Some(anc.to_path_buf());
                }
                if let Ok(content) = fs::read_to_string(&cargo_toml) {
                    if content.contains("[workspace]") {
                        enclosing_ws = Some(anc.to_path_buf());
                        break;
                    }
                }
            }
        }
        if let Some(ws_root) = enclosing_ws.or(nearest_pkg) {
            let ws_target = ws_root.join("target/ridx");
            if !candidates.contains(&ws_target) {
                candidates.push(ws_target);
            }
        }

        let has_ridx_indicators = cur.join("ridx_manifest.json").is_file()
            || cur.join("variant_manifest.json").is_file()
            || fs::read_dir(&cur)
                .ok()
                .map(|mut rd| {
                    rd.any(|e| {
                        e.map(|e| e.path().extension().is_some_and(|ext| ext == "ridx"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
        if has_ridx_indicators {
            candidates.push(cur.clone());
        }

        for cand in candidates {
            if cand.is_dir() {
                if let Ok(ws) = Self::open(&cand) {
                    return Ok(ws);
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not auto-discover target/ridx directory. Specify index path explicitly.",
        ))
    }

    /// Number of crates loaded in this workspace index.
    pub fn crate_count(&self) -> usize {
        self.readers.len()
    }

    /// Rebuilds the internal unit matching index after readers or extern units change.
    pub(crate) fn rebuild_unit_match(&mut self) {
        self.unit_match =
            UnitMatchIndex::build(&self.readers, &self.unit_keys, &self.reader_extern_units);
    }

    /// Bumps generation and rebuilds the internal file path provenance lookup index.
    pub(crate) fn rebuild_path_index(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.path_index = PathIndex::build(self);
    }

    /// Reference to the internal unit matching index.
    #[inline]
    pub fn unit_match(&self) -> &UnitMatchIndex {
        &self.unit_match
    }

    /// Slice of crate index readers.
    pub fn readers(&self) -> &[IndexReader<Mmap>] {
        &self.readers
    }

    /// Paths to the loaded .ridx index files.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Returns the snapshot generation for reader at index i.
    pub fn generation_at(&self, idx: usize) -> Option<&str> {
        self.reader_generations.get(idx).and_then(|g| g.as_deref())
    }

    /// Returns the sidecar metadata path for reader at index i.
    pub fn meta_path_at(&self, idx: usize) -> Option<&Path> {
        self.meta_paths.get(idx).and_then(|p| p.as_deref())
    }

    /// Returns the sidecar metadata modification time for reader at index i.
    pub fn meta_mtime_at(&self, idx: usize) -> Option<std::time::SystemTime> {
        self.meta_mtimes.get(idx).copied().flatten()
    }

    /// Returns the source root path for reader at index i, if known.
    pub fn source_root_at(&self, idx: usize) -> Option<&Path> {
        self.source_roots.get(idx).and_then(|p| p.as_deref())
    }

    /// Aggregated statistics across all crates in the workspace.
    pub fn stats(&self) -> WorkspaceStats {
        let mut stats = WorkspaceStats {
            total_crates: self.readers.len(),
            active_variant: self.active_variant_id.clone(),
            ..Default::default()
        };

        for (i, reader) in self.readers.iter().enumerate() {
            let h = reader.header();
            let size = fs::metadata(&self.paths[i]).map(|m| m.len()).unwrap_or(0);

            stats.total_defs += h.defs_count as usize;
            stats.total_refs += h.refs_count as usize;
            stats.total_types += h.types_count as usize;
            stats.total_files += h.files_count as usize;
            stats.total_bytes += size;

            stats.crate_summaries.push(CrateSummary {
                name: reader.crate_name().to_string(),
                version: reader.crate_version().to_string(),
                path: self.paths[i].clone(),
                file_size: size,
                defs_count: h.defs_count,
                refs_count: h.refs_count,
                types_count: h.types_count,
                files_count: h.files_count,
                is_dependency: self.reader_is_dependency.get(i).copied().unwrap_or(false),
            });
        }

        stats
    }

    /// Currently active build variant ID, if loaded from a variant manifest.
    pub fn active_variant_id(&self) -> Option<&str> {
        self.active_variant_id.as_deref()
    }

    /// Returns the sysroot CAS key from the active variant manifest, if present.
    pub fn sysroot_cas_key(&self) -> Option<&str> {
        self.sysroot_cas_key.as_deref()
    }

    /// Root directory of the workspace if known.
    pub fn workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }

    /// Sets or overrides the workspace root directory.
    pub fn set_workspace_root(&mut self, root: Option<PathBuf>) {
        self.workspace_root = root;
        self.rebuild_path_index();
    }

    /// Checks whether a target file, path, or unit key belongs to an external dependency or outside the workspace.
    pub fn is_external_target(
        &self,
        target_file: &str,
        target_path: &Path,
        target_unit_key: Option<&str>,
    ) -> bool {
        // 1. Explicit sentinel for external reference
        if target_file == "external" {
            return true;
        }

        // 2. Unit-based check: if unit_key is known in the workspace and marked as dependency
        if let Some(unit_key) = target_unit_key {
            for (i, key_opt) in self.unit_keys.iter().enumerate() {
                if key_opt.as_deref() == Some(unit_key) {
                    if self.reader_is_dependency.get(i).copied().unwrap_or(false) {
                        return true;
                    } else {
                        // Known local unit (workspace member or local path dependency)
                        return false;
                    }
                }
            }
            if self.manifest_path.is_some() {
                let is_ws_member = self.unit_keys.iter().enumerate().any(|(i, key_opt)| {
                    key_opt.as_deref() == Some(unit_key)
                        && !self.reader_is_dependency.get(i).copied().unwrap_or(false)
                });
                if !is_ws_member {
                    return true;
                }
            }
        }

        // 3. Path-based check
        if target_file.is_empty() {
            return false;
        }

        if is_external_dependency_path(target_path) {
            return true;
        }

        let effective_root = self.workspace_root().or(self.root_dir.as_deref());

        if let Some(ws_root) = effective_root {
            if target_path.is_absolute() {
                let in_ws = target_path.starts_with(ws_root)
                    || match (target_path.canonicalize(), ws_root.canonicalize()) {
                        (Ok(tp), Ok(wp)) => tp.starts_with(&wp),
                        _ => false,
                    };
                if !in_ws {
                    return true;
                }
            }
        }

        false
    }

    /// Checks whether an arbitrary path belongs to an external dependency or outside the workspace.
    pub fn is_external_path(&self, path: &Path) -> bool {
        let s = path.to_str().unwrap_or("");
        self.is_external_target(s, path, None)
    }

    /// Returns whether the reader at index `idx` is marked as a dependency.
    #[inline]
    pub fn is_reader_dependency(&self, idx: usize) -> bool {
        self.reader_is_dependency.get(idx).copied().unwrap_or(false)
    }

    /// Checks whether the reader at index `idx` belongs to an external dependency or outside the workspace.
    pub fn is_reader_external(&self, idx: usize) -> bool {
        if self.is_reader_dependency(idx) {
            return true;
        }

        let uk = self.unit_keys.get(idx).and_then(|u| u.as_deref());
        let sroot = self.source_root_at(idx);
        let p = self.paths.get(idx).map(|p| p.as_path());
        let path_to_check = sroot.or(p).unwrap_or_else(|| Path::new(""));
        let file_str = path_to_check.to_str().unwrap_or("");

        self.is_external_target(file_str, path_to_check, uk)
    }

    /// Checks whether two readers represent sibling units of the same workspace package
    /// (e.g. `lib` and `lib test` shards of the same package).
    ///
    /// Sibling units share the same non-external source root and crate name, but may have
    /// distinct unit keys (e.g. `unit_lib` vs `unit_test`).
    pub fn are_sibling_workspace_units(&self, idx1: usize, idx2: usize) -> bool {
        if idx1 >= self.readers.len() || idx2 >= self.readers.len() {
            return false;
        }
        if self.is_reader_external(idx1) || self.is_reader_external(idx2) {
            return false;
        }
        if idx1 == idx2 {
            return true;
        }
        if self.readers[idx1].crate_name() != self.readers[idx2].crate_name() {
            return false;
        }
        let (Some(sr1), Some(sr2)) = (self.source_root_at(idx1), self.source_root_at(idx2)) else {
            return false;
        };
        sr1 == sr2
            || match (sr1.canonicalize(), sr2.canonicalize()) {
                (Ok(c1), Ok(c2)) => c1 == c2,
                _ => false,
            }
    }

    /// Returns the unique instance ID for this workspace.
    #[inline]
    pub fn instance_id(&self) -> u64 {
        self.instance_id
    }

    /// Returns the current index generation counter.
    /// Incremented on each index reload or modification.
    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Explicitly bumps the index generation counter.
    #[inline]
    pub fn bump_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
}

/// Checks whether a path points to an external dependency cache or toolchain
/// (e.g. Cargo registry, Cargo git checkouts, Rustup toolchains, standard library).
pub fn is_external_dependency_path(path: &Path) -> bool {
    if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
        let ch = Path::new(&cargo_home);
        if path.starts_with(ch.join("registry")) || path.starts_with(ch.join("git")) {
            return true;
        }
    }

    let path_str = path.to_string_lossy();
    path_str.contains("/.cargo/registry/")
        || path_str.contains(r"\.cargo\registry\")
        || path_str.contains("/.cargo/git/")
        || path_str.contains(r"\.cargo\git\")
        || path_str.contains("/.rustup/toolchains/")
        || path_str.contains(r"\.rustup\toolchains\")
        || path_str.contains("/lib/rustlib/")
        || path_str.contains(r"\lib\rustlib\")
        || path_str.contains("/target/ridx/dependencies/")
}

pub(crate) fn collect_ridx_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    collect_ridx_files_bounded(dir, files, 0, 2)
}

fn collect_ridx_files_bounded(
    dir: &Path,
    files: &mut Vec<PathBuf>,
    depth: usize,
    max_depth: usize,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_file() {
            if p.extension().is_some_and(|ext| ext == "ridx") {
                files.push(p);
            }
        } else if p.is_dir() {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.starts_with('.')
                    || name == "variants"
                    || name == "target"
                    || name == "node_modules"
                    || name == "vendor"
                {
                    continue;
                }
            }
            let _ = collect_ridx_files_bounded(&p, files, depth + 1, max_depth);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::DefKind;
    use crate::writer::{IndexWriter, RawDefinition};
    use std::fs::File;

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

    #[test]
    fn test_collect_ridx_files_depth_bounding_and_exclusions() {
        let test_dir = new_test_dir("collect_depth_test");
        let root = &test_dir.0;
        let d1 = root.join("lvl1");
        let d2 = d1.join("lvl2");
        let d3 = d2.join("lvl3");
        let hidden = root.join(".hidden");
        let target = root.join("target");
        let node_modules = root.join("node_modules");

        fs::create_dir_all(&d3).unwrap();
        fs::create_dir_all(&hidden).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&node_modules).unwrap();

        fs::write(root.join("root.ridx"), b"").unwrap();
        fs::write(d1.join("lvl1.ridx"), b"").unwrap();
        fs::write(d2.join("lvl2.ridx"), b"").unwrap();
        fs::write(d3.join("lvl3.ridx"), b"").unwrap();
        fs::write(hidden.join("hidden.ridx"), b"").unwrap();
        fs::write(target.join("target.ridx"), b"").unwrap();
        fs::write(node_modules.join("nm.ridx"), b"").unwrap();

        let mut collected = Vec::new();
        collect_ridx_files(root, &mut collected).unwrap();

        let names: Vec<String> = collected
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(names.contains(&"root.ridx".to_string()));
        assert!(names.contains(&"lvl1.ridx".to_string()));
        assert!(names.contains(&"lvl2.ridx".to_string()));
        assert!(!names.contains(&"lvl3.ridx".to_string()));
        assert!(!names.contains(&"hidden.ridx".to_string()));
        assert!(!names.contains(&"target.ridx".to_string()));
        assert!(!names.contains(&"nm.ridx".to_string()));
    }

    #[test]
    fn test_workspace_index_accessors_and_stats() {
        let _lock = ENV_LOCK.lock().unwrap();
        let test_dir = new_test_dir("ridx_ws_mod_test");
        let idx = test_dir.0.join("sample.ridx");

        let mut writer = IndexWriter::new("sample", "0.1.0");
        writer.set_generation("gen123");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "test_fn".to_string(),
            path: "sample::test_fn".to_string(),
            signature: Some("fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 2,
            end_col: 2,
        });
        let mut f = File::create(&idx).unwrap();
        writer.write_to(&mut f).unwrap();

        // 1. Open single file
        let ws_file = WorkspaceIndex::open(&idx).unwrap();
        assert_eq!(ws_file.crate_count(), 1);
        assert_eq!(ws_file.readers().len(), 1);
        assert_eq!(ws_file.paths().len(), 1);
        let _ = ws_file.meta_path_at(0);
        let _ = ws_file.meta_mtime_at(0);
        assert!(ws_file.active_variant_id().is_none());
        assert!(ws_file.workspace_root().is_none());
        let mut ws_mut = ws_file;
        ws_mut.set_workspace_root(Some(PathBuf::from("/test_root")));
        assert_eq!(ws_mut.workspace_root(), Some(Path::new("/test_root")));
        let ws_file = ws_mut;

        let stats = ws_file.stats();
        assert_eq!(stats.total_crates, 1);
        assert_eq!(stats.total_defs, 1);
        assert_eq!(stats.crate_summaries.len(), 1);
        assert_eq!(stats.crate_summaries[0].name, "sample");

        // 2. Open directory
        let ws_dir = WorkspaceIndex::open(&test_dir.0).unwrap();
        assert_eq!(ws_dir.crate_count(), 1);

        // 3. Test collect_ridx_files ignores variants dir
        let variants_dir = test_dir.0.join("variants");
        fs::create_dir_all(&variants_dir).unwrap();
        let var_idx = variants_dir.join("ignored.ridx");
        let mut f2 = File::create(&var_idx).unwrap();
        writer.write_to(&mut f2).unwrap();

        let mut collected = Vec::new();
        collect_ridx_files(&test_dir.0, &mut collected).unwrap();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0], idx);

        // 4. Auto-discover with RIDX_INDEX_DIR
        std::env::set_var("RIDX_INDEX_DIR", test_dir.0.to_str().unwrap());
        let ws_auto = WorkspaceIndex::auto_discover().unwrap();
        assert_eq!(ws_auto.crate_count(), 1);
        std::env::remove_var("RIDX_INDEX_DIR");
    }

    #[test]
    fn test_open_with_variant_manifest_and_dependency_shards() {
        let _lock = ENV_LOCK.lock().unwrap();
        let test_dir = new_test_dir("ridx_open_vmanifest");
        let idx = test_dir.0.join("sample.ridx");
        let mut writer = IndexWriter::new("sample", "0.1.0");
        writer.set_unit_key("sample_uk");
        writer.set_generation("sample_gen");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "sample_fn".to_string(),
            path: "sample::sample_fn".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 16,
        });
        let mut f = File::create(&idx).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let ext_cache = test_dir.0.join("external_cache");
        fs::create_dir_all(&ext_cache).unwrap();
        let dep_idx = ext_cache.join("dep_crate.ridx");
        let mut dep_writer = IndexWriter::new("dep_crate", "0.1.0");
        dep_writer.set_unit_key("dep_uk");
        dep_writer.set_generation("dep_gen");
        dep_writer.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "dep_fn".to_string(),
            path: "dep_crate::dep_fn".to_string(),
            signature: None,
            file: "src/dep.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 16,
        });
        let mut f2 = File::create(&dep_idx).unwrap();
        dep_writer.write_to(&mut f2).unwrap();
        drop(f2);

        let mismatch_idx = ext_cache.join("mismatch_crate.ridx");
        let mut mismatch_writer = IndexWriter::new("mismatch_crate", "0.1.0");
        mismatch_writer.set_unit_key("mismatch_uk");
        mismatch_writer.set_generation("dep_gen");
        let mut f3 = File::create(&mismatch_idx).unwrap();
        mismatch_writer.write_to(&mut f3).unwrap();
        drop(f3);

        let v_manifest = test_dir.0.join("variant_manifest.json");
        let manifest_data = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
            "active_variant_id": "v1",
            "variant_dir": "variants/v1",
            "workspace_root": test_dir.0.display().to_string(),
            "dependency_shards": [
                {
                    "name": "dep_crate",
                    "version": "0.1.0",
                    "unit_key": "dep_uk",
                    "generation": "dep_gen",
                    "object_path": dep_idx.display().to_string(),
                    "is_dependency": true,
                    "dependencies": ["some_dep"],
                    "extern_units": {"some_dep": "some_uk"},
                    "file_hashes": {"src/dep.rs": 1234}
                },
                {
                    "name": "mismatch_crate",
                    "version": "0.1.0",
                    "unit_key": "mismatch_uk",
                    "generation": "wrong_gen",
                    "object_path": mismatch_idx.display().to_string(),
                    "is_dependency": true,
                    "dependencies": ["other_dep"],
                    "extern_units": {"other_dep": "other_uk"},
                    "file_hashes": {"src/dep.rs": 5678}
                }
            ]
        });
        fs::write(&v_manifest, serde_json::to_string(&manifest_data).unwrap()).unwrap();

        let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        assert_eq!(ws.crate_count(), 3);
    }

    #[test]
    fn test_workspace_open_validation_errors() {
        let _lock = ENV_LOCK.lock().unwrap();
        let test_dir = new_test_dir("ridx_ws_errs");

        // 1. Invalid json in ridx_manifest.json
        let m_path = test_dir.0.join("ridx_manifest.json");
        fs::write(&m_path, "{ invalid_json }").unwrap();
        assert!(WorkspaceIndex::open(&test_dir.0).is_err());

        // 2. Extractor version mismatch in ridx_manifest.json
        let bad_ext = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": "9999",
            "active_variant_id": "v1",
            "variant_dir": "variants/v1",
        });
        let v1_dir = test_dir.0.join("variants/v1");
        fs::create_dir_all(&v1_dir).unwrap();
        fs::write(&m_path, serde_json::to_string(&bad_ext).unwrap()).unwrap();
        assert!(WorkspaceIndex::open(&test_dir.0).is_err());

        // 3. Extractor version missing in ridx_manifest.json
        let missing_ext = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "active_variant_id": "v1",
            "variant_dir": "variants/v1",
        });
        fs::write(&m_path, serde_json::to_string(&missing_ext).unwrap()).unwrap();
        assert!(WorkspaceIndex::open(&test_dir.0).is_err());

        // Clean up ridx_manifest.json
        fs::remove_file(&m_path).unwrap();

        // 4. variant_manifest.json errors
        let vm_path = test_dir.0.join("variant_manifest.json");
        fs::write(&vm_path, "{ invalid_json }").unwrap();
        assert!(WorkspaceIndex::open(&test_dir.0).is_err());

        // 5. format version mismatch in variant_manifest.json
        let bad_fmt = serde_json::json!({
            "version": "2.0",
            "format_version": 9999,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
        });
        fs::write(&vm_path, serde_json::to_string(&bad_fmt).unwrap()).unwrap();
        assert!(WorkspaceIndex::open(&test_dir.0).is_err());

        // 6. extractor version mismatch in variant_manifest.json
        let bad_vm_ext = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": "9999",
        });
        fs::write(&vm_path, serde_json::to_string(&bad_vm_ext).unwrap()).unwrap();
        assert!(WorkspaceIndex::open(&test_dir.0).is_err());

        // 7. extractor version missing in variant_manifest.json
        let missing_vm_ext = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
        });
        fs::write(&vm_path, serde_json::to_string(&missing_vm_ext).unwrap()).unwrap();
        assert!(WorkspaceIndex::open(&test_dir.0).is_err());
    }

    #[test]
    fn test_workspace_open_target_ridx_and_dep_computed_gen() {
        let _lock = ENV_LOCK.lock().unwrap();
        let test_dir = new_test_dir("ridx_target_open");
        // 1. Empty directory open errors (lines 479-482)
        let empty_dir = test_dir.0.join("empty");
        fs::create_dir_all(&empty_dir).unwrap();
        let err = WorkspaceIndex::open(&empty_dir);
        assert_eq!(err.unwrap_err().kind(), io::ErrorKind::NotFound);

        // 2. Open single file inside target/ridx (lines 93-96)
        let target_ridx = test_dir.0.join("target/ridx");
        fs::create_dir_all(&target_ridx).unwrap();
        let single_file = target_ridx.join("single.ridx");
        let mut w_single = IndexWriter::new("single", "0.1.0");
        let mut f_single = File::create(&single_file).unwrap();
        w_single.write_to(&mut f_single).unwrap();
        drop(f_single);

        let ws_single = WorkspaceIndex::open(&single_file).unwrap();
        assert_eq!(ws_single.workspace_root, Some(test_dir.0.clone()));

        // 3. Open directory ending in target/ridx (lines 492-495)
        let ws_dir = WorkspaceIndex::open(&target_ridx).unwrap();
        assert_eq!(ws_dir.workspace_root, Some(test_dir.0.clone()));

        // 4. Dependency shard with computed generation from file_hashes (lines 423-430)
        let dep_file = test_dir.0.join("dep_comp.ridx");
        let mut w_dep = IndexWriter::new("dep_comp", "0.1.0");
        w_dep.set_unit_key("dep_comp_key");
        let mut file_hashes = HashMap::new();
        file_hashes.insert("src/lib.rs".to_string(), 12345u64);
        let computed_digest = crate::format::compute_source_snapshot_digest(&file_hashes);
        w_dep.set_generation(format!("{:016x}", computed_digest));
        let mut f_dep = File::create(&dep_file).unwrap();
        w_dep.write_to(&mut f_dep).unwrap();
        drop(f_dep);

        let dep_file_mismatch = test_dir.0.join("dep_mismatch.ridx");
        let mut w_dep_m = IndexWriter::new("dep_mismatch", "0.1.0");
        w_dep_m.set_unit_key("dep_mismatch_key");
        w_dep_m.set_generation("mismatched_generation_val");
        let mut f_dep_m = File::create(&dep_file_mismatch).unwrap();
        w_dep_m.write_to(&mut f_dep_m).unwrap();
        drop(f_dep_m);

        let v_comp = target_ridx.join("variants/v_comp");
        fs::create_dir_all(&v_comp).unwrap();
        let mut f_s2 = File::create(v_comp.join("single.ridx")).unwrap();
        w_single.write_to(&mut f_s2).unwrap();
        drop(f_s2);

        let mf = target_ridx.join("ridx_manifest.json");
        let manifest_data = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
            "active_variant_id": "v_comp",
            "variant_dir": "variants/v_comp",
            "workspace_root": test_dir.0.display().to_string(),
            "shards": [{ "name": "single", "version": "0.1.0", "file": "single.ridx", "is_dependency": false }],
            "dependency_shards": [
                {
                    "name": "dep_comp",
                    "version": "0.1.0",
                    "unit_key": "dep_comp_key",
                    "object_path": dep_file.display().to_string(),
                    "source_root": null,
                    "is_dependency": true,
                    "generation": null,
                    "file_hashes": { "src/lib.rs": 12345 },
                    "dependencies": [],
                    "extern_units": {}
                },
                {
                    "name": "dep_mismatch",
                    "version": "0.1.0",
                    "unit_key": "dep_mismatch_key",
                    "object_path": dep_file_mismatch.display().to_string(),
                    "source_root": null,
                    "is_dependency": true,
                    "generation": null,
                    "file_hashes": { "src/other.rs": 99999 },
                    "dependencies": [],
                    "extern_units": {}
                }
            ]
        });
        fs::write(&mf, serde_json::to_string(&manifest_data).unwrap()).unwrap();

        let ws_comp = WorkspaceIndex::open(&target_ridx).unwrap();
        assert_eq!(ws_comp.readers.len(), 3);
    }

    #[test]
    fn test_corrupted_index_skipping_and_auto_discover() {
        let _lock = ENV_LOCK.lock().unwrap();
        let test_dir = new_test_dir("ws_corrupt_test");
        let valid_file = test_dir.0.join("valid.ridx");
        let mut w = IndexWriter::new("valid_crate", "0.1.0");
        let mut f = File::create(&valid_file).unwrap();
        w.write_to(&mut f).unwrap();
        drop(f);

        // Put a corrupted .ridx file in the same directory (lines 342-344)
        let corrupt_file = test_dir.0.join("corrupt.ridx");
        fs::write(&corrupt_file, b"NOT_A_VALID_RIDX_FILE_HEADER").unwrap();

        // WorkspaceIndex::open should skip the corrupt file and successfully load valid.ridx
        let ws = WorkspaceIndex::open(&test_dir.0).unwrap();
        assert_eq!(ws.readers.len(), 1);
        assert_eq!(ws.readers[0].crate_name(), "valid_crate");

        // Test auto_discover fallback (lines 549-552)
        let deep_empty = new_test_dir("ws_empty_discover").0.join("deep/sub/dir");
        fs::create_dir_all(&deep_empty).unwrap();
        let orig_dir = std::env::current_dir().unwrap();
        std::env::remove_var("RIDX_INDEX_DIR");
        std::env::set_current_dir(&deep_empty).unwrap();
        let res = WorkspaceIndex::auto_discover();
        assert!(res.is_err());
        std::env::set_current_dir(orig_dir).unwrap();
    }

    #[test]
    fn test_workspace_open_edge_cases() {
        let _lock = ENV_LOCK.lock().unwrap();
        // 1. Directory with no .ridx files (lines 291-296)
        let empty_dir = new_test_dir("ws_no_ridx");
        let res = WorkspaceIndex::open(&empty_dir.0);
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("No .ridx index files found"));

        // 1b. Directory with only invalid .ridx files (lines 479-482)
        let corrupt_dir = new_test_dir("ws_all_corrupt");
        fs::write(corrupt_dir.0.join("bad.ridx"), b"bad").unwrap();
        let res_corrupt = WorkspaceIndex::open(&corrupt_dir.0);
        assert!(res_corrupt
            .unwrap_err()
            .to_string()
            .contains("Failed to open any valid"));

        // 3. Manifest with empty workspace_root (line 213) and empty active_variant_id (line 219)
        let ws_dir = new_test_dir("ws_legacy_manifest");
        let valid_file = ws_dir.0.join("pkg.ridx");
        let mut w = IndexWriter::new("legacy_pkg", "0.1.0");
        w.set_unit_key("unit_legacy");
        w.set_generation("gen_legacy");
        let mut f = File::create(&valid_file).unwrap();
        w.write_to(&mut f).unwrap();
        drop(f);

        // Variant manifest with empty workspace_root and active_variant_id
        let manifest_json = serde_json::json!({
            "version": "2.0",
            "format_version": crate::format::CURRENT_FORMAT_VERSION,
            "extractor_version": crate::format::EXTRACTOR_VERSION,
            "active_variant_id": "", // empty active_variant_id (line 219)
            "variant_dir": "",
            "base_out_dir": ws_dir.0.display().to_string(),
            "workspace_root": "", // empty workspace_root (line 213)
            "manifest_path": "",
            "profile": "dev",
            "timestamp_epoch_secs": 0,
            "total_crates": 1,
            "total_defs": 0,
            "total_refs": 0,
            "total_types": 0,
            "dependency_shards": [
                {
                    "name": "legacy_pkg",
                    "version": "0.1.0",
                    "unit_key": "different_unit_key", // unit mismatch (line 412)
                    "generation": null,
                    "object_path": valid_file.display().to_string(),
                    "source_root": null,
                    "is_dependency": true,
                    "dependencies": [],
                    "extern_units": {},
                    "file_hashes": { "src/lib.rs": 999999 } // digest mismatch with gen_legacy (lines 428-430)
                }
            ],
            "crates": []
        });
        fs::write(
            ws_dir.0.join("ridx_manifest.json"),
            serde_json::to_string(&manifest_json).unwrap(),
        )
        .unwrap();

        let ws = WorkspaceIndex::open(&ws_dir.0).unwrap();
        assert_eq!(ws.readers.len(), 1);
        assert_eq!(ws.readers[0].crate_name(), "legacy_pkg");

        // 4. Open search_dir with variant_manifest.json and empty workspace_root (lines 221-281, 278)
        let v_dir = new_test_dir("ws_v_manifest");
        let v_file = v_dir.0.join("v_pkg.ridx");
        let mut wv = IndexWriter::new("v_pkg", "0.1.0");
        wv.set_unit_key("unit_v");
        let mut fv = File::create(&v_file).unwrap();
        wv.write_to(&mut fv).unwrap();
        drop(fv);
        let mut v_manifest = manifest_json.clone();
        v_manifest["extractor_version"] =
            serde_json::Value::String(crate::format::EXTRACTOR_VERSION.to_string());
        v_manifest["workspace_root"] = serde_json::Value::String("".to_string());
        fs::write(
            v_dir.0.join("variant_manifest.json"),
            serde_json::to_string(&v_manifest).unwrap(),
        )
        .unwrap();
        let ws_v = WorkspaceIndex::open(&v_dir.0).unwrap();
        assert_eq!(ws_v.readers.len(), 1);
        assert_eq!(ws_v.readers[0].crate_name(), "v_pkg");
        assert_eq!(ws_v.workspace_root, Some(v_dir.0.clone()));

        // 5. Test auto_discover with RIDX_INDEX_DIR set (line 531)
        std::env::set_var("RIDX_INDEX_DIR", v_dir.0.display().to_string());
        let ws_auto = WorkspaceIndex::auto_discover().unwrap();
        assert_eq!(ws_auto.readers.len(), 1);
        std::env::remove_var("RIDX_INDEX_DIR");

        // Test Debug formatting for WorkspaceIndex (lines 52-57)
        let dbg_str = format!("{:?}", ws_auto);
        assert!(dbg_str.contains("WorkspaceIndex"));

        // 6. Manifest with active_variant_id and empty workspace_root (line 210)
        let ws_m_dir = new_test_dir("ws_m_empty_root");
        let m_file = ws_m_dir.0.join("pkg.ridx");
        let mut wm = IndexWriter::new("pkg", "0.1.0");
        wm.set_generation("gen_target");
        let mut fm = File::create(&m_file).unwrap();
        wm.write_to(&mut fm).unwrap();
        drop(fm);

        let v_empty_dir = ws_m_dir.0.join("variants/var_empty");
        let _ = fs::create_dir_all(&v_empty_dir);
        let _ = fs::copy(&m_file, v_empty_dir.join("pkg.ridx"));
        let mut m_empty_ws = manifest_json.clone();
        m_empty_ws["extractor_version"] =
            serde_json::Value::String(crate::format::EXTRACTOR_VERSION.to_string());
        m_empty_ws["active_variant_id"] = serde_json::Value::String("var_empty".to_string());
        m_empty_ws["variant_dir"] = serde_json::Value::String("variants/var_empty".to_string());
        m_empty_ws["workspace_root"] = serde_json::Value::String("".to_string());
        fs::write(
            ws_m_dir.0.join("ridx_manifest.json"),
            serde_json::to_string(&m_empty_ws).unwrap(),
        )
        .unwrap();
        let ws_m = WorkspaceIndex::open(&ws_m_dir.0).unwrap();
        assert_eq!(ws_m.workspace_root, Some(ws_m_dir.0.clone()));

        // 7. Dep shards: fallback to cache_dir (lines 353-355), missing dep (line 357),
        // digest mismatch with null generation (lines 425, 427), and corrupt reader (lines 468-469)
        let cache_dir = ws_m_dir.0.join("cache");
        let _ = fs::create_dir_all(&cache_dir);
        std::env::set_var("RIDX_CACHE_DIR", cache_dir.display().to_string());
        let cached_obj = crate::format::cache_object_path(&cache_dir, "cached_unit");
        let _ = fs::create_dir_all(cached_obj.parent().unwrap());
        let mut w_cached = IndexWriter::new("cached_dep", "0.1.0");
        w_cached.set_unit_key("cached_unit");
        w_cached.set_generation("gen_cached");
        let mut fc = File::create(&cached_obj).unwrap();
        w_cached.write_to(&mut fc).unwrap();
        drop(fc);

        let corrupt_file = ws_m_dir.0.join("corrupt.ridx");
        fs::write(&corrupt_file, b"NOT_A_VALID_RIDX").unwrap();

        let mut m_cache = manifest_json.clone();
        m_cache["extractor_version"] =
            serde_json::Value::String(crate::format::EXTRACTOR_VERSION.to_string());
        m_cache["dependency_shards"] = serde_json::json!([
            {
                "name": "cached_dep",
                "version": "0.1.0",
                "unit_key": "cached_unit",
                "generation": null,
                "object_path": "/nonexistent/object.ridx",
                "source_root": null,
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": { "src/lib.rs": 12345 } // hits line 425: computed_gen != "gen_cached"
            },
            {
                "name": "missing_dep",
                "version": "0.1.0",
                "unit_key": "missing_unit",
                "generation": null,
                "object_path": "/nonexistent/missing.ridx",
                "source_root": null,
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": {}
            },
            {
                "name": "corrupt_dep",
                "version": "0.1.0",
                "unit_key": "corrupt_unit",
                "generation": null,
                "object_path": corrupt_file.display().to_string(),
                "source_root": null,
                "is_dependency": true,
                "dependencies": [],
                "extern_units": {},
                "file_hashes": {}
            }
        ]);
        let _ = fs::remove_file(ws_m_dir.0.join("ridx_manifest.json"));
        fs::write(
            ws_m_dir.0.join("variant_manifest.json"),
            serde_json::to_string(&m_cache).unwrap(),
        )
        .unwrap();
        let ws_cache = WorkspaceIndex::open(&ws_m_dir.0).unwrap();
        assert!(ws_cache
            .readers
            .iter()
            .any(|r| r.crate_name() == "cached_dep"));
        let _ = fs::remove_file(&cached_obj);
        std::env::remove_var("RIDX_CACHE_DIR");

        // 8. Opening single file with no manifest yields None workspace_root (line 494)
        let single_dir = new_test_dir("single_file_ws");
        let single_file = single_dir.0.join("standalone.ridx");
        let mut ws_standalone = IndexWriter::new("standalone", "0.1.0");
        let mut fs_s = File::create(&single_file).unwrap();
        ws_standalone.write_to(&mut fs_s).unwrap();
        drop(fs_s);
        let ws_single = WorkspaceIndex::open(&single_file).unwrap();
        assert_eq!(ws_single.workspace_root, None);

        // 9. auto_discover with nonexistent RIDX_INDEX_DIR (line 526)
        std::env::set_var("RIDX_INDEX_DIR", "/nonexistent/index_dir_9999");
        let _ = WorkspaceIndex::auto_discover();
        std::env::remove_var("RIDX_INDEX_DIR");

        // 10. collect_ridx_files with non-ridx file and variants dir (lines 634, 640)
        let coll_dir = new_test_dir("collect_test");
        fs::write(coll_dir.0.join("ignore.txt"), b"not ridx").unwrap();
        let var_sub = coll_dir.0.join("variants");
        let _ = fs::create_dir_all(&var_sub);
        fs::write(var_sub.join("nested.ridx"), b"").unwrap();
        let normal_sub = coll_dir.0.join("normal_sub");
        let _ = fs::create_dir_all(&normal_sub);
        fs::write(normal_sub.join("found.ridx"), b"").unwrap();
        let mut collected = Vec::new();
        collect_ridx_files(&coll_dir.0, &mut collected).unwrap();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0], normal_sub.join("found.ridx"));
    }
}
