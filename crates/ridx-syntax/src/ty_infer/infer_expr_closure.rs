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
use super::db::{MethodCand, TyLangItem, WhereObligation};
use super::expectation::Expectation;
use super::infer_expr::is_expr_node;
use super::infer_pat::BindingMode;
use super::lang_ops::{extract_expected_callable_sig, unwrap_await_ty};
use super::lower_ty::effective_target_def;
use super::trait_solve_cst::collect_cst_fn_obligations;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::{GlobalDefId, ItemKind, ResTarget};
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
        let exp_sig = extract_expected_callable_sig(&mut self.ctx.table, self.db, self.local_defs, self.root_st, self.root_file_res, self.fn_def, expectation, &mut bound_ins);
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
        let (ds, de) = self.closure_def_span(expr);
        let raw_clos = self.ctx.table.arena.mk_closure(ds, de, param_tys.as_slice(), out_ty);
        let final_ty = match expectation.coercion_target(&mut self.ctx.table) {
            Some(exp) => self.coerce_expr(raw_clos, exp),
            None => raw_clos,
        };
        self.record_expr(expr, final_ty)
    }

    pub(super) fn instantiate_pat_ctor(&mut self, node_with_path: NodeId, expected_adt_ty: InferTyId) -> (GlobalDefId, InlineTyBuf) {
        let mut subst = InlineTyBuf::new();
        let Some(p) = self.st.child(node_with_path, K::Path) else {
            return (GlobalDefId::NONE, subst);
        };
        let rref_target = self.file_res.path_tail_ref_in_st(self.st, p).map(|r| r.target).or_else(|| {
            let _ = self.sub?;
            let tail = self.st.children(p).filter(|&c| self.st.kind(c) == K::PathSegment).last()?;
            let (ts, te) = self.st.own_tokens(tail).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent | T::SelfType)).map_or_else(
                || {
                    let sr = self.st.range(tail);
                    (sr.start as u32, sr.end as u32)
                },
                |(_, t)| (t.start, t.end),
            );
            let (os, oe) = self.map_span_to_orig(ts, te)?;
            self.root_file_res.refs_at_span(os, oe).iter().find(|r| !matches!(r.target, ResTarget::Local(_))).map(|r| r.target)
        });
        let Some(target) = rref_target else {
            return (GlobalDefId::NONE, subst);
        };
        if matches!(target, ResTarget::SelfTy { .. }) {
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
        let raw_def = effective_target_def(target);
        if !raw_def.is_some() {
            return (GlobalDefId::NONE, subst);
        }
        let def_info =
            self.db.def_ty(raw_def, &mut self.ctx.table.arena).or_else(|| self.local_defs.def_ty(raw_def, self.root_st, self.root_file_res, self.db, &mut self.ctx.table));
        let Some((gc, _, tpl_ty)) = def_info else {
            return (raw_def, subst);
        };
        for i in 0..gc as usize {
            subst.push(explicit.as_slice().get(i).copied().unwrap_or_else(|| self.ctx.table.new_ty_var()));
        }
        let inst_adt = self.ctx.table.instantiate(tpl_ty, subst.as_slice());
        let _ = self.ctx.table.equate(expected_adt_ty, inst_adt);
        if let ResTarget::Def { kind: ItemKind::TypeAlias, .. } = target {
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

    pub(super) fn check_completion_method_obligations(&mut self, cand: &MethodCand, subst: &[InferTyId]) -> bool {
        let mut obls = core::mem::take(&mut self.ctx.where_obls_buf);
        obls.clear();
        if cand.bound_owner_def.is_some() {
            self.db.owner_where_obligations(cand.bound_owner_def, &mut self.ctx.table.arena, &mut obls);
        }
        let indexed_owner = self.local_defs.is_clean_indexed() && cand.bound_owner_def.is_some() && !cand.bound_owner_def.is_overlay();
        if obls.is_empty() && cand.cst_fn_node != u32::MAX && !indexed_owner {
            collect_cst_fn_obligations(self.local_defs, crate::tree::NodeId(cand.cst_fn_node), self.root_st, self.root_file_res, self.db, &mut self.ctx.table, &mut obls);
        }
        let mut ok = true;
        for obl in obls.iter().copied() {
            if let WhereObligation::ProjectionEq { lhs_ty, trait_def, assoc_def, trait_args, rhs_ty } = obl {
                let inst_lhs = self.ctx.table.instantiate(lhs_ty, subst);
                let mut inst_args = InlineTyBuf::new();
                for idx in 0..trait_args.len as usize {
                    inst_args.push(self.ctx.table.instantiate(self.ctx.table.arena.slice(trait_args)[idx], subst));
                }
                let alias_ty = self.ctx.table.arena.mk_alias(trait_def, assoc_def, inst_lhs, inst_args.as_slice());
                let proj = self.norm(alias_ty);
                if !matches!(self.ctx.table.arena.kind(proj), InferTyKind::Alias { .. } | InferTyKind::Unknown) {
                    let inst_rhs = self.ctx.table.instantiate(rhs_ty, subst);
                    let norm_rhs = self.norm(inst_rhs);
                    if self.ctx.table.equate(proj, norm_rhs).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if ok {
            for obl in obls.iter().copied() {
                if let WhereObligation::TraitBound { lhs_ty, trait_def, trait_args } = obl {
                    let inst_lhs = self.ctx.table.instantiate(lhs_ty, subst);
                    let norm_lhs = self.norm(inst_lhs);
                    let mut inst_args = InlineTyBuf::new();
                    for idx in 0..trait_args.len as usize {
                        let ia = self.ctx.table.instantiate(self.ctx.table.arena.slice(trait_args)[idx], subst);
                        inst_args.push(self.norm(ia));
                    }
                    if !self.solve_obligation(trait_def, norm_lhs, inst_args.as_slice(), true, 0) {
                        ok = false;
                        break;
                    }
                }
            }
        }
        self.ctx.where_obls_buf = obls;
        ok
    }
}
