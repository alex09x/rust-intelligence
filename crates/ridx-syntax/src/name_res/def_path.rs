/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Multi-segment path walker and glob expander across local modules, external indexed crates,
//! enums, preludes, and builtin primitives.

use super::db::{GlobalDefId, ItemDb, ItemKind, ModuleId, NsMask, VisKind};
use super::def_map::{extern_crate_binding, local_mod_binding, CrateDefMap};
use super::ext_scope::ExtScopeCache;
use super::mod_tree::ModuleTree;
use super::per_ns::{BindingOrigin, LocalItemId, PerNs, PrimTy, ResolvedBinding};
use super::use_tree::{FlatUse, PathAnchor};
use crate::sema_fn::norm_ident;

#[derive(Clone, Copy, Debug)]
enum Container {
    LocalMod(ModuleId),
    ExtMod(GlobalDefId),
    LocalEnum(LocalItemId),
    ExtEnum(GlobalDefId),
}

pub(super) fn resolve_use_leaf(map: &CrateDefMap, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, u: &FlatUse) -> PerNs {
    let spans = map.uses.seg_spans(u);
    if spans.len() <= 16 {
        let mut stack_segs: [&str; 16] = [""; 16];
        for (i, &sp) in spans.iter().enumerate() {
            stack_segs[i] = map.uses.str_span(sp);
        }
        return resolve_segments(map, db, mod_tree, ext_cache, u.module, u.anchor, &stack_segs[..spans.len()], true);
    }
    let mut segs: Vec<&str> = Vec::with_capacity(spans.len());
    for &sp in spans {
        segs.push(map.uses.str_span(sp));
    }
    resolve_segments(map, db, mod_tree, ext_cache, u.module, u.anchor, &segs, true)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_segments(
    map: &CrateDefMap,
    db: &dyn ItemDb,
    mod_tree: &ModuleTree,
    ext_cache: &mut ExtScopeCache,
    from_mod: ModuleId,
    anchor: PathAnchor,
    segs: &[&str],
    in_use_stmt: bool,
) -> PerNs {
    if segs.is_empty() {
        let target_mod = match anchor {
            PathAnchor::CrateRoot => ModuleId::ROOT,
            PathAnchor::SelfMod => from_mod,
            PathAnchor::Super(n) => step_super(mod_tree, from_mod, n),
            _ => return PerNs::EMPTY,
        };
        return local_mod_binding(target_mod, map.scopes[target_mod.0 as usize].index_def, VisKind::Public, ModuleId::NONE);
    }

    let first = norm_ident(segs[0]);
    let (mut cur, start_idx) = match anchor {
        PathAnchor::CrateRoot => (Container::LocalMod(ModuleId::ROOT), 0usize),
        PathAnchor::SelfMod => (Container::LocalMod(from_mod), 0usize),
        PathAnchor::Super(n) => (Container::LocalMod(step_super(mod_tree, from_mod, n)), 0usize),
        PathAnchor::ExternRoot => {
            if map.edition.has_uniform_paths() {
                let Some(&(cid, rdef)) = map.extern_prelude.get(first) else { return PerNs::EMPTY };
                if segs.len() == 1 {
                    return PerNs::from_binding(extern_crate_binding(cid, map.krate, rdef), NsMask::TYPE);
                }
                let cont = if cid == map.krate { Container::LocalMod(ModuleId::ROOT) } else { Container::ExtMod(rdef) };
                (cont, 1usize)
            } else {
                (Container::LocalMod(ModuleId::ROOT), 0usize)
            }
        }
        PathAnchor::Bare => {
            if !map.edition.has_uniform_paths() && in_use_stmt {
                (Container::LocalMod(ModuleId::ROOT), 0usize)
            } else {
                let first_pns = lookup_first_bare_segment(map, mod_tree, from_mod, first);
                if segs.len() == 1 {
                    return first_pns;
                }
                let Some(next_c) = binding_to_container(first_pns.types) else { return PerNs::EMPTY };
                (next_c, 1usize)
            }
        }
    };

    for (pos, &raw_seg) in segs[start_idx..].iter().enumerate() {
        let seg = norm_ident(raw_seg);
        let is_last = start_idx + pos + 1 == segs.len();
        let step = lookup_in_container(map, db, mod_tree, ext_cache, cur, seg, from_mod);
        if is_last {
            return step;
        }
        let Some(next_c) = binding_to_container(step.types) else {
            return PerNs::EMPTY;
        };
        cur = next_c;
    }
    PerNs::EMPTY
}

fn lookup_first_bare_segment(map: &CrateDefMap, mod_tree: &ModuleTree, from_mod: ModuleId, seg: &str) -> PerNs {
    let mut pns = map.scopes[from_mod.0 as usize].get(seg).filter_visible(mod_tree, from_mod);
    if !mod_tree.module(from_mod).has_no_implicit_prelude() {
        if let Some(&pre) = map.prelude.get(seg) {
            pns = pns.or_else_ns(pre);
        }
    }
    if pns.types.is_none() {
        if let Some(&(cid, rdef)) = map.extern_prelude.get(seg) {
            pns.types = Some(extern_crate_binding(cid, map.krate, rdef));
        }
    }
    if pns.types.is_none() {
        if let Some(p) = PrimTy::from_name(seg) {
            pns.types = Some(ResolvedBinding::primitive(p, VisKind::Public, ModuleId::NONE, BindingOrigin::Builtin));
        }
    }
    pns
}

fn binding_to_container(type_b: Option<ResolvedBinding>) -> Option<Container> {
    let b = type_b?;
    if b.local_mod.is_some() {
        return Some(Container::LocalMod(b.local_mod));
    }
    match b.kind {
        ItemKind::Mod if b.def.is_some() => Some(Container::ExtMod(b.def)),
        ItemKind::Enum if b.local.is_some() => Some(Container::LocalEnum(b.local)),
        ItemKind::Enum if b.def.is_some() => Some(Container::ExtEnum(b.def)),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn lookup_in_container(map: &CrateDefMap, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, container: Container, seg: &str, from_mod: ModuleId) -> PerNs {
    match container {
        Container::LocalMod(m) => map.scopes[m.0 as usize].get(seg).filter_visible(mod_tree, from_mod),
        Container::ExtMod(mdef) => ext_cache.module_scope(db, mdef).get(seg),
        Container::ExtEnum(edef) => ext_cache.enum_scope(db, edef).get(seg),
        Container::LocalEnum(elid) => {
            let eit = map.item(elid);
            let start = eit.var_start as usize;
            let end = start + eit.var_len as usize;
            for vit in &map.items[start..end] {
                if map.item_name(vit.id) == seg {
                    let b = ResolvedBinding {
                        def: vit.index_def,
                        local: vit.id,
                        local_mod: ModuleId::NONE,
                        vis_mod: vit.vis_mod,
                        kind: ItemKind::Variant,
                        vis: vit.vis,
                        origin: BindingOrigin::Explicit,
                        prim_ty: 0,
                        ambiguous: false,
                    };
                    return PerNs::from_binding(b, ItemKind::Variant.ns_mask(vit.has_value_ctor));
                }
            }
            PerNs::EMPTY
        }
    }
}

pub(super) fn apply_glob_use(map: &mut CrateDefMap, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, uidx: usize) -> bool {
    let u = map.uses.uses[uidx];
    let target_pns = resolve_use_leaf(map, db, mod_tree, ext_cache, &u);
    let Some(container) = binding_to_container(target_pns.types) else { return false };
    if let Container::LocalMod(src_m) = container {
        if src_m == u.module {
            return false;
        }
    }
    map.resolved_uses[uidx] = target_pns;
    let dst_idx = u.module.0 as usize;
    let mut changed = false;

    let merge_entry = |dst: &mut std::collections::HashMap<String, PerNs>, name: &str, pns: PerNs| -> bool {
        let rebound = pns.rebind(u.vis, u.vis_mod, BindingOrigin::Glob);
        if let Some(slot) = dst.get_mut(name) {
            slot.merge_glob(rebound)
        } else {
            let mut slot = PerNs::EMPTY;
            let ch = slot.merge_glob(rebound);
            dst.insert(name.to_string(), slot);
            ch
        }
    };

    match container {
        Container::LocalMod(src_m) => {
            let src_idx = src_m.0 as usize;
            let (src_sc, dst_sc) = if src_idx < dst_idx {
                let (left, right) = map.scopes.split_at_mut(dst_idx);
                (&left[src_idx], &mut right[0])
            } else {
                let (left, right) = map.scopes.split_at_mut(src_idx);
                (&right[0], &mut left[dst_idx])
            };
            for (k, &pns) in &src_sc.entries {
                let vis_pns = pns.filter_visible(mod_tree, u.module);
                if !vis_pns.is_empty() {
                    changed |= merge_entry(&mut dst_sc.entries, k, vis_pns);
                }
            }
        }
        Container::ExtMod(mdef) => {
            let ext_sc = ext_cache.module_scope(db, mdef);
            let dst = &mut map.scopes[dst_idx].entries;
            for (k, &pns) in &ext_sc.entries {
                changed |= merge_entry(dst, k, pns);
            }
        }
        Container::ExtEnum(edef) => {
            let ext_sc = ext_cache.enum_scope(db, edef);
            let dst = &mut map.scopes[dst_idx].entries;
            for (k, &pns) in &ext_sc.entries {
                changed |= merge_entry(dst, k, pns);
            }
        }
        Container::LocalEnum(elid) => {
            let eit = map.items[elid.0 as usize];
            let start = eit.var_start as usize;
            let end = start + eit.var_len as usize;
            let dst = &mut map.scopes[dst_idx].entries;
            for vit in &map.items[start..end] {
                let vname = &map.item_strings[vit.name_start as usize..vit.name_end as usize];
                let b = ResolvedBinding {
                    def: vit.index_def,
                    local: vit.id,
                    local_mod: ModuleId::NONE,
                    vis_mod: vit.vis_mod,
                    kind: ItemKind::Variant,
                    vis: vit.vis,
                    origin: BindingOrigin::Glob,
                    prim_ty: 0,
                    ambiguous: false,
                };
                let pns = PerNs::from_binding(b, ItemKind::Variant.ns_mask(vit.has_value_ctor));
                changed |= merge_entry(dst, vname, pns);
            }
        }
    }
    changed
}

fn step_super(tree: &ModuleTree, mut cur: ModuleId, count: u16) -> ModuleId {
    for _ in 0..count {
        let p = tree.module(cur).parent;
        if p.is_some() {
            cur = p;
        } else {
            return ModuleId::ROOT;
        }
    }
    cur
}
