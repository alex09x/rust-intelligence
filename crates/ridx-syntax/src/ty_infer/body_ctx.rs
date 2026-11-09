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

use super::{
    db::{MethodCand, TyDefDb, WhereObligation},
    expectation::{BranchCoercer, Expectation},
    infer_expr_cf::enclosing_local_window,
    infer_macro::{SubInferEnv, SubLocalEntry, MAX_RUNTIME_MACRO_DEPTH},
    infer_pat::BindingMode,
    local_defs::{lower_self_param_ty, LocalCstDefs},
    lower_ty::LowerTyEnv,
    ty::InferTyId,
    InferenceTable,
};
use crate::{
    name_res::{FileNameRes, GlobalDefId, ItemKind, LocalBindId},
    parser::ParseScratch,
    syntax_kind::SyntaxKind as K,
    token::TokenKind as T,
    tree::{NodeId, SyntaxTree},
};

/// Type-dependent `.method()` or `.field` reference resolved during body inference (`Checkbox 3.3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedDotRef {
    pub start: u32,
    pub end: u32,
    pub def: GlobalDefId,
    pub kind: ItemKind,
}

/// Expression inferred inside a macro expansion or std-macro argument mapped to caller byte offsets (`Checkbox 3.6.2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacroExprTy {
    pub start: u32,
    pub end: u32,
    pub kind: K,
    pub ty: InferTyId,
}

/// Finalized local type inference output for a single `K::Fn` / `K::Const` / `K::Static` item.
#[derive(Clone, Debug)]
pub struct FnInferResult {
    pub fn_node: NodeId,
    pub ret_ty: InferTyId,
    pub body_ty: InferTyId,
    pub expr_tys: Vec<(NodeId, InferTyId)>,
    pub macro_expr_tys: Vec<MacroExprTy>,
    pub pat_tys: Vec<(NodeId, InferTyId)>,
    pub local_tys: Vec<(LocalBindId, InferTyId)>,
    pub dot_refs: Vec<ResolvedDotRef>,
    /// Byte end offset of an incomplete trailing `.` (`x.` or `self.0.`) paired with receiver type.
    pub incomplete_dots: Vec<(u32, InferTyId)>,
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
    pub(super) macro_scratch: Vec<ParseScratch>,
    pub(super) macro_depth: usize,
    pub(super) sub_locals_buf: Vec<SubLocalEntry>,
    pub(super) macro_local_shadows: Vec<(LocalBindId, InferTyId)>,
    pub(super) deferred_proj_eqs: Vec<(InferTyId, InferTyId)>,
    pub(super) deferred_trait_bounds: Vec<(GlobalDefId, InferTyId, super::ty::TySliceRange)>,
}

impl Default for BodyInferCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl BodyInferCtx {
    pub fn new() -> Self {
        let macro_scratch = (0..MAX_RUNTIME_MACRO_DEPTH).map(|_| ParseScratch::with_capacity()).collect();
        let (table, local_slots) = (InferenceTable::with_capacity(256, 128), Vec::with_capacity(64));
        Self {
            table,
            local_slot_base: 0,
            local_slots,
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
            macro_scratch,
            macro_depth: 0,
            sub_locals_buf: Vec::with_capacity(16),
            macro_local_shadows: Vec::with_capacity(8),
            deferred_proj_eqs: Vec::with_capacity(4),
            deferred_trait_bounds: Vec::with_capacity(8),
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
        self.macro_depth = 0;
        self.sub_locals_buf.clear();
        self.macro_local_shadows.clear();
        self.deferred_proj_eqs.clear();
        self.deferred_trait_bounds.clear();
        let fn_sub_end = st.node(fn_node).subtree_end;
        self.fn_has_inner_trait_scopes = file_res.has_inner_trait_scopes(fn_node, fn_sub_end);
        self.fn_scope_traits_ready = false;
        out.clear();
        out.fn_node = fn_node;

        let (loc_lo, loc_hi) = enclosing_local_window(st, file_res, fn_node);
        self.local_slot_base = loc_lo as u32;
        self.local_slots.clear();
        self.local_slots.resize(loc_hi - loc_lo, InferTyId::NONE);

        let impl_node = st.ancestors(fn_node).skip(1).take_while(|&a| !matches!(st.kind(a), K::Fn | K::BlockExpr | K::Module)).find(|&a| matches!(st.kind(a), K::Impl | K::Trait));
        let (impl_gc, self_ty) = local_defs.owner_generics_and_self_ty(impl_node, st, file_res, db, &mut self.table);
        let fn_def = local_defs.fn_def_for_node(fn_node);
        let is_async_fn = st.own_tokens(fn_node).any(|(_, t)| t.kind == T::Async);

        let mut env = Runner {
            ctx: self,
            st,
            file_res,
            root_st: st,
            root_file_res: file_res,
            db,
            local_defs,
            fn_node,
            root_fn_node: fn_node,
            fn_def,
            impl_node,
            root_impl_node: impl_node,
            impl_gc,
            self_ty,
            loc_lo,
            loc_hi,
            expr_depth: 0,
            sub: None,
            out,
        };
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
    pub root_st: &'a SyntaxTree<'s>,
    pub root_file_res: &'a FileNameRes,
    pub db: &'a dyn TyDefDb,
    pub local_defs: &'a LocalCstDefs,
    pub fn_node: NodeId,
    pub root_fn_node: NodeId,
    pub fn_def: GlobalDefId,
    pub impl_node: Option<NodeId>,
    pub root_impl_node: Option<NodeId>,
    pub impl_gc: u16,
    pub self_ty: Option<InferTyId>,
    pub loc_lo: usize,
    pub loc_hi: usize,
    pub expr_depth: u16,
    pub sub: Option<SubInferEnv<'a>>,
    pub out: &'a mut FnInferResult,
}

impl<'a, 's> Runner<'a, 's> {
    #[inline]
    pub fn ty_env(&self, in_body: bool) -> LowerTyEnv<'a, 's> {
        let sub_map = match self.sub {
            None => None,
            Some(SubInferEnv::StdSlice { .. }) => Some((self.root_st, self.root_file_res, None)),
            Some(SubInferEnv::Expanded { spans, .. }) => Some((self.root_st, self.root_file_res, Some(spans))),
        };
        LowerTyEnv {
            st: self.st,
            file_res: self.file_res,
            db: self.db,
            local_defs: self.local_defs,
            self_ty: self.self_ty,
            impl_owner: self.root_impl_node,
            impl_generics: self.impl_gc,
            item_owner: Some(self.root_fn_node),
            in_body,
            sub_map,
        }
    }

    #[inline]
    pub fn record_expr(&mut self, node: NodeId, ty: InferTyId) -> InferTyId {
        if self.sub.is_none() {
            self.out.expr_tys.push((node, ty));
        } else {
            let (nr, kind) = (self.st.range(node), self.st.kind(node));
            if let Some((os, oe)) = (nr.end > nr.start).then(|| self.map_expr_span_to_orig(nr.start as u32, nr.end as u32, kind)).flatten() {
                self.out.macro_expr_tys.push(MacroExprTy { start: os, end: oe, kind, ty });
            }
        }
        ty
    }

    #[inline]
    pub fn record_pat(&mut self, node: NodeId, ty: InferTyId) -> InferTyId {
        if self.sub.is_none() {
            self.out.pat_tys.push((node, ty));
        }
        ty
    }

    #[inline]
    pub fn get_local(&self, lid: LocalBindId) -> InferTyId {
        if !matches!(self.sub, Some(SubInferEnv::Expanded { .. })) {
            if let Some(&(_, sh_ty)) = self.ctx.macro_local_shadows.iter().rev().find(|&&(id, _)| id == lid) {
                return sh_ty;
            }
        }
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
        (!matches!(self.sub, Some(SubInferEnv::StdSlice { .. })))
            .then(|| self.file_res.locals[self.loc_lo..self.loc_hi].iter().find(|l| l.decl_node == decl_node).map(|l| l.id))
            .flatten()
    }

    fn bind_fn_params_and_infer_body(&mut self, is_async_fn: bool) {
        if self.fn_def.is_some() {
            let _ = self.db.project_param_assoc(self.fn_def, 0, GlobalDefId::NONE, GlobalDefId::NONE, &[], &mut self.ctx.table.arena);
        }
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

        let cst_ret_tn = self.st.child(self.fn_node, K::RetType).and_then(|rt| self.st.children(rt).find(|&c| self.st.kind(c).is_type()));
        let cst_has_impl_tr = cst_ret_tn.is_some_and(|tn| self.st.descendants(tn).any(|d| self.st.kind(d) == K::ImplTraitType));
        let cst_ret_opt = cst_ret_tn.filter(|_| !cst_has_impl_tr).map(|tn| tenv.lower_cst_ty(&mut self.ctx.table, tn));
        let body_ret_ty = if let Some((_, idx_ret)) = indexed_sig {
            let unwr = if is_async_fn {
                let u = super::lang_ops::unwrap_await_ty(&mut self.ctx.table, self.db, idx_ret);
                if u != InferTyId::UNKNOWN {
                    u
                } else {
                    idx_ret
                }
            } else {
                idx_ret
            };
            match cst_ret_opt {
                Some(cr)
                    if !self.ctx.table.arena.flags(cr).has_unknown()
                        && (unwr == InferTyId::UNKNOWN || super::lang_ops::is_deref_attr_ret_mismatch(&self.ctx.table.arena, unwr, cr)) =>
                {
                    cr
                }
                _ => unwr,
            }
        } else {
            cst_ret_opt.or_else(|| cst_ret_tn.map(|tn| tenv.lower_cst_ty(&mut self.ctx.table, tn))).unwrap_or(InferTyId::UNIT)
        };
        let exp_body_ret = super::lang_ops::replace_nested_opaque_with_vars(&mut self.ctx.table, body_ret_ty);
        self.out.ret_ty = exp_body_ret;
        self.ctx.ret_stack.push(exp_body_ret);
        if let Some(body) = self.st.child(self.fn_node, K::BlockExpr) {
            let bty = self.infer_expr(body, Expectation::HasType(exp_body_ret));
            let _ = self.coerce_expr(bty, exp_body_ret);
            self.out.body_ty = bty;
        } else {
            self.out.body_ty = exp_body_ret;
        }
        self.ctx.ret_stack.pop();
    }
}
