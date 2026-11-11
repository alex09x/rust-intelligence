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
use super::trait_solve::SolveEnv;
use super::trait_solve_ufcs::param_has_active_bound;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::{CrateId, GlobalDefId, ItemKind, ResTarget};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

impl<'a, 's> Runner<'a, 's> {
    pub(super) fn infer_record_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let seed_ty = expectation.only_has_type(&mut self.ctx.table).map(|t| self.norm(t)).unwrap_or_else(|| self.ctx.table.new_ty_var());
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
                param_tys.push(self.ctx.table.deep_resolve(pty));
            }
        }
        if !self.ctx.deferred_proj_eqs.is_empty() {
            self.flush_deferred_proj_eqs();
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
            (None, Some((er, exp_async))) if is_async && !exp_async => {
                let uw = unwrap_await_ty(&mut self.ctx.table, self.db, er);
                if uw != InferTyId::UNKNOWN {
                    uw
                } else {
                    self.ctx.table.new_ty_var()
                }
            }
            (None, Some((er, _))) => er,
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
        let final_ty = expectation.coercion_target(&mut self.ctx.table).map_or(raw_clos, |exp| self.coerce_expr(raw_clos, exp));
        self.record_expr(expr, final_ty)
    }

    pub(super) fn flush_deferred_proj_eqs(&mut self) {
        for idx in (0..self.ctx.deferred_proj_eqs.len()).rev() {
            let (alias_ty, rhs_ty) = self.ctx.deferred_proj_eqs[idx];
            let proj = self.norm(alias_ty);
            if !matches!(self.ctx.table.arena.kind(proj), InferTyKind::Alias { .. } | InferTyKind::Unknown) {
                let norm_rhs = self.norm(rhs_ty);
                let _ = self.ctx.table.coerce(proj, norm_rhs);
                self.ctx.deferred_proj_eqs.swap_remove(idx);
            }
        }
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
        if matches!(target, ResTarget::Def { kind: ItemKind::AssocTy, .. }) {
            let raw_ty = self.ty_env(true).lower_cst_path_ty(&mut self.ctx.table, p, false, false, 0);
            let norm_ty = self.norm(raw_ty);
            let _ = self.ctx.table.equate(expected_adt_ty, norm_ty);
            let res_norm = self.ctx.table.shallow_resolve(norm_ty);
            if let InferTyKind::Adt { def, args } = self.ctx.table.arena.kind(res_norm) {
                for &a in self.ctx.table.arena.slice(args) {
                    subst.push(a);
                }
                return (def, subst);
            }
            return (GlobalDefId::NONE, subst);
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

    pub(super) fn unify_inherent_fn_with_qself(&mut self, fn_def: GlobalDefId, subst_r: super::ty::TySliceRange, qself_ty: InferTyId) {
        let InferTyKind::Adt { def: q_def, args: q_args } = self.ctx.table.arena.kind(qself_ty) else { return };
        let mut subst = InlineTyBuf::new();
        for &a in self.ctx.table.arena.slice(subst_r) {
            subst.push(a);
        }
        let mut raw_ins = core::mem::take(&mut self.ctx.sig_inputs_buf);
        let sig_opt = self
            .db
            .fn_sig(fn_def, &mut self.ctx.table.arena, &mut raw_ins)
            .or_else(|| self.local_defs.fn_sig(fn_def, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, &mut raw_ins));
        let first_in = raw_ins.first().copied();
        self.ctx.sig_inputs_buf = raw_ins;
        let Some((_, raw_ret)) = sig_opt else { return };
        let inst_ret = self.ctx.table.instantiate(raw_ret, subst.as_slice());
        match self.ctx.table.arena.kind(inst_ret) {
            InferTyKind::Adt { def, .. } if def == q_def => {
                let _ = self.ctx.table.equate(inst_ret, qself_ty);
                return;
            }
            InferTyKind::Adt { args: rargs, .. } if rargs.len >= 1 => {
                let r0 = self.ctx.table.arena.slice(rargs)[0];
                if matches!(self.ctx.table.arena.kind(r0), InferTyKind::Adt { def, .. } if def == q_def) {
                    let _ = self.ctx.table.equate(r0, qself_ty);
                    return;
                }
            }
            _ => {}
        }
        if let Some(in0) = first_in {
            let mut inst_in0 = self.ctx.table.instantiate(in0, subst.as_slice());
            if let InferTyKind::Ref { inner, .. } = self.ctx.table.arena.kind(inst_in0) {
                inst_in0 = inner;
            }
            if matches!(self.ctx.table.arena.kind(inst_in0), InferTyKind::Adt { def, .. } if def == q_def) {
                let _ = self.ctx.table.equate(inst_in0, qself_ty);
                return;
            }
        }
        if (q_args.len as usize) <= subst.as_slice().len() {
            let mut qb = InlineTyBuf::new();
            for &qa in self.ctx.table.arena.slice(q_args) {
                qb.push(qa);
            }
            for (&sv, &qa) in subst.as_slice().iter().zip(qb.as_slice()) {
                let _ = self.ctx.table.equate(sv, qa);
            }
        }
    }

    /// The index-side bound check accepts any type parameter for any trait. Where an impl bound lands on a type
    /// parameter of the code being completed, that parameter must carry the bound itself (directly or through a supertrait).
    fn impl_bounds_hold_on_params(&mut self, cand: &MethodCand, impl_subst: &[InferTyId]) -> bool {
        let mut obls = core::mem::take(&mut self.ctx.where_obls_buf);
        obls.clear();
        self.db.impl_trait_bounds(cand, &mut self.ctx.table.arena, &mut obls);
        let mut ok = true;
        for obl in obls.iter().copied() {
            let WhereObligation::TraitBound { lhs_ty, trait_def, .. } = obl else { continue };
            let inst = self.ctx.table.instantiate(lhs_ty, impl_subst);
            let mut lhs = self.ctx.table.shallow_resolve(inst);
            while let InferTyKind::Ref { inner, .. } = self.ctx.table.arena.kind(lhs) {
                lhs = self.ctx.table.shallow_resolve(inner);
            }
            if let InferTyKind::Param { idx, def } = self.ctx.table.arena.kind(lhs) {
                if !self.is_own_generic_param(idx, def) {
                    continue;
                }
                let mut bounds = core::mem::take(&mut self.ctx.param_bounds_buf);
                self.collect_ty_direct_bounds(lhs, &mut bounds);
                let declared = bounds.contains(&trait_def);
                self.ctx.param_bounds_buf = bounds;
                let senv = SolveEnv { local_defs: self.local_defs, st: self.root_st, file_res: self.root_file_res, db: self.db, owner: self.norm_owner() };
                if !declared && !param_has_active_bound(senv, &mut self.ctx.table, idx, def, trait_def) {
                    ok = false;
                    break;
                }
            }
        }
        self.ctx.where_obls_buf = obls;
        ok
    }

    /// Whether `Param { idx, def }` is a generic parameter declared by the code being inferred (not one left over
    /// from an indexed type's own generics, such as an unsubstituted default).
    fn is_own_generic_param(&self, idx: u16, def: GlobalDefId) -> bool {
        let owner = self.norm_owner();
        let is_trait_self = idx == 0 && !def.is_some() && self.root_impl_node.is_some_and(|n| self.root_st.kind(n) == K::Trait);
        is_trait_self
            || self.root_file_res.generics.iter().any(|g| {
                (def.is_some() && g.def == def)
                    || (owner.impl_node == Some(g.owner_node) && g.index == idx)
                    || (owner.fn_node == Some(g.owner_node) && owner.impl_gc + g.index == idx)
            })
    }

    pub(super) fn cand_matches_completion_rcvr(&mut self, cand: &MethodCand, rcvr: InferTyId) -> bool {
        let mut raw_ins = core::mem::take(&mut self.ctx.sig_inputs_buf);
        let has_sig = if cand.cst_fn_node != u32::MAX {
            self.local_defs.fn_sig(cand.method_def, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, &mut raw_ins).is_some()
        } else {
            self.db.method_sig(cand, &mut self.ctx.table.arena, &mut raw_ins).is_some()
                || self.local_defs.fn_sig(cand.method_def, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, &mut raw_ins).is_some()
        };
        let Some(&self_param_ty) = raw_ins.first().filter(|_| has_sig) else {
            self.ctx.sig_inputs_buf = raw_ins;
            return false;
        };
        self.ctx.sig_inputs_buf = raw_ins;
        let snap = self.ctx.table.start_snapshot();
        let (mut subst, mut imp_vars) = (InlineTyBuf::new(), InlineTyBuf::new());
        if cand.has_fixed_trait_subst {
            for _ in 0..cand.container_generics {
                imp_vars.push(self.ctx.table.new_ty_var());
            }
            subst.push(if cand.container_generics > 0 { self.ctx.table.instantiate(cand.fixed_self_ty, imp_vars.as_slice()) } else { cand.fixed_self_ty });
            for idx in 0..cand.fixed_trait_args.len as usize {
                let ta = self.ctx.table.arena.slice(cand.fixed_trait_args)[idx];
                subst.push(if ta == InferTyId::NONE {
                    self.ctx.table.new_ty_var()
                } else if cand.container_generics > 0 {
                    self.ctx.table.instantiate(ta, imp_vars.as_slice())
                } else {
                    ta
                });
            }
        } else {
            for _ in 0..cand.container_generics {
                subst.push(self.ctx.table.new_ty_var());
            }
        }
        for _ in 0..cand.method_generics {
            subst.push(self.ctx.table.new_ty_var());
        }
        let xf = self.ctx.table.instantiate(self_param_ty, subst.as_slice());
        let norm_xform = self.norm(xf);
        let mut ok = self.ctx.table.equate(norm_xform, rcvr).is_ok();
        if ok {
            let cgc = (cand.container_generics as usize).min(subst.as_slice().len());
            let impl_subst = if cand.has_fixed_trait_subst { imp_vars.as_slice() } else { &subst.as_slice()[..cgc] };
            ok = if cand.impl_owner.krate == CrateId::LOCAL_OVERLAY {
                self.check_local_cst_impl_bounds(NodeId(cand.impl_owner.index), cand.container_generics, impl_subst)
            } else {
                self.db.check_blanket_method_bounds(cand, impl_subst, &mut self.ctx.table.arena, &mut self.ctx.table.vars) && self.impl_bounds_hold_on_params(cand, impl_subst)
            };
        }
        self.ctx.table.rollback_to(snap);
        ok
    }
}
