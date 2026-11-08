/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Checkbox `3.4` trait obligation solver, blanket-`impl` bound checker, `where`-obligation
//! queries, and `<Param as Trait>::Assoc` projection on [`super::mem_ty_db::MemTyDefDb`].

use super::arena::InferTyArena;
use super::autoderef::impl_target_key_of_ty;
use super::db::{clone_ty_into, MethodCand, TraitAssocFnMeta, WhereObligation};
use super::mem_ty_db::{instantiate_in_arena, MemTyDefDb};
use super::mem_ty_db_methods::TraitImplEntry;
use super::ty::{InferTyId, InferTyKind, TyVarKind};
use super::uf_table::{InlineTyBuf, UnionFindTable};
use super::unify;
use crate::name_res::GlobalDefId;

#[derive(Clone, Debug)]
pub(super) enum MemWhereEntry {
    TraitBound { owner_def: GlobalDefId, lhs_ty: InferTyId, trait_def: GlobalDefId, trait_args: Vec<InferTyId> },
    ProjectionEq { owner_def: GlobalDefId, lhs_ty: InferTyId, trait_def: GlobalDefId, assoc_def: GlobalDefId, trait_args: Vec<InferTyId>, rhs_ty: InferTyId },
}

impl MemTyDefDb {
    pub fn add_where_trait_bound(&mut self, owner_def: GlobalDefId, lhs_ty: InferTyId, trait_def: GlobalDefId, trait_args: &[InferTyId]) {
        self.where_entries.push(MemWhereEntry::TraitBound { owner_def, lhs_ty, trait_def, trait_args: trait_args.to_vec() });
    }

    pub fn add_where_proj_eq(&mut self, owner_def: GlobalDefId, lhs_ty: InferTyId, trait_def: GlobalDefId, assoc_def: GlobalDefId, trait_args: &[InferTyId], rhs_ty: InferTyId) {
        self.where_entries.push(MemWhereEntry::ProjectionEq { owner_def, lhs_ty, trait_def, assoc_def, trait_args: trait_args.to_vec(), rhs_ty });
    }

    pub(super) fn has_supertrait_path(&self, sub: GlobalDefId, target: GlobalDefId) -> bool {
        let mut stack = [GlobalDefId::NONE; 16];
        stack[0] = sub;
        let mut len = 1usize;
        let mut cur = 0usize;
        while cur < len {
            let t = stack[cur];
            if t == target {
                return true;
            }
            cur += 1;
            for &(a, b) in &self.supertraits {
                if a == t && !stack[..len].contains(&b) && len < stack.len() {
                    stack[len] = b;
                    len += 1;
                }
            }
        }
        false
    }

    pub(super) fn query_trait_of_assoc_fn(&self, assoc_fn: GlobalDefId) -> Option<TraitAssocFnMeta> {
        if let Some(tm) = self.trait_methods.iter().find(|m| m.method_def == assoc_fn) {
            return Some(TraitAssocFnMeta { trait_def: tm.trait_def, trait_fn_def: tm.method_def, trait_generics: tm.trait_generics, method_generics: tm.method_generics });
        }
        for imp in &self.trait_impls {
            if let Some((ov_name, _, ov_mg)) = imp.overrides.iter().find(|&&(_, d, _)| d == assoc_fn) {
                let tm = self.trait_methods.iter().find(|m| m.trait_def == imp.trait_def && m.name == *ov_name)?;
                return Some(TraitAssocFnMeta { trait_def: imp.trait_def, trait_fn_def: tm.method_def, trait_generics: tm.trait_generics, method_generics: *ov_mg });
            }
        }
        None
    }

    pub(super) fn query_trait_assoc_fn_by_name(&self, trait_def: GlobalDefId, fn_name: &str) -> Option<TraitAssocFnMeta> {
        let tm = self.trait_methods.iter().find(|m| m.trait_def == trait_def && m.name == fn_name)?;
        Some(TraitAssocFnMeta { trait_def: tm.trait_def, trait_fn_def: tm.method_def, trait_generics: tm.trait_generics, method_generics: tm.method_generics })
    }

    pub(super) fn query_owner_where_obligations(&self, owner_def: GlobalDefId, arena: &mut InferTyArena, out: &mut Vec<WhereObligation>) {
        for w in &self.where_entries {
            match w {
                MemWhereEntry::TraitBound { owner_def: od, lhs_ty, trait_def, trait_args } if *od == owner_def => {
                    let lhs = clone_ty_into(&self.templates, *lhs_ty, arena);
                    let mut ta = InlineTyBuf::new();
                    for &a in trait_args {
                        ta.push(clone_ty_into(&self.templates, a, arena));
                    }
                    let rng = arena.intern_slice(ta.as_slice());
                    out.push(WhereObligation::TraitBound { lhs_ty: lhs, trait_def: *trait_def, trait_args: rng });
                }
                MemWhereEntry::ProjectionEq { owner_def: od, lhs_ty, trait_def, assoc_def, trait_args, rhs_ty } if *od == owner_def => {
                    let lhs = clone_ty_into(&self.templates, *lhs_ty, arena);
                    let rhs = clone_ty_into(&self.templates, *rhs_ty, arena);
                    let mut ta = InlineTyBuf::new();
                    for &a in trait_args {
                        ta.push(clone_ty_into(&self.templates, a, arena));
                    }
                    let rng = arena.intern_slice(ta.as_slice());
                    out.push(WhereObligation::ProjectionEq { lhs_ty: lhs, trait_def: *trait_def, assoc_def: *assoc_def, trait_args: rng, rhs_ty: rhs });
                }
                _ => {}
            }
        }
    }

    pub(super) fn query_blanket_method_bounds(&self, cand: &MethodCand, impl_subst: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        let Some(imp) = self.trait_impls.get(cand.impl_owner.index as usize) else {
            return true;
        };
        self.verify_impl_bounds(imp, impl_subst, arena, vars, true, 0)
    }

    fn verify_impl_bounds(
        &self,
        imp: &TraitImplEntry,
        impl_subst: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        allow_ambig_self_var: bool,
        depth: u8,
    ) -> bool {
        if depth > 8 {
            return false;
        }
        for (lhs_tpl, btr, bargs_tpl) in &imp.bounds {
            let lhs_raw = clone_ty_into(&self.templates, *lhs_tpl, arena);
            let lhs_inst = instantiate_in_arena(arena, lhs_raw, impl_subst);
            let lhs = vars.shallow_resolve(arena, lhs_inst);
            if matches!(arena.kind(lhs), InferTyKind::Unknown | InferTyKind::Infer(_)) && allow_ambig_self_var {
                continue;
            }
            let mut bargs = InlineTyBuf::new();
            for &ba in bargs_tpl {
                let raw_a = clone_ty_into(&self.templates, ba, arena);
                let inst_a = instantiate_in_arena(arena, raw_a, impl_subst);
                bargs.push(vars.shallow_resolve(arena, inst_a));
            }
            if !self.query_solve_trait_obligation(*btr, lhs, bargs.as_slice(), arena, vars, depth + 1) {
                return false;
            }
        }
        true
    }

    pub(super) fn query_solve_trait_obligation(
        &self,
        trait_def: GlobalDefId,
        raw_self: InferTyId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        depth: u8,
    ) -> bool {
        if depth > 8 {
            return false;
        }
        let self_ty = vars.shallow_resolve(arena, raw_self);
        let self_head = impl_target_key_of_ty(arena, self_ty);
        let self_is_var = matches!(arena.kind(self_ty), InferTyKind::Infer(_));
        let mut match_count = 0u32;
        let mut unique_idx = 0usize;
        for (idx, imp) in self.trait_impls.iter().enumerate() {
            if imp.trait_def != trait_def || (!trait_args.is_empty() && imp.trait_args.len() != trait_args.len()) {
                continue;
            }
            if !self_is_var && !imp.is_blanket && imp.target != self_head {
                continue;
            }
            let snap = vars.start_snapshot();
            let ok = self.try_bind_mem_impl(imp, self_ty, trait_args, arena, vars, depth);
            vars.rollback_to(snap);
            if ok {
                match_count += 1;
                unique_idx = idx;
                if match_count > 1 {
                    break;
                }
            }
        }
        if match_count == 1 {
            let snap = vars.start_snapshot();
            if self.try_bind_mem_impl(&self.trait_impls[unique_idx], self_ty, trait_args, arena, vars, depth) {
                vars.commit(snap);
            } else {
                vars.rollback_to(snap);
            }
        }
        match_count >= 1
    }

    fn try_bind_mem_impl(&self, imp: &TraitImplEntry, self_ty: InferTyId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable, depth: u8) -> bool {
        let mut subst = InlineTyBuf::new();
        for _ in 0..imp.impl_generics {
            subst.push(vars.new_var(arena, TyVarKind::General).1);
        }
        let self_tpl = clone_ty_into(&self.templates, imp.self_ty, arena);
        let cand_self = instantiate_in_arena(arena, self_tpl, subst.as_slice());
        if unify::equate(arena, vars, self_ty, cand_self).is_err() {
            return false;
        }
        for (i, &ta) in trait_args.iter().enumerate() {
            let atpl = clone_ty_into(&self.templates, imp.trait_args[i], arena);
            let ca = instantiate_in_arena(arena, atpl, subst.as_slice());
            if unify::equate(arena, vars, ta, ca).is_err() {
                return false;
            }
        }
        self.verify_impl_bounds(imp, subst.as_slice(), arena, vars, false, depth)
    }

    pub(super) fn query_project_param_assoc(
        &self,
        owner_def: GlobalDefId,
        param_idx: u16,
        trait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        _trait_args: &[InferTyId],
        arena: &mut InferTyArena,
    ) -> Option<InferTyId> {
        for w in &self.where_entries {
            if let MemWhereEntry::ProjectionEq { owner_def: od, lhs_ty, trait_def: td, assoc_def: ad, rhs_ty, .. } = w {
                if *od != owner_def || *ad != assoc_def {
                    continue;
                }
                if trait_def.is_some() && *td != trait_def && !self.has_supertrait_path(*td, trait_def) {
                    continue;
                }
                if matches!(self.templates.kind(*lhs_ty), InferTyKind::Param { idx, .. } if idx == param_idx) {
                    return Some(clone_ty_into(&self.templates, *rhs_ty, arena));
                }
            }
        }
        None
    }
}
