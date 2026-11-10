/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Cursor token/macro/cfg eligibility, expression-vs-type node selection, and owner-aware
//! unnormalized CST projection lowering for [`super::query_ty_fmt::query_type_at`].

use ridx_syntax::name_res::{CrateDefMap, CrateId, FileId, FileNameRes, GlobalDefId, ItemKind as IK, LocalBindId, ResTarget};
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{FnInferResult, InferTyId, InferTyKind, InferenceTable, LocalCstDefs, LowerTyEnv};
use ridx_syntax::{token_at, NodeId, SyntaxKind as K, SyntaxTree, TokenKind as T};

use super::query_ty_fmt::TyReader;
use super::v5_db::RidxV5ItemDb;

#[inline]
fn is_typeable_token_kind(k: T) -> bool {
    matches!(k, T::Ident | T::RawIdent | T::SelfKw | T::SelfType | T::Await)
}

/// Returns `true` only when `off` is on (or immediately after) an identifier, `self`, `Self`, or `await`.
pub(crate) fn is_cursor_on_typeable_token(st: &SyntaxTree<'_>, off: u32) -> bool {
    if let Some(idx) = token_at(&st.tokens, off) {
        let tok = st.tokens[idx];
        if is_typeable_token_kind(tok.kind) {
            return true;
        }
        return tok.start == off && idx > 0 && st.tokens[idx - 1].end == off && is_typeable_token_kind(st.tokens[idx - 1].kind);
    }
    st.tokens.last().is_some_and(|t| t.end == off && is_typeable_token_kind(t.kind))
}

/// If `off` lies strictly inside the argument `TokenTree` of a `MacroCall`, returns that `TokenTree`'s byte span.
pub(crate) fn enclosing_macro_tt_span(st: &SyntaxTree<'_>, off: u32) -> Option<(u32, u32)> {
    let b = off as usize;
    let mut best: Option<(usize, (u32, u32))> = None;
    for nid in (0..st.nodes.len() as u32).map(NodeId) {
        if st.kind(nid) == K::MacroCall {
            if let Some(tt) = st.child(nid, K::TokenTree) {
                let r = st.range(tt);
                if r.start < b && b < r.end {
                    let len = r.end - r.start;
                    if best.is_none_or(|(bl, _)| len <= bl) {
                        best = Some((len, (r.start as u32, r.end as u32)));
                    }
                }
            }
        }
    }
    best.map(|(_, sp)| sp)
}

pub(crate) fn is_inside_inactive_target_cfg(st: &SyntaxTree<'_>, off: u32) -> bool {
    let b = off as usize;
    for nid in (0..st.nodes.len() as u32).map(NodeId) {
        let r = st.range(nid);
        if r.start <= b && b <= r.end {
            for ch in st.children(nid).filter(|&c| st.kind(c) == K::Attr) {
                let raw = st.text(ch).trim();
                let Some(inner) = raw.strip_prefix("#[cfg(").or_else(|| raw.strip_prefix("#![cfg(")).and_then(|s| s.strip_suffix(")]")).map(str::trim) else {
                    continue;
                };
                if inner == "miri" || (inner == "windows" && !cfg!(windows)) || (inner == "unix" && !cfg!(unix)) {
                    return true;
                }
                let clauses = inner.strip_prefix("all(").and_then(|s| s.strip_suffix(')')).unwrap_or(if inner.contains('(') { "" } else { inner });
                for cl in clauses.split(',').map(str::trim) {
                    if let Some(os) =
                        cl.strip_prefix("target_os").and_then(|s| s.trim_start().strip_prefix('=')).and_then(|s| s.trim().strip_prefix('"')).and_then(|s| s.strip_suffix('"'))
                    {
                        if os != std::env::consts::OS {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

pub(crate) fn pick_ty_in_fn(st: &SyntaxTree<'_>, file_res: &FileNameRes, res: &FnInferResult, off: u32, macro_tt: Option<(u32, u32)>) -> Option<(u32, InferTyId)> {
    for exact in [true, false] {
        for r in &file_res.refs {
            let hit = if exact { r.start <= off && off < r.end } else { off == r.end && r.start < r.end };
            if hit {
                if let ResTarget::Local(lid) = r.target {
                    if let Some(t) = res.local_ty(lid) {
                        return Some((r.end - r.start, t));
                    }
                }
            }
        }
        for (idx, ld) in file_res.locals.iter().enumerate() {
            let hit = if exact { ld.name_start <= off && off < ld.name_end } else { off == ld.name_end && ld.name_start < ld.name_end };
            if hit {
                if let Some(t) = res.local_ty(LocalBindId(idx as u32)) {
                    return Some((ld.name_end - ld.name_start, t));
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
        if macro_tt.is_some_and(|(ts, te)| s <= ts && te <= e) {
            continue;
        }
        if s <= off && (off < e || (off == e && s < e)) {
            let span = e - s;
            if best.is_none_or(|(bs, _)| span <= bs) {
                best = Some((span, ty));
            }
        }
    }
    for m in &res.macro_expr_tys {
        if !m.ty.is_some() || macro_tt.is_some_and(|(ts, te)| m.start <= ts && te <= m.end) {
            continue;
        }
        if m.start <= off && (off < m.end || (off == m.end && m.start < m.end)) {
            let span = m.end - m.start;
            if best.is_none_or(|(bs, _)| span < bs) {
                best = Some((span, m.ty));
            }
        }
    }
    best
}

pub(crate) fn smallest_type_node_at(st: &SyntaxTree<'_>, off: u32, macro_tt: Option<(u32, u32)>) -> Option<(u32, NodeId)> {
    let b = off as usize;
    let mut best: Option<(u32, NodeId)> = None;
    for nid in (0..st.nodes.len() as u32).map(NodeId) {
        if st.kind(nid).is_type() {
            let r = st.range(nid);
            let (s, e) = (r.start as u32, r.end as u32);
            if macro_tt.is_some_and(|(ts, te)| s <= ts && te <= e) {
                continue;
            }
            if r.start <= b && b <= r.end && r.start < r.end {
                let len = e - s;
                if best.is_none_or(|(bl, _)| len <= bl) {
                    best = Some((len, nid));
                }
            }
        }
    }
    best
}

pub(crate) fn ty_contains_placeholder<A: TyReader>(arena: &A, ty: InferTyId, depth: u8) -> bool {
    if !ty.is_some() || depth > 16 {
        return true;
    }
    match arena.kind(ty) {
        InferTyKind::Unknown | InferTyKind::Infer(_) => true,
        InferTyKind::Adt { args, .. }
        | InferTyKind::Dyn { args, .. }
        | InferTyKind::Opaque { args, .. }
        | InferTyKind::Tuple(args)
        | InferTyKind::FnPtr { sig: args }
        | InferTyKind::Closure { sig: args, .. }
        | InferTyKind::FnDef { args, .. } => arena.slice(args).iter().any(|&a| ty_contains_placeholder(arena, a, depth + 1)),
        InferTyKind::Alias { self_ty, args, .. } => {
            ty_contains_placeholder(arena, self_ty, depth + 1) || arena.slice(args).iter().any(|&a| ty_contains_placeholder(arena, a, depth + 1))
        }
        InferTyKind::Ref { inner, .. } | InferTyKind::Ptr { inner, .. } | InferTyKind::Slice(inner) | InferTyKind::Array { elem: inner, .. } => {
            ty_contains_placeholder(arena, inner, depth + 1)
        }
        _ => false,
    }
}

pub(crate) fn find_enclosing_trait_def(st: &SyntaxTree<'_>, dm: &CrateDefMap, fid: FileId, ty_node: NodeId) -> Option<(NodeId, GlobalDefId)> {
    let tr = st.ancestors(ty_node).find(|&a| st.kind(a) == K::Trait)?;
    let tr_range = st.range(tr);
    let tr_def = dm
        .items
        .iter()
        .find(|it| it.file == fid && it.kind == IK::Trait && (it.span_start as usize) >= tr_range.start && (it.span_end as usize) <= tr_range.end)
        .map_or(GlobalDefId::NONE, |it| if it.index_def.is_some() { it.index_def } else { GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x8000_0000 | it.id.0) });
    Some((tr, tr_def))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_cst_type_with_owners(
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    enclosing_trait: Option<(NodeId, GlobalDefId)>,
    db: &RidxV5ItemDb<'_>,
    local_defs: &LocalCstDefs,
    table: &mut InferenceTable,
    ty_node: NodeId,
    off: u32,
) -> InferTyId {
    table.clear();
    let item_owner = st.ancestors(ty_node).find(|&a| matches!(st.kind(a), K::Fn | K::Struct | K::Enum | K::Union | K::TypeAlias | K::Const | K::Static));
    let impl_node = st.ancestors(ty_node).find(|&a| st.kind(a) == K::Impl);
    let (impl_owner, impl_generics, self_ty) = if let Some(imp) = impl_node {
        let header_self = {
            let has_for = st.own_tokens(imp).any(|(_, t)| t.kind == T::For);
            let mut tys = st.children(imp).filter(|&c| st.kind(c).is_type());
            if has_for {
                tys.nth(1)
            } else {
                tys.next()
            }
        };
        let in_self_header = header_self.is_some_and(|sn| sn == ty_node || st.ancestors(ty_node).any(|a| a == sn));
        if in_self_header {
            let igc = st.child(imp, K::GenericParamList).map_or(0, |gpl| st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)).count() as u16);
            (Some(imp), igc, None)
        } else {
            let (igc, sty) = local_defs.lower_impl_self_ty(imp, st, file_res, db, table);
            (Some(imp), igc, sty.is_some().then_some(sty))
        }
    } else if let Some((tr, tr_def)) = enclosing_trait {
        let gc = st.child(tr, K::GenericParamList).map_or(0, |gpl| st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)).count() as u16);
        let sty = table.arena.mk_param(0, tr_def);
        (Some(tr), 1 + gc, Some(sty))
    } else {
        (None, 0, None)
    };
    let env = LowerTyEnv { st, file_res, db, local_defs, self_ty, impl_owner, impl_generics, item_owner, in_body: false, sub_map: None };
    if let Some(alias_ty) = try_lower_unnormalized_cst_assoc(&env, db, table, ty_node, impl_node, off) {
        return alias_ty;
    }
    env.lower_cst_ty(table, ty_node)
}

fn res_def_gid(r: ResTarget) -> GlobalDefId {
    match r {
        ResTarget::Def { def, local, .. } => {
            if def.is_some() {
                def
            } else if local.is_some() {
                GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x8000_0000 | local.0)
            } else {
                GlobalDefId::NONE
            }
        }
        _ => GlobalDefId::NONE,
    }
}

fn push_seg_type_args(env: &LowerTyEnv<'_, '_>, table: &mut InferenceTable, seg: NodeId, out: &mut InlineTyBuf) {
    if let Some(gal) = env.st.child(seg, K::GenericArgList) {
        for ga in env.st.children(gal).filter(|&c| env.st.kind(c) == K::TypeArg) {
            if let Some(tn) = env.st.children(ga).find(|&c| env.st.kind(c).is_type()) {
                out.push(env.lower_cst_ty(table, tn));
            }
        }
    }
}

fn try_lower_unnormalized_cst_assoc(
    env: &LowerTyEnv<'_, '_>,
    db: &RidxV5ItemDb<'_>,
    table: &mut InferenceTable,
    ty_node: NodeId,
    impl_node: Option<NodeId>,
    off: u32,
) -> Option<InferTyId> {
    if env.st.kind(ty_node) != K::PathType {
        return None;
    }
    let p = env.st.child(ty_node, K::Path)?;
    let mut segs = env.st.children(p).filter(|&c| env.st.kind(c) == K::PathSegment);
    let s0 = segs.next()?;
    let s1 = segs.next()?;
    if segs.next().is_some() {
        return None;
    }
    let assoc_nm = env.st.own_tokens(s1).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent)).map_or("", |(_, t)| t.text(env.st.src).trim_start_matches("r#"));
    if env.st.own_tokens(s0).next().is_some_and(|(_, t)| t.kind == T::Lt) {
        let tp = env.st.child(s0, K::Path)?;
        let qtr = env.file_res.path_tail_ref_in_st(env.st, tp).map_or(GlobalDefId::NONE, |r| res_def_gid(r.target));
        if !qtr.is_some() {
            return None;
        }
        let mut args = InlineTyBuf::new();
        if let Some(tp_tail) = env.st.children(tp).filter(|&c| env.st.kind(c) == K::PathSegment).last() {
            push_seg_type_args(env, table, tp_tail, &mut args);
        }
        let tpr = env.st.range(tp);
        if tpr.start <= off as usize && (off as usize) <= tpr.end {
            return Some(table.arena.mk_dyn(qtr, args.as_slice()));
        }
        let qsty = env.st.children(s0).find(|&c| env.st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |tn| env.lower_cst_ty(table, tn));
        push_seg_type_args(env, table, s1, &mut args);
        let adef = db.lookup_trait_assoc_ty_def(db.canonical_def(qtr), assoc_nm);
        return Some(table.arena.mk_alias(qtr, adef, qsty, args.as_slice()));
    }
    if !env.st.own_tokens(s0).any(|(_, t)| t.kind == T::SelfType) {
        return None;
    }
    let imp = impl_node?;
    if !env.st.own_tokens(imp).any(|(_, t)| t.kind == T::For) {
        return None;
    }
    let sty = env.self_ty?;
    let tr_ty_node = env.st.children(imp).find(|&c| env.st.kind(c).is_type())?;
    let tr_path = env.st.child(tr_ty_node, K::Path)?;
    let qtr = env.file_res.path_tail_ref_in_st(env.st, tr_path).map_or(GlobalDefId::NONE, |r| res_def_gid(r.target));
    if !qtr.is_some() {
        return None;
    }
    let mut args = InlineTyBuf::new();
    if let Some(tp_tail) = env.st.children(tr_path).filter(|&c| env.st.kind(c) == K::PathSegment).last() {
        push_seg_type_args(env, table, tp_tail, &mut args);
    }
    push_seg_type_args(env, table, s1, &mut args);
    let adef = db.lookup_trait_assoc_ty_def(db.canonical_def(qtr), assoc_nm);
    Some(table.arena.mk_alias(qtr, adef, sty, args.as_slice()))
}
