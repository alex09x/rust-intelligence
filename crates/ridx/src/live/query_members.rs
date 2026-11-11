/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The fields and methods a receiver type offers after `.`, in `method_res` autoderef order: candidate names from
//! inherent impls and applicable traits, probed against the receiver, then filtered by visibility and
//! `#[doc(hidden)]`.

use std::collections::HashSet;
use std::path::Path;

use ridx_format::{DefKind, WorkspaceIndex};
use ridx_syntax::name_res::{CrateId, FileId, GlobalDefId, ImplTargetKey, ModuleId};
use ridx_syntax::ty_infer::autoderef::{build_autoderef_steps_with_owner, impl_target_key_of_ty, NormOwner, MAX_AUTODEREF_STEPS};
use ridx_syntax::ty_infer::{CompletionTraitHints, InferTyId, InferTyKind};

use super::query::Cand;
use super::query_dot::{canon_head, deepest_node_at, emit_step_fields};
use super::query_loc::{def_signature_detail, is_def_visible_from, resolve_def_location, write_def_name};
use super::query_state::LiveEngine;
use super::v5_db::RidxV5ItemDb;

/// The members of `recv_ty` at `probe_off` in the function `fn_key` was inferred for.
#[allow(clippy::too_many_arguments)]
pub(crate) fn members_of(
    ws: &WorkspaceIndex,
    engine: &mut LiveEngine,
    r_idx: usize,
    fid: FileId,
    canon: &Path,
    fn_key: (FileId, u64, u64),
    probe_off: u32,
    recv_ty: InferTyId,
) -> Vec<Cand> {
    let Some((_, _, dm_ptr_check)) = engine.active_def_map(r_idx) else { return Vec::new() };
    let cur_krate = dm_ptr_check.krate;
    let cur_mod = dm_ptr_check.mod_tree.module_at_offset(fid, probe_off).unwrap_or(ModuleId::ROOT);

    let mut steps_out = Vec::with_capacity(MAX_AUTODEREF_STEPS);
    let mut hits = Vec::with_capacity(64);
    let mut cand_names: Vec<&str> = Vec::with_capacity(128);
    {
        let parsed = engine.parsed.as_ref().unwrap();
        let st = parsed.tree();
        let entry = engine.fn_cache.get(&fn_key).unwrap();
        let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
        let at_node = deepest_node_at(&st, entry.fn_node, probe_off);
        let fn_gid = entry.local_defs.fn_def_for_node(entry.fn_node);

        let snap = engine.infer_ctx.table.start_snapshot();
        let owner = NormOwner { fn_def: fn_gid, impl_node: None, impl_gc: 0, fn_node: Some(entry.fn_node), self_ty: None };
        let mut preview = [InferTyId::UNKNOWN; MAX_AUTODEREF_STEPS];
        let plen = build_autoderef_steps_with_owner(&mut engine.infer_ctx.table, &db, &entry.local_defs, &st, &entry.file_res, owner, recv_ty, &mut preview);
        engine.infer_ctx.table.rollback_to(snap);

        entry.local_defs.collect_cst_method_names(&st, &mut cand_names);
        let mut traits: Vec<GlobalDefId> = Vec::with_capacity(64);
        entry.file_res.collect_traits_in_scope_at(at_node, &mut traits);
        // Only the receiver's own bounds bring supertrait methods along; a supertrait of a trait in lexical scope is not in scope.
        let scope_len = traits.len();
        let mut step_heads = [ImplTargetKey::Unknown; MAX_AUTODEREF_STEPS + 3];
        let mut sh_len = 0usize;
        let mut push_head = |h: ImplTargetKey| {
            if h != ImplTargetKey::Unknown && !step_heads[..sh_len].contains(&h) && sh_len < step_heads.len() {
                step_heads[sh_len] = h;
                sh_len += 1;
            }
        };
        push_head(ImplTargetKey::Ref { is_mut: false });
        push_head(ImplTargetKey::Ref { is_mut: true });
        let mut extra_traits: Vec<GlobalDefId> = Vec::new();
        for &st_ty in &preview[..plen] {
            let head = canon_head(&db, impl_target_key_of_ty(&engine.infer_ctx.table.arena, st_ty));
            push_head(head);
            if matches!(engine.infer_ctx.table.arena.kind(st_ty), InferTyKind::Array { .. }) {
                push_head(ImplTargetKey::Slice);
            }
            match engine.infer_ctx.table.arena.kind(st_ty) {
                InferTyKind::Dyn { principal, .. } | InferTyKind::Opaque { def: principal, .. } if principal.is_some() => {
                    if !traits[scope_len..].contains(&principal) {
                        traits.push(principal);
                    }
                    extra_traits.push(db.canonical_def(principal));
                }
                InferTyKind::Param { .. } => {
                    let mut bounds = Vec::new();
                    engine.infer_ctx.param_bounds_for_completion(&st, &entry.file_res, &db, &entry.local_defs, entry.fn_node, st_ty, &mut bounds);
                    for bt in bounds {
                        extra_traits.push(db.canonical_def(bt));
                        if !traits.contains(&bt) {
                            traits.push(bt);
                        }
                    }
                }
                _ => {}
            }
        }
        for &h in &step_heads[..sh_len] {
            if let Some(ns) = engine.inherent_names_by_head.get(&h) {
                cand_names.extend(ns.iter().map(|p| p.as_str(ws)));
            }
        }
        let mut cursor = scope_len;
        while cursor < traits.len() && traits.len() < 128 {
            let tdef = traits[cursor];
            cursor += 1;
            entry.local_defs.collect_supertraits_into(tdef, &st, &entry.file_res, &db, &mut traits);
        }
        let has_local_impls = !entry.local_defs.is_clean_indexed();
        let mut known: Vec<GlobalDefId> = Vec::with_capacity(traits.len());
        let mut declared: Vec<(&str, GlobalDefId)> = Vec::with_capacity(512);
        for &tdef in &traits {
            if let Some(ns) = engine.trait_method_names.get(&db.canonical_def(tdef)) {
                known.push(tdef);
                declared.extend(ns.iter().map(|p| (p.as_str(ws), tdef)));
            }
        }
        for tdef in traits {
            let ct = db.canonical_def(tdef);
            let applicable = has_local_impls
                || extra_traits.contains(&ct)
                || db.ty_tables.blanket_trait_impls.contains_key(&ct)
                || step_heads[..sh_len].iter().any(|&h| db.ty_tables.concrete_trait_impls.contains_key(&(ct, h)));
            if applicable {
                if let Some(ns) = engine.trait_method_names.get(&ct) {
                    cand_names.extend(ns.iter().map(|p| p.as_str(ws)));
                }
            }
        }
        cand_names.sort_unstable();
        cand_names.dedup();
        known.sort_unstable_by_key(|d| (d.krate.0, d.index));
        known.dedup();
        declared.sort_unstable_by_key(|&(n, d)| (n, d.krate.0, d.index));
        declared.dedup();
        let declaring: Vec<GlobalDefId> = declared.iter().map(|&(_, d)| d).collect();
        let mut ranges: Vec<(u32, u32)> = Vec::with_capacity(cand_names.len());
        let mut at = 0usize;
        for &nm in &cand_names {
            while at < declared.len() && declared[at].0 < nm {
                at += 1;
            }
            let lo = at;
            while at < declared.len() && declared[at].0 == nm {
                at += 1;
            }
            ranges.push((lo as u32, at as u32));
        }
        let hints = CompletionTraitHints { known: &known, ranges: &ranges, declaring: &declaring };

        engine.infer_ctx.collect_ordered_completion_methods(
            &st,
            &entry.file_res,
            &db,
            &entry.local_defs,
            entry.fn_node,
            at_node,
            recv_ty,
            &cand_names,
            &hints,
            &mut steps_out,
            &mut hits,
        );
    }

    let parsed = engine.parsed.as_ref().unwrap();
    let entry = engine.fn_cache.get(&fn_key).unwrap();
    let (_, _, dm) = engine.active_def_map(r_idx).unwrap();
    let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
    let mut out = Vec::with_capacity(hits.len() + 8);
    let mut seen_fields = HashSet::new();
    let mut seen_methods: HashSet<(String, Option<String>)> = HashSet::new();

    for (step_idx, &st_ty) in steps_out.iter().enumerate() {
        emit_step_fields(&db, dm, cur_krate, cur_mod, canon, parsed, &entry.local_defs, &engine.infer_ctx.table.arena, st_ty, &mut seen_fields, &mut out);
        for h in hits.iter().filter(|h| usize::from(h.step_idx) == step_idx) {
            if h.tier == 0 && !is_def_visible_from(&db, cur_krate, cur_mod, dm, h.method_def) {
                continue;
            }
            let label = cand_names[h.name_idx as usize].to_string();
            let from_trait = h.trait_def.is_some().then(|| {
                let mut tn = String::new();
                let _ = write_def_name(&db, dm, parsed, Some(&entry.local_defs), h.trait_def, false, &mut tn);
                tn
            });
            if h.tier == 0 {
                if !seen_methods.insert((label.clone(), None)) {
                    continue;
                }
            } else if seen_methods.contains(&(label.clone(), None)) || !seen_methods.insert((label.clone(), from_trait.clone())) {
                continue;
            }
            let def_location = resolve_def_location(&db, dm, canon, parsed, Some(&entry.local_defs), h.method_def);
            let def_crate = db.metas.get(db.canonical_def(h.method_def).krate.0 as usize).map(|m| m.crate_name.as_str());
            if h.method_def.krate != CrateId::LOCAL_OVERLAY
                && def_crate.is_some_and(|c| c != db.metas[cur_krate.0 as usize].crate_name)
                && def_location.as_ref().is_some_and(|l| engine.hidden.is_doc_hidden(&l.file, l.line))
            {
                continue;
            }
            let detail = def_signature_detail(&db, parsed, Some(&entry.local_defs), h.method_def);
            out.push(Cand { label, kind: DefKind::AssocFn, detail, def_location, from_trait });
        }
    }
    out
}
