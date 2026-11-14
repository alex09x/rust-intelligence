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
pub mod cargo_targets;
mod cursor_path;
mod cursor_path_assoc;
mod definition_at;
mod discover;
pub mod file_module;
pub mod manifest;
pub mod navigation;
mod notes;
mod open;
pub mod path_index;
pub mod prelude;
pub mod provenance;
pub mod qualifier;
pub mod reload;
pub mod rename_cache;
pub mod rename_factory_idx;
pub mod rename_parse;
pub mod rename_refusal;
pub mod std_map;
pub mod sysroot;
pub mod target_kind;
pub mod unit_match;
mod warmup;

#[cfg(test)]
mod cache_resolver_tests;
#[cfg(test)]
mod cursor_path_tests;
#[cfg(test)]
mod definition_at_tests;
#[cfg(test)]
mod file_id_routing_tests;
#[cfg(test)]
mod file_module_tests;
#[cfg(test)]
mod path_index_tests;
#[cfg(test)]
mod qualified_consumer_tests;
#[cfg(test)]
mod root_module_tests;
#[cfg(test)]
mod routing_tests;
#[cfg(test)]
mod sibling_tests;
#[cfg(test)]
mod sysroot_tests;
#[cfg(test)]
mod target_kind_routing_tests;
#[cfg(test)]
mod target_kind_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transitive_consumer_tests;
#[cfg(test)]
mod unit_match_tests;
#[cfg(test)]
mod variant_definition_tests;

pub(crate) use discover::collect_ridx_files;
pub use manifest::*;
pub use path_index::PathIndex;
pub use prelude::is_prelude_trait;
pub use rename_refusal::RenameRefusal;
pub use target_kind::{classify_target, classify_target_with_manifest, TargetKind};
pub use unit_match::UnitMatchIndex;

use crate::reader::IndexReader;
use memmap2::Mmap;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static WORKSPACE_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_workspace_instance_id() -> u64 {
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
        f.debug_struct("WorkspaceIndex").field("readers_count", &self.readers.len()).field("workspace_root", &self.workspace_root).finish()
    }
}

impl WorkspaceIndex {
    /// Number of crates loaded in this workspace index.
    pub fn crate_count(&self) -> usize {
        self.readers.len()
    }

    /// Rebuilds the internal unit matching index after readers or extern units change.
    pub(crate) fn rebuild_unit_match(&mut self) {
        self.unit_match = UnitMatchIndex::build_with_deps(&self.readers, &self.unit_keys, &self.reader_extern_units, &self.reader_dependencies);
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
        let mut stats = WorkspaceStats { total_crates: self.readers.len(), active_variant: self.active_variant_id.clone(), ..Default::default() };

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
    pub fn is_external_target(&self, target_file: &str, target_path: &Path, target_unit_key: Option<&str>) -> bool {
        // 1. Explicit sentinel for external reference
        if target_file == "external" {
            return true;
        }

        // 2. Unit-based check: if unit_key is known in the workspace and marked as dependency
        if let Some(unit_key) = target_unit_key {
            let matching = self.readers_for_unit_fast(unit_key);
            if let Some(&i) = matching.first() {
                return self.reader_is_dependency.get(i).copied().unwrap_or(false);
            }
            if self.manifest_path.is_some() {
                return true;
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
                let in_ws = target_path.starts_with(ws_root) || crate::paths::canonicalize_best_effort(target_path).starts_with(crate::paths::canonicalize_best_effort(ws_root));
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
        if let Some(u) = uk.filter(|s| !s.is_empty()) {
            if file_str != "external" && self.unit_match.readers_for_unit(u) == Some(std::slice::from_ref(&idx)) {
                return false;
            }
        }

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

#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
