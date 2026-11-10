/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `path_candidates` implementation for [`super::query::LiveDb`]: resolves `Type::`, `Self::`,
//! `<T as Trait>::`, `Trait::`, generic `T::`, `Enum::`, primitive `u32::`/`str::`, and `module::`
//! qualifiers, returning visible variants, associated items (`fn`, `const`, `type`), and module items.

use std::collections::HashSet;
use std::path::Path;

use ridx_format::{DefKind, WorkspaceIndex};
use ridx_syntax::name_res::{
    CrateDefMap, CrateId, ExtScopeCache, FileId, GlobalDefId, ImplTargetKey, ItemDb, ItemKind as IK, ModuleId, ModuleTree, PerNs, PrimTy, ResolvedBinding, VisKind,
};
use ridx_syntax::ty_infer::{LocalCstDefs, TyDefDb};
use ridx_syntax::SyntaxKind as K;

use super::query::Cand;
use super::query_dot::deepest_node_at;
use super::query_loc::{def_signature_detail, is_def_visible_from, map_item_kind_to_def_kind, resolve_def_location, write_def_name};
use super::query_path_qual::{
    extract_colon2_qualifier, find_enclosing_impl_or_trait, find_local_module_id, match_enclosing_type_param, mod_is_within, parse_ufcs_trait, resolve_enclosing_impl_head,
    resolve_qual_pns, unwrap_binding_to_head,
};
use super::query_state::{CachedParsed, LiveEngine};
use super::v5_db::RidxV5ItemDb;

pub(crate) fn query_path_candidates(ws: &WorkspaceIndex, engine: &mut LiveEngine, r_idx: usize, fid: FileId, canon: &Path, env_salt: u64, byte_offset: u32) -> Vec<Cand> {
    let Some(parsed_ref) = engine.parsed.as_ref() else { return Vec::new() };
    let Some((qual_str, colon2_start)) = extract_colon2_qualifier(&parsed_ref.src, byte_offset) else { return Vec::new() };
    let qual = qual_str.clone();
    let fn_key_opt = engine.ensure_fn_inferred(ws, r_idx, fid, env_salt, colon2_start);

    let Some(parsed) = engine.parsed.as_ref() else { return Vec::new() };
    let st = parsed.tree();
    let Some((_, mt_opt, dm)) = engine.active_def_map(r_idx) else { return Vec::new() };
    let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
    let mod_tree: &ModuleTree = mt_opt.unwrap_or(&dm.mod_tree);
    let cur_mod = mod_tree.module_at_offset(fid, colon2_start).unwrap_or(ModuleId::ROOT);
    let cur_krate = dm.krate;

    let mut scope_traits = Vec::new();
    let local_defs_opt = fn_key_opt.and_then(|k| engine.fn_cache.get(&k)).map(|e| {
        let at_node = deepest_node_at(&st, e.fn_node, colon2_start);
        e.file_res.collect_traits_in_scope_at(at_node, &mut scope_traits);
        &e.local_defs
    });
    if scope_traits.is_empty() {
        for rb in dm.traits_in_scope(mod_tree, cur_mod) {
            if rb.def.is_some() {
                scope_traits.push(rb.def);
            }
        }
    }

    let mut out = Vec::new();
    let mut seen: HashSet<(String, DefKind)> = HashSet::new();

    if let Some(ufcs_trait) = parse_ufcs_trait(&qual) {
        if let Some(pns) = resolve_qual_pns(&db, dm, mod_tree, cur_mod, ufcs_trait) {
            if let Some(tb) = pns.types.filter(|b| b.kind == IK::Trait && b.def.is_some()) {
                emit_trait_assocs(&db, dm, canon, parsed, local_defs_opt, tb.def, &mut seen, &mut out);
                return out;
            }
        }
    }

    if qual == "Self" {
        if let Some(owner) = find_enclosing_impl_or_trait(&st, colon2_start) {
            if st.kind(owner) == K::Trait {
                if let Some(nn) = st.child(owner, K::Name) {
                    let tname = st.text(nn).trim_start_matches("r#");
                    if let Some(tb) = dm.resolve_name_in_module(mod_tree, cur_mod, tname).types.filter(|b| b.def.is_some()) {
                        emit_trait_assocs(&db, dm, canon, parsed, local_defs_opt, tb.def, &mut seen, &mut out);
                        return out;
                    }
                }
            } else if let Some(head) = resolve_enclosing_impl_head(&db, dm, mod_tree, cur_mod, &st, owner) {
                emit_type_head_cands(&db, dm, cur_krate, cur_mod, canon, parsed, local_defs_opt, head, &scope_traits, &mut seen, &mut out);
                return out;
            }
        }
    }

    if !qual.contains("::") {
        if let Some(fn_key) = fn_key_opt {
            if let Some(entry) = engine.fn_cache.get(&fn_key) {
                let fn_gid = entry.local_defs.fn_def_for_node(entry.fn_node);
                if let Some(pidx) = match_enclosing_type_param(&st, entry.fn_node, &qual) {
                    let mut ptraits = Vec::new();
                    if fn_gid.is_some() {
                        db.param_trait_bounds(fn_gid, pidx, &mut ptraits);
                    }
                    for tdef in ptraits {
                        emit_trait_assocs(&db, dm, canon, parsed, Some(&entry.local_defs), tdef, &mut seen, &mut out);
                    }
                    if !out.is_empty() {
                        return out;
                    }
                }
            }
        }
    }

    if let Some(pns) = resolve_qual_pns(&db, dm, mod_tree, cur_mod, &qual) {
        if let Some(tb) = pns.types {
            if tb.prim_ty != 0 && !tb.def.is_some() && !tb.local.is_some() {
                if let Some(p) = PrimTy::from_code(tb.prim_ty) {
                    emit_type_head_cands(&db, dm, cur_krate, cur_mod, canon, parsed, local_defs_opt, ImplTargetKey::Prim(p), &scope_traits, &mut seen, &mut out);
                    return out;
                }
            }
            match tb.kind {
                IK::Mod | IK::ExternCrate => {
                    emit_mod_cands(&db, dm, mod_tree, cur_krate, cur_mod, canon, parsed, local_defs_opt, tb, &mut seen, &mut out);
                    return out;
                }
                IK::Trait if tb.def.is_some() => {
                    emit_trait_assocs(&db, dm, canon, parsed, local_defs_opt, tb.def, &mut seen, &mut out);
                    return out;
                }
                IK::Struct | IK::Enum | IK::Union | IK::TypeAlias => {
                    let head = unwrap_binding_to_head(&db, dm, tb);
                    emit_type_head_cands(&db, dm, cur_krate, cur_mod, canon, parsed, local_defs_opt, head, &scope_traits, &mut seen, &mut out);
                    if !qual.contains("::") {
                        if let Some(p) = PrimTy::from_name(&qual) {
                            emit_type_head_cands(&db, dm, cur_krate, cur_mod, canon, parsed, local_defs_opt, ImplTargetKey::Prim(p), &scope_traits, &mut seen, &mut out);
                        }
                    }
                    return out;
                }
                _ => {}
            }
        }
    }
    if !qual.contains("::") {
        if let Some(p) = PrimTy::from_name(&qual) {
            emit_type_head_cands(&db, dm, cur_krate, cur_mod, canon, parsed, local_defs_opt, ImplTargetKey::Prim(p), &scope_traits, &mut seen, &mut out);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_mod_cands(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    mod_tree: &ModuleTree,
    cur_krate: CrateId,
    cur_mod: ModuleId,
    canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    tb: ResolvedBinding,
    seen: &mut HashSet<(String, DefKind)>,
    out: &mut Vec<Cand>,
) {
    if let Some(local_mid) = find_local_module_id(dm, tb) {
        let mut entries: Vec<(&str, &PerNs)> = dm.scope(local_mid).entries.iter().map(|(k, v)| (k.as_str(), v)).collect();
        entries.sort_unstable_by_key(|&(k, _)| k);
        for (name, pns) in entries {
            push_per_ns_cands(db, dm, mod_tree, cur_krate, cur_mod, canon, parsed, local_defs, name, *pns, true, seen, out);
        }
        return;
    }
    if tb.def.is_some() {
        let mut ext_c = ExtScopeCache::new();
        let scope = ext_c.module_scope(db, tb.def);
        let mut entries: Vec<(&str, PerNs)> = scope.entries.iter().map(|(k, &v)| (k.as_str(), v)).collect();
        entries.sort_unstable_by_key(|&(k, _)| k);
        for (name, pns) in entries {
            push_per_ns_cands(db, dm, mod_tree, cur_krate, cur_mod, canon, parsed, local_defs, name, pns, false, seen, out);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_per_ns_cands(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    mod_tree: &ModuleTree,
    cur_krate: CrateId,
    cur_mod: ModuleId,
    canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    name: &str,
    pns: PerNs,
    check_local_vis: bool,
    seen: &mut HashSet<(String, DefKind)>,
    out: &mut Vec<Cand>,
) {
    if name.is_empty() || name == "_" {
        return;
    }
    for b in [pns.types, pns.values, pns.macros].into_iter().flatten() {
        if check_local_vis && b.vis != VisKind::Public && b.vis != VisKind::Crate && !mod_is_within(mod_tree, cur_mod, b.vis_mod) {
            continue;
        }
        if !check_local_vis && b.def.is_some() && !is_def_visible_from(db, cur_krate, cur_mod, dm, b.def) && b.vis != VisKind::Public {
            continue;
        }
        let dk = map_item_kind_to_def_kind(b.kind);
        if dk == DefKind::Other || !seen.insert((name.to_string(), dk)) {
            continue;
        }
        let eff_gid = if b.def.is_some() {
            b.def
        } else if b.local.is_some() {
            GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x8000_0000 | b.local.0)
        } else {
            GlobalDefId::NONE
        };
        let detail = eff_gid.is_some().then(|| def_signature_detail(db, parsed, local_defs, eff_gid)).flatten();
        let loc = eff_gid.is_some().then(|| resolve_def_location(db, dm, canon, parsed, local_defs, eff_gid)).flatten();
        out.push(Cand { label: name.to_string(), kind: dk, detail, def_location: loc, from_trait: None });
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_type_head_cands(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    cur_krate: CrateId,
    cur_mod: ModuleId,
    canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    head: ImplTargetKey,
    scope_traits: &[GlobalDefId],
    seen: &mut HashSet<(String, DefKind)>,
    out: &mut Vec<Cand>,
) {
    let head = match head {
        ImplTargetKey::Adt(d) if d.krate != CrateId::LOCAL_OVERLAY => ImplTargetKey::Adt(db.canonical_def(d)),
        other => other,
    };
    if let ImplTargetKey::Adt(edef) = head {
        let local_enum_id = dm
            .items
            .iter()
            .find(|it| it.file.is_some() && it.kind == IK::Enum && (it.index_def == edef || (edef.krate == CrateId::LOCAL_OVERLAY && (0x8000_0000 | it.id.0) == edef.index)))
            .map(|it| it.id);
        if let Some(lid) = local_enum_id {
            for it in dm.items.iter().filter(|it| it.kind == IK::Variant && it.parent_enum == lid) {
                let vname = dm.item_name(it.id).to_string();
                if seen.insert((vname.clone(), DefKind::Variant)) {
                    let vgid = if it.index_def.is_some() { it.index_def } else { GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x8000_0000 | it.id.0) };
                    let loc = resolve_def_location(db, dm, canon, parsed, local_defs, vgid);
                    out.push(Cand { label: vname, kind: DefKind::Variant, detail: None, def_location: loc, from_trait: None });
                }
            }
        } else {
            let mut vars = Vec::new();
            db.enum_variants(edef, &mut vars);
            for v in vars {
                if seen.insert((v.name.to_string(), DefKind::Variant)) {
                    let loc = resolve_def_location(db, dm, canon, parsed, local_defs, v.id);
                    out.push(Cand { label: v.name.to_string(), kind: DefKind::Variant, detail: None, def_location: loc, from_trait: None });
                }
            }
        }
    }

    for imp in &dm.local_impls.impls {
        if !imp.has_trait && !imp.is_negative && imp.target == head {
            for a in dm.local_impls.impl_assocs(imp) {
                let dk = map_item_kind_to_def_kind(a.kind);
                let nm = dm.local_impls.assoc_name(a).to_string();
                if seen.insert((nm.clone(), dk)) {
                    let loc = a.def.is_some().then(|| resolve_def_location(db, dm, canon, parsed, local_defs, a.def)).flatten();
                    out.push(Cand { label: nm, kind: dk, detail: None, def_location: loc, from_trait: None });
                }
            }
        }
    }

    let mut impls = Vec::new();
    db.impls_for_target(head, &mut impls);
    if impls.is_empty() && dm.local_impls.impls.is_empty() {
        let st = parsed.tree();
        let mod_tree = &dm.mod_tree;
        for nid in (0..st.nodes.len() as u32).map(ridx_syntax::NodeId) {
            if st.kind(nid) == K::Impl
                && !st.own_tokens(nid).any(|(_, t)| t.kind == ridx_syntax::TokenKind::For)
                && resolve_enclosing_impl_head(db, dm, mod_tree, cur_mod, &st, nid) == Some(head)
            {
                if let Some(il) = st.child(nid, K::ItemList) {
                    for ch in st.children(il) {
                        let dk = match st.kind(ch) {
                            K::Fn => DefKind::AssocFn,
                            K::Const => DefKind::AssocConst,
                            K::TypeAlias => DefKind::AssocTy,
                            _ => continue,
                        };
                        if let Some(nn) = st.child(ch, K::Name) {
                            let nm = st.text(nn).trim_start_matches("r#").to_string();
                            if seen.insert((nm.clone(), dk)) {
                                out.push(Cand { label: nm, kind: dk, detail: None, def_location: None, from_trait: None });
                            }
                        }
                    }
                }
            }
        }
    }
    for imp in &impls {
        if imp.is_negative || imp.trait_def.is_some() {
            continue;
        }
        let mut assocs = Vec::new();
        db.assoc_items(imp.id, &mut assocs);
        for a in assocs {
            if !is_def_visible_from(db, cur_krate, cur_mod, dm, a.id) {
                continue;
            }
            let dk = map_item_kind_to_def_kind(a.kind);
            if seen.insert((a.name.to_string(), dk)) {
                let detail = def_signature_detail(db, parsed, local_defs, a.id);
                let loc = resolve_def_location(db, dm, canon, parsed, local_defs, a.id);
                out.push(Cand { label: a.name.to_string(), kind: dk, detail, def_location: loc, from_trait: None });
            }
        }
    }
    for &raw_tr in scope_traits {
        let tr = db.canonical_def(raw_tr);
        let has_impl = db.ty_tables.concrete_trait_impls.contains_key(&(tr, head)) || db.ty_tables.blanket_trait_impls.contains_key(&tr);
        if has_impl {
            emit_trait_assocs_with_origin(db, dm, canon, parsed, local_defs, tr, true, seen, out);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_trait_assocs(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    trait_def: GlobalDefId,
    seen: &mut HashSet<(String, DefKind)>,
    out: &mut Vec<Cand>,
) {
    emit_trait_assocs_with_origin(db, dm, canon, parsed, local_defs, trait_def, false, seen, out);
}

#[allow(clippy::too_many_arguments)]
fn emit_trait_assocs_with_origin(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    raw_trait: GlobalDefId,
    include_from_trait: bool,
    seen: &mut HashSet<(String, DefKind)>,
    out: &mut Vec<Cand>,
) {
    let mut stack = vec![db.canonical_def(raw_trait)];
    let mut visited = HashSet::new();
    while let Some(tgid) = stack.pop() {
        if !visited.insert(tgid) {
            continue;
        }
        let from_tr = include_from_trait.then(|| {
            let mut s = String::new();
            let _ = write_def_name(db, dm, parsed, local_defs, tgid, false, &mut s);
            s
        });
        let mut assocs = Vec::new();
        db.assoc_items(tgid, &mut assocs);
        for a in assocs {
            let dk = map_item_kind_to_def_kind(a.kind);
            if seen.insert((a.name.to_string(), dk)) {
                let detail = def_signature_detail(db, parsed, local_defs, a.id);
                let loc = resolve_def_location(db, dm, canon, parsed, local_defs, a.id);
                out.push(Cand { label: a.name.to_string(), kind: dk, detail, def_location: loc, from_trait: from_tr.clone() });
            }
        }
        let mut supers = Vec::new();
        ItemDb::trait_supertraits(db, tgid, &mut supers);
        stack.extend(supers);
    }
}
