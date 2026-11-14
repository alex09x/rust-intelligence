/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Module item/use population and glob expansion helpers for [`super::snapshot_indexed`].

use std::collections::HashMap;

use super::db::{CrateId, GlobalDefId, ItemDb, ItemKind, ModuleId, VisKind};
use super::def_map::ModuleScope;
use super::ext_scope::{indexed_use_to_per_ns, ExtScopeCache};
use super::ident_key::IdentKey;
use super::mod_tree::ModuleTree;
use super::per_ns::{compute_vis_mod, is_binding_visible, BindingOrigin, LocalItemId, PerNs, ResolvedBinding};
use crate::sema_fn::norm_ident;

#[inline]
pub(super) fn indexed_vis_mod(tree: &ModuleTree, mod_id: ModuleId, vis: VisKind) -> (VisKind, ModuleId) {
    if vis == VisKind::Restricted {
        (VisKind::Crate, ModuleId::ROOT)
    } else {
        compute_vis_mod(tree, mod_id, vis, None)
    }
}

#[inline]
fn idx_binding(def: GlobalDefId, vis_mod: ModuleId, kind: ItemKind, vis: VisKind) -> ResolvedBinding {
    ResolvedBinding { def, local: LocalItemId::NONE, local_mod: ModuleId::NONE, vis_mod, kind, vis, origin: BindingOrigin::Explicit, prim_ty: 0, ambiguous: false }
}

pub(super) struct ModPopBufs<'b, 'a> {
    pub scopes: &'b mut [ModuleScope],
    pub extern_prelude: &'b mut HashMap<IdentKey, (CrateId, GlobalDefId)>,
    pub globs: &'b mut Vec<(usize, GlobalDefId, VisKind, ModuleId)>,
    pub defs: &'b mut Vec<super::db::IndexedDef<'a>>,
    pub uses: &'b mut Vec<super::db::IndexedUse<'a>>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn populate_mod_items<'a>(
    db: &'a dyn ItemDb,
    mod_tree: &ModuleTree,
    krate: CrateId,
    midx: usize,
    skip_private: bool,
    only_private: bool,
    gid_to_mod: &HashMap<GlobalDefId, ModuleId>,
    b: &mut ModPopBufs<'_, 'a>,
) {
    let mod_id = ModuleId(midx as u32);
    let mdef = b.scopes[midx].index_def;
    b.defs.clear();
    db.module_defs(mdef, b.defs);
    for d in &*b.defs {
        if (skip_private && d.vis == VisKind::Private) || (only_private && d.vis != VisKind::Private) {
            continue;
        }
        if d.name.is_empty() || matches!(d.kind, ItemKind::Mod | ItemKind::Use | ItemKind::Impl | ItemKind::Field) {
            continue;
        }
        let (v, vmod) = indexed_vis_mod(mod_tree, mod_id, d.vis);
        let rb = idx_binding(d.id, vmod, d.kind, v);
        b.scopes[midx].entries.entry(IdentKey::new(norm_ident(d.name))).or_insert(PerNs::EMPTY).merge_explicit(PerNs::from_binding(rb, d.kind.ns_mask(d.has_value_ctor)));
    }
    b.uses.clear();
    db.module_uses(mdef, b.uses);
    for u in &*b.uses {
        if u.is_prelude || (skip_private && u.vis == VisKind::Private) || (only_private && u.vis != VisKind::Private) {
            continue;
        }
        let (uvis, vmod) = indexed_vis_mod(mod_tree, mod_id, u.vis);
        if u.is_glob {
            if u.type_def.is_some() && u.type_def != mdef {
                b.globs.push((midx, u.type_def, uvis, vmod));
            }
            continue;
        }
        if u.is_underscore {
            if u.type_def.is_some() && db.def_kind(u.type_def) == Some(ItemKind::Trait) {
                let tb = idx_binding(u.type_def, vmod, ItemKind::Trait, uvis);
                let list = &mut b.scopes[midx].unnamed_traits;
                if !list.iter().any(|x| x.same_target(tb)) {
                    list.push(tb);
                }
            }
            continue;
        }
        if u.name.is_empty() {
            continue;
        }
        let mut pns = indexed_use_to_per_ns(db, u).rebind(uvis, vmod, BindingOrigin::Explicit);
        if let Some(tb) = pns.types.as_mut().filter(|rb| rb.kind == ItemKind::Mod) {
            if let Some(&lm) = gid_to_mod.get(&tb.def) {
                tb.local_mod = lm;
            } else if let Some(pos) = only_private.then(|| b.scopes.iter().position(|s| s.index_def == tb.def)).flatten() {
                tb.local_mod = ModuleId(pos as u32);
            } else if midx == 0 && tb.def.krate != krate && db.crate_root_def(tb.def.krate) == Some(tb.def) {
                b.extern_prelude.entry(IdentKey::new(norm_ident(u.name))).or_insert((tb.def.krate, tb.def));
            }
        }
        if !pns.is_empty() {
            b.scopes[midx].entries.entry(IdentKey::new(norm_ident(u.name))).or_insert(PerNs::EMPTY).merge_explicit(pns);
        }
    }
}

pub(super) fn expand_indexed_globs(
    db: &dyn ItemDb,
    mod_tree: &ModuleTree,
    gid_to_mod: &HashMap<GlobalDefId, ModuleId>,
    globs: &[(usize, GlobalDefId, VisKind, ModuleId)],
    ext_cache: &mut ExtScopeCache,
    scopes: &mut [ModuleScope],
) {
    let mut local_globs: Vec<(usize, usize, VisKind, ModuleId)> = Vec::new();
    for &(midx, gdef, gvis, gvmod) in globs {
        if let Some(&src_m) = gid_to_mod.get(&gdef) {
            local_globs.push((midx, src_m.0 as usize, gvis, gvmod));
            continue;
        }
        if gid_to_mod.is_empty() {
            if let Some(src_m) = scopes.iter().position(|s| s.index_def == gdef) {
                local_globs.push((midx, src_m, gvis, gvmod));
                continue;
            }
        }
        let ext_s = ext_cache.module_scope(db, gdef);
        scopes[midx].entries.reserve(ext_s.entries.len());
        for (k, &v) in &ext_s.entries {
            let reb = v.rebind(gvis, gvmod, BindingOrigin::Glob);
            if let Some(slot) = scopes[midx].entries.get_mut(k) {
                slot.merge_glob(reb);
            } else {
                let mut pns = PerNs::EMPTY;
                pns.merge_glob(reb);
                scopes[midx].entries.insert(k.clone(), pns);
            }
        }
    }
    for _ in 0..if gid_to_mod.is_empty() { 1 } else { 4 } {
        let mut changed = false;
        for &(dst, src, gvis, gvmod) in &local_globs {
            if dst == src {
                continue;
            }
            let dst_id = ModuleId(dst as u32);
            let (src_s, dst_s) = if src < dst {
                let (lo, hi) = scopes.split_at_mut(dst);
                (&lo[src], &mut hi[0])
            } else {
                let (lo, hi) = scopes.split_at_mut(src);
                (&hi[0], &mut lo[dst])
            };
            for (k, &v) in &src_s.entries {
                let fv = PerNs {
                    types: v.types.filter(|&b| is_binding_visible(mod_tree, b, dst_id)),
                    values: v.values.filter(|&b| is_binding_visible(mod_tree, b, dst_id)),
                    macros: v.macros.filter(|&b| is_binding_visible(mod_tree, b, dst_id)),
                }
                .rebind(gvis, gvmod, BindingOrigin::Glob);
                if !fv.is_empty() {
                    if let Some(slot) = dst_s.entries.get_mut(k) {
                        changed |= slot.merge_glob(fv);
                    } else {
                        let mut pns = PerNs::EMPTY;
                        changed |= pns.merge_glob(fv);
                        dst_s.entries.insert(k.clone(), pns);
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}
