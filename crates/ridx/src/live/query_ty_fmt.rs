/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `type_at` cursor resolution and rustc/RA `LiveTy` display formatting for [`super::query::LiveDb`].

use std::fmt::Write;
use std::path::Path;

use ridx_format::WorkspaceIndex;
use ridx_syntax::name_res::{CrateDefMap, FileId, FileNameRes, LocalBindId, ResTarget};
use ridx_syntax::ty_infer::{FrozenTyArena, InferTyArena, InferTyId, InferTyKind, LocalCstDefs, LowerTyEnv, TySliceRange};
use ridx_syntax::{NodeId, SyntaxTree};

use super::query::LiveTy;
use super::query_loc::{collect_ty_def_locs, lookup_cst_param_name, write_def_name};
use super::query_state::{CachedParsed, LiveEngine};
use super::v5_db::RidxV5ItemDb;

pub(crate) trait TyReader {
    fn kind(&self, id: InferTyId) -> InferTyKind;
    fn slice(&self, range: TySliceRange) -> &[InferTyId];
}

impl TyReader for FrozenTyArena {
    #[inline]
    fn kind(&self, id: InferTyId) -> InferTyKind {
        self.kind(id)
    }
    #[inline]
    fn slice(&self, range: TySliceRange) -> &[InferTyId] {
        self.slice(range)
    }
}

impl TyReader for InferTyArena {
    #[inline]
    fn kind(&self, id: InferTyId) -> InferTyKind {
        self.kind(id)
    }
    #[inline]
    fn slice(&self, range: TySliceRange) -> &[InferTyId] {
        self.slice(range)
    }
}

pub(crate) fn query_type_at(ws: &WorkspaceIndex, engine: &mut LiveEngine, r_idx: usize, fid: FileId, canon: &Path, env_salt: u64, byte_offset: u32) -> Option<LiveTy> {
    if let Some(fn_key) = engine.ensure_fn_inferred(ws, r_idx, fid, env_salt, byte_offset) {
        let ty_opt = {
            let parsed = engine.parsed.as_ref()?;
            let st = parsed.tree();
            let entry = engine.fn_cache.get(&fn_key)?;
            pick_ty_in_fn(&st, &entry.file_res, &entry.result, byte_offset)
        };
        if let Some(ty) = ty_opt {
            let parsed = engine.parsed.as_ref()?;
            let entry = engine.fn_cache.get(&fn_key)?;
            if ty.is_some() && !matches!(entry.arena.kind(ty), InferTyKind::Unknown) {
                let (_, _, dm) = engine.active_def_map(r_idx)?;
                let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
                return Some(build_live_ty(&db, dm, canon, parsed, Some(&entry.local_defs), Some(entry.fn_node), &entry.arena, ty));
            }
        }
        let ty_node = {
            let st = engine.parsed.as_ref()?.tree();
            smallest_type_node_at(&st, byte_offset)?
        };
        engine.infer_ctx.table.clear();
        let ty = {
            let st = engine.parsed.as_ref()?.tree();
            let entry = engine.fn_cache.get(&fn_key)?;
            let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
            let env = LowerTyEnv {
                st: &st,
                file_res: &entry.file_res,
                db: &db,
                local_defs: &entry.local_defs,
                self_ty: None,
                impl_owner: None,
                impl_generics: 0,
                item_owner: Some(entry.fn_node),
                in_body: false,
                sub_map: None,
            };
            env.lower_cst_ty(&mut engine.infer_ctx.table, ty_node)
        };
        if !ty.is_some() || matches!(engine.infer_ctx.table.arena.kind(ty), InferTyKind::Unknown) {
            return None;
        }
        let (_, _, dm) = engine.active_def_map(r_idx)?;
        let parsed = engine.parsed.as_ref()?;
        let entry = engine.fn_cache.get(&fn_key)?;
        let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
        return Some(build_live_ty(&db, dm, canon, parsed, Some(&entry.local_defs), Some(entry.fn_node), &engine.infer_ctx.table.arena, ty));
    }
    let ty_node = {
        let st = engine.parsed.as_ref()?.tree();
        smallest_type_node_at(&st, byte_offset)?
    };
    let mut file_res = FileNameRes::new();
    let mut local_defs = LocalCstDefs::new();
    let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
    {
        let (snap, mt_opt, dm) = engine.active_def_map(r_idx)?;
        let st = engine.parsed.as_ref()?.tree();
        snap.resolve_dirty_file_paths_into(&db, mt_opt, dm, fid, &st, &mut file_res);
        local_defs.index_file_with_item_db(&st, Some(dm), Some(&db), fid);
    }
    engine.infer_ctx.table.clear();
    let st = engine.parsed.as_ref()?.tree();
    let env = LowerTyEnv {
        st: &st,
        file_res: &file_res,
        db: &db,
        local_defs: &local_defs,
        self_ty: None,
        impl_owner: None,
        impl_generics: 0,
        item_owner: None,
        in_body: false,
        sub_map: None,
    };
    let ty = env.lower_cst_ty(&mut engine.infer_ctx.table, ty_node);
    if !ty.is_some() || matches!(engine.infer_ctx.table.arena.kind(ty), InferTyKind::Unknown) {
        return None;
    }
    let (_, _, dm) = engine.active_def_map(r_idx)?;
    let parsed = engine.parsed.as_ref()?;
    Some(build_live_ty(&db, dm, canon, parsed, Some(&local_defs), None, &engine.infer_ctx.table.arena, ty))
}

fn pick_ty_in_fn(st: &SyntaxTree<'_>, file_res: &FileNameRes, res: &ridx_syntax::ty_infer::FnInferResult, off: u32) -> Option<InferTyId> {
    for exact in [true, false] {
        for r in &file_res.refs {
            let hit = if exact { r.start <= off && off < r.end } else { off == r.end && r.start < r.end };
            if hit {
                if let ResTarget::Local(lid) = r.target {
                    if let Some(t) = res.local_ty(lid) {
                        return Some(t);
                    }
                }
            }
        }
        for (idx, ld) in file_res.locals.iter().enumerate() {
            let hit = if exact { ld.name_start <= off && off < ld.name_end } else { off == ld.name_end && ld.name_start < ld.name_end };
            if hit {
                if let Some(t) = res.local_ty(LocalBindId(idx as u32)) {
                    return Some(t);
                }
            }
        }
    }
    let mut best: Option<(u32, InferTyId)> = None;
    for &(nid, ty) in res.pat_tys.iter().chain(res.expr_tys.iter()) {
        if !ty.is_some() {
            continue;
        }
        let r = st.range(nid);
        let (s, e) = (r.start as u32, r.end as u32);
        if s <= off && (off < e || (off == e && s < e)) {
            let span = e - s;
            if best.is_none_or(|(bs, _)| span <= bs) {
                best = Some((span, ty));
            }
        }
    }
    for m in &res.macro_expr_tys {
        if m.ty.is_some() && m.start <= off && (off < m.end || (off == m.end && m.start < m.end)) {
            let span = m.end - m.start;
            if best.is_none_or(|(bs, _)| span < bs) {
                best = Some((span, m.ty));
            }
        }
    }
    best.map(|(_, t)| t)
}

fn smallest_type_node_at(st: &SyntaxTree<'_>, off: u32) -> Option<NodeId> {
    let b = off as usize;
    let mut best: Option<(usize, NodeId)> = None;
    for nid in (0..st.nodes.len() as u32).map(NodeId) {
        if st.kind(nid).is_type() {
            let r = st.range(nid);
            if r.start <= b && b <= r.end && r.start < r.end {
                let len = r.end - r.start;
                if best.is_none_or(|(bl, _)| len <= bl) {
                    best = Some((len, nid));
                }
            }
        }
    }
    best.map(|(_, n)| n)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_live_ty<A: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    fn_node: Option<NodeId>,
    arena: &A,
    ty: InferTyId,
) -> LiveTy {
    let mut display = String::with_capacity(32);
    let mut full_display = String::with_capacity(48);
    let _ = write_live_ty(db, dm, parsed, local_defs, fn_node, arena, ty, false, 0, &mut display);
    let _ = write_live_ty(db, dm, parsed, local_defs, fn_node, arena, ty, true, 0, &mut full_display);
    let mut defs = Vec::new();
    collect_ty_def_locs(db, dm, active_canon, parsed, local_defs, arena, ty, 0, &mut defs);
    LiveTy { display, full_display, defs }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_live_ty<A: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    fn_node: Option<NodeId>,
    arena: &A,
    ty: InferTyId,
    qualified: bool,
    depth: u8,
    out: &mut String,
) -> std::fmt::Result {
    if !ty.is_some() || depth > 28 {
        return out.write_str("{unknown}");
    }
    match arena.kind(ty) {
        InferTyKind::Unknown | InferTyKind::Infer(_) => out.write_str("{unknown}"),
        InferTyKind::Never => out.write_str("!"),
        InferTyKind::Prim(p) => out.write_str(p.name()),
        InferTyKind::Ref { is_mut, inner } => {
            out.write_str(if is_mut { "&mut " } else { "&" })?;
            write_live_ty(db, dm, parsed, local_defs, fn_node, arena, inner, qualified, depth + 1, out)
        }
        InferTyKind::Ptr { is_mut, inner } => {
            out.write_str(if is_mut { "*mut " } else { "*const " })?;
            write_live_ty(db, dm, parsed, local_defs, fn_node, arena, inner, qualified, depth + 1, out)
        }
        InferTyKind::Slice(e) => {
            out.write_char('[')?;
            write_live_ty(db, dm, parsed, local_defs, fn_node, arena, e, qualified, depth + 1, out)?;
            out.write_char(']')
        }
        InferTyKind::Array { elem, len } => {
            out.write_char('[')?;
            write_live_ty(db, dm, parsed, local_defs, fn_node, arena, elem, qualified, depth + 1, out)?;
            if len == u64::MAX {
                out.write_str("; _]")
            } else {
                write!(out, "; {len}]")
            }
        }
        InferTyKind::Tuple(r) => {
            out.write_char('(')?;
            for (i, &el) in arena.slice(r).iter().enumerate() {
                if i > 0 {
                    out.write_str(", ")?;
                }
                write_live_ty(db, dm, parsed, local_defs, fn_node, arena, el, qualified, depth + 1, out)?;
            }
            if r.len == 1 {
                out.write_char(',')?;
            }
            out.write_char(')')
        }
        InferTyKind::Adt { def, args } => {
            write_def_name(db, dm, parsed, local_defs, def, qualified, out)?;
            write_trimmed_args(db, dm, parsed, local_defs, fn_node, arena, args, qualified, depth + 1, out)
        }
        InferTyKind::Dyn { principal, args } => {
            out.write_str("dyn ")?;
            write_def_name(db, dm, parsed, local_defs, principal, qualified, out)?;
            write_trimmed_args(db, dm, parsed, local_defs, fn_node, arena, args, qualified, depth + 1, out)
        }
        InferTyKind::Opaque { def, args } => {
            out.write_str("impl ")?;
            write_def_name(db, dm, parsed, local_defs, def, qualified, out)?;
            write_trimmed_args(db, dm, parsed, local_defs, fn_node, arena, args, qualified, depth + 1, out)
        }
        InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
            out.write_char('<')?;
            write_live_ty(db, dm, parsed, local_defs, fn_node, arena, self_ty, qualified, depth + 1, out)?;
            if trait_def.is_some() {
                out.write_str(" as ")?;
                write_def_name(db, dm, parsed, local_defs, trait_def, qualified, out)?;
            }
            out.write_str(">::")?;
            write_def_name(db, dm, parsed, local_defs, assoc_def, false, out)?;
            write_trimmed_args(db, dm, parsed, local_defs, fn_node, arena, args, qualified, depth + 1, out)
        }
        InferTyKind::FnPtr { sig } | InferTyKind::Closure { sig, .. } => {
            let is_clos = matches!(arena.kind(ty), InferTyKind::Closure { .. });
            out.write_str(if is_clos { "impl Fn(" } else { "fn(" })?;
            let s = arena.slice(sig);
            let n_in = s.len().saturating_sub(1);
            for (i, &inp) in s[..n_in].iter().enumerate() {
                if i > 0 {
                    out.write_str(", ")?;
                }
                write_live_ty(db, dm, parsed, local_defs, fn_node, arena, inp, qualified, depth + 1, out)?;
            }
            out.write_char(')')?;
            let ret = *s.last().unwrap_or(&InferTyId::UNIT);
            if ret != InferTyId::UNIT {
                out.write_str(" -> ")?;
                write_live_ty(db, dm, parsed, local_defs, fn_node, arena, ret, qualified, depth + 1, out)?;
            }
            Ok(())
        }
        InferTyKind::FnDef { def, .. } => {
            out.write_str("fn ")?;
            write_def_name(db, dm, parsed, local_defs, def, qualified, out)
        }
        InferTyKind::Param { idx, def } => {
            if def.is_some() {
                return write_def_name(db, dm, parsed, local_defs, def, false, out);
            }
            if let Some(name) = lookup_cst_param_name(parsed, fn_node, idx) {
                return out.write_str(name);
            }
            write!(out, "T{idx}")
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_trimmed_args<A: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    fn_node: Option<NodeId>,
    arena: &A,
    args: TySliceRange,
    qualified: bool,
    depth: u8,
    out: &mut String,
) -> std::fmt::Result {
    let mut slice = arena.slice(args);
    while let Some(&last) = slice.last() {
        if let InferTyKind::Adt { def, args: a2 } = arena.kind(last) {
            if a2.is_empty() && db.ty_tables.global_gid.is_some_and(|g| db.canonical_def(def) == g) {
                slice = &slice[..slice.len() - 1];
                continue;
            }
        }
        break;
    }
    if slice.is_empty() {
        return Ok(());
    }
    out.write_char('<')?;
    for (i, &arg) in slice.iter().enumerate() {
        if i > 0 {
            out.write_str(", ")?;
        }
        write_live_ty(db, dm, parsed, local_defs, fn_node, arena, arg, qualified, depth, out)?;
    }
    out.write_char('>')
}
