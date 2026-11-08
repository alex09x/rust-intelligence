/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Call (`CallExpr`), function/constructor signature instantiation, and `Fn*` bound argument
//! checking for [`super::body_ctx::Runner`].

use super::body_ctx::Runner;
use super::db::TyLangItem;
use super::expectation::Expectation;
use super::infer_expr::is_expr_node;
use super::lang_ops::{extract_expected_callable_sig, unwrap_await_ty};
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::GlobalDefId;
use crate::syntax_kind::SyntaxKind as K;
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

    fn resolve_fn_or_ctor_sig(&mut self, def: GlobalDefId, raw_ins: &mut Vec<InferTyId>) -> Option<InferTyId> {
        if let Some((_, ret)) =
            self.db.fn_sig(def, &mut self.ctx.table.arena, raw_ins).or_else(|| self.local_defs.fn_sig(def, self.st, self.file_res, self.db, &mut self.ctx.table, raw_ins))
        {
            return Some(ret);
        }
        let has_fields = self.db.ctor_tuple_fields(def, &mut self.ctx.table.arena, raw_ins).is_some()
            || self.local_defs.ctor_tuple_fields(def, self.st, self.file_res, self.db, &mut self.ctx.table, raw_ins).is_some();
        if !has_fields {
            return None;
        }
        self.db.def_ty(def, &mut self.ctx.table.arena).or_else(|| self.local_defs.def_ty(def, self.st, self.file_res, self.db, &mut self.ctx.table)).map(|(_, _, ty)| ty)
    }

    fn infer_fn_def_call(&mut self, def: GlobalDefId, subst_range: super::ty::TySliceRange, args: &[NodeId], expectation: Expectation) -> InferTyId {
        let mut subst = InlineTyBuf::new();
        for &a in self.ctx.table.arena.slice(subst_range) {
            subst.push(a);
        }
        let mut raw_ins = core::mem::take(&mut self.ctx.sig_inputs_buf);
        let Some(raw_ret) = self.resolve_fn_or_ctor_sig(def, &mut raw_ins) else {
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
        if !matches!(expectation, Expectation::None) {
            let pre_ret = self.norm(inst_ret);
            self.pre_unify_call_return(pre_ret, expectation);
        }
        self.check_call_args_with_bounds(def, raw_params.as_slice(), subst.as_slice(), args);
        if let Some(meta) = self.db.trait_of_assoc_fn(def).or_else(|| self.local_defs.trait_of_assoc_fn(def, self.st, self.file_res, self.db)) {
            let t_end = 1 + meta.trait_generics as usize;
            if subst.as_slice().len() >= t_end {
                let _ = self.solve_obligation(meta.trait_def, subst.as_slice()[0], &subst.as_slice()[1..t_end], false, 0);
            }
        }
        let cst_fn = self.local_defs.fn_node_for_def(def);
        self.solve_call_obligations(def, cst_fn, subst.as_slice());
        self.norm(inst_ret)
    }

    pub(super) fn check_call_args_with_bounds(&mut self, bound_owner_def: GlobalDefId, raw_params: &[InferTyId], subst: &[InferTyId], args: &[NodeId]) {
        for (i, &arg_node) in args.iter().enumerate() {
            if self.st.kind(arg_node) == K::ClosureExpr {
                continue;
            }
            if let Some(&rp) = raw_params.get(i) {
                let raw_p = self.ctx.table.instantiate(rp, subst);
                let p_ty = self.norm(raw_p);
                let a_ty = self.infer_expr(arg_node, Expectation::HasType(p_ty));
                let _ = self.coerce_expr(a_ty, p_ty);
                if let InferTyKind::Param { idx: gp_idx, .. } = self.ctx.table.arena.kind(rp) {
                    self.unify_fn_item_arg_with_bound(bound_owner_def, gp_idx, subst, a_ty);
                }
            } else {
                let _ = self.infer_expr(arg_node, Expectation::None);
            }
        }

        for (i, &arg_node) in args.iter().enumerate() {
            if self.st.kind(arg_node) != K::ClosureExpr {
                continue;
            }
            let Some(&rp) = raw_params.get(i) else {
                let _ = self.infer_expr(arg_node, Expectation::None);
                continue;
            };
            if let InferTyKind::Param { idx: gp_idx, .. } = self.ctx.table.arena.kind(rp) {
                if let Some((b_ins, b_ret, is_async_b)) = self.instantiate_fn_bound_for_call(bound_owner_def, gp_idx, subst) {
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
                    let p_ty = self.ctx.table.instantiate(rp, subst);
                    let _ = self.ctx.table.equate(clos_ty, p_ty);
                    continue;
                }
            }
            let raw_p = self.ctx.table.instantiate(rp, subst);
            let p_ty = self.norm(raw_p);
            let a_ty = self.infer_expr(arg_node, Expectation::HasType(p_ty));
            let _ = self.coerce_expr(a_ty, p_ty);
        }
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
                let inst_i = self.ctx.table.instantiate(inp, subst);
                ins.push(self.norm(inst_i));
            }
            let inst_r = self.ctx.table.instantiate(raw_ret, subst);
            let ret = self.norm(inst_r);
            (ins, ret, li.is_async_fn_trait())
        });
        self.ctx.bound_inputs_buf = raw_b_ins;
        res
    }

    fn unify_fn_item_arg_with_bound(&mut self, bound_owner_def: GlobalDefId, gp_idx: u16, subst: &[InferTyId], arg_ty: InferTyId) {
        let res_arg = self.ctx.table.shallow_resolve(arg_ty);
        match self.ctx.table.arena.kind(res_arg) {
            InferTyKind::FnDef { def, args } => {
                let Some((b_ins, b_ret, _)) = self.instantiate_fn_bound_for_call(bound_owner_def, gp_idx, subst) else {
                    return;
                };
                let mut f_subst = InlineTyBuf::new();
                for &x in self.ctx.table.arena.slice(args) {
                    f_subst.push(x);
                }
                let mut raw_ins = core::mem::take(&mut self.ctx.sig_inputs_buf);
                if let Some(raw_ret) = self.resolve_fn_or_ctor_sig(def, &mut raw_ins) {
                    let mut inst_ins = InlineTyBuf::new();
                    for &ri in &raw_ins {
                        let ii = self.ctx.table.instantiate(ri, f_subst.as_slice());
                        inst_ins.push(self.norm(ii));
                    }
                    self.ctx.sig_inputs_buf = raw_ins;
                    let ir = self.ctx.table.instantiate(raw_ret, f_subst.as_slice());
                    let norm_r = self.norm(ir);
                    for (&fi, &bi) in inst_ins.as_slice().iter().zip(b_ins.as_slice().iter()) {
                        let _ = self.ctx.table.equate(fi, bi);
                    }
                    let _ = self.ctx.table.coerce(norm_r, b_ret);
                } else {
                    self.ctx.sig_inputs_buf = raw_ins;
                }
            }
            InferTyKind::FnPtr { sig } if sig.len >= 1 => {
                let Some((b_ins, b_ret, _)) = self.instantiate_fn_bound_for_call(bound_owner_def, gp_idx, subst) else {
                    return;
                };
                let mut f_ins = InlineTyBuf::new();
                let sl = self.ctx.table.arena.slice(sig);
                for &x in &sl[..sl.len() - 1] {
                    f_ins.push(x);
                }
                let f_ret = sl[sl.len() - 1];
                for (&fi, &bi) in f_ins.as_slice().iter().zip(b_ins.as_slice().iter()) {
                    let _ = self.ctx.table.equate(fi, bi);
                }
                let _ = self.ctx.table.coerce(f_ret, b_ret);
            }
            _ => {}
        }
    }

    pub(super) fn pre_unify_call_return(&mut self, ret_ty: InferTyId, expectation: Expectation) {
        if let Some(raw_exp) = expectation.only_has_type(&mut self.ctx.table) {
            let exp_ret = self.norm(raw_exp);
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
}
