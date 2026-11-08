/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Call (`CallExpr`), struct literal (`RecordExpr`), closure (`ClosureExpr`), and method-call
//! fallback inference for [`super::body_ctx::Runner`].

use super::body_ctx::Runner;
use super::db::TyLangItem;
use super::expectation::Expectation;
use super::infer_expr::is_expr_node;
use super::infer_pat::BindingMode;
use super::lang_ops::{extract_expected_callable_sig, unwrap_await_ty};
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::GlobalDefId;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

impl<'a, 's> Runner<'a, 's> {
    pub(super) fn infer_call_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let Some(callee) = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))) else {
            return InferTyId::UNKNOWN;
        };
        let mut arg_nodes = [NodeId(0); 24];
        let mut arg_len = 0usize;
        if let Some(al) = self.st.child(expr, K::ArgList) {
            for ch in self.st.children(al).filter(|&c| is_expr_node(self.st.kind(c))) {
                if arg_len < arg_nodes.len() {
                    arg_nodes[arg_len] = ch;
                    arg_len += 1;
                }
            }
        }
        let args = &arg_nodes[..arg_len];
        let callee_ty = self.infer_expr(callee, Expectation::None);
        let resolved_callee = self.ctx.table.shallow_resolve(callee_ty);

        match self.ctx.table.arena.kind(resolved_callee) {
            InferTyKind::FnDef { def, args: subst_range } => self.infer_fn_def_call(def, subst_range, args, expectation),
            InferTyKind::FnPtr { sig } | InferTyKind::Closure { sig, .. } if sig.len >= 1 => {
                let mut ins = InlineTyBuf::new();
                let s = self.ctx.table.arena.slice(sig);
                for &p in &s[..s.len() - 1] {
                    ins.push(p);
                }
                let ret = s[s.len() - 1];
                self.pre_unify_call_return(ret, expectation);
                self.infer_call_args_slice(ins.as_slice(), args);
                self.ctx.table.shallow_resolve(ret)
            }
            _ => {
                let mut bound_ins = core::mem::take(&mut self.ctx.bound_inputs_buf);
                let callable = extract_expected_callable_sig(
                    &mut self.ctx.table,
                    self.db,
                    self.local_defs,
                    self.st,
                    self.file_res,
                    self.fn_def,
                    Expectation::HasType(resolved_callee),
                    &mut bound_ins,
                );
                if let Some((ret, is_async)) = callable {
                    let mut ins = InlineTyBuf::new();
                    for &p in &bound_ins {
                        ins.push(p);
                    }
                    self.ctx.bound_inputs_buf = bound_ins;
                    self.pre_unify_call_return(ret, expectation);
                    self.infer_call_args_slice(ins.as_slice(), args);
                    let r = self.ctx.table.shallow_resolve(ret);
                    if is_async {
                        if let Some(fut_def) = self.db.lang_item(TyLangItem::Future) {
                            return self.ctx.table.arena.mk_opaque(fut_def, &[r]);
                        }
                    }
                    r
                } else {
                    self.ctx.bound_inputs_buf = bound_ins;
                    for &a in args {
                        let _ = self.infer_expr(a, Expectation::None);
                    }
                    InferTyId::UNKNOWN
                }
            }
        }
    }

    fn infer_fn_def_call(&mut self, def: GlobalDefId, subst_range: super::ty::TySliceRange, args: &[NodeId], expectation: Expectation) -> InferTyId {
        let mut subst = InlineTyBuf::new();
        for &a in self.ctx.table.arena.slice(subst_range) {
            subst.push(a);
        }
        let mut raw_ins = core::mem::take(&mut self.ctx.sig_inputs_buf);
        let sig_opt = self
            .db
            .fn_sig(def, &mut self.ctx.table.arena, &mut raw_ins)
            .or_else(|| self.local_defs.fn_sig(def, self.st, self.file_res, self.db, &mut self.ctx.table, &mut raw_ins));
        let Some((_, raw_ret)) = sig_opt else {
            self.ctx.sig_inputs_buf = raw_ins;
            for &a in args {
                let _ = self.infer_expr(a, Expectation::None);
            }
            return InferTyId::UNKNOWN;
        };
        let mut raw_params = InlineTyBuf::new();
        for &p in &raw_ins {
            raw_params.push(p);
        }
        self.ctx.sig_inputs_buf = raw_ins;

        let inst_ret = self.ctx.table.instantiate(raw_ret, subst.as_slice());
        self.pre_unify_call_return(inst_ret, expectation);

        // Phase 1: infer non-closure arguments first so `T` in `fn map<T, F: Fn(T) -> U>(x: T, f: F)`
        // (and reversed `(f, x)`) resolves before typing closure parameters.
        for (i, &arg_node) in args.iter().enumerate() {
            if self.st.kind(arg_node) == K::ClosureExpr {
                continue;
            }
            if let Some(&rp) = raw_params.as_slice().get(i) {
                let p_ty = self.ctx.table.instantiate(rp, subst.as_slice());
                let a_ty = self.infer_expr(arg_node, Expectation::HasType(p_ty));
                let _ = self.coerce_expr(a_ty, p_ty);
            } else {
                let _ = self.infer_expr(arg_node, Expectation::None);
            }
        }

        // Phase 2: infer closure arguments, propagating instantiated `Fn*` bounds when the formal param is `Param(gp)`.
        for (i, &arg_node) in args.iter().enumerate() {
            if self.st.kind(arg_node) != K::ClosureExpr {
                continue;
            }
            let Some(&rp) = raw_params.as_slice().get(i) else {
                let _ = self.infer_expr(arg_node, Expectation::None);
                continue;
            };
            if let InferTyKind::Param { idx: gp_idx, .. } = self.ctx.table.arena.kind(rp) {
                if let Some((b_ins, b_ret, is_async_b)) = self.instantiate_fn_bound_for_call(def, gp_idx, subst.as_slice()) {
                    let hint_fn = if is_async_b {
                        if let Some(tr_def) = self.db.lang_item(TyLangItem::AsyncFnOnce).or_else(|| self.db.lang_item(TyLangItem::AsyncFn)) {
                            let tup = self.ctx.table.arena.mk_tuple(b_ins.as_slice());
                            self.ctx.table.arena.mk_opaque(tr_def, &[tup, b_ret])
                        } else {
                            self.ctx.table.arena.mk_fn_ptr(b_ins.as_slice(), b_ret)
                        }
                    } else {
                        self.ctx.table.arena.mk_fn_ptr(b_ins.as_slice(), b_ret)
                    };
                    let clos_ty = self.infer_expr(arg_node, Expectation::HasType(hint_fn));
                    let res_clos = self.ctx.table.shallow_resolve(clos_ty);
                    if let InferTyKind::Closure { sig, .. } = self.ctx.table.arena.kind(res_clos) {
                        let cs = self.ctx.table.arena.slice(sig);
                        let mut c_ins = InlineTyBuf::new();
                        for &ci in &cs[..cs.len() - 1] {
                            c_ins.push(ci);
                        }
                        let mut c_ret = cs[cs.len() - 1];
                        if is_async_b {
                            c_ret = unwrap_await_ty(&mut self.ctx.table, self.db, c_ret);
                        }
                        for (ci, &bi) in c_ins.as_slice().iter().zip(b_ins.as_slice().iter()) {
                            let _ = self.ctx.table.equate(*ci, bi);
                        }
                        let _ = self.ctx.table.coerce(c_ret, b_ret);
                    }
                    let p_ty = self.ctx.table.instantiate(rp, subst.as_slice());
                    let _ = self.ctx.table.equate(clos_ty, p_ty);
                    continue;
                }
            }
            let p_ty = self.ctx.table.instantiate(rp, subst.as_slice());
            let a_ty = self.infer_expr(arg_node, Expectation::HasType(p_ty));
            let _ = self.coerce_expr(a_ty, p_ty);
        }
        self.ctx.table.shallow_resolve(inst_ret)
    }

    fn instantiate_fn_bound_for_call(&mut self, fn_def: GlobalDefId, param_idx: u16, subst: &[InferTyId]) -> Option<(InlineTyBuf, InferTyId, bool)> {
        let mut raw_b_ins = core::mem::take(&mut self.ctx.bound_inputs_buf);
        let b_opt = self
            .db
            .param_fn_bound(fn_def, param_idx, &mut self.ctx.table.arena, &mut raw_b_ins)
            .or_else(|| self.local_defs.param_fn_bound(fn_def, param_idx, self.st, self.file_res, self.db, &mut self.ctx.table, &mut raw_b_ins));
        let res = b_opt.map(|(li, raw_ret)| {
            let mut ins = InlineTyBuf::new();
            for &inp in &raw_b_ins {
                ins.push(self.ctx.table.instantiate(inp, subst));
            }
            let ret = self.ctx.table.instantiate(raw_ret, subst);
            (ins, ret, li.is_async_fn_trait())
        });
        self.ctx.bound_inputs_buf = raw_b_ins;
        res
    }

    fn pre_unify_call_return(&mut self, ret_ty: InferTyId, expectation: Expectation) {
        if let Some(exp_ret) = expectation.only_has_type(&mut self.ctx.table) {
            let snap = self.ctx.table.start_snapshot();
            if self.ctx.table.coerce(ret_ty, exp_ret).is_ok() {
                self.ctx.table.commit(snap);
            } else {
                self.ctx.table.rollback_to(snap);
            }
        }
    }

    fn infer_call_args_slice(&mut self, params: &[InferTyId], args: &[NodeId]) {
        for pass_closures in [false, true] {
            for (i, &arg_node) in args.iter().enumerate() {
                if (self.st.kind(arg_node) == K::ClosureExpr) != pass_closures {
                    continue;
                }
                if let Some(&p_ty) = params.get(i) {
                    let a_ty = self.infer_expr(arg_node, Expectation::HasType(p_ty));
                    let _ = self.coerce_expr(a_ty, p_ty);
                } else {
                    let _ = self.infer_expr(arg_node, Expectation::None);
                }
            }
        }
    }

    pub(super) fn infer_record_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let seed_ty = expectation.only_has_type(&mut self.ctx.table).unwrap_or_else(|| self.ctx.table.new_ty_var());
        let (owner_def, subst) = self.instantiate_pat_ctor(expr, seed_ty);
        let adt_ty = self.ctx.table.shallow_resolve(seed_ty);

        if let Some(fl) = self.st.child(expr, K::RecordExprFieldList) {
            for ch in self.st.children(fl) {
                if self.st.kind(ch) == K::RecordExprField {
                    let has_colon = self.st.own_tokens(ch).any(|(_, t)| t.kind == T::Colon);
                    let Some(val_expr) = self.st.children(ch).find(|&c| is_expr_node(self.st.kind(c))) else {
                        continue;
                    };
                    let fname = if has_colon {
                        self.st.own_tokens(ch).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent | T::Int)).map(|(_, t)| norm_ident(t.text(self.st.src)))
                    } else {
                        Some(norm_ident(self.st.text(val_expr).trim()))
                    };
                    let fty = match fname.filter(|_| owner_def.is_some()) {
                        Some(n) => self.lookup_instantiated_field_ty(owner_def, n, subst.as_slice()),
                        None => InferTyId::UNKNOWN,
                    };
                    let vty = self.infer_expr(val_expr, Expectation::from_option(Some(fty)));
                    if fty != InferTyId::UNKNOWN {
                        let _ = self.coerce_expr(vty, fty);
                    }
                } else if is_expr_node(self.st.kind(ch)) {
                    // Struct update base `..base`.
                    let bty = self.infer_expr(ch, Expectation::HasType(adt_ty));
                    let _ = self.coerce_expr(bty, adt_ty);
                }
            }
        }
        self.ctx.table.shallow_resolve(adt_ty)
    }

    pub(super) fn infer_closure_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let mut bound_ins = core::mem::take(&mut self.ctx.bound_inputs_buf);
        let exp_sig = extract_expected_callable_sig(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, self.fn_def, expectation, &mut bound_ins);
        let mut exp_params = InlineTyBuf::new();
        if exp_sig.is_some() {
            for &p in &bound_ins {
                exp_params.push(p);
            }
        }
        self.ctx.bound_inputs_buf = bound_ins;

        let is_async = self.st.own_tokens(expr).any(|(_, t)| t.kind == T::Async);
        let tenv = self.ty_env(true);
        let mut param_tys = InlineTyBuf::new();
        if let Some(pl) = self.st.child(expr, K::ParamList) {
            for (idx, param) in self.st.children(pl).filter(|&c| self.st.kind(c) == K::Param).enumerate() {
                let explicit_ty = self.st.children(param).find(|&c| self.st.kind(c).is_type()).map(|tn| tenv.lower_cst_ty(&mut self.ctx.table, tn));
                let exp_p = exp_params.as_slice().get(idx).copied();
                let pty = match (explicit_ty, exp_p) {
                    (Some(ann), Some(ep)) => {
                        let _ = self.ctx.table.equate(ann, ep);
                        ann
                    }
                    (Some(ann), None) => ann,
                    (None, Some(ep)) => ep,
                    (None, None) => self.ctx.table.new_ty_var(),
                };
                if let Some(pat) = self.st.children(param).find(|&c| self.st.kind(c).is_pat()) {
                    self.infer_pat(pat, pty, BindingMode::ByValue, None);
                }
                param_tys.push(pty);
            }
        }

        let explicit_ret =
            self.st.child(expr, K::RetType).and_then(|rt| self.st.children(rt).find(|&c| self.st.kind(c).is_type())).map(|tn| tenv.lower_cst_ty(&mut self.ctx.table, tn));

        let body_ret = match (explicit_ret, exp_sig) {
            (Some(ann), Some((er, exp_async))) => {
                if !is_async || exp_async {
                    let _ = self.ctx.table.equate(ann, er);
                }
                ann
            }
            (Some(ann), None) => ann,
            (None, Some((er, exp_async))) => {
                if is_async && !exp_async {
                    let uw = unwrap_await_ty(&mut self.ctx.table, self.db, er);
                    if uw != InferTyId::UNKNOWN {
                        uw
                    } else {
                        self.ctx.table.new_ty_var()
                    }
                } else {
                    er
                }
            }
            (None, None) => self.ctx.table.new_ty_var(),
        };

        self.ctx.ret_stack.push(body_ret);
        if let Some(body) = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))) {
            let bty = self.infer_expr(body, Expectation::HasType(body_ret));
            let _ = self.coerce_expr(bty, body_ret);
        }
        self.ctx.ret_stack.pop();

        let mut out_ty = self.ctx.table.shallow_resolve(body_ret);
        if is_async {
            if let Some(fut_def) = self.db.lang_item(TyLangItem::Future) {
                out_ty = self.ctx.table.arena.mk_opaque(fut_def, &[out_ty]);
                if let Some((er, false)) = exp_sig {
                    let _ = self.ctx.table.equate(out_ty, er);
                }
            }
        }
        let cid = self.ctx.closure_seq;
        self.ctx.closure_seq += 1;
        let raw_clos = self.ctx.table.arena.mk_closure(cid, param_tys.as_slice(), out_ty);
        let final_ty = match expectation.coercion_target(&mut self.ctx.table) {
            Some(exp) => self.coerce_expr(raw_clos, exp),
            None => raw_clos,
        };
        self.record_expr(expr, final_ty)
    }

    pub(super) fn infer_method_call_fallback(&mut self, expr: NodeId) -> InferTyId {
        if let Some(recv) = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))) {
            let recv_ty = self.infer_expr(recv, Expectation::None);
            if let Some(nn) = self.st.child(expr, K::Name) {
                if norm_ident(self.st.text(nn)) == "len" {
                    let r = self.ctx.table.shallow_resolve(recv_ty);
                    if matches!(self.ctx.table.arena.kind(r), InferTyKind::Slice(_) | InferTyKind::Array { .. }) || r == InferTyId::STR_REF || r == InferTyId::STR {
                        if let Some(al) = self.st.child(expr, K::ArgList) {
                            for ch in self.st.children(al).filter(|&c| is_expr_node(self.st.kind(c))) {
                                let _ = self.infer_expr(ch, Expectation::None);
                            }
                        }
                        return InferTyId::USIZE;
                    }
                }
            }
        }
        if let Some(al) = self.st.child(expr, K::ArgList) {
            for ch in self.st.children(al).filter(|&c| is_expr_node(self.st.kind(c))) {
                let _ = self.infer_expr(ch, Expectation::None);
            }
        }
        InferTyId::UNKNOWN
    }
}
