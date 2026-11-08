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

/// Pre-built clean crate snapshot accelerating single-file / dirty-files overlay updates
/// without re-parsing or re-walking unchanged `.rs` files in the crate.
pub struct CrateSnapshot {
    pub mod_tree: ModuleTree,
    pub clean_def_map: CrateDefMap,
    clean_collected: CollectedCrate,
    clean_file_mod_hashes: HashMap<FileId, u64>,
    clean_solver_base: CleanSolverBase,
    pub(super) ext_cache: RefCell<ExtScopeCache>,
}

impl CrateSnapshot {
    /// Builds a reusable clean snapshot of `krate` rooted at `root_file`.
    pub fn build_clean(db: &dyn ItemDb, sources: &dyn SourceDb, krate: CrateId, root_file: FileId) -> Self {
        Self::build_clean_cached(db, sources, krate, root_file, |_| None)
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

        let mut clean_file_mod_hashes = HashMap::with_capacity(mod_tree.modules.len());
        for m in &mod_tree.modules {
            if !m.file.is_some() || clean_file_mod_hashes.contains_key(&m.file) {
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
            clean_file_mod_hashes.insert(m.file, file_mod_fingerprint(st, db, krate));
        }
        let clean_solver_base = CleanSolverBase::build(db, &mod_tree, &clean_collected, &clean_def_map);

        Self { mod_tree, clean_def_map, clean_collected, clean_file_mod_hashes, clean_solver_base, ext_cache: RefCell::new(ext_cache) }
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
            let dm = CrateDefMap::from_collected(db, &self.mod_tree, self.clean_collected.clone(), &[], &mut ext_c);
            return (None, dm);
        }
        if self.mod_structure_maybe_changed(db, overlay_sources, dirty_files, &mut get_tree) {
            let root_file = self.mod_tree.module(ModuleId::ROOT).file;
            let new_mt = ModuleTree::build_cached(db, overlay_sources, self.mod_tree.krate, root_file, &mut get_tree);
            if !mod_trees_structurally_equal(&self.mod_tree, &new_mt) {
                let coll = collect_crate_items(db, overlay_sources, &new_mt, dirty_files, &mut get_tree);
                let mut ext_c = self.ext_cache.borrow_mut();
                let dm = CrateDefMap::from_collected(db, &new_mt, coll, dirty_files, &mut ext_c);
                return (Some(new_mt), dm);
            }
        }

        let spliced = self.clean_collected.splice_dirty_files(db, overlay_sources, &self.mod_tree, dirty_files, &mut get_tree);
        let next_seq = spliced.next_overlay_seq;
        let mut ext_c = self.ext_cache.borrow_mut();
        let mut map = self.clean_solver_base.solve_dirty(db, &self.mod_tree, &self.clean_collected, &self.clean_def_map, spliced, dirty_files, &mut ext_c);
        resolve_local_impls_and_assocs(&mut map, db, &self.mod_tree, &mut ext_c, next_seq);
        (None, map)
    }

    fn mod_structure_maybe_changed<'t>(
        &self,
        db: &dyn ItemDb,
        overlay_sources: &dyn SourceDb,
        dirty_files: &[FileId],
        get_tree: &mut impl FnMut(FileId) -> Option<&'t SyntaxTree<'t>>,
    ) -> bool {
        let krate = self.mod_tree.krate;
        for &df in dirty_files {
            let Some(&expected_hash) = self.clean_file_mod_hashes.get(&df) else {
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
