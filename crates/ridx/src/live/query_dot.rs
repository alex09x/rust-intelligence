/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `dot_candidates` implementation for [`super::query::LiveDb`]: resolves receiver type before `.`
//! (including partial identifiers `x.pre|`, macro-argument expressions, and broken-statement fallback),
//! then emits visible fields, inherent methods, concrete/bound trait methods, and blanket trait methods
//! in exact `method_res` autoderef order.

use std::collections::HashSet;
use std::path::Path;

use ridx_format::{DefKind, ItemVis, WorkspaceIndex};
use ridx_syntax::name_res::{CrateDefMap, CrateId, FileId, GlobalDefId, ImplTargetKey, ModuleId};
use ridx_syntax::ty_infer::autoderef::{build_autoderef_steps_with_owner, impl_target_key_of_ty, NormOwner, MAX_AUTODEREF_STEPS};
use ridx_syntax::ty_infer::{InferTyArena, InferTyId, InferTyKind, LocalCstDefs, TyDefDb};
use ridx_syntax::{parse, NodeId, SyntaxTree};

use super::query::Cand;
use super::query_loc::{def_signature_detail, is_def_visible_from, resolve_def_location, write_def_name};
use super::query_state::{CachedParsed, LiveEngine};
use super::v5_db::RidxV5ItemDb;
use super::v5_ty_inst::clone_reader_ty;

pub(crate) fn query_dot_candidates(ws: &WorkspaceIndex, engine: &mut LiveEngine, r_idx: usize, fid: FileId, canon: &Path, env_salt: u64, byte_offset: u32) -> Vec<Cand> {
    let Some(parsed_ref) = engine.parsed.as_ref() else { return Vec::new() };
    let Some(dot_end) = locate_dot_end(&parsed_ref.src, byte_offset) else { return Vec::new() };
    let probe_off = dot_end.saturating_sub(1);
    let Some(fn_key) = engine.ensure_fn_inferred(ws, r_idx, fid, env_salt, probe_off) else { return Vec::new() };

    engine.infer_ctx.table.clear();
    let (fn_node, recv_ty) = {
        let Some(parsed) = engine.parsed.as_ref() else { return Vec::new() };
        let st = parsed.tree();
        let Some(entry) = engine.fn_cache.get(&fn_key) else { return Vec::new() };
        let raw_ty = entry.result.receiver_ty_at_dot(&st, dot_end).or_else(|| macro_recv_at_dot(&parsed.src, &entry.result, dot_end));
        let cloned =
            raw_ty.filter(|t| t.is_some() && !matches!(entry.arena.kind(*t), InferTyKind::Unknown)).map(|t| clone_reader_ty(&entry.arena, t, &mut engine.infer_ctx.table.arena));
        (entry.fn_node, cloned)
    };

    let recv_ty = match recv_ty {
        Some(t) => t,
        None => match recover_masked_dot_receiver(ws, engine, fn_key, fn_node, dot_end) {
            Some(t) => t,
            None => return Vec::new(),
        },
    };
    if !recv_ty.is_some() || matches!(engine.infer_ctx.table.arena.kind(recv_ty), InferTyKind::Unknown) {
        return Vec::new();
    }

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
                    if !traits.contains(&principal) {
                        traits.push(principal);
                    }
                    extra_traits.push(db.canonical_def(principal));
                }
                InferTyKind::Param { idx, .. } if fn_gid.is_some() => {
                    let before = traits.len();
                    db.param_trait_bounds(fn_gid, idx, &mut traits);
                    for &bt in &traits[before..] {
                        extra_traits.push(db.canonical_def(bt));
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
        let mut cursor = 0usize;
        while cursor < traits.len() && traits.len() < 128 {
            let tdef = traits[cursor];
            cursor += 1;
            entry.local_defs.collect_supertraits_into(tdef, &st, &entry.file_res, &db, &mut traits);
        }
        let has_local_impls = !entry.local_defs.is_clean_indexed();
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

        engine.infer_ctx.collect_ordered_completion_methods(&st, &entry.file_res, &db, &entry.local_defs, entry.fn_node, at_node, recv_ty, &cand_names, &mut steps_out, &mut hits);
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
            let detail = def_signature_detail(&db, parsed, Some(&entry.local_defs), h.method_def);
            let def_location = resolve_def_location(&db, dm, canon, parsed, Some(&entry.local_defs), h.method_def);
            out.push(Cand { label, kind: DefKind::AssocFn, detail, def_location, from_trait });
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_step_fields(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    cur_krate: CrateId,
    cur_mod: ModuleId,
    canon: &Path,
    parsed: &CachedParsed,
    local_defs: &LocalCstDefs,
    arena: &InferTyArena,
    st_ty: InferTyId,
    seen: &mut HashSet<String>,
    out: &mut Vec<Cand>,
) {
    match arena.kind(st_ty) {
        InferTyKind::Tuple(r) => {
            for idx in 0..r.len {
                let lbl = idx.to_string();
                if seen.insert(lbl.clone()) {
                    out.push(Cand { label: lbl, kind: DefKind::Field, detail: None, def_location: None, from_trait: None });
                }
            }
        }
        InferTyKind::Adt { def, .. } => {
            let use_cst = if def.krate == CrateId::LOCAL_OVERLAY || dm.is_dirty_file(parsed.file) {
                true
            } else {
                let cdef = db.canonical_def(def);
                db.ws.readers().get(cdef.krate.0 as usize).is_none_or(|r| r.fields_for_owner(cdef.index).next().is_none())
            };
            if use_cst && (def.krate == CrateId::LOCAL_OVERLAY || local_defs.fn_node_for_def(def) != u32::MAX) {
                let st = parsed.tree();
                let mut fspans = Vec::new();
                local_defs.cst_struct_field_spans(def, &st, &mut fspans);
                for (fname, sb, eb) in fspans {
                    if seen.insert(fname.clone()) {
                        let (sl, sc) = parsed.lines.to_line_col(&parsed.src, sb);
                        let (el, ec) = parsed.lines.to_line_col(&parsed.src, eb);
                        let loc = super::query::LiveLocation { file: canon.to_path_buf(), line: sl, col: sc, end_line: el, end_col: ec };
                        out.push(Cand { label: fname, kind: DefKind::Field, detail: None, def_location: Some(loc), from_trait: None });
                    }
                }
                return;
            }
            let cdef = db.canonical_def(def);
            let Some(rdr) = db.ws.readers().get(cdef.krate.0 as usize) else { return };
            let Some(d) = rdr.definitions().get(cdef.index as usize) else { return };
            if !matches!(DefKind::from(d.kind), DefKind::Struct | DefKind::Union) {
                return;
            }
            for fv in rdr.fields_for_owner(cdef.index) {
                let f_gid = fv.def_index.map_or(GlobalDefId::NONE, |di| GlobalDefId::new(cdef.krate, di));
                let vis_ok = if fv.vis == ItemVis::Public {
                    true
                } else if cdef.krate != cur_krate {
                    false
                } else if fv.vis == ItemVis::Crate {
                    true
                } else {
                    f_gid.is_some() && is_def_visible_from(db, cur_krate, cur_mod, dm, f_gid)
                };
                if !vis_ok {
                    continue;
                }
                let lbl = fv.name.trim_start_matches("r#").to_string();
                if !lbl.is_empty() && seen.insert(lbl.clone()) {
                    let loc = f_gid.is_some().then(|| resolve_def_location(db, dm, canon, parsed, Some(local_defs), f_gid)).flatten();
                    out.push(Cand { label: lbl, kind: DefKind::Field, detail: None, def_location: loc, from_trait: None });
                }
            }
        }
        _ => {}
    }
}

fn locate_dot_end(src: &str, byte_offset: u32) -> Option<u32> {
    let bytes = src.as_bytes();
    let mut i = (byte_offset as usize).min(bytes.len());
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        i -= 1;
    }
    if i >= 2 && &bytes[i - 2..i] == b"r#" {
        i -= 2;
    }
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != b'.' || (i >= 2 && bytes[i - 2] == b'.') {
        return None;
    }
    Some(i as u32)
}

fn macro_recv_at_dot(src: &str, res: &ridx_syntax::ty_infer::FnInferResult, dot_end: u32) -> Option<InferTyId> {
    let bytes = src.as_bytes();
    let mut recv_end = dot_end.checked_sub(1)? as usize;
    while recv_end > 0 && bytes[recv_end - 1].is_ascii_whitespace() {
        recv_end -= 1;
    }
    res.macro_expr_tys.iter().filter(|m| m.end == recv_end as u32 && m.ty.is_some()).max_by_key(|m| m.end.saturating_sub(m.start)).map(|m| m.ty)
}

fn recover_masked_dot_receiver(ws: &WorkspaceIndex, engine: &mut LiveEngine, fn_key: (FileId, u64, u64), fn_node: NodeId, dot_end: u32) -> Option<InferTyId> {
    let parsed = engine.parsed.as_ref()?;
    let st = parsed.tree();
    let fn_r = st.range(fn_node);
    let de = dot_end as usize;
    if de > parsed.src.len() || de > fn_r.end {
        return None;
    }
    let mut masked = parsed.src.as_bytes().to_vec();
    for b in &mut masked[de..fn_r.end] {
        if !matches!(*b, b'\n' | b'{' | b'}' | b';') {
            *b = b' ';
        } else if *b == b';' {
            *b = b' ';
            break;
        } else if *b == b'}' {
            break;
        }
    }
    let masked_str = String::from_utf8(masked).ok()?;
    let mst = parse(&masked_str);
    let mfn = mst.children(mst.root()).chain(mst.descendants(mst.root())).find(|&n| mst.kind(n) == ridx_syntax::SyntaxKind::Fn && mst.range(n).start == fn_r.start)?;
    let entry = engine.fn_cache.get(&fn_key)?;
    let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
    let mut tmp_res = ridx_syntax::ty_infer::FnInferResult::new();
    engine.infer_ctx.infer_fn_into(&mst, &entry.file_res, &db, &entry.local_defs, mfn, &mut tmp_res);
    tmp_res.receiver_ty_at_dot(&mst, dot_end)
}

#[inline]
fn canon_head(db: &RidxV5ItemDb<'_>, key: ImplTargetKey) -> ImplTargetKey {
    match key {
        ImplTargetKey::Adt(d) => ImplTargetKey::Adt(db.canonical_def(d)),
        ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(db.canonical_def(d)),
        other => other,
    }
}

pub(crate) fn deepest_node_at(st: &SyntaxTree<'_>, root: NodeId, byte_off: u32) -> NodeId {
    let b = byte_off as usize;
    let mut cur = root;
    loop {
        let Some(next) = st.children(cur).find(|&ch| {
            let r = st.range(ch);
            r.start <= b && b < r.end
        }) else {
            return cur;
        };
        cur = next;
    }
}
