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
use ridx_syntax::ty_infer::lang_ops::unwrap_await_ty;
use ridx_syntax::ty_infer::{InferTyArena, InferTyId, InferTyKind, LocalCstDefs};
use ridx_syntax::{parse, NodeId, SyntaxTree};

use super::query::Cand;
use super::query_loc::{is_def_visible_from, resolve_def_location};
use super::query_postfix::PostfixFacts;
use super::query_state::{CachedParsed, LiveEngine};
use super::v5_db::RidxV5ItemDb;
use super::v5_ty_inst::clone_reader_ty;

pub(crate) fn query_dot_candidates(
    ws: &WorkspaceIndex,
    engine: &mut LiveEngine,
    r_idx: usize,
    fid: FileId,
    canon: &Path,
    env_salt: u64,
    byte_offset: u32,
) -> (Vec<Cand>, Option<PostfixFacts>) {
    engine.ensure_method_index(ws);
    let Some(parsed_ref) = engine.parsed.as_ref() else { return (Vec::new(), None) };
    let Some(dot_end) = locate_dot_end(&parsed_ref.src, byte_offset) else { return (Vec::new(), None) };
    let probe_off = dot_end.saturating_sub(1);
    let Some(fn_key) = engine.ensure_fn_inferred(ws, r_idx, fid, env_salt, probe_off) else { return (Vec::new(), None) };

    engine.infer_ctx.table.clear();
    let (fn_node, recv_ty) = {
        let Some(parsed) = engine.parsed.as_ref() else { return (Vec::new(), None) };
        let st = parsed.tree();
        let Some(entry) = engine.fn_cache.get(&fn_key) else { return (Vec::new(), None) };
        let raw_ty = entry.result.receiver_ty_at_dot(&st, dot_end).or_else(|| macro_recv_at_dot(&parsed.src, &entry.result, dot_end));
        let cloned =
            raw_ty.filter(|t| t.is_some() && !matches!(entry.arena.kind(*t), InferTyKind::Unknown)).map(|t| clone_reader_ty(&entry.arena, t, &mut engine.infer_ctx.table.arena));
        (entry.fn_node, cloned)
    };

    let recv_ty = match recv_ty {
        Some(t) => t,
        None => match recover_masked_dot_receiver(ws, engine, fn_key, fn_node, dot_end) {
            Some(t) => t,
            None => return (Vec::new(), None),
        },
    };
    if !recv_ty.is_some() || matches!(engine.infer_ctx.table.arena.kind(recv_ty), InferTyKind::Unknown) {
        return (Vec::new(), None);
    }

    let mut out = super::query_members::members_of(ws, engine, r_idx, fid, canon, fn_key, probe_off, recv_ty);
    let (facts, await_out) = {
        let parsed = engine.parsed.as_ref().unwrap();
        let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
        let facts = super::query_postfix::postfix_facts(&parsed.tree(), dot_end, &db, &engine.infer_ctx.table.arena, recv_ty, &out);
        (facts, unwrap_await_ty(&mut engine.infer_ctx.table, &db, recv_ty))
    };
    // A future also completes `await` and, as `await.name`, the members of its output (rust-analyzer's auto-await).
    let mut facts = facts;
    let is_future = {
        let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
        implements_future(&db, &engine.infer_ctx.table.arena, recv_ty)
    };
    if is_future && await_out.is_some() && !matches!(engine.infer_ctx.table.arena.kind(await_out), InferTyKind::Unknown | InferTyKind::Alias { .. } | InferTyKind::Infer(_)) {
        if let Some(f) = facts.as_mut() {
            f.is_future = true;
        }
        let awaited = super::query_members::members_of(ws, engine, r_idx, fid, canon, fn_key, probe_off, await_out);
        out.extend(awaited.into_iter().map(|c| Cand { label: format!("await.{}", c.label), ..c }));
    }
    (out, facts)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_step_fields(
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
/// Whether `ty` (through references, `Pin` and `Box`) is `impl Future` / `dyn Future` or has a concrete impl of
/// `Future` or `IntoFuture`; blanket impls do not count, the index-side solver does not check their bounds.
fn implements_future(db: &RidxV5ItemDb<'_>, arena: &InferTyArena, ty: InferTyId) -> bool {
    use ridx_syntax::ty_infer::{TyDefDb, TyLangItem};
    let traits: Vec<GlobalDefId> = [TyLangItem::Future, TyLangItem::IntoFuture].into_iter().filter_map(|l| db.lang_item(l)).map(|d| db.canonical_def(d)).collect();
    let mut cur = ty;
    for _ in 0..6 {
        match arena.kind(cur) {
            InferTyKind::Opaque { def, .. } | InferTyKind::Dyn { principal: def, .. } if traits.contains(&db.canonical_def(def)) => return true,
            InferTyKind::Ref { inner, .. } => cur = inner,
            InferTyKind::Adt { def, args } if matches!(db.lang_item_of_def(def), Some(TyLangItem::Pin | TyLangItem::OwnedBox)) && args.len >= 1 => cur = arena.slice(args)[0],
            _ => {
                let head = canon_head(db, ridx_syntax::ty_infer::autoderef::impl_target_key_of_ty(arena, cur));
                return head != ImplTargetKey::Unknown && traits.iter().any(|&t| !db.ty_tables.concrete_impls(&(t, head)).is_empty());
            }
        }
    }
    false
}

pub(super) fn canon_head(db: &RidxV5ItemDb<'_>, key: ImplTargetKey) -> ImplTargetKey {
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
