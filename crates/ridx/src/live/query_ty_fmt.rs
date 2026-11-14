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
use ridx_syntax::name_res::{CrateDefMap, FileId};
use ridx_syntax::ty_infer::{FnInferResult, FrozenTyArena, InferTyArena, InferTyId, InferTyKind, LocalCstDefs, TySliceRange};
use ridx_syntax::{NodeId, SyntaxKind as K};

use super::query::LiveTy;
use super::query_cfg::is_inside_inactive_target_cfg;
use super::query_cursor::{
    enclosing_macro_tt_span, find_enclosing_trait_def, is_cursor_on_typeable_token, lower_cst_type_with_owners, pick_ty_in_fn, smallest_type_node_at, ty_contains_placeholder,
};
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
    let macro_tt = {
        let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
        let st = engine.parsed.as_ref()?.tree();
        if !is_cursor_on_typeable_token(&st, byte_offset) || is_inside_inactive_target_cfg(&st, byte_offset, &db, r_idx, canon) {
            return None;
        }
        enclosing_macro_tt_span(&st, byte_offset)
    };
    let direct_ty_node = macro_tt
        .is_none()
        .then(|| {
            let st = engine.parsed.as_ref()?.tree();
            let (_, tn) = smallest_type_node_at(&st, byte_offset, None)?;
            let slice = &st.nodes[tn.0 as usize..st.node(tn).subtree_end as usize];
            (!st.text(tn).contains('_') && !slice.iter().any(|n| n.kind.is_expr() || n.kind == K::MacroCall)).then_some(tn)
        })
        .flatten();
    if let Some(ty_node) = direct_ty_node {
        if let Some(res) = try_lower_standalone_ty_node(ws, engine, r_idx, fid, canon, byte_offset, ty_node) {
            return Some(res);
        }
    }
    if let Some(fn_key) = engine.ensure_fn_inferred(ws, r_idx, fid, env_salt, byte_offset) {
        let (expr_hit, ty_hit) = {
            let parsed = engine.parsed.as_ref()?;
            let st = parsed.tree();
            let entry = engine.fn_cache.get(&fn_key)?;
            (pick_ty_in_fn(&st, &entry.file_res, &entry.result, byte_offset, macro_tt), smallest_type_node_at(&st, byte_offset, macro_tt))
        };
        let prefer_ty_node = ty_hit.is_some_and(|(ts, _)| expr_hit.is_none_or(|(es, _)| ts < es));
        if prefer_ty_node {
            if let Some((_, ty_node)) = ty_hit {
                if let Some(res) = lower_ty_hit_in_fn(ws, engine, r_idx, fid, canon, byte_offset, fn_key, ty_node, true) {
                    return Some(res);
                }
            }
        }
        if let Some((_, ty)) = expr_hit {
            let parsed = engine.parsed.as_ref()?;
            let entry = engine.fn_cache.get(&fn_key)?;
            if ty.is_some() && !matches!(entry.arena.kind(ty), InferTyKind::Unknown) {
                let (_, _, dm) = engine.active_def_map(r_idx)?;
                let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
                return Some(build_live_ty(&db, dm, canon, parsed, Some(&entry.local_defs), Some(&entry.result), byte_offset, Some(entry.fn_node), &entry.arena, ty));
            }
        }
        if !prefer_ty_node {
            if let Some((_, ty_node)) = ty_hit {
                if let Some(res) = lower_ty_hit_in_fn(ws, engine, r_idx, fid, canon, byte_offset, fn_key, ty_node, false) {
                    return Some(res);
                }
            }
        }
        return None;
    }
    let (_, ty_node) = {
        let st = engine.parsed.as_ref()?.tree();
        smallest_type_node_at(&st, byte_offset, macro_tt)?
    };
    try_lower_standalone_ty_node(ws, engine, r_idx, fid, canon, byte_offset, ty_node)
}

#[allow(clippy::too_many_arguments)]
fn lower_ty_hit_in_fn(
    ws: &WorkspaceIndex,
    engine: &mut LiveEngine,
    r_idx: usize,
    fid: FileId,
    canon: &Path,
    byte_offset: u32,
    fn_key: (FileId, u64, u64),
    ty_node: NodeId,
    with_let_fallback: bool,
) -> Option<LiveTy> {
    let encl_tr = {
        let (_, _, dm) = engine.active_def_map(r_idx)?;
        let st = engine.parsed.as_ref()?.tree();
        find_enclosing_trait_def(&st, dm, fid, ty_node)
    };
    let (ty, let_pat_ty) = {
        let st = engine.parsed.as_ref()?.tree();
        let entry = engine.fn_cache.get(&fn_key)?;
        let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
        let lowered = lower_cst_type_with_owners(&st, &entry.file_res, encl_tr, &db, &entry.local_defs, &mut engine.infer_ctx.table, ty_node, byte_offset);
        let lpt = (with_let_fallback && ty_contains_placeholder(&engine.infer_ctx.table.arena, lowered, 0))
            .then(|| st.ancestors(ty_node).find(|&a| st.kind(a) == K::LetStmt))
            .flatten()
            .and_then(|ls| st.children(ls).find(|&c| st.kind(c).is_pat()))
            .and_then(|pat| entry.result.pat_tys.iter().find(|&&(n, _)| n == pat).map(|&(_, t)| t));
        (lowered, lpt)
    };
    let (_, _, dm) = engine.active_def_map(r_idx)?;
    let parsed = engine.parsed.as_ref()?;
    let entry = engine.fn_cache.get(&fn_key)?;
    let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
    if let Some(pty) = let_pat_ty.filter(|&t| t.is_some() && !matches!(entry.arena.kind(t), InferTyKind::Unknown)) {
        return Some(build_live_ty(&db, dm, canon, parsed, Some(&entry.local_defs), Some(&entry.result), byte_offset, Some(entry.fn_node), &entry.arena, pty));
    }
    if ty.is_some() && !matches!(engine.infer_ctx.table.arena.kind(ty), InferTyKind::Unknown) {
        return Some(build_live_ty(&db, dm, canon, parsed, Some(&entry.local_defs), None, byte_offset, Some(entry.fn_node), &engine.infer_ctx.table.arena, ty));
    }
    None
}

fn try_lower_standalone_ty_node(ws: &WorkspaceIndex, engine: &mut LiveEngine, r_idx: usize, fid: FileId, canon: &Path, byte_offset: u32, ty_node: NodeId) -> Option<LiveTy> {
    let mut file_res = std::mem::take(&mut engine.shared_file_res);
    let mut local_defs = std::mem::take(&mut engine.shared_macro_defs);
    let out = try_lower_standalone_ty_inner(ws, engine, r_idx, fid, canon, byte_offset, ty_node, &mut file_res, &mut local_defs);
    engine.shared_file_res = file_res;
    engine.shared_macro_defs = local_defs;
    out
}

#[allow(clippy::too_many_arguments)]
fn try_lower_standalone_ty_inner(
    ws: &WorkspaceIndex,
    engine: &mut LiveEngine,
    r_idx: usize,
    fid: FileId,
    canon: &Path,
    byte_offset: u32,
    ty_node: NodeId,
    file_res: &mut ridx_syntax::name_res::FileNameRes,
    local_defs: &mut LocalCstDefs,
) -> Option<LiveTy> {
    let (encl_tr, item_fn) = {
        let ext_c = std::mem::take(&mut engine.shared_ext_cache);
        let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
        let Some((snap, mt_opt, dm)) = engine.active_def_map(r_idx) else {
            engine.shared_ext_cache = ext_c;
            return None;
        };
        snap.put_ext_cache(ext_c);
        let Some(parsed) = engine.parsed.as_ref() else {
            engine.shared_ext_cache = snap.take_ext_cache();
            return None;
        };
        let st = parsed.tree();
        let ifn = st.ancestors(ty_node).find(|&a| st.kind(a) == K::Fn);
        let has_v5_sigs = ws.readers().get(r_idx).is_some_and(|r| !r.fn_sigs().is_empty());
        if has_v5_sigs && mt_opt.is_none() && !dm.is_dirty_file(fid) {
            let target = ifn.unwrap_or(ty_node);
            snap.resolve_fn_paths_into(&db, fid, &st, target, file_res);
            local_defs.index_target_fn_with_item_db(&st, Some(dm), Some(&db), fid, target);
        } else {
            snap.resolve_dirty_file_paths_into(&db, mt_opt, dm, fid, &st, file_res);
            local_defs.index_file_with_item_db(&st, Some(dm), Some(&db), fid);
        }
        let tr = find_enclosing_trait_def(&st, dm, fid, ty_node);
        drop(st);
        engine.shared_ext_cache = snap.take_ext_cache();
        (tr, ifn)
    };
    let ty = {
        let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
        let st = engine.parsed.as_ref()?.tree();
        lower_cst_type_with_owners(&st, file_res, encl_tr, &db, local_defs, &mut engine.infer_ctx.table, ty_node, byte_offset)
    };
    if !ty.is_some() || matches!(engine.infer_ctx.table.arena.kind(ty), InferTyKind::Unknown) {
        return None;
    }
    let (_, _, dm) = engine.active_def_map(r_idx)?;
    let parsed = engine.parsed.as_ref()?;
    let db = RidxV5ItemDb::view(ws, &engine.src, &engine.state);
    Some(build_live_ty(&db, dm, canon, parsed, Some(local_defs), None, byte_offset, item_fn, &engine.infer_ctx.table.arena, ty))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_live_ty<A: TyReader>(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    active_canon: &Path,
    parsed: &CachedParsed,
    local_defs: Option<&LocalCstDefs>,
    fn_infer: Option<&FnInferResult>,
    cursor_off: u32,
    fn_node: Option<NodeId>,
    arena: &A,
    ty: InferTyId,
) -> LiveTy {
    let mut display = String::with_capacity(32);
    let mut full_display = String::with_capacity(48);
    let mut fmt = TyFmt { db, dm, parsed, local_defs, fn_node, arena, qualified: false };
    let _ = fmt.write_ty(ty, 0, &mut display);
    fmt.qualified = true;
    let _ = fmt.write_ty(ty, 0, &mut full_display);
    let mut defs = Vec::new();
    collect_ty_def_locs(db, dm, active_canon, parsed, local_defs, fn_infer, cursor_off, arena, ty, 0, &mut defs);
    LiveTy { display, full_display, defs }
}

struct TyFmt<'a, A: TyReader> {
    db: &'a RidxV5ItemDb<'a>,
    dm: &'a CrateDefMap,
    parsed: &'a CachedParsed,
    local_defs: Option<&'a LocalCstDefs>,
    fn_node: Option<NodeId>,
    arena: &'a A,
    qualified: bool,
}

impl<A: TyReader> TyFmt<'_, A> {
    fn write_ty(&self, ty: InferTyId, depth: u8, out: &mut String) -> std::fmt::Result {
        if !ty.is_some() || depth > 28 {
            return out.write_str("{unknown}");
        }
        match self.arena.kind(ty) {
            InferTyKind::Unknown | InferTyKind::Infer(_) => out.write_str("{unknown}"),
            InferTyKind::Never => out.write_str("!"),
            InferTyKind::Prim(p) => out.write_str(p.name()),
            InferTyKind::Ref { is_mut, inner } => {
                out.write_str(if is_mut { "&mut " } else { "&" })?;
                self.write_ty(inner, depth + 1, out)
            }
            InferTyKind::Ptr { is_mut, inner } => {
                out.write_str(if is_mut { "*mut " } else { "*const " })?;
                self.write_ty(inner, depth + 1, out)
            }
            InferTyKind::Slice(e) => {
                out.write_char('[')?;
                self.write_ty(e, depth + 1, out)?;
                out.write_char(']')
            }
            InferTyKind::Array { elem, len } => {
                out.write_char('[')?;
                self.write_ty(elem, depth + 1, out)?;
                if len == u64::MAX {
                    out.write_str("; _]")
                } else {
                    write!(out, "; {len}]")
                }
            }
            InferTyKind::Tuple(r) => {
                out.write_char('(')?;
                for (i, &el) in self.arena.slice(r).iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.write_ty(el, depth + 1, out)?;
                }
                if r.len == 1 {
                    out.write_char(',')?;
                }
                out.write_char(')')
            }
            InferTyKind::Adt { def, args } => {
                write_def_name(self.db, self.dm, self.parsed, self.local_defs, def, self.qualified, out)?;
                self.write_trimmed_args(args, depth + 1, out)
            }
            InferTyKind::Dyn { principal, args } => {
                out.write_str("dyn ")?;
                write_def_name(self.db, self.dm, self.parsed, self.local_defs, principal, self.qualified, out)?;
                self.write_trimmed_args(args, depth + 1, out)
            }
            InferTyKind::Opaque { def, args } => {
                out.write_str("impl ")?;
                write_def_name(self.db, self.dm, self.parsed, self.local_defs, def, self.qualified, out)?;
                self.write_trimmed_args(args, depth + 1, out)
            }
            InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
                out.write_char('<')?;
                self.write_ty(self_ty, depth + 1, out)?;
                if trait_def.is_some() {
                    out.write_str(" as ")?;
                    write_def_name(self.db, self.dm, self.parsed, self.local_defs, trait_def, self.qualified, out)?;
                }
                out.write_str(">::")?;
                if assoc_def.is_some() {
                    write_def_name(self.db, self.dm, self.parsed, self.local_defs, assoc_def, false, out)?;
                } else {
                    out.write_str("Assoc")?;
                }
                self.write_trimmed_args(args, depth + 1, out)
            }
            InferTyKind::FnPtr { sig } | InferTyKind::Closure { sig, .. } => {
                let is_clos = matches!(self.arena.kind(ty), InferTyKind::Closure { .. });
                out.write_str(if is_clos { "impl Fn(" } else { "fn(" })?;
                let s = self.arena.slice(sig);
                let n_in = s.len().saturating_sub(1);
                for (i, &inp) in s[..n_in].iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    self.write_ty(inp, depth + 1, out)?;
                }
                out.write_char(')')?;
                let ret = *s.last().unwrap_or(&InferTyId::UNIT);
                if ret != InferTyId::UNIT {
                    out.write_str(" -> ")?;
                    self.write_ty(ret, depth + 1, out)?;
                }
                Ok(())
            }
            InferTyKind::FnDef { def, .. } => {
                out.write_str("fn ")?;
                write_def_name(self.db, self.dm, self.parsed, self.local_defs, def, self.qualified, out)
            }
            InferTyKind::Param { idx, def } => {
                if def.is_some() {
                    return write_def_name(self.db, self.dm, self.parsed, self.local_defs, def, false, out);
                }
                if let Some(name) = lookup_cst_param_name(self.parsed, self.fn_node, idx) {
                    return out.write_str(name);
                }
                write!(out, "T{idx}")
            }
        }
    }

    fn write_trimmed_args(&self, args: TySliceRange, depth: u8, out: &mut String) -> std::fmt::Result {
        let mut slice = self.arena.slice(args);
        while let Some(&last) = slice.last() {
            if let InferTyKind::Adt { def, args: a2 } = self.arena.kind(last) {
                if a2.is_empty() && self.db.ty_tables.global_gid.is_some_and(|g| self.db.canonical_def(def) == g) {
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
            self.write_ty(arg, depth, out)?;
        }
        out.write_char('>')
    }
}
