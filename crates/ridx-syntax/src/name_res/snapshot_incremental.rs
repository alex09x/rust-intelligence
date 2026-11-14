/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pre-seeded clean solver base (`CleanSolverBase`) and two-tier exact incremental `CrateDefMap`
//! solver (local-module delta + pre-sized clean-base fallback) for [`super::crate_snapshot::CrateSnapshot`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::db::{FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, VisKind};
use super::def_collect::CollectedCrate;
use super::def_map::{finalize_module_trait_table, CrateDefMap, ModuleScope, UseDiag};
use super::def_solver::{run_use_fixed_point, seed_index_fallback_defs_for_mod, ImportSolver, MAX_FIXED_POINT_ROUNDS};
use super::dirty_splice::SplicedCrate;
use super::ext_scope::{indexed_use_to_per_ns, ExtScopeCache};
use super::ident_key::IdentKey;
use super::mod_tree::ModuleTree;
use super::per_ns::{compute_vis_mod, BindingOrigin, LocalItemId, PerNs, ResolvedBinding};
use super::snapshot_seed::{
    global_invariants_hold, run_use_fixed_point_with_clean_ext, run_use_range_fixed_point, seed_explicit_items, seed_extern_crates, seed_mod_children, seed_textual_macros,
};
use super::use_tree::FlatUseKind;
use crate::sema_fn::norm_ident;

struct CleanModFallbackUses {
    named: Vec<(IdentKey, PerNs)>,
    globs: Vec<(GlobalDefId, VisKind, ModuleId)>,
    unnamed_traits: Vec<ResolvedBinding>,
}

pub(super) struct CleanSolverBase {
    base_scopes: Vec<ModuleScope>,
    fallback_uses: Vec<CleanModFallbackUses>,
    clean_use_segments: HashSet<IdentKey>,
    glob_imported_mod: Vec<bool>,
    has_unresolved_glob: bool,
}

impl CleanSolverBase {
    pub(super) fn build(db: &dyn ItemDb, mod_tree: &ModuleTree, clean_collected: &CollectedCrate, clean_map: &CrateDefMap) -> Self {
        let mod_count = mod_tree.modules.len();
        let mut base_scopes: Vec<ModuleScope> = clean_collected
            .mod_index_defs
            .iter()
            .enumerate()
            .map(|(m, &index_def)| ModuleScope { index_def, entries: HashMap::with_capacity(clean_map.scopes[m].entries.len().saturating_add(4)), ..ModuleScope::default() })
            .collect();

        for idx in 0..mod_count {
            seed_mod_children(mod_tree, idx, &mut base_scopes);
        }
        seed_explicit_items(clean_collected, None, &mut base_scopes);
        seed_textual_macros(clean_collected, None, &mut base_scopes);
        seed_extern_crates(db, mod_tree, &clean_map.extern_prelude, clean_collected, None, &mut base_scopes);
        let mut scratch_defs = Vec::new();
        for (midx, sc) in base_scopes.iter_mut().enumerate() {
            seed_index_fallback_defs_for_mod(db, mod_tree, &[], midx, sc, &mut scratch_defs);
        }

        let mut raw_uses = Vec::new();
        let mut fallback_uses = Vec::with_capacity(mod_count);
        for (midx, bsc) in base_scopes.iter().enumerate() {
            let idx_def = bsc.index_def;
            let mut named = Vec::new();
            let mut globs = Vec::new();
            let mut unnamed_traits = Vec::new();
            if idx_def.is_some() && !idx_def.is_overlay() {
                raw_uses.clear();
                db.module_uses(idx_def, &mut raw_uses);
                let mod_id = ModuleId(midx as u32);
                for u in &raw_uses {
                    if u.is_prelude {
                        continue;
                    }
                    let (_, vmod) = compute_vis_mod(mod_tree, mod_id, u.vis, None);
                    if u.is_glob {
                        if u.type_def.is_some() && u.type_def != idx_def {
                            globs.push((u.type_def, u.vis, vmod));
                        }
                    } else if u.is_underscore {
                        if u.type_def.is_some() && db.def_kind(u.type_def) == Some(ItemKind::Trait) {
                            unnamed_traits.push(ResolvedBinding {
                                def: u.type_def,
                                local: LocalItemId::NONE,
                                local_mod: ModuleId::NONE,
                                vis_mod: vmod,
                                kind: ItemKind::Trait,
                                vis: u.vis,
                                origin: BindingOrigin::Explicit,
                                prim_ty: 0,
                                ambiguous: false,
                            });
                        }
                    } else if !u.name.is_empty() {
                        let pns = indexed_use_to_per_ns(db, u).rebind(u.vis, vmod, BindingOrigin::Explicit);
                        if !pns.is_empty() {
                            named.push((IdentKey::new(u.name), pns));
                        }
                    }
                }
            }
            fallback_uses.push(CleanModFallbackUses { named, globs, unnamed_traits });
        }

        let mut clean_use_segments = HashSet::with_capacity(clean_map.uses.uses.len().saturating_mul(2));
        let mut glob_imported_mod = vec![false; mod_count];
        let mut has_unresolved_glob = false;
        for (uidx, u) in clean_map.uses.uses.iter().enumerate() {
            for &sp in clean_map.uses.seg_spans(u) {
                clean_use_segments.insert(IdentKey::new(norm_ident(clean_map.uses.str_span(sp))));
            }
            if u.kind == FlatUseKind::Glob {
                match clean_map.resolved_uses[uidx].types {
                    Some(b) if b.local_mod.is_some() => glob_imported_mod[b.local_mod.0 as usize] = true,
                    Some(_) => {}
                    None => has_unresolved_glob = true,
                }
            }
        }

        Self { base_scopes, fallback_uses, clean_use_segments, glob_imported_mod, has_unresolved_glob }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn solve_dirty(
        &self,
        db: &dyn ItemDb,
        mod_tree: &ModuleTree,
        clean_collected: &CollectedCrate,
        clean_map: &CrateDefMap,
        spliced: SplicedCrate,
        dirty_files: &[FileId],
        ext_cache: &mut ExtScopeCache,
    ) -> CrateDefMap {
        let mod_count = mod_tree.modules.len();
        if mod_count == 0 {
            return ImportSolver::solve(db, mod_tree, spliced.collected, dirty_files, ext_cache);
        }
        let dirty_mod_mask: Vec<bool> = (0..mod_count).map(|m| dirty_files.contains(&mod_tree.modules[m].file)).collect();
        if !global_invariants_hold(clean_collected, &spliced.collected, dirty_files, &dirty_mod_mask) {
            return ImportSolver::solve(db, mod_tree, spliced.collected, dirty_files, ext_cache);
        }

        if let Some(fast_map) = self.try_solve_local_delta(db, mod_tree, clean_map, &spliced, dirty_files, &dirty_mod_mask, ext_cache) {
            return fast_map;
        }

        self.solve_with_clean_base(db, mod_tree, clean_map, spliced, dirty_files, &dirty_mod_mask, ext_cache)
    }

    #[allow(clippy::too_many_arguments)]
    fn try_solve_local_delta(
        &self,
        db: &dyn ItemDb,
        mod_tree: &ModuleTree,
        clean_map: &CrateDefMap,
        spliced: &SplicedCrate,
        dirty_files: &[FileId],
        dirty_mod_mask: &[bool],
        ext_cache: &mut ExtScopeCache,
    ) -> Option<CrateDefMap> {
        let mod_count = mod_tree.modules.len();
        let clean_use_count = spliced.clean_use_map.len();
        let total_uses = spliced.collected.uses.uses.len();

        let mut scopes = clean_map.scopes.clone();
        let mut scratch_defs = Vec::new();
        for midx in 0..mod_count {
            if !dirty_mod_mask[midx] {
                continue;
            }
            scopes[midx] = ModuleScope {
                index_def: spliced.collected.mod_index_defs[midx],
                entries: HashMap::with_capacity(clean_map.scopes[midx].entries.len().saturating_add(8)),
                ..ModuleScope::default()
            };
            seed_mod_children(mod_tree, midx, &mut scopes);
        }
        seed_explicit_items(&spliced.collected, Some(dirty_mod_mask), &mut scopes);
        seed_textual_macros(&spliced.collected, Some(dirty_mod_mask), &mut scopes);
        seed_extern_crates(db, mod_tree, &clean_map.extern_prelude, &spliced.collected, Some(dirty_mod_mask), &mut scopes);
        for (midx, sc) in scopes.iter_mut().enumerate() {
            if dirty_mod_mask[midx] {
                seed_index_fallback_defs_for_mod(db, mod_tree, dirty_files, midx, sc, &mut scratch_defs);
            }
        }

        let mut resolved_uses = vec![PerNs::EMPTY; total_uses];
        for (new_idx, &old_idx) in spliced.clean_use_map.iter().enumerate() {
            resolved_uses[new_idx] = clean_map.resolved_uses[old_idx as usize];
        }

        let mut macro_bodies = spliced.collected.macro_bodies.clone();
        macro_bodies.sort_unstable_by_key(|&(lid, ..)| lid);
        let mut map = CrateDefMap {
            krate: mod_tree.krate,
            edition: mod_tree.edition,
            prelude_mod: clean_map.prelude_mod,
            prelude: clean_map.prelude.clone(),
            prelude_traits: Arc::clone(&clean_map.prelude_traits),
            prelude_traits_sorted: Arc::clone(&clean_map.prelude_traits_sorted),
            extern_prelude: clean_map.extern_prelude.clone(),
            items: spliced.collected.items.clone(),
            item_strings: spliced.collected.strings.clone(),
            uses: spliced.collected.uses.clone(),
            scopes,
            resolved_uses,
            dirty_files: dirty_files.to_vec(),
            indexed_clean: false,
            mod_tree: Arc::new(mod_tree.clone()),
            macro_bodies,
            local_impls: spliced.collected.local_impls.clone(),
            local_alias_targets: HashMap::new(),
            diagnostics: Vec::new(),
        };

        run_use_range_fixed_point(&mut map, db, mod_tree, ext_cache, clean_use_count..total_uses, MAX_FIXED_POINT_ROUNDS);

        for (midx, &is_dirty) in dirty_mod_mask.iter().enumerate() {
            if !is_dirty {
                continue;
            }
            if !self.dirty_scope_exports_invariant(mod_tree, ModuleId(midx as u32), &clean_map.scopes[midx], &map.scopes[midx]) {
                return None;
            }
        }

        let mut named_buf = Vec::new();
        for (midx, &is_dirty) in dirty_mod_mask.iter().enumerate() {
            if is_dirty {
                finalize_module_trait_table(&mut map.scopes[midx], mod_tree, ModuleId(midx as u32), &mut named_buf);
            }
        }
        for uidx in 0..total_uses {
            let u = map.uses.uses[uidx];
            if u.kind != FlatUseKind::Glob && map.resolved_uses[uidx].is_empty() {
                map.diagnostics.push(UseDiag::UnresolvedUse { module: u.module, use_index: uidx as u32 });
            }
        }
        Some(map)
    }

    fn dirty_scope_exports_invariant(&self, mod_tree: &ModuleTree, dm: ModuleId, old_sc: &ModuleScope, new_sc: &ModuleScope) -> bool {
        let has_children = !mod_tree.children(dm).is_empty();
        let dm_glob_imported = self.glob_imported_mod[dm.0 as usize] || self.has_unresolved_glob;
        for (k, &old_pns) in &old_sc.entries {
            let new_pns = new_sc.get(k);
            if new_pns != old_pns
                && (has_children || pns_has_non_private(old_pns) || pns_has_non_private(new_pns))
                && (dm_glob_imported || self.clean_use_segments.contains(k.as_str()))
            {
                return false;
            }
        }
        for (k, &new_pns) in &new_sc.entries {
            if old_sc.entries.contains_key(k) {
                continue;
            }
            if (has_children || pns_has_non_private(new_pns)) && (dm_glob_imported || self.clean_use_segments.contains(k.as_str())) {
                return false;
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn solve_with_clean_base(
        &self,
        db: &dyn ItemDb,
        mod_tree: &ModuleTree,
        clean_map: &CrateDefMap,
        spliced: SplicedCrate,
        dirty_files: &[FileId],
        dirty_mod_mask: &[bool],
        ext_cache: &mut ExtScopeCache,
    ) -> CrateDefMap {
        let mod_count = mod_tree.modules.len();
        let mut scopes = self.base_scopes.clone();
        let mut scratch_defs = Vec::new();
        for midx in 0..mod_count {
            if !dirty_mod_mask[midx] {
                continue;
            }
            scopes[midx] = ModuleScope {
                index_def: spliced.collected.mod_index_defs[midx],
                entries: HashMap::with_capacity(clean_map.scopes[midx].entries.len().saturating_add(8)),
                ..ModuleScope::default()
            };
            seed_mod_children(mod_tree, midx, &mut scopes);
        }
        seed_explicit_items(&spliced.collected, Some(dirty_mod_mask), &mut scopes);
        seed_textual_macros(&spliced.collected, Some(dirty_mod_mask), &mut scopes);
        seed_extern_crates(db, mod_tree, &clean_map.extern_prelude, &spliced.collected, Some(dirty_mod_mask), &mut scopes);
        for (midx, sc) in scopes.iter_mut().enumerate() {
            if dirty_mod_mask[midx] {
                seed_index_fallback_defs_for_mod(db, mod_tree, dirty_files, midx, sc, &mut scratch_defs);
            }
        }

        let mut macro_bodies = spliced.collected.macro_bodies;
        macro_bodies.sort_unstable_by_key(|&(lid, ..)| lid);
        let use_count = spliced.collected.uses.uses.len();
        let mut map = CrateDefMap {
            krate: mod_tree.krate,
            edition: mod_tree.edition,
            prelude_mod: clean_map.prelude_mod,
            prelude: clean_map.prelude.clone(),
            prelude_traits: Arc::clone(&clean_map.prelude_traits),
            prelude_traits_sorted: Arc::clone(&clean_map.prelude_traits_sorted),
            extern_prelude: clean_map.extern_prelude.clone(),
            items: spliced.collected.items,
            item_strings: spliced.collected.strings,
            uses: spliced.collected.uses,
            scopes,
            resolved_uses: vec![PerNs::EMPTY; use_count],
            dirty_files: dirty_files.to_vec(),
            indexed_clean: false,
            mod_tree: Arc::new(mod_tree.clone()),
            macro_bodies,
            local_impls: spliced.collected.local_impls,
            local_alias_targets: HashMap::new(),
            diagnostics: Vec::new(),
        };

        run_use_fixed_point_with_clean_ext(&mut map, db, mod_tree, ext_cache, clean_map, &spliced.clean_use_map, MAX_FIXED_POINT_ROUNDS);
        if self.apply_cached_fallback_uses(&mut map, db, dirty_mod_mask, ext_cache) {
            run_use_fixed_point(&mut map, db, mod_tree, ext_cache, 4);
        }

        for uidx in 0..use_count {
            let u = map.uses.uses[uidx];
            if u.kind != FlatUseKind::Glob && map.resolved_uses[uidx].is_empty() {
                map.diagnostics.push(UseDiag::UnresolvedUse { module: u.module, use_index: uidx as u32 });
            }
        }

        let mut named_buf = Vec::new();
        for (midx, &is_dirty) in dirty_mod_mask.iter().enumerate() {
            let same = !is_dirty && map.scopes[midx].unnamed_traits == clean_map.scopes[midx].unnamed_traits && map.scopes[midx].entries == clean_map.scopes[midx].entries;
            if same {
                map.scopes[midx].visible_traits = Arc::clone(&clean_map.scopes[midx].visible_traits);
                map.scopes[midx].visible_traits_sorted = Arc::clone(&clean_map.scopes[midx].visible_traits_sorted);
                map.scopes[midx].visible_traits_hash = clean_map.scopes[midx].visible_traits_hash;
            } else {
                finalize_module_trait_table(&mut map.scopes[midx], mod_tree, ModuleId(midx as u32), &mut named_buf);
            }
        }
        map
    }

    fn apply_cached_fallback_uses(&self, map: &mut CrateDefMap, db: &dyn ItemDb, dirty_mod_mask: &[bool], ext_cache: &mut ExtScopeCache) -> bool {
        let mut any_changed = false;
        for (midx, fbu) in self.fallback_uses.iter().enumerate() {
            if dirty_mod_mask[midx] {
                continue;
            }
            for &tb in &fbu.unnamed_traits {
                let list = &mut map.scopes[midx].unnamed_traits;
                if !list.iter().any(|x| x.same_target(tb)) {
                    list.push(tb);
                    any_changed = true;
                }
            }
            for (k, pns) in &fbu.named {
                let slot = map.scopes[midx].entries.entry(k.clone()).or_insert(PerNs::EMPTY);
                let next = slot.or_else_ns(*pns);
                if next != *slot {
                    *slot = next;
                    any_changed = true;
                }
            }
            for &(gdef, gvis, gvmod) in &fbu.globs {
                let ext_s = ext_cache.module_scope(db, gdef);
                for (k, &v) in &ext_s.entries {
                    any_changed |= map.scopes[midx].entries.entry(k.clone()).or_insert(PerNs::EMPTY).merge_glob(v.rebind(gvis, gvmod, BindingOrigin::Glob));
                }
            }
        }
        any_changed
    }
}

#[inline]
fn pns_has_non_private(pns: PerNs) -> bool {
    pns.types.is_some_and(|b| b.vis != VisKind::Private) || pns.values.is_some_and(|b| b.vis != VisKind::Private) || pns.macros.is_some_and(|b| b.vis != VisKind::Private)
}
