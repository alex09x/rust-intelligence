/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pre-built clean crate snapshot (`CrateSnapshot`) enabling single-file / dirty-files incremental
//! overlay updates without re-parsing or re-walking unchanged `.rs` files in the crate.

use std::cell::RefCell;
use std::collections::HashMap;

use super::db::{CrateId, FileId, ItemDb, ModuleId, SourceDb};
use super::def_collect::{collect_crate_items, CollectedCrate};
use super::def_map::CrateDefMap;
use super::ext_scope::ExtScopeCache;
use super::impl_resolve::resolve_local_impls_and_assocs;
use super::mod_fingerprint::{file_mod_fingerprint, mod_trees_structurally_equal};
use super::mod_tree::ModuleTree;
use super::snapshot_incremental::CleanSolverBase;
use crate::parser::parse;
use crate::tree::SyntaxTree;

struct DirtySnapshotBase {
    clean_collected: CollectedCrate,
    clean_file_mod_hashes: HashMap<FileId, u64>,
    clean_solver_base: CleanSolverBase,
}

/// Pre-built clean crate snapshot accelerating single-file / dirty-files overlay updates
/// without re-parsing or re-walking unchanged `.rs` files in the crate.
pub struct CrateSnapshot {
    pub mod_tree: ModuleTree,
    pub clean_def_map: CrateDefMap,
    pub(super) indexed_only: bool,
    dirty_base: Option<DirtySnapshotBase>,
    pub(super) ext_cache: RefCell<ExtScopeCache>,
}

impl CrateSnapshot {
    #[inline]
    pub(super) fn from_parts(mod_tree: ModuleTree, clean_def_map: CrateDefMap, ext_cache: ExtScopeCache) -> Self {
        Self { mod_tree, clean_def_map, indexed_only: true, dirty_base: None, ext_cache: RefCell::new(ext_cache) }
    }

    #[inline]
    pub const fn is_indexed_only(&self) -> bool {
        self.indexed_only
    }

    /// Builds a reusable clean snapshot of `krate` rooted at `root_file`, parsing each reached `.rs` file once.
    pub fn build_clean(db: &dyn ItemDb, sources: &dyn SourceDb, krate: CrateId, root_file: FileId) -> Self {
        let (mod_tree, trees) = ModuleTree::build_capturing_trees(db, sources, krate, root_file);
        let mut ext_cache = ExtScopeCache::new();
        let clean_collected = collect_crate_items(db, sources, &mod_tree, &[], |fid| trees.get(&fid));
        let clean_def_map = CrateDefMap::from_collected(db, &mod_tree, clean_collected.clone(), &[], &mut ext_cache);
        let clean_file_mod_hashes = compute_mod_hashes(&mod_tree, db, sources, |fid| trees.get(&fid));
        drop(trees);
        let clean_solver_base = CleanSolverBase::build(db, &mod_tree, &clean_collected, &clean_def_map);
        Self {
            mod_tree,
            clean_def_map,
            indexed_only: false,
            dirty_base: Some(DirtySnapshotBase { clean_collected, clean_file_mod_hashes, clean_solver_base }),
            ext_cache: RefCell::new(ext_cache),
        }
    }

    /// Builds a clean snapshot (`mod_tree` + `clean_def_map`) parsing each reached `.rs` file once,
    /// deferring `DirtySnapshotBase` allocation until an unsaved buffer actually modifies `krate`.
    pub fn build_clean_for_query(db: &dyn ItemDb, sources: &dyn SourceDb, krate: CrateId, root_file: FileId) -> Self {
        let (mod_tree, trees) = ModuleTree::build_capturing_trees(db, sources, krate, root_file);
        let mut ext_cache = ExtScopeCache::new();
        let clean_collected = collect_crate_items(db, sources, &mod_tree, &[], |fid| trees.get(&fid));
        drop(trees);
        let clean_def_map = CrateDefMap::from_collected(db, &mod_tree, clean_collected, &[], &mut ext_cache);
        Self { mod_tree, clean_def_map, indexed_only: false, dirty_base: None, ext_cache: RefCell::new(ext_cache) }
    }

    /// Ensures the incremental dirty-overlay base tables are populated before calling [`Self::with_dirty_files`].
    pub fn ensure_dirty_base(&mut self, db: &dyn ItemDb, clean_sources: &dyn SourceDb) {
        if self.dirty_base.is_some() {
            return;
        }
        let mut trees: HashMap<FileId, SyntaxTree<'_>> = HashMap::with_capacity(self.mod_tree.modules.len());
        for m in &self.mod_tree.modules {
            if m.file.is_some() && !trees.contains_key(&m.file) {
                trees.insert(m.file, parse(clean_sources.file_text(m.file)));
            }
        }
        let clean_collected = collect_crate_items(db, clean_sources, &self.mod_tree, &[], |fid| trees.get(&fid));
        let clean_file_mod_hashes = compute_mod_hashes(&self.mod_tree, db, clean_sources, |fid| trees.get(&fid));
        drop(trees);
        let clean_solver_base = CleanSolverBase::build(db, &self.mod_tree, &clean_collected, &self.clean_def_map);
        self.dirty_base = Some(DirtySnapshotBase { clean_collected, clean_file_mod_hashes, clean_solver_base });
    }

    /// Same as [`Self::build_clean`], borrowing caller-cached [`SyntaxTree`]s when available.
    pub fn build_clean_cached<'t>(
        db: &dyn ItemDb,
        sources: &dyn SourceDb,
        krate: CrateId,
        root_file: FileId,
        mut get_tree: impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
    ) -> Self {
        let mod_tree = ModuleTree::build_cached(db, sources, krate, root_file, &mut get_tree);
        let mut ext_cache = ExtScopeCache::new();
        let clean_collected = collect_crate_items(db, sources, &mod_tree, &[], &mut get_tree);
        let clean_def_map = CrateDefMap::from_collected(db, &mod_tree, clean_collected.clone(), &[], &mut ext_cache);
        let clean_file_mod_hashes = compute_mod_hashes(&mod_tree, db, sources, &mut get_tree);
        let clean_solver_base = CleanSolverBase::build(db, &mod_tree, &clean_collected, &clean_def_map);
        Self {
            mod_tree,
            clean_def_map,
            indexed_only: false,
            dirty_base: Some(DirtySnapshotBase { clean_collected, clean_file_mod_hashes, clean_solver_base }),
            ext_cache: RefCell::new(ext_cache),
        }
    }

    /// Incrementally merges `dirty_files` from `overlay_sources` over the cached clean crate state.
    /// Returns `(Some(new_mod_tree), def_map)` only when a dirty edit altered `mod` declarations;
    /// returns `(None, def_map)` on the fast path (`self.mod_tree` remains valid as-is).
    pub fn with_dirty_files<'t>(
        &self,
        db: &dyn ItemDb,
        overlay_sources: &dyn SourceDb,
        dirty_files: &[FileId],
        mut get_tree: impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
    ) -> (Option<ModuleTree>, CrateDefMap) {
        if dirty_files.is_empty() {
            let mut ext_c = self.ext_cache.borrow_mut();
            if let Some(base) = &self.dirty_base {
                return (None, CrateDefMap::from_collected(db, &self.mod_tree, base.clean_collected.clone(), &[], &mut ext_c));
            }
            let coll = collect_crate_items(db, overlay_sources, &self.mod_tree, &[], &mut get_tree);
            return (None, CrateDefMap::from_collected(db, &self.mod_tree, coll, &[], &mut ext_c));
        }
        let Some(base) = &self.dirty_base else {
            let root_file = self.mod_tree.module(ModuleId::ROOT).file;
            let new_mt = ModuleTree::build_cached(db, overlay_sources, self.mod_tree.krate, root_file, &mut get_tree);
            let coll = collect_crate_items(db, overlay_sources, &new_mt, dirty_files, &mut get_tree);
            let mut ext_c = self.ext_cache.borrow_mut();
            let dm = CrateDefMap::from_collected(db, &new_mt, coll, dirty_files, &mut ext_c);
            let changed = !mod_trees_structurally_equal(&self.mod_tree, &new_mt);
            return (changed.then_some(new_mt), dm);
        };
        if Self::mod_structure_maybe_changed(&self.mod_tree, &base.clean_file_mod_hashes, db, overlay_sources, dirty_files, &mut get_tree) {
            let root_file = self.mod_tree.module(ModuleId::ROOT).file;
            let new_mt = ModuleTree::build_cached(db, overlay_sources, self.mod_tree.krate, root_file, &mut get_tree);
            if !mod_trees_structurally_equal(&self.mod_tree, &new_mt) {
                let coll = collect_crate_items(db, overlay_sources, &new_mt, dirty_files, &mut get_tree);
                let mut ext_c = self.ext_cache.borrow_mut();
                let dm = CrateDefMap::from_collected(db, &new_mt, coll, dirty_files, &mut ext_c);
                return (Some(new_mt), dm);
            }
        }

        let spliced = base.clean_collected.splice_dirty_files(db, overlay_sources, &self.mod_tree, dirty_files, &mut get_tree);
        let next_seq = spliced.next_overlay_seq;
        let mut ext_c = self.ext_cache.borrow_mut();
        let mut map = base.clean_solver_base.solve_dirty(db, &self.mod_tree, &base.clean_collected, &self.clean_def_map, spliced, dirty_files, &mut ext_c);
        resolve_local_impls_and_assocs(&mut map, db, &self.mod_tree, &mut ext_c, next_seq);
        (None, map)
    }

    fn mod_structure_maybe_changed<'t>(
        mod_tree: &ModuleTree,
        clean_file_mod_hashes: &HashMap<FileId, u64>,
        db: &dyn ItemDb,
        overlay_sources: &dyn SourceDb,
        dirty_files: &[FileId],
        get_tree: &mut impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
    ) -> bool {
        let krate = mod_tree.krate;
        for &df in dirty_files {
            let Some(&expected_hash) = clean_file_mod_hashes.get(&df) else {
                return true;
            };
            let owned;
            let st: &SyntaxTree<'_> = match get_tree(df) {
                Some(t) => t,
                None => {
                    owned = parse(overlay_sources.file_text(df));
                    &owned
                }
            };
            if file_mod_fingerprint(st, db, krate) != expected_hash {
                return true;
            }
        }
        false
    }
}

fn compute_mod_hashes<'t>(mod_tree: &ModuleTree, db: &dyn ItemDb, sources: &dyn SourceDb, mut get_tree: impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>) -> HashMap<FileId, u64> {
    let mut out = HashMap::with_capacity(mod_tree.modules.len());
    for m in &mod_tree.modules {
        if !m.file.is_some() || out.contains_key(&m.file) {
            continue;
        }
        let owned;
        let st: &SyntaxTree<'_> = match get_tree(m.file) {
            Some(t) => t,
            None => {
                owned = parse(sources.file_text(m.file));
                &owned
            }
        };
        out.insert(m.file, file_mod_fingerprint(st, db, mod_tree.krate));
    }
    out
}
