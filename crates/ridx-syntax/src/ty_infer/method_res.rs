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

use super::autoderef::{build_autoderef_steps, impl_target_key_of_ty, normalize_ty, MAX_AUTODEREF_STEPS};
use super::body_ctx::{ResolvedDotRef, Runner};
use super::db::MethodCand;
use super::expectation::Expectation;
use super::infer_expr::is_expr_node;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use crate::name_res::{GlobalDefId, ImplTargetKey, ItemKind};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::NodeId;

struct MatchedMethod {
    cand: MethodCand,
    subst: InlineTyBuf,
    raw_params: InlineTyBuf,
    inst_ret: InferTyId,
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
        let step_len = build_autoderef_steps(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, recv_ty, &mut steps);
        if step_len == 1 && matches!(self.ctx.table.arena.kind(steps[0]), InferTyKind::Unknown | InferTyKind::Infer(_)) {
            self.infer_untyped_args(args);
            return InferTyId::UNKNOWN;
        }

        let Some(matched) = self.probe_method_steps(expr, method_name, &steps[..step_len], explicit_m_args.as_slice()) else {
            // Builtin slice/str `.len()` fallback when running without a stdlib `.ridx` index.
            self.infer_untyped_args(args);
            if method_name == "len" && args.is_empty() {
                for &st_ty in &steps[..step_len] {
                    if matches!(self.ctx.table.arena.kind(st_ty), InferTyKind::Slice(_) | InferTyKind::Array { .. }) || st_ty == InferTyId::STR {
                        return InferTyId::USIZE;
                    }
                }
            }
            return InferTyId::UNKNOWN;
        };

        if matched.cand.method_def.is_some() {
            self.out.dot_refs.push(ResolvedDotRef { start: name_range.start as u32, end: name_range.end as u32, def: matched.cand.method_def, kind: ItemKind::AssocFn });
        }

        let norm_ret = normalize_ty(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, matched.inst_ret);
        self.pre_unify_call_return(norm_ret, expectation);
        self.check_call_args_with_bounds(matched.cand.bound_owner_def, &matched.raw_params.as_slice()[1..], matched.subst.as_slice(), args);
        normalize_ty(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, norm_ret)
    }

    fn collect_gal_turbofish(&mut self, gal: NodeId, out: &mut InlineTyBuf) {
        let tenv = self.ty_env(true);
        for ga in self.st.children(gal) {
            if matches!(self.st.kind(ga), K::TypeArg | K::AssocTypeArg) {
                if let Some(tn) = self.st.children(ga).find(|&c| self.st.kind(c).is_type()) {
                    out.push(tenv.lower_cst_ty(&mut self.ctx.table, tn));
                }
            } else if self.st.kind(ga) == K::ConstArg {
                out.push(InferTyId::UNKNOWN);
            }
        }
    }

    fn infer_untyped_args(&mut self, args: &[NodeId]) {
        for &a in args {
            let _ = self.infer_expr(a, Expectation::None);
        }
    }

    fn probe_method_steps(&mut self, expr: NodeId, method_name: &str, steps: &[InferTyId], explicit_m_args: &[InferTyId]) -> Option<MatchedMethod> {
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

    fn gather_inherent_cands(&mut self, method_name: &str, steps: &[InferTyId], cands: &mut Vec<MethodCand>) -> usize {
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
            self.local_defs.inherent_method_cands(h, method_name, self.st, self.file_res, self.db, &mut self.ctx.table, cands);
            let before = cands.len();
            self.db.inherent_method_cands(h, method_name, &mut self.ctx.table.arena, cands);
            // Deduplicate if both dirty local CST and clean `.ridx` emitted the same `method_def`.
            let mut w = before;
            for r in before..cands.len() {
                let md = cands[r].method_def;
                if !cands[..before].iter().any(|c| c.method_def == md) {
                    cands[w] = cands[r];
                    w += 1;
                }
            }
            cands.truncate(w);
        }
        cands.len()
    }

    fn gather_trait_and_bound_cands(&mut self, expr: NodeId, method_name: &str, steps: &[InferTyId], scope_traits: &mut Vec<GlobalDefId>, cands: &mut Vec<MethodCand>) {
        // 1. Direct receiver bounds (`dyn Trait`, `impl Trait`, and `T: BoundTrait` type parameters).
        for &st_ty in steps {
            match self.ctx.table.arena.kind(st_ty) {
                InferTyKind::Dyn { principal, args } | InferTyKind::Opaque { def: principal, args } if principal.is_some() => {
                    let mut tr_args = InlineTyBuf::new();
                    for idx in 0..args.len as usize {
                        tr_args.push(self.ctx.table.arena.slice(args)[idx]);
                    }
                    if let Some(c) = self.local_defs.trait_decl_method_cand(principal, tr_args.as_slice(), st_ty, method_name, self.st, self.file_res, self.db, &mut self.ctx.table)
                    {
                        cands.push(c);
                    }
                }
                InferTyKind::Param { idx, def } => {
                    self.collect_param_bounds(idx, def, scope_traits);
                    for &btr in scope_traits.iter() {
                        if let Some(c) = self.local_defs.trait_decl_method_cand(btr, &[], st_ty, method_name, self.st, self.file_res, self.db, &mut self.ctx.table) {
                            cands.push(c);
                        }
                    }
                    scope_traits.clear();
                }
                _ => {}
            }
        }

        // 2. Traits in lexical / prelude / enclosing-impl scope + transitive supertraits.
        self.file_res.collect_traits_in_scope_at(expr, scope_traits);
        let mut cursor = 0usize;
        while cursor < scope_traits.len() && scope_traits.len() < 96 {
            let tdef = scope_traits[cursor];
            cursor += 1;
            self.local_defs.collect_supertraits_into(tdef, self.st, self.file_res, self.db, scope_traits);
        }

        for &tdef in scope_traits.iter() {
            // Fast O(1)-filtered check: skip traits that do not declare `method_name` directly!
            let declares = self.local_defs.trait_decl_method_cand_direct(tdef, &[], InferTyId::UNKNOWN, method_name, self.st, &mut self.ctx.table).is_some()
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
                self.push_trait_impl_cands_for(ImplTargetKey::Param, tdef, method_name, ref_sh, cands);
            }
        }
    }

    fn push_trait_impl_cands_for(&mut self, head: ImplTargetKey, tdef: GlobalDefId, method_name: &str, probe_ty: InferTyId, cands: &mut Vec<MethodCand>) {
        self.local_defs.trait_impl_method_cands(head, tdef, method_name, probe_ty, self.st, self.file_res, self.db, &mut self.ctx.table, cands);
        self.db.trait_impl_method_cands(head, tdef, method_name, probe_ty, &mut self.ctx.table.arena, &mut self.ctx.table.vars, cands);
    }

    fn collect_param_bounds(&self, idx: u16, pdef: GlobalDefId, out: &mut Vec<GlobalDefId>) {
        out.clear();
        for &(gid, tdef) in self.file_res.param_trait_bounds.iter() {
            let g = self.file_res.generic_data(gid);
            let same_param = (pdef.is_some() && g.def == pdef)
                || (self.impl_node == Some(g.owner_node) && g.index == idx)
                || (Some(self.fn_node) == Some(g.owner_node) && self.impl_gc + g.index == idx);
            if same_param && tdef.is_some() && !out.contains(&tdef) {
                out.push(tdef);
            }
        }
        if self.fn_def.is_some() {
            self.db.param_trait_bounds(self.fn_def, idx, out);
        }
        let mut cursor = 0usize;
        while cursor < out.len() && out.len() < 48 {
            let tdef = out[cursor];
            cursor += 1;
            self.local_defs.collect_supertraits_into(tdef, self.st, self.file_res, self.db, out);
        }
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
            self.local_defs.fn_sig(cand.method_def, self.st, self.file_res, self.db, &mut self.ctx.table, &mut raw_ins).map(|(_, r)| r)
        } else {
            self.db
                .method_sig(cand, &mut self.ctx.table.arena, &mut raw_ins)
                .or_else(|| self.local_defs.fn_sig(cand.method_def, self.st, self.file_res, self.db, &mut self.ctx.table, &mut raw_ins).map(|(_, r)| r))
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
        if cand.has_fixed_trait_subst {
            let mut imp_vars = InlineTyBuf::new();
            for _ in 0..cand.container_generics {
                imp_vars.push(self.ctx.table.new_ty_var());
            }
            let s_ty = if cand.container_generics > 0 { self.ctx.table.instantiate(cand.fixed_self_ty, imp_vars.as_slice()) } else { cand.fixed_self_ty };
            subst.push(s_ty);
            for idx in 0..cand.fixed_trait_args.len as usize {
                let ta = self.ctx.table.arena.slice(cand.fixed_trait_args)[idx];
                let inst_ta = if cand.container_generics > 0 { self.ctx.table.instantiate(ta, imp_vars.as_slice()) } else { ta };
                subst.push(inst_ta);
            }
        } else {
            for _ in 0..cand.container_generics {
                subst.push(self.ctx.table.new_ty_var());
            }
        }
        for j in 0..cand.method_generics as usize {
            subst.push(explicit_m_args.get(j).copied().unwrap_or_else(|| self.ctx.table.new_ty_var()));
        }

        let xform_self = self.ctx.table.instantiate(raw_params.as_slice()[0], subst.as_slice());
        let norm_xform = normalize_ty(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, xform_self);
        let cgc = (cand.container_generics as usize).min(subst.as_slice().len());
        if self.ctx.table.equate(norm_xform, rcvr).is_ok()
            && (!cand.is_blanket_impl || self.db.check_blanket_method_bounds(cand.method_def, &subst.as_slice()[..cgc], &mut self.ctx.table.arena, &mut self.ctx.table.vars))
        {
            self.ctx.table.commit(snap);
            let inst_ret = self.ctx.table.instantiate(raw_ret, subst.as_slice());
            return Some(MatchedMethod { cand: *cand, subst, raw_params, inst_ret });
        }
        self.ctx.table.rollback_to(snap);
        None
    }
}
