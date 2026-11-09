/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Token-slice type lowering (`LowerTyEnv::lower_tok_slice_ty` / `lower_fn_ptr_params`) used for
//! types stored inside CST `TokenTree` nodes (`struct` fields, tuple-variant payloads, `fn(...)`).

use super::lower_ty::LowerTyEnv;
use super::mem_ty_db::instantiate_in_arena;
use super::tok_ty_scan::{find_top_tok, for_each_top_comma, for_each_top_comma_flag, is_double_colon, matching_angle, matching_delim, next_sig_idx, parse_uint_lit};
use super::ty::InferTyId;
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{GlobalDefId, ItemKind, ResTarget};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::{Token, TokenKind as T};
use crate::tree::NodeId;

impl<'a, 's> LowerTyEnv<'a, 's> {
    pub(super) fn lower_fn_ptr_params(&self, table: &mut InferenceTable, toks: &[Token], out: &mut InlineTyBuf, depth: u8) {
        for_each_top_comma(toks, |clause| {
            let Some(mut idx) = next_sig_idx(clause, 0) else {
                return;
            };
            while clause[idx].kind == T::Hash {
                if let Some(br) = next_sig_idx(clause, idx + 1).filter(|&i| clause[i].kind == T::LBracket) {
                    if let Some(end_b) = matching_delim(clause, br) {
                        if let Some(nx) = next_sig_idx(clause, end_b + 1) {
                            idx = nx;
                            continue;
                        }
                        return;
                    }
                }
                break;
            }
            let mut probe = idx;
            if clause[probe].kind == T::Mut {
                if let Some(nx) = next_sig_idx(clause, probe + 1) {
                    probe = nx;
                }
            }
            if matches!(clause[probe].kind, T::Ident | T::RawIdent | T::Underscore) {
                if let Some(ci) = next_sig_idx(clause, probe + 1) {
                    if clause[ci].kind == T::Colon && !is_double_colon(clause, ci) {
                        if let Some(after) = next_sig_idx(clause, ci + 1) {
                            idx = after;
                        }
                    }
                }
            }
            out.push(self.lower_tok_slice_ty(table, &clause[idx..], depth));
        });
    }

    /// Lowers a raw token slice representing a Rust type (used inside `TokenTree` fields and `fn(...)`).
    pub fn lower_tok_slice_ty(&self, table: &mut InferenceTable, toks: &[Token], depth: u8) -> InferTyId {
        if depth > 48 {
            return InferTyId::UNKNOWN;
        }
        let Some(i0) = next_sig_idx(toks, 0) else {
            return InferTyId::UNKNOWN;
        };
        match toks[i0].kind {
            T::Bang => InferTyId::NEVER,
            T::Underscore => self.placeholder_ty(table),
            T::Amp => {
                let mut cur = i0 + 1;
                if let Some(li) = next_sig_idx(toks, cur).filter(|&i| toks[i].kind == T::Lifetime) {
                    cur = li + 1;
                }
                let mut is_mut = false;
                if let Some(mi) = next_sig_idx(toks, cur).filter(|&i| toks[i].kind == T::Mut) {
                    is_mut = true;
                    cur = mi + 1;
                }
                let inner = self.lower_tok_slice_ty(table, &toks[cur..], depth + 1);
                table.arena.mk_ref(is_mut, inner)
            }
            T::Star => {
                let mut cur = i0 + 1;
                let mut is_mut = false;
                if let Some(mi) = next_sig_idx(toks, cur).filter(|&i| matches!(toks[i].kind, T::Mut | T::Const)) {
                    is_mut = toks[mi].kind == T::Mut;
                    cur = mi + 1;
                }
                let inner = self.lower_tok_slice_ty(table, &toks[cur..], depth + 1);
                table.arena.mk_ptr(is_mut, inner)
            }
            T::LBracket => {
                let Some(close) = matching_delim(toks, i0) else {
                    return InferTyId::UNKNOWN;
                };
                let inner = &toks[i0 + 1..close];
                if let Some(semi) = find_top_tok(inner, T::Semi) {
                    let elem = self.lower_tok_slice_ty(table, &inner[..semi], depth + 1);
                    let len = next_sig_idx(&inner[semi + 1..], 0).and_then(|k| parse_uint_lit(inner[semi + 1 + k].text(self.st.src))).unwrap_or(u64::MAX);
                    table.arena.mk_array(elem, len)
                } else {
                    let elem = self.lower_tok_slice_ty(table, inner, depth + 1);
                    table.arena.mk_slice(elem)
                }
            }
            T::LParen => {
                let Some(close) = matching_delim(toks, i0) else {
                    return InferTyId::UNKNOWN;
                };
                let inner = &toks[i0 + 1..close];
                if next_sig_idx(inner, 0).is_none() {
                    return InferTyId::UNIT;
                }
                let mut elems = InlineTyBuf::new();
                let mut saw_comma = false;
                for_each_top_comma_flag(inner, &mut saw_comma, |cl| {
                    if next_sig_idx(cl, 0).is_some() {
                        elems.push(self.lower_tok_slice_ty(table, cl, depth + 1));
                    }
                });
                if elems.as_slice().len() == 1 && !saw_comma {
                    elems.as_slice()[0]
                } else {
                    table.arena.mk_tuple(elems.as_slice())
                }
            }
            T::Dyn | T::Impl => {
                let is_opaque = toks[i0].kind == T::Impl;
                self.lower_tok_path_ty(table, &toks[i0 + 1..], !is_opaque, is_opaque, depth + 1)
            }
            T::Fn | T::Unsafe | T::Extern => {
                let Some(fn_i) = (i0..toks.len()).find(|&i| toks[i].kind == T::Fn) else {
                    return InferTyId::UNKNOWN;
                };
                let Some(lp) = next_sig_idx(toks, fn_i + 1).filter(|&i| toks[i].kind == T::LParen) else {
                    return InferTyId::UNKNOWN;
                };
                let Some(rp) = matching_delim(toks, lp) else {
                    return InferTyId::UNKNOWN;
                };
                let mut inputs = InlineTyBuf::new();
                self.lower_fn_ptr_params(table, &toks[lp + 1..rp], &mut inputs, depth + 1);
                let mut out = InferTyId::UNIT;
                if let Some(m) = next_sig_idx(toks, rp + 1).filter(|&i| toks[i].kind == T::Minus) {
                    if let Some(g) = next_sig_idx(toks, m + 1).filter(|&i| toks[i].kind == T::Gt) {
                        out = self.lower_tok_slice_ty(table, &toks[g + 1..], depth + 1);
                    }
                }
                table.arena.mk_fn_ptr(inputs.as_slice(), out)
            }
            _ => self.lower_tok_path_ty(table, &toks[i0..], false, false, depth + 1),
        }
    }

    fn lower_tok_path_ty(&self, table: &mut InferenceTable, toks: &[Token], as_dyn: bool, as_opaque: bool, depth: u8) -> InferTyId {
        let mut cur = 0usize;
        let mut first_ident: Option<Token> = None;
        let mut last_ident: Option<Token> = None;
        let mut args = InlineTyBuf::new();
        while let Some(i) = next_sig_idx(toks, cur) {
            match toks[i].kind {
                T::Colon if is_double_colon(toks, i) => cur = i + 2,
                T::Ident | T::RawIdent | T::SelfType | T::SelfKw | T::Super | T::Crate => {
                    if first_ident.is_none() {
                        first_ident = Some(toks[i]);
                    }
                    last_ident = Some(toks[i]);
                    args = InlineTyBuf::new();
                    cur = i + 1;
                }
                T::Lt => {
                    let Some(gt) = matching_angle(toks, i) else {
                        break;
                    };
                    for_each_top_comma(&toks[i + 1..gt], |arg_cl| {
                        if let Some(a0) = next_sig_idx(arg_cl, 0) {
                            if arg_cl[a0].kind != T::Lifetime {
                                let rhs = find_top_tok(arg_cl, T::Eq).map_or(arg_cl, |eq| &arg_cl[eq + 1..]);
                                args.push(self.lower_tok_slice_ty(table, rhs, depth + 1));
                            }
                        }
                    });
                    cur = gt + 1;
                }
                T::LParen => {
                    let Some(rp) = matching_delim(toks, i) else {
                        break;
                    };
                    let mut fn_ins = InlineTyBuf::new();
                    for_each_top_comma(&toks[i + 1..rp], |cl| {
                        if next_sig_idx(cl, 0).is_some() {
                            fn_ins.push(self.lower_tok_slice_ty(table, cl, depth + 1));
                        }
                    });
                    let tup = table.arena.mk_tuple(fn_ins.as_slice());
                    let mut ret = InferTyId::UNIT;
                    if let Some(m) = next_sig_idx(toks, rp + 1).filter(|&k| toks[k].kind == T::Minus) {
                        if let Some(g) = next_sig_idx(toks, m + 1).filter(|&k| toks[k].kind == T::Gt) {
                            ret = self.lower_tok_slice_ty(table, &toks[g + 1..], depth + 1);
                        }
                    }
                    args.push(tup);
                    args.push(ret);
                    break;
                }
                _ => break,
            }
        }
        let Some(id_tok) = last_ident else {
            return InferTyId::UNKNOWN;
        };
        let Some(rref) = self.file_res.refs_at_span(id_tok.start, id_tok.end).first() else {
            return InferTyId::UNKNOWN;
        };
        let mut qself_ty = None;
        if matches!(rref.target, ResTarget::Def { kind: ItemKind::AssocTy, .. }) {
            if let Some(ftok) = first_ident.filter(|ft| ft.start != id_tok.start) {
                qself_ty = Some(if ftok.kind == T::SelfType {
                    self.self_ty.unwrap_or(InferTyId::UNKNOWN)
                } else if let Some(fref) = self.file_res.refs_at_span(ftok.start, ftok.end).first() {
                    match fref.target {
                        ResTarget::Generic(gid) => self.resolve_generic_param(table, gid),
                        ResTarget::SelfTy { .. } => self.self_ty.unwrap_or(InferTyId::UNKNOWN),
                        ResTarget::Prim(p) => InferTyId::from_prim(p),
                        _ => InferTyId::UNKNOWN,
                    }
                } else {
                    InferTyId::UNKNOWN
                });
            }
        }
        let aname = crate::sema_fn::norm_ident(id_tok.text(self.st.src));
        self.finish_resolved_type(table, rref.target, args.as_slice(), qself_ty, GlobalDefId::NONE, aname, as_dyn, as_opaque, depth)
    }

    #[inline]
    pub(super) fn finish_ufcs_qself_type(&self, table: &mut InferenceTable, target: ResTarget, qseg: NodeId, explicit_args: &[InferTyId]) -> InferTyId {
        let use_defaults = self.st.child(qseg, K::GenericArgList).is_some();
        self.finish_resolved_type_ext(table, target, explicit_args, None, GlobalDefId::NONE, "", false, false, use_defaults, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn fill_type_args(
        &self,
        table: &mut InferenceTable,
        def: GlobalDefId,
        g_count: u16,
        explicit_args: &[InferTyId],
        use_defaults: bool,
        depth: u8,
        out: &mut InlineTyBuf,
    ) {
        for i in 0..g_count as usize {
            if let Some(&ea) = explicit_args.get(i) {
                out.push(ea);
                continue;
            }
            if use_defaults && depth <= 32 {
                let (dst, dfr) = self.defs_ctx();
                let def_ty = self
                    .db
                    .generic_param_default_ty(def, i as u16, &mut table.arena)
                    .or_else(|| self.local_defs.generic_param_default_ty(def, i as u16, dst, dfr, self.db, table, depth + 1));
                if let Some(dty) = def_ty {
                    out.push(instantiate_in_arena(&mut table.arena, dty, out.as_slice()));
                    continue;
                }
            }
            out.push(self.placeholder_ty(table));
        }
    }

    pub(super) fn resolve_mapped_type_target(&self, tail_seg: NodeId) -> Option<ResTarget> {
        let (_, rfr, spans_opt) = self.sub_map?;
        let (ts, te) = self.st.own_tokens(tail_seg).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent | T::SelfType)).map_or_else(
            || {
                let sr = self.st.range(tail_seg);
                (sr.start as u32, sr.end as u32)
            },
            |(_, t)| (t.start, t.end),
        );
        let (os, oe) = match spans_opt {
            None => (ts, te),
            Some(spans) => crate::name_res::mbe_expand::map_span_range(spans, ts, te)?,
        };
        rfr.refs_at_span(os, oe).iter().find(|r| !matches!(r.target, ResTarget::Local(_))).map(|r| r.target)
    }

    pub(super) fn resolve_unbracketed_assoc_qself(&self, table: &mut InferenceTable, first_seg: Option<NodeId>, tail_seg: Option<NodeId>, depth: u8) -> InferTyId {
        let Some(fseg) = first_seg.filter(|&f| Some(f) != tail_seg) else {
            return self.self_ty.unwrap_or(InferTyId::UNKNOWN);
        };
        let Some((_, tok)) = self.st.own_tokens(fseg).find(|(_, t)| matches!(t.kind, T::SelfType | T::Ident | T::RawIdent)) else {
            return InferTyId::UNKNOWN;
        };
        if tok.kind == T::SelfType {
            return self.self_ty.unwrap_or(InferTyId::UNKNOWN);
        }
        let Some(fref) = self.file_res.refs_at_span(tok.start, tok.end).first() else { return InferTyId::UNKNOWN };
        match fref.target {
            ResTarget::Generic(gid) => self.resolve_generic_param(table, gid),
            ResTarget::SelfTy { .. } => self.self_ty.unwrap_or(InferTyId::UNKNOWN),
            ResTarget::Prim(p) => InferTyId::from_prim(p),
            ResTarget::Def { kind: ItemKind::Struct | ItemKind::Enum | ItemKind::Union | ItemKind::TypeAlias, .. } => {
                let mut fargs = InlineTyBuf::new();
                self.collect_segment_args(table, fseg, false, &mut fargs, depth + 1);
                self.finish_resolved_type(table, fref.target, fargs.as_slice(), None, GlobalDefId::NONE, "", false, false, depth + 1)
            }
            _ => InferTyId::UNKNOWN,
        }
    }
}

pub(super) fn trait_default_assoc(item: super::db::TyLangItem) -> Option<super::db::TyLangItem> {
    use super::db::TyLangItem;
    match item {
        TyLangItem::Try => Some(TyLangItem::TryOutput),
        TyLangItem::Future => Some(TyLangItem::FutureOutput),
        TyLangItem::IntoFuture => Some(TyLangItem::IntoFutureOutput),
        TyLangItem::Deref => Some(TyLangItem::DerefTarget),
        TyLangItem::Index => Some(TyLangItem::IndexOutput),
        TyLangItem::IntoIterator => Some(TyLangItem::IntoIteratorItem),
        TyLangItem::Iterator => Some(TyLangItem::IteratorItem),
        _ => None,
    }
}
