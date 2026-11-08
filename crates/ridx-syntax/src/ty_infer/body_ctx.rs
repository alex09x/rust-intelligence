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

use super::db::{MethodCand, TyDefDb, WhereObligation};
use super::expectation::{BranchCoercer, Expectation};
use super::infer_expr::is_expr_node;
use super::infer_pat::BindingMode;
use super::local_defs::{lower_self_param_ty, LocalCstDefs};
use super::lower_ty::LowerTyEnv;
use super::ty::InferTyId;
use super::InferenceTable;
use crate::name_res::{FileNameRes, GlobalDefId, ItemKind, LocalBindId};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

/// Type-dependent `.method()` or `.field` reference resolved during body inference (`Checkbox 3.3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedDotRef {
    pub start: u32,
    pub end: u32,
    pub def: GlobalDefId,
    pub kind: ItemKind,
}

/// Finalized local type inference output for a single `K::Fn` / `K::Const` / `K::Static` item.
#[derive(Clone, Debug)]
pub struct FnInferResult {
    pub fn_node: NodeId,
    pub ret_ty: InferTyId,
    pub body_ty: InferTyId,
    pub expr_tys: Vec<(NodeId, InferTyId)>,
    pub pat_tys: Vec<(NodeId, InferTyId)>,
    pub local_tys: Vec<(LocalBindId, InferTyId)>,
    pub dot_refs: Vec<ResolvedDotRef>,
    /// Byte end offset of an incomplete trailing `.` (`x.` or `self.0.`) paired with receiver type.
    pub incomplete_dots: Vec<(u32, InferTyId)>,
}

impl Default for FnInferResult {
    fn default() -> Self {
        Self::new()
    }
}

impl FnInferResult {
    pub fn new() -> Self {
        Self {
            fn_node: NodeId(0),
            ret_ty: InferTyId::UNKNOWN,
            body_ty: InferTyId::UNKNOWN,
            expr_tys: Vec::new(),
            pat_tys: Vec::new(),
            local_tys: Vec::new(),
            dot_refs: Vec::new(),
            incomplete_dots: Vec::new(),
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.fn_node = NodeId(0);
        self.ret_ty = InferTyId::UNKNOWN;
        self.body_ty = InferTyId::UNKNOWN;
        self.expr_tys.clear();
        self.pat_tys.clear();
        self.local_tys.clear();
        self.dot_refs.clear();
        self.incomplete_dots.clear();
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
    pub(super) method_cands_buf: Vec<MethodCand>,
    pub(super) scope_traits_buf: Vec<GlobalDefId>,
    pub(super) fn_scope_traits_buf: Vec<GlobalDefId>,
    pub(super) param_bounds_buf: Vec<GlobalDefId>,
    pub(super) where_obls_buf: Vec<WhereObligation>,
    pub(super) fn_has_inner_trait_scopes: bool,
    pub(super) fn_scope_traits_ready: bool,
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
            method_cands_buf: Vec::with_capacity(16),
            scope_traits_buf: Vec::with_capacity(32),
            fn_scope_traits_buf: Vec::with_capacity(32),
            param_bounds_buf: Vec::with_capacity(16),
            where_obls_buf: Vec::with_capacity(16),
            fn_has_inner_trait_scopes: false,
            fn_scope_traits_ready: false,
            closure_seq: 0,
        }
    }

    /// Infers all expressions, patterns, and local bindings inside `fn_node`, returning a new [`FnInferResult`].
    pub fn infer_fn(&mut self, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, local_defs: &LocalCstDefs, fn_node: NodeId) -> FnInferResult {
        let mut out = FnInferResult::new();
        self.infer_fn_into(st, file_res, db, local_defs, fn_node, &mut out);
        out
    }

    /// Infers `fn_node` (`K::Fn`, `K::Const`, or `K::Static`) into caller-owned `out`, reusing capacities.
    pub fn infer_fn_into(&mut self, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, local_defs: &LocalCstDefs, fn_node: NodeId, out: &mut FnInferResult) {
        self.table.clear();
        self.break_stack.clear();
        self.ret_stack.clear();
        self.or_pat_binds.clear();
        self.sig_inputs_buf.clear();
        self.bound_inputs_buf.clear();
        self.exp_elems_buf.clear();
        self.method_cands_buf.clear();
        self.scope_traits_buf.clear();
        self.fn_scope_traits_buf.clear();
        self.param_bounds_buf.clear();
        self.where_obls_buf.clear();
        let fn_sub_end = st.node(fn_node).subtree_end;
        self.fn_has_inner_trait_scopes = file_res.has_inner_trait_scopes(fn_node, fn_sub_end);
        self.fn_scope_traits_ready = false;
        self.closure_seq = 0;
        out.clear();
        out.fn_node = fn_node;

        let (loc_lo, loc_hi) = enclosing_local_window(st, file_res, fn_node);
        self.local_slot_base = loc_lo as u32;
        self.local_slots.clear();
        self.local_slots.resize(loc_hi - loc_lo, InferTyId::NONE);

        let impl_node = st.ancestors(fn_node).find(|&a| matches!(st.kind(a), K::Impl | K::Trait));
        let (impl_gc, self_ty) = local_defs.owner_generics_and_self_ty(impl_node, st, file_res, db, &mut self.table);
        let fn_def = local_defs.fn_def_for_node(fn_node);
        let is_async_fn = st.own_tokens(fn_node).any(|(_, t)| t.kind == T::Async);

        let mut env = Runner { ctx: self, st, file_res, db, local_defs, fn_node, fn_def, impl_node, impl_gc, self_ty, loc_lo, loc_hi, expr_depth: 0, out };
        if matches!(st.kind(fn_node), K::Const | K::Static) {
            env.infer_const_or_static_body();
        } else {
            env.bind_fn_params_and_infer_body(is_async_fn);
        }
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
    pub expr_depth: u16,
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
                        let cst_tn = self.st.children(p).find(|&c| self.st.kind(c).is_type());
                        let has_impl_tr = cst_tn.is_some_and(|tn| self.st.descendants(tn).any(|d| self.st.kind(d) == K::ImplTraitType));
                        let pty = match idx_inputs.get(param_cursor).copied().filter(|_| !has_impl_tr) {
                            Some(t) => t,
                            None => match cst_tn {
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

    fn infer_const_or_static_body(&mut self) {
        let tenv = self.ty_env(false);
        let decl_ty = self
            .fn_def
            .is_some()
            .then(|| self.db.def_ty(self.fn_def, &mut self.ctx.table.arena).map(|(_, _, t)| t))
            .flatten()
            .unwrap_or_else(|| self.st.children(self.fn_node).find(|&c| self.st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |tn| tenv.lower_cst_ty(&mut self.ctx.table, tn)));
        self.out.ret_ty = decl_ty;
        if let Some(init) = self.st.children(self.fn_node).find(|&c| is_expr_node(self.st.kind(c))) {
            let bty = self.infer_expr(init, Expectation::HasType(decl_ty));
            let _ = self.coerce_expr(bty, decl_ty);
            self.out.body_ty = bty;
        }
    }

    #[inline]
    fn finalize_ty(&mut self, owner: super::autoderef::NormOwner, raw: InferTyId) -> InferTyId {
        let r1 = self.ctx.table.resolve_with_fallback(raw);
        let norm = super::autoderef::normalize_ty_with_owner(&mut self.ctx.table, self.db, self.local_defs, self.st, self.file_res, owner, r1);
        self.ctx.table.resolve_with_fallback(norm)
    }

    fn finalize_result(&mut self) {
        let owner = self.norm_owner();
        self.out.ret_ty = self.finalize_ty(owner, self.out.ret_ty);
        self.out.body_ty = self.finalize_ty(owner, self.out.body_ty);
        for i in 0..self.out.expr_tys.len() {
            self.out.expr_tys[i].1 = self.finalize_ty(owner, self.out.expr_tys[i].1);
        }
        for i in 0..self.out.pat_tys.len() {
            self.out.pat_tys[i].1 = self.finalize_ty(owner, self.out.pat_tys[i].1);
        }
        let fn_end = self.st.node(self.fn_node).subtree_end;
        for off in 0..self.ctx.local_slots.len() {
            let raw = self.ctx.local_slots[off];
            if raw.is_some() {
                let lid = LocalBindId(self.ctx.local_slot_base + off as u32);
                let decl = self.file_res.local_data(lid).decl_node;
                if decl.0 >= self.fn_node.0 && decl.0 < fn_end {
                    let norm = self.finalize_ty(owner, raw);
                    self.out.local_tys.push((lid, norm));
                }
            }
        }
        for i in 0..self.out.incomplete_dots.len() {
            self.out.incomplete_dots[i].1 = self.finalize_ty(owner, self.out.incomplete_dots[i].1);
        }
        self.out.expr_tys.sort_unstable_by_key(|&(n, _)| n);
        self.out.pat_tys.sort_unstable_by_key(|&(n, _)| n);
        self.out.local_tys.sort_unstable_by_key(|&(l, _)| l);
        self.out.dot_refs.sort_unstable_by_key(|a| (a.start, a.end, a.def, a.kind as u8));
        self.out.dot_refs.dedup_by(|b, a| a.start == b.start && a.end == b.end && a.def == b.def);
        self.out.incomplete_dots.sort_unstable_by_key(|&(off, _)| off);
        self.out.incomplete_dots.dedup_by_key(|p| p.0);
    }
}

fn enclosing_local_window(st: &SyntaxTree<'_>, file_res: &FileNameRes, fn_node: NodeId) -> (usize, usize) {
    let top_item = st.ancestors(fn_node).filter(|&a| matches!(st.kind(a), K::Fn | K::Const | K::Static)).last().unwrap_or(fn_node);
    let top_end = st.node(top_item).subtree_end;
    let lo = file_res.locals.partition_point(|l| l.decl_node.0 < top_item.0);
    let hi = lo + file_res.locals[lo..].partition_point(|l| l.decl_node.0 < top_end);
    (lo, hi)
}
