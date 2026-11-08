/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Post-import-resolution lowering of [`super::impl_collect::LocalImplArena`] headers into
//! [`ImplTargetKey`]s and `trait_def`s, preserving compiler [`GlobalDefId`]s for surviving dirty
//! items and assigning `LOCAL_OVERLAY` IDs to newly introduced items.

use std::collections::HashMap;

use super::db::{CrateId, GlobalDefId, IndexedDef, ItemDb, ItemKind, ModuleId};
use super::db_impls::{ImplTargetKey, IndexedImpl};
use super::def_map::CrateDefMap;
use super::def_path::resolve_segments;
use super::ext_scope::ExtScopeCache;
use super::impl_collect::RawSelfHead;
use super::mod_tree::ModuleTree;
use super::per_ns::{PrimTy, ResolvedBinding};
use super::use_tree::PathAnchor;

pub(super) fn resolve_local_impls_and_assocs(map: &mut CrateDefMap, db: &dyn ItemDb, mod_tree: &ModuleTree, ext_cache: &mut ExtScopeCache, mut next_overlay_seq: u32) {
    // 1. Resolve local `type Alias = Target;` targets (up to 4 passes for chained local aliases).
    let mut alias_targets: HashMap<GlobalDefId, ImplTargetKey> = HashMap::new();
    for _ in 0..4 {
        let mut progressed = false;
        for ta_idx in 0..map.local_impls.type_aliases.len() {
            let ta = map.local_impls.type_aliases[ta_idx];
            let alias_def = map.items[ta.item_id.0 as usize].index_def;
            if !alias_def.is_some() || alias_targets.contains_key(&alias_def) {
                continue;
            }
            let key = eval_raw_head(map, db, mod_tree, ext_cache, &alias_targets, ta.module, ta.gp_start, ta.gp_len, ta.raw_head);
            if key != ImplTargetKey::Unknown {
                alias_targets.insert(alias_def, key);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }

    // 2. Match trait associated items against `ItemDb::assoc_items` (or assign `LOCAL_OVERLAY`),
    // and resolve transitive supertraits for locally scanned `trait` definitions.
    map.local_impls.local_trait_supers.clear();
    let mut direct_supers = Vec::with_capacity(map.local_impls.raw_trait_supers.len());
    for idx in 0..map.local_impls.raw_trait_supers.len() {
        let rts = map.local_impls.raw_trait_supers[idx];
        if let Some(tb) = resolve_arena_path_types(map, db, mod_tree, ext_cache, rts.module, rts.anchor, rts.seg_start, rts.seg_len) {
            if tb.kind == ItemKind::Trait && tb.def.is_some() {
                direct_supers.push((rts.trait_item, tb.def));
            }
        }
    }
    for &(tlid, _, _) in &map.local_impls.trait_assocs {
        let mut chain = Vec::with_capacity(4);
        for &(slid, sdef) in &direct_supers {
            if slid == tlid && !chain.contains(&sdef) {
                chain.push(sdef);
            }
        }
        let mut head = 0usize;
        let mut idx_supers = Vec::new();
        while head < chain.len() && chain.len() < 16 {
            let cur = chain[head];
            head += 1;
            if let Some(&(sub_lid, _, _)) = map.local_impls.trait_assocs.iter().find(|&&(l, _, _)| map.items[l.0 as usize].index_def == cur) {
                for &(slid, sdef) in &direct_supers {
                    if slid == sub_lid && !chain.contains(&sdef) && chain.len() < 16 {
                        chain.push(sdef);
                    }
                }
            } else {
                idx_supers.clear();
                db.trait_supertraits(cur, &mut idx_supers);
                for &sdef in &idx_supers {
                    if sdef.is_some() && !chain.contains(&sdef) && chain.len() < 16 {
                        chain.push(sdef);
                    }
                }
            }
        }
        for sdef in chain {
            map.local_impls.local_trait_supers.push((tlid, sdef));
        }
    }

    let mut scratch_assoc: Vec<IndexedDef<'_>> = Vec::new();
    for &(trait_lid, a_start, a_len) in &map.local_impls.trait_assocs {
        let trait_def = map.items[trait_lid.0 as usize].index_def;
        scratch_assoc.clear();
        if trait_def.is_some() && !trait_def.is_overlay() {
            db.assoc_items(trait_def, &mut scratch_assoc);
        }
        let s = a_start as usize;
        let e = s + a_len as usize;
        let str_buf = &map.local_impls.strings;
        for ai in &mut map.local_impls.assoc_items[s..e] {
            let aname = &str_buf[ai.name_start as usize..ai.name_end as usize];
            if let Some(found) = scratch_assoc.iter().find(|d| d.name == aname && d.kind.ns_mask(false) == ai.kind.ns_mask(false)) {
                ai.def = found.id;
                ai.kind = found.kind;
            } else {
                ai.def = GlobalDefId::new(CrateId::LOCAL_OVERLAY, next_overlay_seq);
                next_overlay_seq += 1;
            }
        }
    }

    // 3. Collect prior indexed impls in `map.krate` so surviving dirty impls preserve `GlobalDefId`s.
    let mut prior_crate_impls: Vec<IndexedImpl> = Vec::new();
    db.crate_impls(map.krate, &mut prior_crate_impls);
    let mut claimed_prior = vec![false; prior_crate_impls.len()];

    // 4. Resolve each `LocalImplData` header (`target`, `trait_def`) and match associated item IDs.
    let impl_len = map.local_impls.impls.len();
    for idx in 0..impl_len {
        let imp = map.local_impls.impls[idx];
        let target = eval_raw_head(map, db, mod_tree, ext_cache, &alias_targets, imp.module, imp.gp_start, imp.gp_len, imp.raw_head);
        let trait_def = if imp.has_trait && imp.trait_seg_len > 0 {
            resolve_arena_path_types(map, db, mod_tree, ext_cache, imp.module, imp.trait_anchor, imp.trait_seg_start, imp.trait_seg_len)
                .filter(|b| b.kind == ItemKind::Trait)
                .map_or(GlobalDefId::NONE, |b| b.def)
        } else {
            GlobalDefId::NONE
        };

        let matched_prior = prior_crate_impls
            .iter()
            .enumerate()
            .find(|&(pi, p)| !claimed_prior[pi] && p.file == imp.file && p.target == target && p.trait_def == trait_def && p.is_negative == imp.is_negative)
            .map(|(pi, p)| {
                claimed_prior[pi] = true;
                p.id
            });

        let impl_def = matched_prior.unwrap_or_else(|| {
            let g = GlobalDefId::new(CrateId::LOCAL_OVERLAY, next_overlay_seq);
            next_overlay_seq += 1;
            g
        });

        scratch_assoc.clear();
        if let Some(p_id) = matched_prior {
            db.assoc_items(p_id, &mut scratch_assoc);
        }
        let s = imp.assoc_start as usize;
        let e = s + imp.assoc_len as usize;
        let str_buf = &map.local_impls.strings;
        for ai in &mut map.local_impls.assoc_items[s..e] {
            let aname = &str_buf[ai.name_start as usize..ai.name_end as usize];
            if let Some(found) = scratch_assoc.iter().find(|d| d.name == aname && d.kind.ns_mask(false) == ai.kind.ns_mask(false)) {
                ai.def = found.id;
                ai.kind = found.kind;
            } else {
                ai.def = GlobalDefId::new(CrateId::LOCAL_OVERLAY, next_overlay_seq);
                next_overlay_seq += 1;
            }
        }

        let slot = &mut map.local_impls.impls[idx];
        slot.target = target;
        slot.trait_def = trait_def;
        slot.index_def = impl_def;
    }

    map.local_alias_targets = alias_targets;
}

#[allow(clippy::too_many_arguments)]
fn eval_raw_head(
    map: &CrateDefMap,
    db: &dyn ItemDb,
    mod_tree: &ModuleTree,
    ext_cache: &mut ExtScopeCache,
    alias_targets: &HashMap<GlobalDefId, ImplTargetKey>,
    from_mod: ModuleId,
    gp_start: u32,
    gp_len: u16,
    head: RawSelfHead,
) -> ImplTargetKey {
    match head {
        RawSelfHead::Unknown => ImplTargetKey::Unknown,
        RawSelfHead::Slice => ImplTargetKey::Slice,
        RawSelfHead::Array => ImplTargetKey::Array,
        RawSelfHead::Tuple(n) => ImplTargetKey::Tuple(n),
        RawSelfHead::Ref { is_mut } => ImplTargetKey::Ref { is_mut },
        RawSelfHead::Ptr { is_mut } => ImplTargetKey::Ptr { is_mut },
        RawSelfHead::FnPtr { arity } => ImplTargetKey::FnPtr { arity },
        RawSelfHead::Never => ImplTargetKey::Never,
        RawSelfHead::DynPath { anchor, seg_start, seg_len } => {
            if seg_len == 0 {
                return ImplTargetKey::Unknown;
            }
            resolve_arena_path_types(map, db, mod_tree, ext_cache, from_mod, anchor, seg_start, seg_len)
                .filter(|b| b.kind == ItemKind::Trait && b.def.is_some())
                .map_or(ImplTargetKey::Unknown, |b| ImplTargetKey::Dyn(b.def))
        }
        RawSelfHead::Path { anchor, seg_start, seg_len } => {
            if seg_len == 0 {
                return ImplTargetKey::Unknown;
            }
            if anchor == PathAnchor::Bare && seg_len == 1 {
                let single_sp = map.local_impls.segs[seg_start as usize];
                let single = &map.local_impls.strings[single_sp.0 as usize..single_sp.1 as usize];
                let gp_s = gp_start as usize;
                let gp_e = gp_s + gp_len as usize;
                for &(gs, ge) in &map.local_impls.generic_params[gp_s..gp_e] {
                    if &map.local_impls.strings[gs as usize..ge as usize] == single {
                        return ImplTargetKey::Param;
                    }
                }
            }
            let Some(tb) = resolve_arena_path_types(map, db, mod_tree, ext_cache, from_mod, anchor, seg_start, seg_len) else {
                return ImplTargetKey::Unknown;
            };
            if tb.prim_ty != 0 {
                return PrimTy::from_code(tb.prim_ty).map_or(ImplTargetKey::Unknown, ImplTargetKey::Prim);
            }
            match tb.kind {
                ItemKind::Struct | ItemKind::Enum | ItemKind::Union if tb.def.is_some() => ImplTargetKey::Adt(tb.def),
                ItemKind::TypeAlias if tb.def.is_some() => alias_targets.get(&tb.def).copied().or_else(|| db.type_alias_target(tb.def)).unwrap_or(ImplTargetKey::Unknown),
                _ => ImplTargetKey::Unknown,
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_arena_path_types(
    map: &CrateDefMap,
    db: &dyn ItemDb,
    mod_tree: &ModuleTree,
    ext_cache: &mut ExtScopeCache,
    from_mod: ModuleId,
    anchor: PathAnchor,
    seg_start: u32,
    seg_len: u16,
) -> Option<ResolvedBinding> {
    let s = seg_start as usize;
    let spans = &map.local_impls.segs[s..s + seg_len as usize];
    if spans.len() <= 16 {
        let mut buf: [&str; 16] = [""; 16];
        for (i, &(bs, be)) in spans.iter().enumerate() {
            buf[i] = &map.local_impls.strings[bs as usize..be as usize];
        }
        return resolve_segments(map, db, mod_tree, ext_cache, from_mod, anchor, &buf[..spans.len()], false).types;
    }
    let segs: Vec<&str> = spans.iter().map(|&(bs, be)| &map.local_impls.strings[bs as usize..be as usize]).collect();
    resolve_segments(map, db, mod_tree, ext_cache, from_mod, anchor, &segs, false).types
}
