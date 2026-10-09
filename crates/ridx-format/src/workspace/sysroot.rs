/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Discovery and attachment of cached sysroot index shards (`core`, `alloc`, `std`).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::reader::IndexReader;

/// Discovers cached sysroot shard paths (`core.ridx`, `alloc.ridx`, `std.ridx`) for a specific CAS key
/// inside the global cache directory `~/.cache/ridx/v3/sysroot/<cas_key>/`.
pub fn discover_sysroot_shards_for_key(cas_key: &str) -> Vec<PathBuf> {
    let base_cache = crate::resolve_cache_dir().join("sysroot");
    discover_sysroot_shards_for_key_in(&base_cache, cas_key)
}

/// Discovers cached sysroot shard paths for a specific CAS key inside a specified base cache path.
pub fn discover_sysroot_shards_for_key_in(base_cache: &Path, cas_key: &str) -> Vec<PathBuf> {
    let target_dir = base_cache.join(cas_key);
    if !target_dir.is_dir() {
        return Vec::new();
    }

    let mut shards = Vec::new();
    for cname in ["core", "alloc", "std"] {
        let p = target_dir.join(format!("{cname}.ridx"));
        if p.is_file() {
            shards.push(p);
        }
    }
    shards
}

impl super::WorkspaceIndex {
    /// Returns true if sysroot shards (`core`) are attached to the workspace.
    pub fn has_sysroot(&self) -> bool {
        self.readers.iter().any(|r| r.crate_name() == "core")
    }

    /// Attaches cached sysroot shards for the given `cas_key` to the workspace if not already present.
    pub fn attach_sysroot_shards_for_key(&mut self, cas_key: &str) {
        if self.has_sysroot() {
            return;
        }
        let shards = discover_sysroot_shards_for_key(cas_key);
        if shards.is_empty() {
            static LOGGED_ONCE: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !LOGGED_ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!(
                    "[ridx] Warning: no sysroot shards found for cas_key {:?} in cache",
                    cas_key
                );
            }
            return;
        }
        self.attach_shard_paths(&shards);
    }

    /// Attaches cached sysroot shards for the given `cas_key` located in `base_cache`.
    pub fn attach_sysroot_shards_for_key_in(&mut self, base_cache: &Path, cas_key: &str) {
        if self.has_sysroot() {
            return;
        }
        let shards = discover_sysroot_shards_for_key_in(base_cache, cas_key);
        self.attach_shard_paths(&shards);
    }

    pub fn attach_shard_paths(&mut self, shards: &[PathBuf]) {
        for sys_path in shards {
            if let Ok(reader) = IndexReader::open(sys_path) {
                let mtime = fs::metadata(sys_path).and_then(|m| m.modified()).ok();
                let ukey = reader.unit_key().to_string();
                let gen = if !reader.generation().is_empty() {
                    Some(reader.generation().to_string())
                } else {
                    None
                };
                self.readers.push(reader);
                self.paths.push(sys_path.clone());
                self.mtimes.push(mtime);
                self.source_roots.push(None);
                self.unit_keys.push(Some(ukey));
                self.reader_generations.push(gen);
                self.reader_dependencies.push(Vec::new());
                self.reader_extern_units.push(HashMap::new());
                self.reader_file_hashes.push(HashMap::new());
                self.reader_is_dependency.push(true);
                self.meta_paths.push(None);
                self.meta_mtimes.push(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IndexWriter;

    #[test]
    fn test_discover_and_attach_sysroot() {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_sysroot_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        let base_cache = temp_dir.join("sysroot");
        let sys_dir = base_cache.join("test_cas_key");
        fs::create_dir_all(&sys_dir).unwrap();

        for cname in ["core", "alloc", "std"] {
            let mut writer = IndexWriter::new(cname, "1.97.1");
            writer.set_unit_key(format!("{cname}-1.97.1"));
            let mut buf = Vec::new();
            writer
                .write_to(&mut std::io::Cursor::new(&mut buf))
                .unwrap();
            fs::write(sys_dir.join(format!("{cname}.ridx")), buf).unwrap();
        }

        let shards = discover_sysroot_shards_for_key_in(&base_cache, "test_cas_key");
        assert_eq!(shards.len(), 3, "must discover 3 sysroot shards");

        // Create a dummy project directory with a crate index
        let proj_dir = temp_dir.join("user_crate");
        fs::create_dir_all(&proj_dir).unwrap();
        let mut user_writer = IndexWriter::new("user_crate", "0.1.0");
        user_writer.set_unit_key("user_crate-0.1.0");
        let mut user_buf = Vec::new();
        user_writer
            .write_to(&mut std::io::Cursor::new(&mut user_buf))
            .unwrap();
        fs::write(proj_dir.join("user_crate.ridx"), user_buf).unwrap();

        let mut ws = super::super::WorkspaceIndex::open(&proj_dir).expect("open must succeed");
        assert_eq!(ws.readers.len(), 1);

        ws.attach_sysroot_shards_for_key_in(&base_cache, "test_cas_key");
        // Must contain user_crate + core + alloc + std
        assert_eq!(ws.readers.len(), 4);
        assert_eq!(ws.readers[0].crate_name(), "user_crate");
        assert_eq!(ws.readers[1].crate_name(), "core");
        assert_eq!(ws.readers[2].crate_name(), "alloc");
        assert_eq!(ws.readers[3].crate_name(), "std");

        // Calling attach again should be idempotent
        ws.attach_sysroot_shards_for_key_in(&base_cache, "test_cas_key");
        assert_eq!(ws.readers.len(), 4);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
