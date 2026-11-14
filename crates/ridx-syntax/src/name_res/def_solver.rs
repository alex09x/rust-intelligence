/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fixed-point worklist import, glob, and re-export solver for [`CrateDefMap`] (path stepping
//! helpers live in [`super::def_path`]).

use std::collections::HashMap;
use std::sync::Arc;

use super::db::{CrateId, FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, NsMask, VisKind};
use super::def_collect::CollectedCrate;
use super::def_map::{extern_crate_binding, finalize_in_scope_trait_tables, local_mod_binding, CrateDefMap, ModuleScope, UseDiag};
use super::def_path::{apply_glob_use, resolve_segments, resolve_use_leaf};
use super::ext_scope::{indexed_use_to_per_ns, ExtScopeCache};
use super::ident_key::IdentKey;
use super::mod_tree::ModuleTree;
use super::per_ns::{compute_vis_mod, BindingOrigin, LocalItemId, PerNs, ResolvedBinding};
use super::use_tree::{FlatUseKind, PathAnchor};

pub(super) const MAX_FIXED_POINT_ROUNDS: usize = 64;

pub(super) struct ImportSolver;

impl ImportSolver {
    pub(super) fn solve(db: &dyn ItemDb, mod_tree: &ModuleTree, collected: CollectedCrate, dirty_files: &[FileId], ext_cache: &mut ExtScopeCache) -> CrateDefMap {
        let mod_count = mod_tree.modules.len();
        let mut scopes: Vec<ModuleScope> = collected.mod_index_defs.iter().map(|&index_def| ModuleScope { index_def, ..ModuleScope::default() }).collect();

        let mut extern_prelude: HashMap<IdentKey, (CrateId, GlobalDefId)> = HashMap::new();
        let mut raw_ext = Vec::new();
        db.extern_crates(mod_tree.krate, &mut raw_ext);
        for (ename, cid) in raw_ext {
            let rdef = db.crate_root_def(cid).unwrap_or(GlobalDefId::new(cid, 0));
            extern_prelude.insert(IdentKey::new(ename), (cid, rdef));
        }
        for builtin_ext in ["core", "std", "alloc", "proc_macro", "test"] {
            if (builtin_ext == "std" && mod_tree.no_std) || mod_tree.no_core {
                continue;
            }
            if let Some(cid) = db.extern_crate(mod_tree.krate, builtin_ext) {
                let rdef = db.crate_root_def(cid).unwrap_or(GlobalDefId::new(cid, 0));
                extern_prelude.entry(IdentKey::new(builtin_ext)).or_insert((cid, rdef));
            }
        }

        let prelude_mod = ext_cache.resolve_prelude_mod(db, mod_tree);
        let mut prelude: HashMap<IdentKey, PerNs> = HashMap::new();
        if let Some(pmod) = prelude_mod {
            let pscope = ext_cache.module_scope(db, pmod);
            for (k, &pns) in &pscope.entries {
                prelude.insert(k.clone(), pns.rebind(VisKind::Public, ModuleId::NONE, BindingOrigin::Prelude));
            }
        }

        let mut diagnostics = Vec::new();

        for idx in 0..mod_count {
            let pid = ModuleId(idx as u32);
            for edge in mod_tree.children(pid) {
                let cname = IdentKey::new(mod_tree.child_name(edge));
                let (_, vmod) = compute_vis_mod(mod_tree, pid, edge.vis, None);
                let cdef = scopes[edge.child.0 as usize].index_def;
                scopes[idx].entries.entry(cname).or_insert(PerNs::EMPTY).merge_explicit(local_mod_binding(edge.child, cdef, edge.vis, vmod));
            }
        }

        for it in &collected.items {
            if !it.module.is_some() || it.parent_enum.is_some() {
                continue;
            }
            let name = &collected.strings[it.name_start as usize..it.name_end as usize];
            let b = ResolvedBinding {
                def: it.index_def,
                local: it.id,
                local_mod: ModuleId::NONE,
                vis_mod: it.vis_mod,
                kind: it.kind,
                vis: it.vis,
                origin: BindingOrigin::Explicit,
                prim_ty: 0,
                ambiguous: false,
            };
            let mask = it.kind.ns_mask(it.has_value_ctor);
            scopes[it.module.0 as usize].entries.entry(IdentKey::new(name)).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(b, mask));
            if it.macro_export && !scopes.is_empty() {
                let exp_b = b.rebind(VisKind::Public, ModuleId::NONE, BindingOrigin::Explicit);
                scopes[ModuleId::ROOT.0 as usize].entries.entry(IdentKey::new(name)).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(exp_b, NsMask::MACRO));
            }
        }

        // Seed `#[macro_use]` textual macros into descendant / parent module scopes.
        for (midx, tlist) in collected.textual_macros.iter().enumerate() {
            for &(lid, gdef) in tlist {
                let it = &collected.items[lid.0 as usize];
                let mname = &collected.strings[it.name_start as usize..it.name_end as usize];
                let mb = ResolvedBinding {
                    def: gdef,
                    local: lid,
                    local_mod: ModuleId::NONE,
                    vis_mod: ModuleId(midx as u32),
                    kind: ItemKind::Macro,
                    vis: VisKind::Private,
                    origin: BindingOrigin::Prelude,
                    prim_ty: 0,
                    ambiguous: false,
                };
                let slot = &mut scopes[midx].entries.entry(IdentKey::new(mname)).or_insert(PerNs::EMPTY).macros;
                if slot.is_none() {
                    *slot = Some(mb);
                }
            }
        }

        for ec in &collected.uses.extern_crates {
            let orig = collected.uses.extern_orig(ec);
            let bind = collected.uses.extern_bind(ec);
            if ec.is_self {
                if !ec.is_underscore {
                    let root_def = scopes.first().map_or(GlobalDefId::NONE, |s| s.index_def);
                    let pns = local_mod_binding(ModuleId::ROOT, root_def, ec.vis, ec.vis_mod);
                    scopes[ec.module.0 as usize].entries.entry(IdentKey::new(bind)).or_insert(PerNs::EMPTY).merge_explicit(pns);
                    if ec.module == ModuleId::ROOT {
                        extern_prelude.insert(IdentKey::new(bind), (mod_tree.krate, root_def));
                    }
                }
                continue;
            }
            let resolved_ext = extern_prelude.get(orig).copied().or_else(|| {
                let cid = db.extern_crate(mod_tree.krate, orig)?;
                let rdef = db.crate_root_def(cid).unwrap_or(GlobalDefId::new(cid, 0));
                Some((cid, rdef))
            });
            let Some((cid, rdef)) = resolved_ext else {
                diagnostics.push(UseDiag::UnresolvedExternCrate { module: ec.module, name: orig.to_string() });
                continue;
            };
            if ec.module == ModuleId::ROOT && !ec.is_underscore {
                extern_prelude.insert(IdentKey::new(bind), (cid, rdef));
            }
            if !ec.is_underscore {
                let mut b = extern_crate_binding(cid, mod_tree.krate, rdef);
                b.vis = ec.vis;
                b.vis_mod = ec.vis_mod;
                b.origin = BindingOrigin::Explicit;
                scopes[ec.module.0 as usize].entries.entry(IdentKey::new(bind)).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(b, NsMask::TYPE));
            }
        }

        seed_index_fallback_defs(db, mod_tree, dirty_files, &mut scopes);

        let mut macro_bodies = collected.macro_bodies;
        macro_bodies.sort_unstable_by_key(|&(lid, ..)| lid);
        let use_count = collected.uses.uses.len();
        let mut map = CrateDefMap {
            krate: mod_tree.krate,
            edition: mod_tree.edition,
            prelude_mod,
            prelude,
            prelude_traits: Arc::from([]),
            prelude_traits_sorted: Arc::from([]),
            extern_prelude,
            items: collected.items,
            item_strings: collected.strings,
            uses: collected.uses,
            scopes,
            resolved_uses: vec![PerNs::EMPTY; use_count],
            dirty_files: dirty_files.to_vec(),
            indexed_clean: false,
            mod_tree: Arc::new(mod_tree.clone()),
            macro_bodies,
            local_impls: collected.local_impls,
            local_alias_targets: HashMap::new(),
            diagnostics,
        };

        run_use_fixed_point(&mut map, db, mod_tree, ext_cache, MAX_FIXED_POINT_ROUNDS);
        if seed_index_fallback_uses(&mut map, db, mod_tree, dirty_files, ext_cache) {
            run_use_fixed_point(&mut map, db, mod_tree, ext_cache, 4);
        }

        for uidx in 0..use_count {
            let u = map.uses.uses[uidx];
            if u.kind != FlatUseKind::Glob && map.resolved_uses[uidx].is_empty() {
                map.diagnostics.push(UseDiag::UnresolvedUse { module: u.module, use_index: uidx as u32 });
            }
        }
        finalize_in_scope_trait_tables(&mut map, mod_tree);
        map
    }

    pub(super) fn resolve_arbitrary_path(
        map: &CrateDefMap,
        db: &dyn ItemDb,
        mod_tree: &ModuleTree,
        ext_cache: &mut ExtScopeCache,
        from_mod: ModuleId,
        anchor: PathAnchor,
        segments: &[&str],
    ) -> PerNs {
        resolve_segments(map, db, mod_tree, ext_cache, from_mod, anchor, segments, false)
    }
}

pub(super) fn run_use_fixed_point(map: &mut CrateDefMap, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, max_rounds: usize) {
    let use_count = map.uses.uses.len();
    for _round in 0..max_rounds {
        let mut changed = false;
        for uidx in 0..use_count {
            let u = map.uses.uses[uidx];
            match u.kind {
                FlatUseKind::Named | FlatUseKind::Underscore => {
                    let resolved = resolve_use_leaf(map, db, mod_tree, ext_cache, &u);
                    if resolved.is_empty() || map.resolved_uses[uidx] == resolved {
                        continue;
                    }
                    map.resolved_uses[uidx] = resolved;
                    changed = true;
                    if u.kind == FlatUseKind::Underscore {
                        if let Some(tb) = resolved.types.filter(|b| b.kind == ItemKind::Trait) {
                            let list = &mut map.scopes[u.module.0 as usize].unnamed_traits;
                            if !list.iter().any(|x| x.same_target(tb)) {
                                list.push(tb);
                            }
                        }
                    } else {
                        let bind = map.uses.bind_name(&u);
                        let rebound = resolved.rebind(u.vis, u.vis_mod, BindingOrigin::Explicit);
                        let entries = &mut map.scopes[u.module.0 as usize].entries;
                        if let Some(slot) = entries.get_mut(bind) {
                            slot.merge_explicit(rebound);
                        } else {
                            let mut slot = PerNs::EMPTY;
                            slot.merge_explicit(rebound);
                            entries.insert(IdentKey::new(bind), slot);
                        }
                    }
                }
                FlatUseKind::Glob => {
                    changed |= apply_glob_use(map, db, mod_tree, ext_cache, uidx);
                }
            }
        }
        if !changed {
            break;
        }
    }
}

pub(super) fn seed_index_fallback_defs(db: &dyn ItemDb, mod_tree: &ModuleTree, dirty_files: &[FileId], scopes: &mut [ModuleScope]) {
    let mut defs = Vec::new();
    for (midx, scope) in scopes.iter_mut().enumerate() {
        seed_index_fallback_defs_for_mod(db, mod_tree, dirty_files, midx, scope, &mut defs);
    }
}

pub(super) fn seed_index_fallback_defs_for_mod<'a>(
    db: &'a dyn ItemDb,
    mod_tree: &ModuleTree,
    dirty_files: &[FileId],
    midx: usize,
    scope: &mut ModuleScope,
    defs: &mut Vec<super::db::IndexedDef<'a>>,
) {
    let idx_def = scope.index_def;
    if !idx_def.is_some() || idx_def.is_overlay() {
        return;
    }
    let mod_file = mod_tree.modules[midx].file;
    let file_dirty = dirty_files.contains(&mod_file);
    defs.clear();
    db.module_defs(idx_def, defs);
    let mod_id = ModuleId(midx as u32);
    for d in &*defs {
        if d.name.is_empty() || (file_dirty && db.def_file(d.id) == mod_file) {
            continue;
        }
        let (_, vmod) = compute_vis_mod(mod_tree, mod_id, d.vis, None);
        let b = ResolvedBinding {
            def: d.id,
            local: LocalItemId::NONE,
            local_mod: ModuleId::NONE,
            vis_mod: vmod,
            kind: d.kind,
            vis: d.vis,
            origin: BindingOrigin::Explicit,
            prim_ty: 0,
            ambiguous: false,
        };
        let fb = PerNs::from_binding(b, d.kind.ns_mask(d.has_value_ctor));
        let slot = scope.entries.entry(IdentKey::new(d.name)).or_insert(PerNs::EMPTY);
        *slot = slot.or_else_ns(fb);
    }
}

fn seed_index_fallback_uses(map: &mut CrateDefMap, db: &dyn ItemDb, mod_tree: &ModuleTree, dirty_files: &[FileId], ext_cache: &mut ExtScopeCache) -> bool {
    let mut uses = Vec::new();
    let mut glob_targets: Vec<(GlobalDefId, VisKind, ModuleId)> = Vec::new();
    let mut any_changed = false;
    for midx in 0..map.scopes.len() {
        let idx_def = map.scopes[midx].index_def;
        let mod_file = mod_tree.modules[midx].file;
        if !idx_def.is_some() || idx_def.is_overlay() || dirty_files.contains(&mod_file) {
            continue;
        }
        uses.clear();
        glob_targets.clear();
        db.module_uses(idx_def, &mut uses);
        let mod_id = ModuleId(midx as u32);
        for u in &uses {
            if u.is_prelude {
                continue;
            }
            let (_, vmod) = compute_vis_mod(mod_tree, mod_id, u.vis, None);
            if u.is_glob {
                if u.type_def.is_some() && u.type_def != idx_def {
                    glob_targets.push((u.type_def, u.vis, vmod));
                }
                continue;
            }
            if u.is_underscore {
                if u.type_def.is_some() && db.def_kind(u.type_def) == Some(ItemKind::Trait) {
                    let tb = ResolvedBinding {
                        def: u.type_def,
                        local: LocalItemId::NONE,
                        local_mod: ModuleId::NONE,
                        vis_mod: vmod,
                        kind: ItemKind::Trait,
                        vis: u.vis,
                        origin: BindingOrigin::Explicit,
                        prim_ty: 0,
                        ambiguous: false,
                    };
                    let list = &mut map.scopes[midx].unnamed_traits;
                    if !list.iter().any(|x| x.same_target(tb)) {
                        list.push(tb);
                        any_changed = true;
                    }
                }
                continue;
            }
            if u.name.is_empty() {
                continue;
            }
            let pns = indexed_use_to_per_ns(db, u).rebind(u.vis, vmod, BindingOrigin::Explicit);
            if !pns.is_empty() {
                let slot = map.scopes[midx].entries.entry(IdentKey::new(u.name)).or_insert(PerNs::EMPTY);
                let next = slot.or_else_ns(pns);
                if next != *slot {
                    *slot = next;
                    any_changed = true;
                }
            }
        }
        for (gdef, gvis, gvmod) in &glob_targets {
            let ext_s = ext_cache.module_scope(db, *gdef);
            let pairs: Vec<(IdentKey, PerNs)> = ext_s.entries.iter().map(|(k, &v)| (k.clone(), v.rebind(*gvis, *gvmod, BindingOrigin::Glob))).collect();
            for (k, pns) in pairs {
                any_changed |= map.scopes[midx].entries.entry(k).or_insert(PerNs::EMPTY).merge_glob(pns);
            }
        }
    }
    any_changed
}
