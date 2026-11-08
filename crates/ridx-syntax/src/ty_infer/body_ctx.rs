/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-function body inference context ([`BodyInferCtx`]) and queryable output table
//! ([`FnInferResult`]), engineered for `0` steady-state heap allocations across functions.

use super::db::TyDefDb;
use super::expectation::{BranchCoercer, Expectation};
use super::infer_pat::BindingMode;
use super::local_defs::{lower_self_param_ty, LocalCstDefs};
use super::lower_ty::LowerTyEnv;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{FileNameRes, GlobalDefId, LocalBindId};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

/// Finalized local type inference output for a single `K::Fn` item.
#[derive(Clone, Debug)]
pub struct FnInferResult {
    pub fn_node: NodeId,
    pub ret_ty: InferTyId,
    pub body_ty: InferTyId,
    pub expr_tys: Vec<(NodeId, InferTyId)>,
    pub pat_tys: Vec<(NodeId, InferTyId)>,
    pub local_tys: Vec<(LocalBindId, InferTyId)>,
}

impl Default for FnInferResult {
    fn default() -> Self {
        Self::new()
    }
}

impl FnInferResult {
    pub fn new() -> Self {
        Self { fn_node: NodeId(0), ret_ty: InferTyId::UNKNOWN, body_ty: InferTyId::UNKNOWN, expr_tys: Vec::new(), pat_tys: Vec::new(), local_tys: Vec::new() }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.fn_node = NodeId(0);
        self.ret_ty = InferTyId::UNKNOWN;
        self.body_ty = InferTyId::UNKNOWN;
        self.expr_tys.clear();
        self.pat_tys.clear();
        self.local_tys.clear();
    }

    #[inline]
    pub fn expr_ty(&self, node: NodeId) -> Option<InferTyId> {
        self.expr_tys.binary_search_by_key(&node, |&(n, _)| n).ok().map(|i| self.expr_tys[i].1)
    }

    #[inline]
    pub fn pat_ty(&self, node: NodeId) -> Option<InferTyId> {
        self.pat_tys.binary_search_by_key(&node, |&(n, _)| n).ok().map(|i| self.pat_tys[i].1)
    }

    #[inline]
    pub fn local_ty(&self, id: LocalBindId) -> Option<InferTyId> {
        self.local_tys.binary_search_by_key(&id, |&(l, _)| l).ok().map(|i| self.local_tys[i].1)
    }
}

#[derive(Debug)]
pub(super) struct BreakTarget {
    pub label_start: u32,
    pub label_end: u32,
    pub coercer: BranchCoercer,
    pub break_count: u32,
    pub allow_unlabeled: bool,
    pub allow_value: bool,
}

/// Reusable function-body type inference engine (`0` allocations after initial warmup).
#[derive(Debug)]
pub struct BodyInferCtx {
    pub table: InferenceTable,
    pub(super) local_slot_base: u32,
    pub(super) local_slots: Vec<InferTyId>,
    pub(super) break_stack: Vec<BreakTarget>,
    pub(super) ret_stack: Vec<InferTyId>,
    pub(super) or_pat_binds: Vec<(u32, u32, LocalBindId)>,
    pub(super) sig_inputs_buf: Vec<InferTyId>,
    pub(super) bound_inputs_buf: Vec<InferTyId>,
    pub(super) exp_elems_buf: Vec<Expectation>,
    pub(super) closure_seq: u32,
}

impl Default for BodyInferCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl BodyInferCtx {
    pub fn new() -> Self {
        Self {
            table: InferenceTable::with_capacity(256, 128),
            local_slot_base: 0,
            local_slots: Vec::with_capacity(64),
            break_stack: Vec::with_capacity(8),
            ret_stack: Vec::with_capacity(8),
            or_pat_binds: Vec::with_capacity(16),
            sig_inputs_buf: Vec::with_capacity(16),
            bound_inputs_buf: Vec::with_capacity(8),
            exp_elems_buf: Vec::with_capacity(16),
            closure_seq: 0,
        }
    }

    /// Infers all expressions, patterns, and local bindings inside `fn_node`, returning a new [`FnInferResult`].
    pub fn infer_fn(&mut self, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, local_defs: &LocalCstDefs, fn_node: NodeId) -> FnInferResult {
        let mut out = FnInferResult::new();
        self.infer_fn_into(st, file_res, db, local_defs, fn_node, &mut out);
        out
    }

    /// Infers `fn_node` into caller-owned `out`, reusing both `self.table` and `out`'s `Vec` capacities.
    pub fn infer_fn_into(&mut self, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, local_defs: &LocalCstDefs, fn_node: NodeId, out: &mut FnInferResult) {
        self.table.clear();
        self.break_stack.clear();
        self.ret_stack.clear();
        self.or_pat_binds.clear();
        self.sig_inputs_buf.clear();
        self.bound_inputs_buf.clear();
        self.exp_elems_buf.clear();
        self.closure_seq = 0;
        out.clear();
        out.fn_node = fn_node;

        let (loc_lo, loc_hi) = enclosing_local_window(st, file_res, fn_node);
        self.local_slot_base = loc_lo as u32;
        self.local_slots.clear();
        self.local_slots.resize(loc_hi - loc_lo, InferTyId::NONE);

        let impl_node = st.ancestors(fn_node).find(|&a| st.kind(a) == K::Impl);
        let (impl_gc, self_ty) = match impl_node {
            Some(im) => {
                let (gc, sty) = local_defs.lower_impl_self_ty(im, st, file_res, db, &mut self.table);
                (gc, Some(sty))
            }
            None => (0, None),
        };
        let fn_def = local_defs.fn_def_for_node(fn_node);
        let is_async_fn = st.own_tokens(fn_node).any(|(_, t)| t.kind == T::Async);

        let mut env = Runner { ctx: self, st, file_res, db, local_defs, fn_node, fn_def, impl_node, impl_gc, self_ty, loc_lo, loc_hi, out };
        env.bind_fn_params_and_infer_body(is_async_fn);
        env.finalize_result();
    }
}

pub(super) struct Runner<'a, 's> {
    pub ctx: &'a mut BodyInferCtx,
    pub st: &'a SyntaxTree<'s>,
    pub file_res: &'a FileNameRes,
    pub db: &'a dyn TyDefDb,
    pub local_defs: &'a LocalCstDefs,
    pub fn_node: NodeId,
    pub fn_def: GlobalDefId,
    pub impl_node: Option<NodeId>,
    pub impl_gc: u16,
    pub self_ty: Option<InferTyId>,
    pub loc_lo: usize,
    pub loc_hi: usize,
    pub out: &'a mut FnInferResult,
}

impl<'a, 's> Runner<'a, 's> {
    #[inline]
    pub fn ty_env(&self, in_body: bool) -> LowerTyEnv<'a, 's> {
        LowerTyEnv {
            st: self.st,
            file_res: self.file_res,
            db: self.db,
            local_defs: self.local_defs,
            self_ty: self.self_ty,
            impl_owner: self.impl_node,
            impl_generics: self.impl_gc,
            item_owner: Some(self.fn_node),
            in_body,
        }
    }

    #[inline]
    pub fn record_expr(&mut self, node: NodeId, ty: InferTyId) -> InferTyId {
        self.out.expr_tys.push((node, ty));
        ty
    }

    #[inline]
    pub fn record_pat(&mut self, node: NodeId, ty: InferTyId) -> InferTyId {
        self.out.pat_tys.push((node, ty));
        ty
    }

    #[inline]
    pub fn get_local(&self, lid: LocalBindId) -> InferTyId {
        let off = lid.0.wrapping_sub(self.ctx.local_slot_base) as usize;
        self.ctx.local_slots.get(off).copied().filter(|t| t.is_some()).unwrap_or(InferTyId::UNKNOWN)
    }

    #[inline]
    pub fn set_local(&mut self, lid: LocalBindId, ty: InferTyId) {
        let off = lid.0.wrapping_sub(self.ctx.local_slot_base) as usize;
        if let Some(slot) = self.ctx.local_slots.get_mut(off) {
            if !slot.is_some() {
                *slot = ty;
            } else {
                let prev = *slot;
                let _ = self.ctx.table.coerce(ty, prev);
            }
        }
    }

    #[inline]
    pub fn find_decl_local(&self, decl_node: NodeId) -> Option<LocalBindId> {
        self.file_res.locals[self.loc_lo..self.loc_hi].iter().find(|l| l.decl_node == decl_node).map(|l| l.id)
    }

    /// Coerces `actual` toward `expected` (extending `unify::coerce` with `FnDef -> FnPtr` and `Closure/FnDef -> impl Fn*`).
    pub fn coerce_expr(&mut self, actual: InferTyId, expected: InferTyId) -> InferTyId {
        let act = self.ctx.table.shallow_resolve(actual);
        let exp = self.ctx.table.shallow_resolve(expected);
        if act == InferTyId::NEVER {
            return InferTyId::NEVER;
        }
        match (self.ctx.table.arena.kind(act), self.ctx.table.arena.kind(exp)) {
            (InferTyKind::FnDef { def, args }, InferTyKind::FnPtr { .. }) => {
                if let Some(fp) = self.fn_def_as_fn_ptr(def, args) {
                    let _ = self.ctx.table.coerce(fp, exp);
                    return self.ctx.table.shallow_resolve(exp);
                }
            }
            (InferTyKind::Closure { sig, .. }, InferTyKind::Opaque { def, args }) | (InferTyKind::Closure { sig, .. }, InferTyKind::Dyn { principal: def, args })
                if self.db.lang_item_of_def(def).is_some_and(|l| l.is_fn_trait()) && args.len >= 2 =>
            {
                let s = self.ctx.table.arena.slice(sig);
                let mut in_buf = InlineTyBuf::new();
                for &inp in &s[..s.len() - 1] {
                    in_buf.push(inp);
                }
                let out_ty = s[s.len() - 1];
                let act_tup = self.ctx.table.arena.mk_tuple(in_buf.as_slice());
                let exp_tup = self.ctx.table.arena.slice(args)[0];
                let exp_ret = self.ctx.table.arena.slice(args)[1];
                let _ = self.ctx.table.equate(act_tup, exp_tup);
                let _ = self.ctx.table.coerce(out_ty, exp_ret);
                return self.ctx.table.shallow_resolve(exp);
            }
            _ => {}
        }
        let _ = self.ctx.table.coerce(act, exp);
        self.ctx.table.shallow_resolve(act)
    }

    pub fn fn_def_as_fn_ptr(&mut self, def: GlobalDefId, args: super::ty::TySliceRange) -> Option<InferTyId> {
        let mut subst = InlineTyBuf::new();
        for &a in self.ctx.table.arena.slice(args) {
            subst.push(a);
        }
        let mut raw_ins = core::mem::take(&mut self.ctx.sig_inputs_buf);
        let sig_opt = self
            .db
            .fn_sig(def, &mut self.ctx.table.arena, &mut raw_ins)
            .or_else(|| self.local_defs.fn_sig(def, self.st, self.file_res, self.db, &mut self.ctx.table, &mut raw_ins));
        let res = sig_opt.map(|(_, raw_ret)| {
            let mut inst_ins = InlineTyBuf::new();
            for &inp in &raw_ins {
                inst_ins.push(self.ctx.table.instantiate(inp, subst.as_slice()));
            }
            let inst_ret = self.ctx.table.instantiate(raw_ret, subst.as_slice());
            self.ctx.table.arena.mk_fn_ptr(inst_ins.as_slice(), inst_ret)
        });
        self.ctx.sig_inputs_buf = raw_ins;
        res
    }

    fn bind_fn_params_and_infer_body(&mut self, is_async_fn: bool) {
        let tenv = self.ty_env(false);
        let mut idx_inputs = core::mem::take(&mut self.ctx.sig_inputs_buf);
        idx_inputs.clear();
        let indexed_sig = self.fn_def.is_some().then(|| self.db.fn_sig(self.fn_def, &mut self.ctx.table.arena, &mut idx_inputs)).flatten();
        if indexed_sig.is_none() {
            idx_inputs.clear();
        }

        let mut param_cursor = 0usize;
        if let Some(pl) = self.st.child(self.fn_node, K::ParamList) {
            for p in self.st.children(pl) {
                match self.st.kind(p) {
                    K::SelfParam => {
                        let pty = idx_inputs.get(param_cursor).copied().unwrap_or_else(|| lower_self_param_ty(self.st, p, &tenv, &mut self.ctx.table));
                        param_cursor += 1;
                        if let Some(lid) = self.find_decl_local(p) {
                            self.set_local(lid, pty);
                        }
                        self.record_pat(p, pty);
                    }
                    K::Param => {
                        let pty = match idx_inputs.get(param_cursor).copied() {
                            Some(t) => t,
                            None => match self.st.children(p).find(|&c| self.st.kind(c).is_type()) {
                                Some(tn) => tenv.lower_cst_ty(&mut self.ctx.table, tn),
                                None => self.ctx.table.new_ty_var(),
                            },
                        };
                        param_cursor += 1;
                        if let Some(pat) = self.st.children(p).find(|&c| self.st.kind(c).is_pat()) {
                            self.infer_pat(pat, pty, BindingMode::ByValue, None);
                        }
                    }
                    _ => {}
                }
            }
        }
        self.ctx.sig_inputs_buf = idx_inputs;

        let body_ret_ty = if let Some((_, idx_ret)) = indexed_sig {
            if is_async_fn {
                super::lang_ops::unwrap_await_ty(&mut self.ctx.table, self.db, idx_ret)
            } else {
                idx_ret
            }
        } else {
            self.st
                .child(self.fn_node, K::RetType)
                .and_then(|rt| self.st.children(rt).find(|&c| self.st.kind(c).is_type()))
                .map_or(InferTyId::UNIT, |tn| tenv.lower_cst_ty(&mut self.ctx.table, tn))
        };

        self.out.ret_ty = body_ret_ty;
        self.ctx.ret_stack.push(body_ret_ty);
        if let Some(body) = self.st.child(self.fn_node, K::BlockExpr) {
            let bty = self.infer_expr(body, Expectation::HasType(body_ret_ty));
            let _ = self.coerce_expr(bty, body_ret_ty);
            self.out.body_ty = bty;
        } else {
            self.out.body_ty = body_ret_ty;
        }
        self.ctx.ret_stack.pop();
    }

    fn finalize_result(&mut self) {
        self.out.ret_ty = self.ctx.table.resolve_with_fallback(self.out.ret_ty);
        self.out.body_ty = self.ctx.table.resolve_with_fallback(self.out.body_ty);
        for pair in &mut self.out.expr_tys {
            pair.1 = self.ctx.table.resolve_with_fallback(pair.1);
        }
        for pair in &mut self.out.pat_tys {
            pair.1 = self.ctx.table.resolve_with_fallback(pair.1);
        }
        let fn_end = self.st.node(self.fn_node).subtree_end;
        for (off, &raw) in self.ctx.local_slots.iter().enumerate() {
            if raw.is_some() {
                let lid = LocalBindId(self.ctx.local_slot_base + off as u32);
                let decl = self.file_res.local_data(lid).decl_node;
                if decl.0 >= self.fn_node.0 && decl.0 < fn_end {
                    let resolved = self.ctx.table.resolve_with_fallback(raw);
                    self.out.local_tys.push((lid, resolved));
                }
            }
        }
        self.out.expr_tys.sort_unstable_by_key(|&(n, _)| n);
        self.out.pat_tys.sort_unstable_by_key(|&(n, _)| n);
        self.out.local_tys.sort_unstable_by_key(|&(l, _)| l);
    }
}

fn enclosing_local_window(st: &SyntaxTree<'_>, file_res: &FileNameRes, fn_node: NodeId) -> (usize, usize) {
    let top_item = st.ancestors(fn_node).filter(|&a| matches!(st.kind(a), K::Fn | K::Const | K::Static)).last().unwrap_or(fn_node);
    let top_end = st.node(top_item).subtree_end;
    let lo = file_res.locals.partition_point(|l| l.decl_node.0 < top_item.0);
    let hi = lo + file_res.locals[lo..].partition_point(|l| l.decl_node.0 < top_end);
    (lo, hi)
}
