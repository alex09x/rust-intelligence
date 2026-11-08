/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pattern type inference and RFC 2005 match ergonomics (`BindingMode`) across all Rust pattern kinds.

use super::body_ctx::Runner;
use super::db::TyLangItem;
use super::lower_ty::effective_target_def;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::{GlobalDefId, ItemKind, LocalBindId, PathCtx, ResTarget};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

/// Match-ergonomics binding mode propagated downward into pattern bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingMode {
    ByValue,
    ByRef(bool),
}

impl BindingMode {
    #[inline]
    pub const fn peel_ref(self, ref_is_mut: bool) -> Self {
        match self {
            Self::ByValue => Self::ByRef(ref_is_mut),
            Self::ByRef(prev_mut) => Self::ByRef(prev_mut && ref_is_mut),
        }
    }
}

impl<'a, 's> Runner<'a, 's> {
    fn peel_ergonomics(&mut self, raw_ty: InferTyId, mut mode: BindingMode) -> (InferTyId, BindingMode) {
        let mut cur = self.ctx.table.shallow_resolve(raw_ty);
        while let InferTyKind::Ref { is_mut, inner } = self.ctx.table.arena.kind(cur) {
            mode = mode.peel_ref(is_mut);
            cur = self.ctx.table.shallow_resolve(inner);
        }
        (cur, mode)
    }

    pub(super) fn infer_pat(&mut self, pat: NodeId, expected_ty: InferTyId, mode: BindingMode, or_base: Option<usize>) -> InferTyId {
        let exp = self.ctx.table.shallow_resolve(expected_ty);
        match self.st.kind(pat) {
            K::WildcardPat | K::RestPat | K::Error => self.record_pat(pat, exp),
            K::ParenPat => {
                let inner = self.st.children(pat).find(|&c| self.st.kind(c).is_pat()).map_or(exp, |p| self.infer_pat(p, exp, mode, or_base));
                self.record_pat(pat, inner)
            }
            K::OrPat => {
                let mark = self.ctx.or_pat_binds.len();
                let base = or_base.unwrap_or(mark);
                for alt in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()) {
                    self.infer_pat(alt, exp, mode, Some(base));
                }
                if or_base.is_none() {
                    self.ctx.or_pat_binds.truncate(mark);
                }
                self.record_pat(pat, exp)
            }
            K::RefPat => {
                let pat_mut = self.st.own_tokens(pat).any(|(_, t)| t.kind == T::Mut);
                let inner_ty = match self.ctx.table.arena.kind(exp) {
                    InferTyKind::Ref { inner, .. } => self.ctx.table.shallow_resolve(inner),
                    _ => {
                        let v = self.ctx.table.new_ty_var();
                        let r = self.ctx.table.arena.mk_ref(pat_mut, v);
                        let _ = self.ctx.table.equate(exp, r);
                        v
                    }
                };
                if let Some(sub) = self.st.children(pat).find(|&c| self.st.kind(c).is_pat()) {
                    self.infer_pat(sub, inner_ty, BindingMode::ByValue, or_base);
                }
                self.record_pat(pat, exp)
            }
            K::BoxPat => {
                let (peeled, sub_mode) = self.peel_ergonomics(exp, mode);
                let inner_ty = match self.ctx.table.arena.kind(peeled) {
                    InferTyKind::Adt { def, args } if self.db.lang_item_of_def(def) == Some(TyLangItem::OwnedBox) && args.len >= 1 => self.ctx.table.arena.slice(args)[0],
                    _ => self.ctx.table.new_ty_var(),
                };
                if let Some(sub) = self.st.children(pat).find(|&c| self.st.kind(c).is_pat()) {
                    self.infer_pat(sub, inner_ty, sub_mode, or_base);
                }
                self.record_pat(pat, exp)
            }
            K::IdentPat => self.infer_ident_pat(pat, exp, mode, or_base),
            K::TuplePat => {
                let (peeled, sub_mode) = self.peel_ergonomics(exp, mode);
                let mut stack_pats = [NodeId(0); 24];
                let mut sub_len = 0usize;
                let mut rest_pos: Option<usize> = None;
                for (i, sp) in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()).enumerate() {
                    if self.st.kind(sp) == K::RestPat && rest_pos.is_none() {
                        rest_pos = Some(i);
                    }
                    if sub_len < stack_pats.len() {
                        stack_pats[sub_len] = sp;
                        sub_len += 1;
                    }
                }
                let sub_pats = &stack_pats[..sub_len];
                match self.ctx.table.arena.kind(peeled) {
                    InferTyKind::Tuple(r) => {
                        let total = r.len as usize;
                        let mut elems = InlineTyBuf::new();
                        for idx in 0..total {
                            elems.push(self.ctx.table.arena.slice(r)[idx]);
                        }
                        if let Some(rp) = rest_pos {
                            let head = rp;
                            let tail = sub_pats.len().saturating_sub(rp + 1);
                            for (i, &sp) in sub_pats[..head].iter().enumerate() {
                                let ety = elems.as_slice().get(i).copied().unwrap_or(InferTyId::UNKNOWN);
                                self.infer_pat(sp, ety, sub_mode, or_base);
                            }
                            self.record_pat(sub_pats[rp], InferTyId::UNIT);
                            for (k, &sp) in sub_pats[rp + 1..].iter().enumerate() {
                                let idx = total.saturating_sub(tail) + k;
                                let ety = elems.as_slice().get(idx).copied().unwrap_or(InferTyId::UNKNOWN);
                                self.infer_pat(sp, ety, sub_mode, or_base);
                            }
                        } else {
                            for (i, &sp) in sub_pats.iter().enumerate() {
                                let ety = elems.as_slice().get(i).copied().unwrap_or(InferTyId::UNKNOWN);
                                self.infer_pat(sp, ety, sub_mode, or_base);
                            }
                        }
                    }
                    _ if rest_pos.is_none() => {
                        let mut vars = InlineTyBuf::new();
                        for &sp in sub_pats {
                            let v = self.ctx.table.new_ty_var();
                            self.infer_pat(sp, v, sub_mode, or_base);
                            vars.push(v);
                        }
                        let tup = self.ctx.table.arena.mk_tuple(vars.as_slice());
                        let _ = self.ctx.table.equate(peeled, tup);
                    }
                    _ => {
                        for &sp in sub_pats {
                            self.infer_pat(sp, InferTyId::UNKNOWN, sub_mode, or_base);
                        }
                    }
                }
                self.record_pat(pat, exp)
            }
            K::SlicePat => {
                let (peeled, sub_mode) = self.peel_ergonomics(exp, mode);
                let (elem_ty, rest_ty) = match self.ctx.table.arena.kind(peeled) {
                    InferTyKind::Array { elem, len } => {
                        let non_rest = self.st.children(pat).filter(|&c| self.st.kind(c).is_pat() && !is_rest_like_pat(self.st, c)).count() as u64;
                        let rem = if len != u64::MAX { len.saturating_sub(non_rest) } else { u64::MAX };
                        let sub_arr = if matches!(sub_mode, BindingMode::ByRef(_)) { self.ctx.table.arena.mk_slice(elem) } else { self.ctx.table.arena.mk_array(elem, rem) };
                        (elem, sub_arr)
                    }
                    InferTyKind::Slice(elem) => (elem, peeled),
                    _ => {
                        let ev = self.ctx.table.new_ty_var();
                        let sl = self.ctx.table.arena.mk_slice(ev);
                        let _ = self.ctx.table.equate(peeled, sl);
                        (ev, sl)
                    }
                };
                for sp in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()) {
                    if is_rest_like_pat(self.st, sp) {
                        self.infer_pat(sp, rest_ty, sub_mode, or_base);
                    } else {
                        self.infer_pat(sp, elem_ty, sub_mode, or_base);
                    }
                }
                self.record_pat(pat, exp)
            }
            K::LiteralPat => {
                let (peeled, _) = self.peel_ergonomics(exp, mode);
                let lty = self.infer_literal_tokens(pat);
                let _ = self.ctx.table.equate(peeled, lty);
                self.record_pat(pat, exp)
            }
            K::RangePat => {
                let (peeled, sub_mode) = self.peel_ergonomics(exp, mode);
                for ch in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()) {
                    self.infer_pat(ch, peeled, sub_mode, or_base);
                }
                if !self.st.children(pat).any(|c| self.st.kind(c).is_pat()) {
                    let lty = self.infer_literal_tokens(pat);
                    if lty != InferTyId::UNKNOWN {
                        let _ = self.ctx.table.equate(peeled, lty);
                    }
                }
                self.record_pat(pat, exp)
            }
            K::PathPat => {
                let (peeled, _) = self.peel_ergonomics(exp, mode);
                if let Some(p) = self.st.child(pat, K::Path) {
                    let pty = self.infer_value_path(p);
                    let _ = self.ctx.table.equate(peeled, pty);
                }
                self.record_pat(pat, exp)
            }
            K::TupleStructPat => self.infer_tuple_struct_pat(pat, exp, mode, or_base),
            K::RecordPat => self.infer_record_pat(pat, exp, mode, or_base),
            _ => self.record_pat(pat, exp),
        }
    }

    fn infer_ident_pat(&mut self, pat: NodeId, exp: InferTyId, mode: BindingMode, or_base: Option<usize>) -> InferTyId {
        let subpat = self.st.children(pat).find(|&c| self.st.kind(c).is_pat());
        if let Some(nn) = self.st.child(pat, K::Name) {
            let r = self.st.range(nn);
            if subpat.is_none() {
                if let Some(rref) = self.file_res.refs_at_span(r.start as u32, r.end as u32).iter().find(|rf| rf.ctx == PathCtx::PatValue) {
                    let (peeled, _) = self.peel_ergonomics(exp, mode);
                    let item_ty = self.instantiate_target_value(rref.target, &[]);
                    let _ = self.ctx.table.equate(peeled, item_ty);
                    return self.record_pat(pat, exp);
                }
            }
            let has_ref = self.st.own_tokens(pat).any(|(_, t)| t.kind == T::Ref);
            let has_mut = self.st.own_tokens(pat).any(|(_, t)| t.kind == T::Mut);
            let eff_mode = if has_ref { BindingMode::ByRef(has_mut) } else { mode };
            let bound_ty = match eff_mode {
                BindingMode::ByValue => exp,
                BindingMode::ByRef(m) => self.ctx.table.arena.mk_ref(m, exp),
            };
            let norm = norm_ident(self.st.text(nn));
            if norm != "_" {
                let lid = self.resolve_pat_local_id(pat, r.start as u32, r.end as u32, norm, or_base);
                if lid.is_some() {
                    self.set_local(lid, bound_ty);
                }
            }
            if let Some(sp) = subpat {
                self.infer_pat(sp, exp, mode, or_base);
            }
            return self.record_pat(pat, bound_ty);
        }
        self.record_pat(pat, exp)
    }

    fn resolve_pat_local_id(&mut self, pat: NodeId, start: u32, end: u32, norm: &str, or_base: Option<usize>) -> LocalBindId {
        if let Some(lid) = self.find_decl_local(pat) {
            if or_base.is_some() {
                self.ctx.or_pat_binds.push((start, end, lid));
            }
            return lid;
        }
        if let Some(base) = or_base {
            for &(s, e, lid) in &self.ctx.or_pat_binds[base..] {
                if norm_ident(&self.st.src[s as usize..e as usize]) == norm {
                    return lid;
                }
            }
        }
        LocalBindId::NONE
    }

    fn infer_tuple_struct_pat(&mut self, pat: NodeId, exp: InferTyId, mode: BindingMode, or_base: Option<usize>) -> InferTyId {
        let (peeled, sub_mode) = self.peel_ergonomics(exp, mode);
        let (owner_def, subst) = self.instantiate_pat_ctor(pat, peeled);
        let mut raw_fields = core::mem::take(&mut self.ctx.sig_inputs_buf);
        raw_fields.clear();
        let found = owner_def.is_some()
            && (self.db.ctor_tuple_fields(owner_def, &mut self.ctx.table.arena, &mut raw_fields).is_some()
                || self.local_defs.ctor_tuple_fields(owner_def, self.st, self.file_res, self.db, &mut self.ctx.table, &mut raw_fields).is_some());
        let mut stack_pats = [NodeId(0); 24];
        let mut sub_len = 0usize;
        for c in self.st.children(pat).filter(|&c| self.st.kind(c).is_pat()) {
            if sub_len < stack_pats.len() {
                stack_pats[sub_len] = c;
                sub_len += 1;
            }
        }
        let sub_pats = &stack_pats[..sub_len];
        let rest_pos = sub_pats.iter().position(|&c| self.st.kind(c) == K::RestPat);
        let total = raw_fields.len();
        for (i, &sp) in sub_pats.iter().enumerate() {
            if Some(i) == rest_pos {
                self.record_pat(sp, InferTyId::UNIT);
                continue;
            }
            let field_idx = match rest_pos {
                Some(rp) if i > rp => total.saturating_sub(sub_pats.len() - 1 - rp) + (i - rp - 1),
                _ => i,
            };
            let fty =
                if found { raw_fields.get(field_idx).copied().map_or(InferTyId::UNKNOWN, |raw| self.ctx.table.instantiate(raw, subst.as_slice())) } else { InferTyId::UNKNOWN };
            self.infer_pat(sp, fty, sub_mode, or_base);
        }
        self.ctx.sig_inputs_buf = raw_fields;
        self.record_pat(pat, exp)
    }

    fn infer_record_pat(&mut self, pat: NodeId, exp: InferTyId, mode: BindingMode, or_base: Option<usize>) -> InferTyId {
        let (peeled, sub_mode) = self.peel_ergonomics(exp, mode);
        let (owner_def, subst) = self.instantiate_pat_ctor(pat, peeled);
        for ch in self.st.children(pat) {
            if self.st.kind(ch) != K::RecordPatField {
                continue;
            }
            let has_colon = self.st.own_tokens(ch).any(|(_, t)| t.kind == T::Colon);
            let Some(sub_pat) = self.st.children(ch).find(|&c| self.st.kind(c).is_pat()) else {
                continue;
            };
            let fname = if has_colon {
                self.st.own_tokens(ch).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent | T::Int)).map(|(_, t)| norm_ident(t.text(self.st.src)))
            } else {
                self.st.child(sub_pat, K::Name).map(|nn| norm_ident(self.st.text(nn)))
            };
            let fty = match fname.filter(|_| owner_def.is_some()) {
                Some(n) => self.lookup_instantiated_field_ty(owner_def, n, subst.as_slice()),
                None => InferTyId::UNKNOWN,
            };
            self.infer_pat(sub_pat, fty, sub_mode, or_base);
        }
        self.record_pat(pat, exp)
    }

    pub(super) fn instantiate_pat_ctor(&mut self, node_with_path: NodeId, expected_adt_ty: InferTyId) -> (GlobalDefId, InlineTyBuf) {
        let mut subst = InlineTyBuf::new();
        let Some(p) = self.st.child(node_with_path, K::Path) else {
            return (GlobalDefId::NONE, subst);
        };
        let Some(rref) = self.file_res.path_tail_ref(p) else {
            return (GlobalDefId::NONE, subst);
        };
        if matches!(rref.target, ResTarget::SelfTy { .. }) {
            if let Some(sty) = self.self_ty {
                let _ = self.ctx.table.equate(expected_adt_ty, sty);
                let res_sty = self.ctx.table.shallow_resolve(sty);
                if let InferTyKind::Adt { def, args } = self.ctx.table.arena.kind(res_sty) {
                    for &a in self.ctx.table.arena.slice(args) {
                        subst.push(a);
                    }
                    return (def, subst);
                }
            }
        }
        let mut explicit = InlineTyBuf::new();
        self.collect_path_turbofish(p, &mut explicit);
        let raw_def = effective_target_def(rref.target);
        if !raw_def.is_some() {
            return (GlobalDefId::NONE, subst);
        }
        let def_info = self.db.def_ty(raw_def, &mut self.ctx.table.arena).or_else(|| self.local_defs.def_ty(raw_def, self.st, self.file_res, self.db, &mut self.ctx.table));
        let Some((gc, _, tpl_ty)) = def_info else {
            return (raw_def, subst);
        };
        for i in 0..gc as usize {
            subst.push(explicit.as_slice().get(i).copied().unwrap_or_else(|| self.ctx.table.new_ty_var()));
        }
        let inst_adt = self.ctx.table.instantiate(tpl_ty, subst.as_slice());
        let _ = self.ctx.table.equate(expected_adt_ty, inst_adt);
        if let ResTarget::Def { kind: ItemKind::TypeAlias, .. } = rref.target {
            let res_inst = self.ctx.table.shallow_resolve(inst_adt);
            if let InferTyKind::Adt { def: real_def, args } = self.ctx.table.arena.kind(res_inst) {
                let mut real_subst = InlineTyBuf::new();
                for &a in self.ctx.table.arena.slice(args) {
                    real_subst.push(a);
                }
                return (real_def, real_subst);
            }
        }
        (raw_def, subst)
    }
}

fn is_rest_like_pat(st: &crate::tree::SyntaxTree<'_>, pat: NodeId) -> bool {
    st.kind(pat) == K::RestPat || (st.kind(pat) == K::IdentPat && st.children(pat).any(|c| st.kind(c) == K::RestPat))
}
