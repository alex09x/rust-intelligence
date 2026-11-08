/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Struct literal (`RecordExpr`) and closure (`ClosureExpr`) inference for [`super::body_ctx::Runner`].

use super::body_ctx::Runner;
use super::db::TyLangItem;
use super::expectation::Expectation;
use super::infer_expr::is_expr_node;
use super::infer_pat::BindingMode;
use super::lang_ops::{extract_expected_callable_sig, unwrap_await_ty};
use super::ty::InferTyId;
use super::uf_table::InlineTyBuf;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

impl<'a, 's> Runner<'a, 's> {
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
}
