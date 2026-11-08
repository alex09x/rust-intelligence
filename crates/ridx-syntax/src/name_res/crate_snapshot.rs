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

use super::attr::{eval_inner_attrs, eval_outer_attrs, extract_visibility, EffectiveAttrs};
use super::db::{CrateId, FileId, ItemDb, ModuleId, SourceDb};
use super::def_collect::{collect_crate_items, CollectedCrate};
use super::def_map::CrateDefMap;
use super::def_solver::ImportSolver;
use super::dirty_splice::find_module_container_in_file;
use super::ext_scope::ExtScopeCache;
use super::impl_resolve::resolve_local_impls_and_assocs;
use super::mod_macro_scope::has_mod_or_macro_kw;
use super::mod_tree::{ModuleData, ModuleTree, MOD_FLAG_MACRO_USE};
use crate::parser::parse;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

/// Pre-built clean crate snapshot accelerating single-file / dirty-files overlay updates
/// without re-parsing or re-walking unchanged `.rs` files in the crate.
pub struct CrateSnapshot {
    pub mod_tree: ModuleTree,
    pub clean_def_map: CrateDefMap,
    clean_collected: CollectedCrate,
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
        Self { mod_tree, clean_def_map, clean_collected, ext_cache: RefCell::new(ext_cache) }
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
            let coll = collect_crate_items(db, overlay_sources, &new_mt, dirty_files, &mut get_tree);
            let mut ext_c = self.ext_cache.borrow_mut();
            let dm = CrateDefMap::from_collected(db, &new_mt, coll, dirty_files, &mut ext_c);
            return (Some(new_mt), dm);
        }

        // Fast path: splice ONLY modules residing in `dirty_files` over `self.clean_collected`!
        let spliced = self.clean_collected.splice_dirty_files(db, overlay_sources, &self.mod_tree, dirty_files, &mut get_tree);
        let mut ext_c = self.ext_cache.borrow_mut();
        let mut map = ImportSolver::solve(db, &self.mod_tree, spliced.0, dirty_files, &mut ext_c);
        resolve_local_impls_and_assocs(&mut map, db, &self.mod_tree, &mut ext_c, spliced.1);
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
            let Some(file_root_mod) = self.mod_tree.modules.iter().find(|m| m.file == df && !m.is_inline()) else {
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
            let mut root_inner = EffectiveAttrs::default();
            eval_inner_attrs(st, st.root(), db, krate, &mut root_inner);
            if !root_inner.enabled {
                return true;
            }
            let expected_no_prelude = file_root_mod.parent.is_some() && self.mod_tree.module(file_root_mod.parent).has_no_implicit_prelude() || root_inner.no_implicit_prelude;
            if expected_no_prelude != file_root_mod.has_no_implicit_prelude() {
                return true;
            }
            for m in self.mod_tree.modules.iter().filter(|m| m.file == df) {
                if !self.module_children_unchanged(m, st, db, krate) {
                    return true;
                }
            }
        }
        false
    }

    fn module_children_unchanged(&self, m: &ModuleData, st: &SyntaxTree<'_>, db: &dyn ItemDb, krate: CrateId) -> bool {
        if (m.flags & MOD_FLAG_MACRO_USE) != 0 || (m.is_inline() && m.inline_node == u32::MAX) {
            return false;
        }
        let Some(cont) = find_module_container_in_file(&self.mod_tree, m.id, st) else {
            return false;
        };
        if m.is_inline() {
            let mut inner = EffectiveAttrs::default();
            eval_inner_attrs(st, cont, db, krate, &mut inner);
            let parent_np = m.parent.is_some() && self.mod_tree.module(m.parent).has_no_implicit_prelude();
            if !inner.enabled || (parent_np || inner.no_implicit_prelude) != m.has_no_implicit_prelude() {
                return false;
            }
        }
        let expected_children = self.mod_tree.children(m.id);
        let mut child_idx = 0usize;
        for ch in st.children(cont) {
            match st.kind(ch) {
                K::Module => {
                    let attrs = eval_outer_attrs(st, ch, db, krate);
                    if !attrs.enabled {
                        continue;
                    }
                    if attrs.explicit_path.is_some() || attrs.macro_use || child_idx >= expected_children.len() {
                        return false;
                    }
                    let Some(nn) = st.child(ch, K::Name) else {
                        return false;
                    };
                    let name = norm_ident(st.text(nn));
                    let is_inline = st.child(ch, K::ItemList).is_some();
                    let (vis, _) = extract_visibility(st, ch);
                    let exp_cid: ModuleId = expected_children[child_idx].child;
                    let exp_m = self.mod_tree.module(exp_cid);
                    if self.mod_tree.name(exp_cid) != name || exp_m.is_inline() != is_inline || exp_m.vis != vis {
                        return false;
                    }
                    child_idx += 1;
                }
                K::MacroRules if !expected_children.is_empty() => return false,
                K::MacroCall if call_might_emit_mod(st, ch) => return false,
                _ => {}
            }
        }
        child_idx == expected_children.len()
    }
}

fn call_might_emit_mod(st: &SyntaxTree<'_>, call_node: NodeId) -> bool {
    st.child(call_node, K::TokenTree).is_some_and(|tt| has_mod_or_macro_kw(st.text(tt)))
}
