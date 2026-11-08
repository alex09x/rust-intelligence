/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Literals, field access (`.field`, `.0`, `.0.1` with autoderef), indexing (`Index::Output`),
//! `?` (`Try`), `.await` (`Future`/`IntoFuture`), borrows/derefs, unary/binary operators, casts,
//! and range constructors (`Range*`) for [`super::body_ctx::Runner`].

use super::autoderef::{autoderef_step, normalize_ty};
use super::body_ctx::{ResolvedDotRef, Runner};
use super::db::TyLangItem;
use super::expectation::Expectation;
use super::infer_expr::is_expr_node;
use super::lang_ops::{deref_step, infer_index_op, mk_range_adt, unwrap_await_ty, unwrap_try_ty};
use super::tok_ty_scan::{decoded_byte_str_len, parse_float_suffix, parse_int_or_float_suffix};
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::{GlobalDefId, ItemKind};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

impl<'a, 's> Runner<'a, 's> {
    pub(super) fn infer_literal_tokens(&mut self, node: NodeId) -> InferTyId {
        let mut lit_tok = None;
        for (_, tok) in self.st.own_tokens(node) {
            if tok.kind != T::Minus {
                lit_tok = Some(tok);
                break;
            }
        }
        let Some(tok) = lit_tok else {
            return InferTyId::UNKNOWN;
        };
        match tok.kind {
            T::True | T::False => InferTyId::BOOL,
            T::Char => InferTyId::CHAR,
            T::Byte => InferTyId::U8,
            T::Str | T::RawStr => InferTyId::STR_REF,
            T::ByteStr | T::RawByteStr => {
                let n = decoded_byte_str_len(tok.text(self.st.src), tok.kind == T::RawByteStr);
                let arr = self.ctx.table.arena.mk_array(InferTyId::U8, n);
                self.ctx.table.arena.mk_ref(false, arr)
            }
            T::CStr | T::RawCStr => match self.db.cstr_def() {
                Some(def) => {
                    let cstr = self.ctx.table.arena.mk_adt(def, &[]);
                    self.ctx.table.arena.mk_ref(false, cstr)
                }
                None => InferTyId::UNKNOWN,
            },
            T::Int => parse_int_or_float_suffix(tok.text(self.st.src)).unwrap_or_else(|| self.ctx.table.new_int_var()),
            T::Float => parse_float_suffix(tok.text(self.st.src)).unwrap_or_else(|| self.ctx.table.new_float_var()),
            _ => InferTyId::UNKNOWN,
        }
    }

    pub(super) fn lookup_instantiated_field_ty(&mut self, owner_def: GlobalDefId, name: &str, subst: &[InferTyId]) -> InferTyId {
        self.lookup_instantiated_field_ty_and_def(owner_def, name, subst).map_or(InferTyId::UNKNOWN, |(t, _)| t)
    }

    fn lookup_instantiated_field_ty_and_def(&mut self, owner_def: GlobalDefId, name: &str, subst: &[InferTyId]) -> Option<(InferTyId, GlobalDefId)> {
        let (_, raw_fty, fdef) = self
            .db
            .field_ty_and_def(owner_def, name, &mut self.ctx.table.arena)
            .or_else(|| self.local_defs.field_ty_and_def(owner_def, name, self.st, self.file_res, self.db, &mut self.ctx.table))?;
        let inst = self.ctx.table.instantiate(raw_fty, subst);
        Some((normalize_ty(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, inst), fdef))
    }

    pub(super) fn infer_field_expr(&mut self, expr: NodeId) -> InferTyId {
        let Some(recv) = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))) else {
            return InferTyId::UNKNOWN;
        };
        let recv_ty = self.infer_expr(recv, Expectation::None);
        if let Some(nn) = self.st.child(expr, K::Name) {
            let fname = norm_ident(self.st.text(nn));
            let r = self.st.range(nn);
            return self.resolve_field_with_autoderef(recv_ty, fname, None, (r.start as u32, r.end as u32));
        }
        let Some((_, idx_tok)) = self.st.own_tokens(expr).find(|(_, t)| matches!(t.kind, T::Int | T::Float)) else {
            if let Some((_, dtok)) = self.st.own_tokens(expr).filter(|(_, t)| t.kind == T::Dot).last() {
                self.out.incomplete_dots.push((dtok.end, recv_ty));
            }
            return InferTyId::UNKNOWN;
        };
        let mut cur = recv_ty;
        let tok_text = idx_tok.text(self.st.src);
        let mut rel_off = 0u32;
        for raw_part in tok_text.split('.') {
            let p_start = idx_tok.start + rel_off;
            let p_end = p_start + raw_part.len() as u32;
            rel_off += raw_part.len() as u32 + 1;
            let part = raw_part.trim();
            if part.is_empty() {
                continue;
            }
            cur = self.resolve_field_with_autoderef(cur, part, part.parse::<usize>().ok(), (p_start, p_end));
        }
        if tok_text.ends_with('.') {
            self.out.incomplete_dots.push((idx_tok.end, cur));
            return InferTyId::UNKNOWN;
        }
        cur
    }

    fn resolve_field_with_autoderef(&mut self, start_ty: InferTyId, fname: &str, tuple_idx: Option<usize>, span: (u32, u32)) -> InferTyId {
        let mut cur = normalize_ty(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, start_ty);
        for _ in 0..12 {
            match self.ctx.table.arena.kind(cur) {
                InferTyKind::Tuple(r) => {
                    if let Some(i) = tuple_idx {
                        if i < r.len as usize {
                            let elem = self.ctx.table.arena.slice(r)[i];
                            return normalize_ty(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, elem);
                        }
                    }
                }
                InferTyKind::Adt { def, args } => {
                    let mut subst = InlineTyBuf::new();
                    for &a in self.ctx.table.arena.slice(args) {
                        subst.push(a);
                    }
                    if let Some((fty, fdef)) = self.lookup_instantiated_field_ty_and_def(def, fname, subst.as_slice()) {
                        if fdef.is_some() {
                            self.out.dot_refs.push(ResolvedDotRef { start: span.0, end: span.1, def: fdef, kind: ItemKind::Field });
                        }
                        return fty;
                    }
                }
                _ => {}
            }
            let Some(next) = autoderef_step(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, cur) else {
                break;
            };
            if next == cur {
                break;
            }
            cur = next;
        }
        InferTyId::UNKNOWN
    }

    pub(super) fn infer_index_expr_node(&mut self, expr: NodeId) -> InferTyId {
        let mut kids = self.st.children(expr).filter(|&c| is_expr_node(self.st.kind(c)));
        let Some(base) = kids.next() else {
            return InferTyId::UNKNOWN;
        };
        let base_ty = self.infer_expr(base, Expectation::None);
        let idx_ty = kids.next().map_or(InferTyId::UNKNOWN, |idx| self.infer_expr(idx, Expectation::None));
        let raw = infer_index_op(&mut self.ctx.table, self.db, base_ty, idx_ty);
        let owner = self.norm_owner();
        super::autoderef::normalize_ty_with_owner(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, owner, raw)
    }

    pub(super) fn infer_try_expr(&mut self, expr: NodeId) -> InferTyId {
        let op_ty = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))).map_or(InferTyId::UNKNOWN, |c| self.infer_expr(c, Expectation::None));
        let raw = unwrap_try_ty(&mut self.ctx.table, self.db, op_ty);
        let owner = self.norm_owner();
        super::autoderef::normalize_ty_with_owner(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, owner, raw)
    }

    pub(super) fn infer_await_expr(&mut self, expr: NodeId) -> InferTyId {
        let op_ty = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))).map_or(InferTyId::UNKNOWN, |c| self.infer_expr(c, Expectation::None));
        let raw = unwrap_await_ty(&mut self.ctx.table, self.db, op_ty);
        let owner = self.norm_owner();
        super::autoderef::normalize_ty_with_owner(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, owner, raw)
    }

    pub(super) fn infer_ref_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let is_raw = self.st.own_tokens(expr).any(|(_, t)| t.kind == T::Ident && t.text(self.st.src) == "raw");
        let is_mut = self.st.own_tokens(expr).any(|(_, t)| t.kind == T::Mut);
        let Some(sub) = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))) else {
            return self.record_expr(expr, InferTyId::UNKNOWN);
        };
        if is_raw {
            let inner = self.infer_expr(sub, Expectation::None);
            let ptr = self.ctx.table.arena.mk_ptr(is_mut, inner);
            let final_ty = match expectation.coercion_target(&mut self.ctx.table) {
                Some(exp) => self.coerce_expr(ptr, exp),
                None => ptr,
            };
            return self.record_expr(expr, final_ty);
        }

        let sub_exp = expectation.peel_for_borrow(&mut self.ctx.table, is_mut);
        let inner_ty = self.infer_expr(sub, sub_exp);
        let raw_ref = if let Expectation::RvalueLikeUnsized(unsz_ty) = sub_exp.resolve(&mut self.ctx.table) {
            let in_res = self.ctx.table.shallow_resolve(inner_ty);
            match (self.ctx.table.arena.kind(in_res), self.ctx.table.arena.kind(unsz_ty)) {
                (InferTyKind::Array { elem: ae, .. }, InferTyKind::Slice(se)) => {
                    let _ = self.ctx.table.equate(ae, se);
                    self.ctx.table.arena.mk_ref(is_mut, unsz_ty)
                }
                (_, InferTyKind::Dyn { .. }) => {
                    let _ = self.coerce_expr(in_res, unsz_ty);
                    self.ctx.table.arena.mk_ref(is_mut, unsz_ty)
                }
                _ => self.ctx.table.arena.mk_ref(is_mut, in_res),
            }
        } else {
            self.ctx.table.arena.mk_ref(is_mut, inner_ty)
        };
        let final_ty = match expectation.coercion_target(&mut self.ctx.table) {
            Some(exp) => self.coerce_expr(raw_ref, exp),
            None => raw_ref,
        };
        self.record_expr(expr, final_ty)
    }

    pub(super) fn infer_prefix_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let op = self.st.own_tokens(expr).next().map(|(_, t)| t.kind);
        let Some(sub) = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))) else {
            return InferTyId::UNKNOWN;
        };
        match op {
            Some(T::Star) => {
                let sub_ty = self.infer_expr(sub, Expectation::None);
                deref_step(&mut self.ctx.table, self.db, sub_ty).unwrap_or(InferTyId::UNKNOWN)
            }
            Some(T::Bang) => {
                let sub_ty = self.infer_expr(sub, expectation);
                let r = self.ctx.table.shallow_resolve(sub_ty);
                if matches!(self.ctx.table.arena.kind(r), InferTyKind::Adt { .. } | InferTyKind::Ref { .. }) {
                    if let Some(out) = self.db.project_assoc_ty(TyLangItem::Not, r, &[], &mut self.ctx.table.arena, &mut self.ctx.table.vars) {
                        return out;
                    }
                }
                r
            }
            Some(T::Minus) => {
                let sub_ty = self.infer_expr(sub, expectation);
                let r = self.ctx.table.shallow_resolve(sub_ty);
                if matches!(self.ctx.table.arena.kind(r), InferTyKind::Adt { .. } | InferTyKind::Ref { .. }) {
                    if let Some(out) = self.db.project_assoc_ty(TyLangItem::Neg, r, &[], &mut self.ctx.table.arena, &mut self.ctx.table.vars) {
                        return out;
                    }
                }
                r
            }
            _ => self.infer_expr(sub, expectation),
        }
    }

    pub(super) fn infer_bin_expr(&mut self, expr: NodeId) -> InferTyId {
        let mut kids = self.st.children(expr).filter(|&c| is_expr_node(self.st.kind(c)));
        let Some(lhs) = kids.next() else {
            return InferTyId::UNKNOWN;
        };
        let rhs_opt = kids.next();
        let mut op_buf = [0u8; 4];
        let mut op_len = 0usize;
        for (_, t) in self.st.own_tokens(expr) {
            if let Some(b) = t.kind.punct_byte() {
                if op_len < op_buf.len() {
                    op_buf[op_len] = b;
                    op_len += 1;
                }
            }
        }
        let op = &op_buf[..op_len];
        match op {
            b"=" => {
                let lty = self.infer_expr(lhs, Expectation::None);
                if let Some(rhs) = rhs_opt {
                    let rty = self.infer_expr(rhs, Expectation::HasType(lty));
                    let _ = self.coerce_expr(rty, lty);
                }
                InferTyId::UNIT
            }
            b"+=" | b"-=" | b"*=" | b"/=" | b"%=" | b"&=" | b"|=" | b"^=" => {
                let lty = self.infer_expr(lhs, Expectation::None);
                if let Some(rhs) = rhs_opt {
                    let rty = self.infer_expr(rhs, Expectation::HasType(lty));
                    let _ = self.coerce_expr(rty, lty);
                }
                InferTyId::UNIT
            }
            b"<<=" | b">>=" => {
                let _ = self.infer_expr(lhs, Expectation::None);
                if let Some(rhs) = rhs_opt {
                    let _ = self.infer_expr(rhs, Expectation::None);
                }
                InferTyId::UNIT
            }
            b"&&" | b"||" => {
                let l = self.infer_expr(lhs, Expectation::HasType(InferTyId::BOOL));
                let _ = self.coerce_expr(l, InferTyId::BOOL);
                if let Some(rhs) = rhs_opt {
                    let r = self.infer_expr(rhs, Expectation::HasType(InferTyId::BOOL));
                    let _ = self.coerce_expr(r, InferTyId::BOOL);
                }
                InferTyId::BOOL
            }
            b"==" | b"!=" | b"<" | b"<=" | b">" | b">=" => {
                let lty = self.infer_expr(lhs, Expectation::None);
                if let Some(rhs) = rhs_opt {
                    let rty = self.infer_expr(rhs, Expectation::HasType(lty));
                    let _ = self.coerce_expr(rty, lty);
                }
                InferTyId::BOOL
            }
            b"<<" | b">>" => {
                let lty = self.infer_expr(lhs, Expectation::None);
                let Some(rhs) = rhs_opt else {
                    return InferTyId::UNKNOWN;
                };
                let rty = self.infer_expr(rhs, Expectation::None);
                let li = if op == b"<<" { TyLangItem::Shl } else { TyLangItem::Shr };
                self.db.project_assoc_ty(li, lty, &[rty], &mut self.ctx.table.arena, &mut self.ctx.table.vars).unwrap_or_else(|| self.ctx.table.shallow_resolve(lty))
            }
            _ => {
                let lty = self.infer_expr(lhs, Expectation::None);
                let Some(rhs) = rhs_opt else {
                    return InferTyId::UNKNOWN;
                };
                let op_li = match op {
                    b"+" => Some(TyLangItem::Add),
                    b"-" => Some(TyLangItem::Sub),
                    b"*" => Some(TyLangItem::Mul),
                    b"/" => Some(TyLangItem::Div),
                    b"%" => Some(TyLangItem::Rem),
                    b"&" => Some(TyLangItem::BitAnd),
                    b"|" => Some(TyLangItem::BitOr),
                    b"^" => Some(TyLangItem::BitXor),
                    _ => None,
                };
                let l_res = self.ctx.table.shallow_resolve(lty);
                if let Some(li) = op_li.filter(|_| matches!(self.ctx.table.arena.kind(l_res), InferTyKind::Adt { .. } | InferTyKind::Ref { .. })) {
                    let rty = self.infer_expr(rhs, Expectation::None);
                    if let Some(out) = self.db.project_assoc_ty(li, l_res, &[rty], &mut self.ctx.table.arena, &mut self.ctx.table.vars) {
                        return out;
                    }
                    let _ = self.ctx.table.equate(l_res, rty);
                    return self.ctx.table.shallow_resolve(l_res);
                }
                let rty = self.infer_expr(rhs, Expectation::HasType(l_res));
                let _ = self.coerce_expr(rty, l_res);
                self.ctx.table.shallow_resolve(l_res)
            }
        }
    }

    pub(super) fn infer_cast_expr(&mut self, expr: NodeId) -> InferTyId {
        let tenv = self.ty_env(true);
        let target_ty = self.st.children(expr).find(|&c| self.st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |tn| tenv.lower_cst_ty(&mut self.ctx.table, tn));
        if let Some(sub) = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))) {
            let _ = self.infer_expr(sub, Expectation::CastableTo(target_ty));
        }
        target_ty
    }

    pub(super) fn infer_range_expr(&mut self, expr: NodeId) -> InferTyId {
        let is_inclusive = self.st.own_tokens(expr).any(|(_, t)| t.kind == T::Eq);
        let first_dot_start = self.st.own_tokens(expr).find(|(_, t)| t.kind == T::Dot).map_or(0, |(_, t)| t.start as usize);
        let mut kids = self.st.children(expr).filter(|&c| is_expr_node(self.st.kind(c)));
        match (kids.next(), kids.next()) {
            (None, _) => mk_range_adt(&mut self.ctx.table, self.db, TyLangItem::RangeFull, None),
            (Some(lhs), Some(rhs)) => {
                let l_ty = self.infer_expr(lhs, Expectation::None);
                let r_ty = self.infer_expr(rhs, Expectation::HasType(l_ty));
                let joined = self.ctx.table.lub(l_ty, r_ty).unwrap_or(l_ty);
                let li = if is_inclusive { TyLangItem::RangeInclusive } else { TyLangItem::Range };
                mk_range_adt(&mut self.ctx.table, self.db, li, Some(joined))
            }
            (Some(only), None) => {
                let b_ty = self.infer_expr(only, Expectation::None);
                if self.st.range(only).start < first_dot_start {
                    mk_range_adt(&mut self.ctx.table, self.db, TyLangItem::RangeFrom, Some(b_ty))
                } else {
                    let li = if is_inclusive { TyLangItem::RangeToInclusive } else { TyLangItem::RangeTo };
                    mk_range_adt(&mut self.ctx.table, self.db, li, Some(b_ty))
                }
            }
        }
    }
}
