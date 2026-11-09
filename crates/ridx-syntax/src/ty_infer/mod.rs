/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-free local type inference engine (`ty_infer`) — Phase 3 (`Checkboxes 3.1 + 3.2 + 3.3`):
//! - [`arena`]: intrusive hash-consed [`InferTyArena`] with pre-interned builtins and `O(1)` [`TyFlags`].
//! - [`uf_table`]: Tarjan union-find over [`TyVarId`] with union-by-rank, snapshot-safe path compression,
//!   occurs-check, and Rust integer (`i32`) / float (`f64`) literal fallbacks.
//! - [`unify`]: invariant [`equate`], directional [`coerce`], and multi-branch [`lub`].
//! - [`expectation`]: bidirectional [`Expectation`] propagation and [`BranchCoercer`] arm joiner.
//! - [`db`] / [`mem_ty_db`] / [`local_defs`]: zero-dependency [`TyDefDb`] boundary, [`TyLangItem`],
//!   in-memory [`MemTyDefDb`], and CST fallback [`LocalCstDefs`].
//! - [`autoderef`] / `method_res`: cycle-guarded `Deref` chain, receiver autoref/unsizing, inherent-first
//!   then scoped-trait method resolution, generic signature substitution, and `.field` / `.method()` [`ResolvedDotRef`](body_ctx::ResolvedDotRef).
//! - [`body_ctx`] (`BodyInferCtx`, `FnInferResult`): zero-allocation per-function expression, pattern,
//!   control-flow, `?` (`Try`), `.await` (`Future`), closure `Fn*`, range, and `Index` inference.

pub mod arena;
pub mod autoderef;
pub mod body_ctx;
pub mod db;
pub mod expectation;
mod infer_expr;
mod infer_expr_call;
mod infer_expr_cf;
mod infer_expr_closure;
mod infer_expr_ops;
mod infer_macro;
mod infer_macro_std;
pub mod infer_pat;
pub mod lang_ops;
pub mod local_defs;
mod local_defs_adt;
mod local_defs_impl;
mod local_defs_macros;
mod local_defs_proj;
pub mod lower_ty;
mod lower_ty_tok;
pub mod mem_ty_db;
mod mem_ty_db_methods;
mod mem_ty_db_traits;
mod method_res;
pub mod partial_code;
mod tok_ty_scan;
pub mod trait_solve;
mod trait_solve_cst;
mod trait_solve_ufcs;
pub mod ty;
pub mod uf_table;
pub mod unify;

pub use arena::InferTyArena;
pub use body_ctx::{BodyInferCtx, FnInferResult, MacroExprTy, ResolvedDotRef};
pub use db::{clone_ty_into, CtorShapeKind, EmptyTyDefDb, MethodCand, StdMacroTy, TraitAssocFnMeta, TyDefDb, TyLangItem, WhereObligation};
pub use expectation::{BranchCoercer, Expectation};
pub use infer_pat::BindingMode;
pub use local_defs::{effective_local_def_id, LocalCstDefs};
pub use lower_ty::LowerTyEnv;
pub use mem_ty_db::MemTyDefDb;
pub use ty::{is_float_prim, is_int_prim, InferTyId, InferTyKind, TyFlags, TySliceRange, TyVarId, TyVarKind};
pub use uf_table::{InferSnapshot, UnionFindTable};
pub use unify::{coerce, equate, lub, TypeError, UnifyResult};

use uf_table::InlineTyBuf;

/// Reusable per-function type inference state (`0` heap allocations after initial warmup via [`Self::clear`]).
#[derive(Debug, Default)]
pub struct InferenceTable {
    pub arena: InferTyArena,
    pub vars: UnionFindTable,
    pub(crate) gid_scratch: Vec<crate::name_res::GlobalDefId>,
}

impl InferenceTable {
    pub fn new() -> Self {
        Self::with_capacity(256, 128)
    }

    pub fn with_capacity(arena_cap: usize, var_cap: usize) -> Self {
        Self { arena: InferTyArena::with_capacity(arena_cap), vars: UnionFindTable::with_capacity(var_cap), gid_scratch: Vec::with_capacity(48) }
    }

    /// Resets all dynamic types and inference variables while keeping allocated buffers.
    #[inline]
    pub fn clear(&mut self) {
        self.arena.clear();
        self.vars.clear();
        self.gid_scratch.clear();
    }

    #[inline]
    pub fn new_ty_var(&mut self) -> InferTyId {
        self.vars.new_var(&mut self.arena, TyVarKind::General).1
    }

    #[inline]
    pub fn new_int_var(&mut self) -> InferTyId {
        self.vars.new_var(&mut self.arena, TyVarKind::IntVar).1
    }

    #[inline]
    pub fn new_float_var(&mut self) -> InferTyId {
        self.vars.new_var(&mut self.arena, TyVarKind::FloatVar).1
    }

    #[inline]
    pub fn new_var_with_id(&mut self, kind: TyVarKind) -> (TyVarId, InferTyId) {
        self.vars.new_var(&mut self.arena, kind)
    }

    #[inline]
    pub fn start_snapshot(&mut self) -> InferSnapshot {
        self.vars.start_snapshot()
    }

    #[inline]
    pub fn commit(&mut self, snap: InferSnapshot) {
        self.vars.commit(snap);
    }

    #[inline]
    pub fn rollback_to(&mut self, snap: InferSnapshot) {
        self.vars.rollback_to(snap);
    }

    #[inline]
    pub fn shallow_resolve(&mut self, ty: InferTyId) -> InferTyId {
        self.vars.shallow_resolve(&self.arena, ty)
    }

    /// Substitutes solved inference variables while preserving remaining unbound variables.
    #[inline]
    pub fn deep_resolve(&mut self, ty: InferTyId) -> InferTyId {
        self.vars.deep_resolve(&mut self.arena, ty, false)
    }

    /// Finalizes `ty`, applying Rust's default fallbacks (`{integer} -> i32`, `{float} -> f64`, `_ -> {unknown}`).
    #[inline]
    pub fn resolve_with_fallback(&mut self, ty: InferTyId) -> InferTyId {
        self.vars.deep_resolve(&mut self.arena, ty, true)
    }

    #[inline]
    pub fn equate(&mut self, a: InferTyId, b: InferTyId) -> UnifyResult {
        unify::equate(&mut self.arena, &mut self.vars, a, b)
    }

    /// Equates `a` and `b`; on mismatch or cycle, recovers to [`InferTyId::UNKNOWN`] without panicking.
    #[inline]
    pub fn equate_or_recover(&mut self, a: InferTyId, b: InferTyId) -> InferTyId {
        if self.equate(a, b).is_ok() {
            self.shallow_resolve(a)
        } else {
            InferTyId::UNKNOWN
        }
    }

    #[inline]
    pub fn coerce(&mut self, actual: InferTyId, expected: InferTyId) -> UnifyResult {
        unify::coerce(&mut self.arena, &mut self.vars, actual, expected)
    }

    #[inline]
    pub fn lub(&mut self, a: InferTyId, b: InferTyId) -> Result<InferTyId, TypeError> {
        unify::lub(&mut self.arena, &mut self.vars, a, b)
    }

    /// Appends `count` fresh general `TyVar`s to `out` for generic call-site instantiation.
    pub fn fresh_subst(&mut self, count: u16, out: &mut Vec<InferTyId>) {
        out.clear();
        out.reserve(count as usize);
        for _ in 0..count {
            out.push(self.new_ty_var());
        }
    }

    /// Substitutes `Param { idx, .. }` inside `ty` using `subst` (`O(1)` no-op when `!has_param()`).
    pub fn instantiate(&mut self, ty: InferTyId, subst: &[InferTyId]) -> InferTyId {
        self.instantiate_rec(ty, subst, 0)
    }

    fn instantiate_rec(&mut self, ty: InferTyId, subst: &[InferTyId], depth: u8) -> InferTyId {
        if depth > 64 || !self.arena.flags(ty).has_param() {
            return ty;
        }
        match self.arena.kind(ty) {
            InferTyKind::Param { idx, .. } => subst.get(idx as usize).copied().unwrap_or(InferTyId::UNKNOWN),
            InferTyKind::Ref { is_mut, inner } => {
                let s = self.instantiate_rec(inner, subst, depth + 1);
                self.arena.mk_ref(is_mut, s)
            }
            InferTyKind::Ptr { is_mut, inner } => {
                let s = self.instantiate_rec(inner, subst, depth + 1);
                self.arena.mk_ptr(is_mut, s)
            }
            InferTyKind::Slice(elem) => {
                let s = self.instantiate_rec(elem, subst, depth + 1);
                self.arena.mk_slice(s)
            }
            InferTyKind::Array { elem, len } => {
                let s = self.instantiate_rec(elem, subst, depth + 1);
                let l = self.arena.resolve_array_len(len, subst);
                self.arena.mk_array(s, l)
            }
            InferTyKind::Tuple(r) => {
                let sub = self.instantiate_slice(r, subst, depth + 1);
                self.arena.mk_tuple(sub.as_slice())
            }
            InferTyKind::Adt { def, args } => {
                let sub = self.instantiate_slice(args, subst, depth + 1);
                self.arena.mk_adt(def, sub.as_slice())
            }
            InferTyKind::FnPtr { sig } => {
                let sub = self.instantiate_slice(sig, subst, depth + 1);
                let s = sub.as_slice();
                self.arena.mk_fn_ptr(&s[..s.len() - 1], s[s.len() - 1])
            }
            InferTyKind::FnDef { def, args } => {
                let sub = self.instantiate_slice(args, subst, depth + 1);
                self.arena.mk_fn_def(def, sub.as_slice())
            }
            InferTyKind::Closure { def_start, def_end, sig } => {
                let sub = self.instantiate_slice(sig, subst, depth + 1);
                let s = sub.as_slice();
                self.arena.mk_closure(def_start, def_end, &s[..s.len() - 1], s[s.len() - 1])
            }
            InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
                let ns = self.instantiate_rec(self_ty, subst, depth + 1);
                let sub = self.instantiate_slice(args, subst, depth + 1);
                self.arena.mk_alias(trait_def, assoc_def, ns, sub.as_slice())
            }
            InferTyKind::Dyn { principal, args } => {
                let sub = self.instantiate_slice(args, subst, depth + 1);
                self.arena.mk_dyn(principal, sub.as_slice())
            }
            InferTyKind::Opaque { def, args } => {
                let sub = self.instantiate_slice(args, subst, depth + 1);
                self.arena.mk_opaque(def, sub.as_slice())
            }
            InferTyKind::Unknown | InferTyKind::Never | InferTyKind::Prim(_) | InferTyKind::Infer(_) => ty,
        }
    }

    fn instantiate_slice(&mut self, r: TySliceRange, subst: &[InferTyId], depth: u8) -> InlineTyBuf {
        let mut buf = InlineTyBuf::new();
        for idx in 0..r.len as usize {
            let orig = self.arena.slice(r)[idx];
            let next = self.instantiate_rec(orig, subst, depth);
            buf.push(next);
        }
        buf
    }

    /// Returns `true` if `ty` contains any inference variable (`InferTyKind::Infer`) not yet bound to a concrete type.
    #[inline]
    pub fn has_unresolved_infer(&mut self, ty: InferTyId) -> bool {
        vars_has_unresolved_infer(&self.arena, &mut self.vars, ty)
    }
}

/// Returns `true` if `ty` contains any inference variable (`InferTyKind::Infer`) not yet bound in `vars`.
#[inline]
pub fn vars_has_unresolved_infer(arena: &InferTyArena, vars: &mut UnionFindTable, ty: InferTyId) -> bool {
    vars_has_unresolved_infer_rec(arena, vars, ty, 0)
}

fn vars_has_unresolved_infer_rec(arena: &InferTyArena, vars: &mut UnionFindTable, ty: InferTyId, depth: u8) -> bool {
    if depth > 16 || !ty.is_some() || !arena.flags(ty).has_infer() {
        return false;
    }
    let cur = vars.shallow_resolve(arena, ty);
    if !arena.flags(cur).has_infer() {
        return false;
    }
    match arena.kind(cur) {
        InferTyKind::Infer(_) => true,
        InferTyKind::Ref { inner, .. } | InferTyKind::Ptr { inner, .. } | InferTyKind::Slice(inner) | InferTyKind::Array { elem: inner, .. } => {
            vars_has_unresolved_infer_rec(arena, vars, inner, depth + 1)
        }
        InferTyKind::Tuple(r)
        | InferTyKind::Adt { args: r, .. }
        | InferTyKind::FnPtr { sig: r }
        | InferTyKind::FnDef { args: r, .. }
        | InferTyKind::Closure { sig: r, .. }
        | InferTyKind::Dyn { args: r, .. }
        | InferTyKind::Opaque { args: r, .. } => {
            for idx in 0..r.len as usize {
                let elem = arena.slice(r)[idx];
                if vars_has_unresolved_infer_rec(arena, vars, elem, depth + 1) {
                    return true;
                }
            }
            false
        }
        InferTyKind::Alias { self_ty, args, .. } => {
            if vars_has_unresolved_infer_rec(arena, vars, self_ty, depth + 1) {
                return true;
            }
            for idx in 0..args.len as usize {
                let elem = arena.slice(args)[idx];
                if vars_has_unresolved_infer_rec(arena, vars, elem, depth + 1) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}
