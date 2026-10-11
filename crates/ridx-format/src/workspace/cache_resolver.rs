/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Resilient cache lookup for external dependency index shards.
//!
//! When a workspace manifest references a dependency object path or unit key from an older
//! compiler driver run that no longer exists on disk, `CacheObjectResolver` discovers candidate
//! shards in `cache_dir/objects/` strictly matching the crate's name, version, and 64-bit generation digest.

use crate::reader::IndexReader;
use crate::workspace::manifest::DependencyShardInfo;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CachedObjectInfo {
    version: String,
    generation: String,
    unit_key: String,
    path: PathBuf,
}

/// Resolves dependency index shard files from primary paths or the global cache directory.
pub struct CacheObjectResolver {
    cache_dir: PathBuf,
    cache_map: Option<HashMap<String, Vec<CachedObjectInfo>>>,
}

impl CacheObjectResolver {
    /// Creates a new resolver backed by the specified global cache directory.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            cache_map: None,
        }
    }

    /// Resolves an existing `.ridx` shard file for the given dependency shard.
    ///
    /// Resolution order:
    /// 1. Exact `dep.object_path` if it exists on disk.
    /// 2. `cache_object_path(cache_dir, dep.unit_key)` if it exists on disk.
    /// 3. Resilient fallback: scans `cache_dir/objects/*/*.ridx` for shards matching `dep.name`,
    ///    strictly requiring exact version and generation match.
    pub fn resolve(&mut self, dep: &DependencyShardInfo) -> Option<PathBuf> {
        let obj_path = PathBuf::from(&dep.object_path);
        if obj_path.is_file() {
            return Some(obj_path);
        }

        let fallback = crate::format::cache_object_path(&self.cache_dir, &dep.unit_key);
        if fallback.is_file() {
            return Some(fallback);
        }

        self.resolve_resilient(dep)
    }

    fn resolve_resilient(&mut self, dep: &DependencyShardInfo) -> Option<PathBuf> {
        let expected_gen = match dep.generation.as_deref() {
            Some(g) if !g.is_empty() => g,
            _ => return None,
        };

        if self.cache_map.is_none() {
            self.populate_cache_map();
        }

        let map = self.cache_map.as_ref()?;
        let candidates = map.get(&dep.name)?;

        // Invariant & Design Decision:
        // When exact object_path and unit_key fail to resolve (e.g. following a rebuild of
        // ridx-driver or toolchain bump where driver hash shifts across unit_keys), resolve_resilient
        // discovers candidate shards matching exact crate name, semantic version, and generation digest.
        //
        // While unit_key additionally hashes active Cargo features, compilation target, and driver
        // version, `generation` is a 64-bit content digest over source file contents.
        // In LSP and code navigation workflows, reusing a source-identical shard of the same version
        // across targets or features is an accepted compromise to preserve cross-crate intelligence
        // and avoid full dependency re-indexing when the driver or toolchain updates;
        // steps 1 and 2 are tried first.
        candidates
            .iter()
            .find(|c| c.version == dep.version && c.generation == expected_gen)
            .map(|c| c.path.clone())
    }

    fn populate_cache_map(&mut self) {
        let mut map: HashMap<String, Vec<CachedObjectInfo>> = HashMap::new();
        let objects_dir = self.cache_dir.join("objects");

        if let Ok(entries) = fs::read_dir(&objects_dir) {
            for entry in entries.flatten() {
                let sub = entry.path();
                if sub.is_dir() {
                    if let Ok(files) = fs::read_dir(&sub) {
                        for f in files.flatten() {
                            let p = f.path();
                            if p.extension().and_then(|s| s.to_str()) == Some("ridx") {
                                if let Ok(reader) = IndexReader::open(&p) {
                                    let name = reader.crate_name().to_string();
                                    if !name.is_empty() {
                                        map.entry(name).or_default().push(CachedObjectInfo {
                                            version: reader.crate_version().to_string(),
                                            generation: reader.generation().to_string(),
                                            unit_key: reader.unit_key().to_string(),
                                            path: p,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.cache_map = Some(map);
    }
}
