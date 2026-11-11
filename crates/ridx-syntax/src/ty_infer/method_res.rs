/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Checkbox `3.3` method resolution (`Runner::infer_method_call_expr`):
//! autoderef + `xform_self_ty` autoref order, inherent -> concrete trait -> blanket trait precedence,
//! method turbofish (`::<...>`), `Fn*` closure bound propagation, and `.method()` `dot_refs`.

use super::autoderef::{build_autoderef_steps_with_owner, impl_target_key_of_ty, MAX_AUTODEREF_STEPS};
use super::body_ctx::Runner;
use super::db::MethodCand;
use super::expectation::Expectation;
use super::infer_expr::is_expr_node;
use super::infer_macro::SubInferEnv;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::{CrateId, GlobalDefId, ImplTargetKey, ItemKind};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::NodeId;

pub(super) struct MatchedMethod {
    pub cand: MethodCand,
    pub subst: InlineTyBuf,
    pub raw_params: InlineTyBuf,
    pub inst_ret: InferTyId,
}

impl<'a, 's> Runner<'a, 's> {
    pub(super) fn infer_method_call_expr(&mut self, expr: NodeId, expectation: Expectation) -> InferTyId {
        let Some(recv) = self.st.children(expr).find(|&c| is_expr_node(self.st.kind(c))) else {
            return InferTyId::UNKNOWN;
        };
        let mut arg_nodes = [NodeId(0); 24];
        let mut arg_len = 0usize;
        if let Some(al) = self.st.child(expr, K::ArgList) {
            for ch in self.st.children(al).filter(|&c| is_expr_node(self.st.kind(c))) {
                if arg_len < arg_nodes.len() {
                    arg_nodes[arg_len] = ch;
                    arg_len += 1;
                } else {
                    let _ = self.infer_expr(ch, Expectation::None);
                }
            }
        }
        let args = &arg_nodes[..arg_len];
        let mut explicit_m_args = InlineTyBuf::new();
        if let Some(gal) = self.st.child(expr, K::GenericArgList) {
            self.collect_gal_turbofish(gal, &mut explicit_m_args);
        }

        let recv_ty = self.infer_expr(recv, Expectation::None);
        let Some(name_node) = self.st.child(expr, K::Name) else {
            self.infer_untyped_args(args);
            return InferTyId::UNKNOWN;
        };
        let method_name = norm_ident(self.st.text(name_node));
        let name_range = self.st.range(name_node);

        let mut steps = [InferTyId::UNKNOWN; MAX_AUTODEREF_STEPS];
        let owner = self.norm_owner();
        let step_len = build_autoderef_steps_with_owner(&mut self.ctx.table, self.db, self.local_defs, self.root_st, self.root_file_res, owner, recv_ty, &mut steps);
        if step_len == 1 && matches!(self.ctx.table.arena.kind(steps[0]), InferTyKind::Unknown | InferTyKind::Infer(_)) {
            self.infer_untyped_args(args);
            return InferTyId::UNKNOWN;
        }

        let Some(matched) = self.probe_method_steps(expr, method_name, &steps[..step_len], explicit_m_args.as_slice()) else {
            self.infer_untyped_args(args);
            if method_name == "len"
                && args.is_empty()
                && steps[..step_len].iter().any(|&t| matches!(self.ctx.table.arena.kind(t), InferTyKind::Slice(_) | InferTyKind::Array { .. }) || t == InferTyId::STR)
            {
                return InferTyId::USIZE;
            }
            return InferTyId::UNKNOWN;
        };

        if matched.cand.method_def.is_some() {
            self.record_dot_ref(name_range.start as u32, name_range.end as u32, matched.cand.method_def, ItemKind::AssocFn);
        }

        if !matches!(expectation, super::Expectation::None) {
            let pre_ret = self.norm(matched.inst_ret);
            self.pre_unify_call_return(pre_ret, expectation, matched.subst.as_slice());
        }
        self.check_call_args_with_bounds(matched.cand.bound_owner_def, &matched.raw_params.as_slice()[1..], matched.subst.as_slice(), args, None);
        let pre_norm_ret = self.norm(matched.inst_ret);
        if !self.call_has_live_infer(matched.raw_params.as_slice(), matched.subst.as_slice(), pre_norm_ret) {
            return pre_norm_ret;
        }
        let t_end = (1 + matched.cand.fixed_trait_args.len as usize).min(matched.subst.as_slice().len());
        if matched.cand.trait_def.is_some() && matched.cand.has_fixed_trait_subst && t_end > 1 {
            let tslice = &matched.subst.as_slice()[1..t_end];
            if self.ctx.deferred_trait_bounds.len() < 16
                && tslice.iter().any(|&a| {
                    let r = self.ctx.table.shallow_resolve(a);
                    matches!(self.ctx.table.arena.kind(r), InferTyKind::Infer(_))
                })
            {
                let ar = self.ctx.table.arena.intern_slice(tslice);
                self.ctx.deferred_trait_bounds.push((matched.cand.trait_def, matched.subst.as_slice()[0], ar));
            }
            let _ = self.solve_obligation(matched.cand.trait_def, matched.subst.as_slice()[0], tslice, false, 0);
        }
        self.solve_call_obligations(matched.cand.bound_owner_def, matched.cand.cst_fn_node, matched.subst.as_slice());
        self.norm(matched.inst_ret)
    }

    pub(super) fn probe_method_steps(&mut self, expr: NodeId, method_name: &str, steps: &[InferTyId], explicit_m_args: &[InferTyId]) -> Option<MatchedMethod> {
        let mut cands = core::mem::take(&mut self.ctx.method_cands_buf);
        let mut scope_traits = core::mem::take(&mut self.ctx.scope_traits_buf);
        cands.clear();
        scope_traits.clear();

        let inh_end = self.gather_inherent_cands(method_name, steps, &mut cands);
        self.gather_trait_and_bound_cands(expr, method_name, steps, &mut scope_traits, &mut cands);
        // Partition trait candidates so concrete/bound impls precede blanket `impl<T> Trait for T`.
        let mut write_concrete = inh_end;
        for i in inh_end..cands.len() {
            if !cands[i].is_blanket_impl {
                cands.swap(write_concrete, i);
                write_concrete += 1;
            }
        }
        let mut hit: Option<MatchedMethod> = None;
        for &step_ty in steps {
            if let Some(m) = self.probe_one_step(step_ty, &cands[..inh_end], &cands[inh_end..], explicit_m_args) {
                hit = Some(m);
                break;
            }
        }
        self.ctx.method_cands_buf = cands;
        self.ctx.scope_traits_buf = scope_traits;
        hit
    }

    pub(super) fn gather_inherent_cands(&mut self, method_name: &str, steps: &[InferTyId], cands: &mut Vec<MethodCand>) -> usize {
        let mut heads = [ImplTargetKey::Unknown; 16];
        let mut hlen = 0usize;
        let mut push_head = |h: ImplTargetKey| {
            if h != ImplTargetKey::Unknown && h != ImplTargetKey::Param && !heads[..hlen].contains(&h) && hlen < heads.len() {
                heads[hlen] = h;
                hlen += 1;
            }
        };
        for &st_ty in steps {
            push_head(impl_target_key_of_ty(&self.ctx.table.arena, st_ty));
            if matches!(self.ctx.table.arena.kind(st_ty), InferTyKind::Array { .. }) {
                push_head(ImplTargetKey::Slice);
            }
        }
        for &h in &heads[..hlen] {
            self.local_defs.inherent_method_cands(h, method_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, cands);
            let before = cands.len();
            self.db.inherent_method_cands(h, method_name, &mut self.ctx.table.arena, cands);
            super::local_defs::LocalCstDefs::dedup_inherent_cands(before, cands);
        }
        cands.len()
    }

    pub(super) fn gather_trait_and_bound_cands(&mut self, expr: NodeId, method_name: &str, steps: &[InferTyId], scope_traits: &mut Vec<GlobalDefId>, cands: &mut Vec<MethodCand>) {
        // 1. Direct receiver bounds (`dyn Trait`, `impl Trait`, and `T: BoundTrait` type parameters).
        for &st_ty in steps {
            match self.ctx.table.arena.kind(st_ty) {
                InferTyKind::Dyn { principal, args } | InferTyKind::Opaque { def: principal, args } if principal.is_some() => {
                    let mut tr_args = InlineTyBuf::new();
                    for idx in 0..args.len as usize {
                        tr_args.push(self.ctx.table.arena.slice(args)[idx]);
                    }
                    if let Some(c) =
                        self.local_defs.trait_decl_method_cand(principal, tr_args.as_slice(), st_ty, method_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table)
                    {
                        cands.push(c);
                    }
                }
                InferTyKind::Param { .. } | InferTyKind::Alias { .. } => {
                    if let InferTyKind::Param { idx, .. } = self.ctx.table.arena.kind(st_ty) {
                        self.push_param_bound_cands_with_args(st_ty, idx, method_name, cands);
                    }
                    let mut p_bounds = core::mem::take(&mut self.ctx.param_bounds_buf);
                    self.collect_ty_direct_bounds(st_ty, &mut p_bounds);
                    for &btr in p_bounds.iter() {
                        if let Some(c) = self.local_defs.trait_decl_method_cand(btr, &[], st_ty, method_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table) {
                            cands.push(c);
                        }
                    }
                    self.ctx.param_bounds_buf = p_bounds;
                }
                _ => {}
            }
        }

        // 2. Traits in lexical / prelude / enclosing-impl scope + transitive supertraits (cached per fn when no inner scopes).
        let use_fn_cache = !self.ctx.fn_has_inner_trait_scopes && self.sub.is_none();
        let mut fn_cached = core::mem::take(&mut self.ctx.fn_scope_traits_buf);
        let active_traits: &mut Vec<GlobalDefId> = if use_fn_cache {
            if !self.ctx.fn_scope_traits_ready {
                fn_cached.clear();
                self.root_file_res.collect_traits_in_scope_at(expr, &mut fn_cached);
                if !self.ctx.compl_exact_scope {
                    self.expand_supertraits(&mut fn_cached, 96);
                }
                self.ctx.fn_scope_traits_ready = true;
            }
            &mut fn_cached
        } else {
            match self.sub {
                None => self.root_file_res.collect_traits_in_scope_at(expr, scope_traits),
                Some(senv) => {
                    self.root_file_res.collect_traits_in_scope_at(senv.callsite_node(), scope_traits);
                    if matches!(senv, SubInferEnv::Expanded { .. }) {
                        self.file_res.collect_traits_in_scope_at(expr, &mut fn_cached);
                        for &t in &fn_cached {
                            if !scope_traits.contains(&t) {
                                scope_traits.push(t);
                            }
                        }
                    }
                }
            }
            if !self.ctx.compl_exact_scope {
                self.expand_supertraits(scope_traits, 96);
            }
            scope_traits
        };

        for &tdef in active_traits.iter() {
            if !self.ctx.compl_known_traits.is_empty()
                && self.ctx.compl_known_traits.binary_search_by_key(&(tdef.krate.0, tdef.index), |d| (d.krate.0, d.index)).is_ok()
                && !self.ctx.compl_name_traits.contains(&tdef)
            {
                continue;
            }
            let declares = self.local_defs.trait_decl_method_cand_direct(tdef, &[], InferTyId::UNKNOWN, method_name, self.root_st, &mut self.ctx.table).is_some()
                || self.db.trait_decl_method_cand(tdef, &[], InferTyId::UNKNOWN, method_name, &mut self.ctx.table.arena).is_some();
            if !declares {
                continue;
            }
            for &st_ty in steps {
                let head = impl_target_key_of_ty(&self.ctx.table.arena, st_ty);
                if head != ImplTargetKey::Param {
                    self.push_trait_impl_cands_for(head, tdef, method_name, st_ty, cands);
                }
                let ref_sh = self.ctx.table.arena.mk_ref(false, st_ty);
                self.push_trait_impl_cands_for(ImplTargetKey::Ref { is_mut: false }, tdef, method_name, ref_sh, cands);
                let ref_mut = self.ctx.table.arena.mk_ref(true, st_ty);
                self.push_trait_impl_cands_for(ImplTargetKey::Ref { is_mut: true }, tdef, method_name, ref_mut, cands);
                if let InferTyKind::Array { elem, .. } = self.ctx.table.arena.kind(st_ty) {
                    let sl = self.ctx.table.arena.mk_slice(elem);
                    self.push_trait_impl_cands_for(ImplTargetKey::Slice, tdef, method_name, sl, cands);
                    let sl_ref = self.ctx.table.arena.mk_ref(false, sl);
                    self.push_trait_impl_cands_for(ImplTargetKey::Ref { is_mut: false }, tdef, method_name, sl_ref, cands);
                }
                self.push_trait_impl_cands_for(ImplTargetKey::Param, tdef, method_name, st_ty, cands);
                if !matches!(self.ctx.table.arena.kind(st_ty), InferTyKind::Ref { .. }) {
                    self.push_trait_impl_cands_for(ImplTargetKey::Param, tdef, method_name, ref_sh, cands);
                }
            }
        }
        self.ctx.fn_scope_traits_buf = fn_cached;
    }

    fn push_trait_impl_cands_for(&mut self, head: ImplTargetKey, tdef: GlobalDefId, method_name: &str, probe_ty: InferTyId, cands: &mut Vec<MethodCand>) {
        let start = cands.len();
        self.local_defs.trait_impl_method_cands(head, tdef, method_name, probe_ty, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, cands);
        self.db.trait_impl_method_cands(head, tdef, method_name, probe_ty, &mut self.ctx.table.arena, &mut self.ctx.table.vars, cands);
        self.local_defs.collapse_overloaded_trait_cands(start, tdef, method_name, probe_ty, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, cands);
    }

    fn probe_one_step(&mut self, step_ty: InferTyId, inh_cands: &[MethodCand], tr_cands: &[MethodCand], explicit_m_args: &[InferTyId]) -> Option<MatchedMethod> {
        let mut rcvrs = [InferTyId::NONE; 7];
        let mut rlen = 0usize;
        rcvrs[rlen] = step_ty;
        rlen += 1;
        if let InferTyKind::Ref { is_mut: true, inner } = self.ctx.table.arena.kind(step_ty) {
            rcvrs[rlen] = self.ctx.table.arena.mk_ref(false, inner);
            rlen += 1;
        }
        rcvrs[rlen] = self.ctx.table.arena.mk_ref(false, step_ty);
        rlen += 1;
        rcvrs[rlen] = self.ctx.table.arena.mk_ref(true, step_ty);
        rlen += 1;
        if let InferTyKind::Array { elem, .. } = self.ctx.table.arena.kind(step_ty) {
            let sl = self.ctx.table.arena.mk_slice(elem);
            rcvrs[rlen] = sl;
            rlen += 1;
            rcvrs[rlen] = self.ctx.table.arena.mk_ref(false, sl);
            rlen += 1;
            rcvrs[rlen] = self.ctx.table.arena.mk_ref(true, sl);
            rlen += 1;
        }

        for &rcvr in &rcvrs[..rlen] {
            for cand in inh_cands {
                if let Some(m) = self.try_match_cand_rcvr(cand, rcvr, explicit_m_args) {
                    return Some(m);
                }
            }
            for cand in tr_cands {
                if let Some(m) = self.try_match_cand_rcvr(cand, rcvr, explicit_m_args) {
                    return Some(m);
                }
            }
        }
        None
    }

    fn try_match_cand_rcvr(&mut self, cand: &MethodCand, rcvr: InferTyId, explicit_m_args: &[InferTyId]) -> Option<MatchedMethod> {
        let mut raw_ins = core::mem::take(&mut self.ctx.sig_inputs_buf);
        let sig_ret = if cand.cst_fn_node != u32::MAX {
            self.local_defs.fn_sig(cand.method_def, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, &mut raw_ins).map(|(_, r)| r)
        } else {
            self.db
                .method_sig(cand, &mut self.ctx.table.arena, &mut raw_ins)
                .or_else(|| self.local_defs.fn_sig(cand.method_def, self.root_st, self.root_file_res, self.db, &mut self.ctx.table, &mut raw_ins).map(|(_, r)| r))
        };
        let Some(raw_ret) = sig_ret.filter(|_| !raw_ins.is_empty()) else {
            self.ctx.sig_inputs_buf = raw_ins;
            return None;
        };
        let mut raw_params = InlineTyBuf::new();
        for &p in &raw_ins {
            raw_params.push(p);
        }
        self.ctx.sig_inputs_buf = raw_ins;

        let snap = self.ctx.table.start_snapshot();
        let mut subst = InlineTyBuf::new();
        let mut imp_vars = InlineTyBuf::new();
        if cand.has_fixed_trait_subst {
            for _ in 0..cand.container_generics {
                imp_vars.push(self.ctx.table.new_ty_var());
            }
            if let Some((pi, clen)) = self.ctx.table.arena.extract_array_const_param(cand.fixed_self_ty, rcvr) {
                imp_vars.set(pi, self.ctx.table.arena.mk_array(InferTyId::NEVER, clen));
            }
            let s_ty = if cand.container_generics > 0 { self.ctx.table.instantiate(cand.fixed_self_ty, imp_vars.as_slice()) } else { cand.fixed_self_ty };
            subst.push(s_ty);
            for idx in 0..cand.fixed_trait_args.len as usize {
                let ta = self.ctx.table.arena.slice(cand.fixed_trait_args)[idx];
                let inst_ta = if ta == InferTyId::NONE {
                    self.ctx.table.new_ty_var()
                } else if cand.container_generics > 0 {
                    self.ctx.table.instantiate(ta, imp_vars.as_slice())
                } else {
                    ta
                };
                subst.push(inst_ta);
            }
        } else {
            for _ in 0..cand.container_generics {
                subst.push(self.ctx.table.new_ty_var());
            }
            if let Some((pi, clen)) = self.ctx.table.arena.extract_array_const_param(raw_params.as_slice()[0], rcvr) {
                subst.set(pi, self.ctx.table.arena.mk_array(InferTyId::NEVER, clen));
            }
        }
        for j in 0..cand.method_generics as usize {
            subst.push(explicit_m_args.get(j).copied().unwrap_or_else(|| self.ctx.table.new_ty_var()));
        }

        let xform_self = self.ctx.table.instantiate(raw_params.as_slice()[0], subst.as_slice());
        let norm_xform = self.norm(xform_self);
        if self.ctx.table.equate(norm_xform, rcvr).is_ok() {
            let cgc = (cand.container_generics as usize).min(subst.as_slice().len());
            let impl_subst = if cand.has_fixed_trait_subst { imp_vars.as_slice() } else { &subst.as_slice()[..cgc] };
            let blanket_ok = if !cand.is_blanket_impl {
                true
            } else if cand.impl_owner.krate == CrateId::LOCAL_OVERLAY {
                self.check_local_cst_impl_bounds(NodeId(cand.impl_owner.index), cand.container_generics, impl_subst)
            } else {
                self.db.check_blanket_method_bounds(cand, impl_subst, &mut self.ctx.table.arena, &mut self.ctx.table.vars)
            };
            if blanket_ok {
                self.ctx.table.commit(snap);
                let inst_ret = self.ctx.table.instantiate(raw_ret, subst.as_slice());
                return Some(MatchedMethod { cand: *cand, subst, raw_params, inst_ret });
            }
        }
        self.ctx.table.rollback_to(snap);
        None
    }
}
