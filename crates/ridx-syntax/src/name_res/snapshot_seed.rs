/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Module-scope seeding and incremental `use` fixed-point helpers for [`super::snapshot_incremental::CleanSolverBase`].

use std::collections::HashMap;

use super::db::{CrateId, FileId, GlobalDefId, ItemDb, ItemKind, ModuleId, NsMask, VisKind};
use super::def_collect::CollectedCrate;
use super::def_map::{extern_crate_binding, local_mod_binding, CrateDefMap, ModuleScope};
use super::def_path::{apply_glob_use, resolve_use_leaf};
use super::ext_scope::ExtScopeCache;
use super::ident_key::IdentKey;
use super::mod_tree::ModuleTree;
use super::per_ns::{compute_vis_mod, BindingOrigin, PerNs, ResolvedBinding};
use super::use_tree::{FlatUse, FlatUseKind, PathAnchor};
use crate::sema_fn::norm_ident;

pub(super) fn global_invariants_hold(clean: &CollectedCrate, spliced: &CollectedCrate, dirty_files: &[FileId], dirty_mod_mask: &[bool]) -> bool {
    if dirty_mod_mask.first().copied().unwrap_or(false) {
        let root_ec_a: Vec<_> = clean.uses.extern_crates.iter().filter(|ec| ec.module == ModuleId::ROOT).collect();
        let root_ec_b: Vec<_> = spliced.uses.extern_crates.iter().filter(|ec| ec.module == ModuleId::ROOT).collect();
        if root_ec_a.len() != root_ec_b.len() {
            return false;
        }
        for (a, b) in root_ec_a.into_iter().zip(root_ec_b) {
            if a.vis != b.vis
                || a.is_self != b.is_self
                || a.is_underscore != b.is_underscore
                || clean.uses.extern_orig(a) != spliced.uses.extern_orig(b)
                || clean.uses.extern_bind(a) != spliced.uses.extern_bind(b)
            {
                return false;
            }
        }
    }
    if clean.items.iter().any(|it| dirty_files.contains(&it.file) && it.macro_export) {
        return false;
    }
    if spliced.items[clean.items.len()..].iter().any(|it| it.macro_export) {
        return false;
    }
    for (midx, &is_dirty) in dirty_mod_mask.iter().enumerate() {
        if !is_dirty && clean.textual_macros[midx] != spliced.textual_macros[midx] {
            return false;
        }
    }
    true
}

pub(super) fn seed_mod_children(mod_tree: &ModuleTree, idx: usize, scopes: &mut [ModuleScope]) {
    let pid = ModuleId(idx as u32);
    for edge in mod_tree.children(pid) {
        let cname = IdentKey::new(mod_tree.child_name(edge));
        let (_, vmod) = compute_vis_mod(mod_tree, pid, edge.vis, None);
        let cdef = scopes[edge.child.0 as usize].index_def;
        scopes[idx].entries.entry(cname).or_insert(PerNs::EMPTY).merge_explicit(local_mod_binding(edge.child, cdef, edge.vis, vmod));
    }
}

pub(super) fn seed_explicit_items(collected: &CollectedCrate, only_mask: Option<&[bool]>, scopes: &mut [ModuleScope]) {
    for it in &collected.items {
        if !it.module.is_some() || it.parent_enum.is_some() {
            continue;
        }
        if only_mask.is_some_and(|m| !m[it.module.0 as usize]) {
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
        if it.macro_export && !scopes.is_empty() && only_mask.is_none() {
            let exp_b = b.rebind(VisKind::Public, ModuleId::NONE, BindingOrigin::Explicit);
            scopes[ModuleId::ROOT.0 as usize].entries.entry(IdentKey::new(name)).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(exp_b, NsMask::MACRO));
        }
    }
}

pub(super) fn seed_textual_macros(collected: &CollectedCrate, only_mask: Option<&[bool]>, scopes: &mut [ModuleScope]) {
    for (midx, tlist) in collected.textual_macros.iter().enumerate() {
        if only_mask.is_some_and(|m| !m[midx]) {
            continue;
        }
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
}

pub(super) fn seed_extern_crates(
    db: &dyn ItemDb,
    mod_tree: &ModuleTree,
    extern_prelude: &HashMap<IdentKey, (CrateId, GlobalDefId)>,
    collected: &CollectedCrate,
    only_mask: Option<&[bool]>,
    scopes: &mut [ModuleScope],
) {
    for ec in &collected.uses.extern_crates {
        if only_mask.is_some_and(|m| !m[ec.module.0 as usize]) || ec.is_underscore {
            continue;
        }
        let bind = collected.uses.extern_bind(ec);
        if ec.is_self {
            let root_def = scopes.first().map_or(GlobalDefId::NONE, |s| s.index_def);
            let pns = local_mod_binding(ModuleId::ROOT, root_def, ec.vis, ec.vis_mod);
            scopes[ec.module.0 as usize].entries.entry(IdentKey::new(bind)).or_insert(PerNs::EMPTY).merge_explicit(pns);
            continue;
        }
        let orig = collected.uses.extern_orig(ec);
        let resolved_ext = extern_prelude.get(orig).copied().or_else(|| {
            let cid = db.extern_crate(mod_tree.krate, orig)?;
            let rdef = db.crate_root_def(cid).unwrap_or(GlobalDefId::new(cid, 0));
            Some((cid, rdef))
        });
        if let Some((cid, rdef)) = resolved_ext {
            let mut b = extern_crate_binding(cid, mod_tree.krate, rdef);
            b.vis = ec.vis;
            b.vis_mod = ec.vis_mod;
            b.origin = BindingOrigin::Explicit;
            scopes[ec.module.0 as usize].entries.entry(IdentKey::new(bind)).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(b, NsMask::TYPE));
        }
    }
}

pub(super) fn run_use_range_fixed_point(
    map: &mut CrateDefMap,
    db: &dyn ItemDb,
    mod_tree: &ModuleTree,
    ext_cache: &mut ExtScopeCache,
    range: std::ops::Range<usize>,
    max_rounds: usize,
) {
    if range.is_empty() {
        return;
    }
    for _ in 0..max_rounds {
        let mut changed = false;
        for uidx in range.clone() {
            changed |= step_single_use(map, db, mod_tree, ext_cache, uidx);
        }
        if !changed {
            break;
        }
    }
}

pub(super) fn run_use_fixed_point_with_clean_ext(
    map: &mut CrateDefMap,
    db: &dyn ItemDb,
    mod_tree: &ModuleTree,
    ext_cache: &mut ExtScopeCache,
    clean_map: &CrateDefMap,
    clean_use_map: &[u32],
    max_rounds: usize,
) {
    let use_count = map.uses.uses.len();
    for round in 0..max_rounds {
        let mut changed = false;
        for uidx in 0..use_count {
            let u = map.uses.uses[uidx];
            if let Some(&old_idx) = clean_use_map.get(uidx) {
                if matches!(u.kind, FlatUseKind::Named | FlatUseKind::Underscore) && is_unshadowed_ext_use(map, mod_tree, &u) {
                    if round == 0 {
                        let cached = clean_map.resolved_uses[old_idx as usize];
                        if !cached.is_empty() {
                            map.resolved_uses[uidx] = cached;
                            changed |= apply_resolved_use_binding(map, &u, cached);
                        }
                    }
                    continue;
                }
            }
            changed |= step_single_use(map, db, mod_tree, ext_cache, uidx);
        }
        if !changed {
            break;
        }
    }
}

#[inline]
fn is_unshadowed_ext_use(map: &CrateDefMap, mod_tree: &ModuleTree, u: &FlatUse) -> bool {
    let spans = map.uses.seg_spans(u);
    if spans.len() < 2 || !map.edition.has_uniform_paths() {
        return false;
    }
    let first = norm_ident(map.uses.str_span(spans[0]));
    match u.anchor {
        PathAnchor::ExternRoot => map.extern_prelude.get(first).is_some_and(|&(cid, _)| cid != map.krate),
        PathAnchor::Bare => {
            if !map.extern_prelude.get(first).is_some_and(|&(cid, _)| cid != map.krate) {
                return false;
            }
            if map.prelude.get(first).is_some_and(|p| p.types.is_some()) {
                return false;
            }
            map.scopes[u.module.0 as usize].get(first).filter_visible(mod_tree, u.module).types.is_none()
        }
        _ => false,
    }
}

#[inline]
fn step_single_use(map: &mut CrateDefMap, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, uidx: usize) -> bool {
    let u = map.uses.uses[uidx];
    match u.kind {
        FlatUseKind::Named | FlatUseKind::Underscore => {
            let resolved = resolve_use_leaf(map, db, mod_tree, ext_cache, &u);
            if resolved.is_empty() || map.resolved_uses[uidx] == resolved {
                return false;
            }
            map.resolved_uses[uidx] = resolved;
            apply_resolved_use_binding(map, &u, resolved);
            true
        }
        FlatUseKind::Glob => apply_glob_use(map, db, mod_tree, ext_cache, uidx),
    }
}

#[inline]
fn apply_resolved_use_binding(map: &mut CrateDefMap, u: &FlatUse, resolved: PerNs) -> bool {
    if u.kind == FlatUseKind::Underscore {
        if let Some(tb) = resolved.types.filter(|b| b.kind == ItemKind::Trait) {
            let list = &mut map.scopes[u.module.0 as usize].unnamed_traits;
            if !list.iter().any(|x| x.same_target(tb)) {
                list.push(tb);
                return true;
            }
        }
        false
    } else {
        let bind = map.uses.bind_name(u);
        let rebound = resolved.rebind(u.vis, u.vis_mod, BindingOrigin::Explicit);
        let entries = &mut map.scopes[u.module.0 as usize].entries;
        if let Some(slot) = entries.get_mut(bind) {
            let before = *slot;
            slot.merge_explicit(rebound);
            *slot != before
        } else {
            let mut slot = PerNs::EMPTY;
            slot.merge_explicit(rebound);
            entries.insert(IdentKey::new(bind), slot);
            true
        }
    }
}
