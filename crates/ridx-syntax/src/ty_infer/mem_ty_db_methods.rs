/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Inherent method, trait method, trait `impl`, supertrait, generic bound, and trait associated-type
//! query support for [`super::mem_ty_db::MemTyDefDb`] (`0` heap allocations per query).

use super::arena::InferTyArena;
use super::autoderef::impl_target_key_of_ty;
use super::db::{clone_ty_into, MethodCand};
use super::mem_ty_db::{instantiate_in_arena, MemTyDefDb};
use super::ty::{InferTyId, InferTyKind, TyVarKind};
use super::uf_table::{InlineTyBuf, UnionFindTable};
use super::unify;
use crate::name_res::{CrateId, GlobalDefId, ImplTargetKey};

#[derive(Clone, Debug)]
pub(super) struct InherentMethodEntry {
    pub target: ImplTargetKey,
    pub name: String,
    pub method_def: GlobalDefId,
    pub impl_generics: u16,
    pub method_generics: u16,
}

#[derive(Clone, Debug)]
pub(super) struct TraitMethodDeclEntry {
    pub trait_def: GlobalDefId,
    pub trait_generics: u16,
    pub name: String,
    pub method_def: GlobalDefId,
    pub method_generics: u16,
}

#[derive(Clone, Debug)]
pub(super) struct TraitImplEntry {
    pub target: ImplTargetKey,
    pub trait_def: GlobalDefId,
    pub impl_generics: u16,
    pub self_ty: InferTyId,
    pub trait_args: Vec<InferTyId>,
    pub bounds: Vec<(InferTyId, GlobalDefId, Vec<InferTyId>)>,
    pub overrides: Vec<(String, GlobalDefId, u16)>,
    pub is_blanket: bool,
}

#[derive(Clone, Debug)]
pub(super) struct TraitAssocRuleEntry {
    pub trait_def: GlobalDefId,
    pub assoc_def: GlobalDefId,
    pub generic_count: u16,
    pub for_ty: InferTyId,
    pub trait_args: Vec<InferTyId>,
    pub output_ty: InferTyId,
}

impl MemTyDefDb {
    /// Registers an inherent method `impl<T..> Target { fn name<M..>(&self, ..) -> R }`
    /// (signature stored via [`Self::add_fn`] with `generic_count = impl_generics + method_generics`).
    pub fn add_inherent_method(&mut self, target: ImplTargetKey, name: &str, method_def: GlobalDefId, impl_generics: u16, method_generics: u16) {
        self.inherent_methods.push(InherentMethodEntry { target, name: name.to_string(), method_def, impl_generics, method_generics });
    }

    /// Registers a `trait<A..> Trait { fn name<M..>(&self, ..) -> R }` method declaration
    /// (`Param(0) = Self`, `Param(1..1+trait_generics) = A..`, `Param(1+trait_generics..) = M..`).
    pub fn add_trait_method(&mut self, trait_def: GlobalDefId, trait_generics: u16, name: &str, method_def: GlobalDefId, method_generics: u16) {
        self.trait_methods.push(TraitMethodDeclEntry { trait_def, trait_generics, name: name.to_string(), method_def, method_generics });
    }

    /// Registers `impl<G..> Trait<Args..> for SelfTy` with optional method overrides `(name, impl_fn_def, method_generics)`.
    pub fn add_trait_impl(&mut self, trait_def: GlobalDefId, impl_generics: u16, self_ty: InferTyId, trait_args: &[InferTyId], overrides: &[(&str, GlobalDefId, u16)]) {
        self.add_trait_impl_with_bounds(trait_def, impl_generics, self_ty, trait_args, &[], overrides);
    }

    /// Registers `impl<G..> Trait<Args..> for SelfTy where Bounds..` (`Checkbox 3.4`).
    pub fn add_trait_impl_with_bounds(
        &mut self,
        trait_def: GlobalDefId,
        impl_generics: u16,
        self_ty: InferTyId,
        trait_args: &[InferTyId],
        bounds: &[(InferTyId, GlobalDefId, &[InferTyId])],
        overrides: &[(&str, GlobalDefId, u16)],
    ) {
        let is_blanket = matches!(self.templates.kind(self_ty), InferTyKind::Param { .. });
        let target = if is_blanket { ImplTargetKey::Param } else { impl_target_key_of_ty(&self.templates, self_ty) };
        let bvec = bounds.iter().map(|&(lhs, td, args)| (lhs, td, args.to_vec())).collect();
        let ov = overrides.iter().map(|&(n, d, mg)| (n.to_string(), d, mg)).collect();
        self.trait_impls.push(TraitImplEntry { target, trait_def, impl_generics, self_ty, trait_args: trait_args.to_vec(), bounds: bvec, overrides: ov, is_blanket });
    }

    pub fn add_supertrait(&mut self, sub_trait: GlobalDefId, super_trait: GlobalDefId) {
        self.supertraits.push((sub_trait, super_trait));
    }

    pub fn add_param_trait_bound(&mut self, owner_def: GlobalDefId, param_idx: u16, trait_def: GlobalDefId) {
        self.param_bounds.push((owner_def, param_idx, trait_def));
    }

    pub fn add_trait_assoc_rule(&mut self, trait_def: GlobalDefId, assoc_def: GlobalDefId, generic_count: u16, for_ty: InferTyId, trait_args: &[InferTyId], output_ty: InferTyId) {
        self.trait_assoc_rules.push(TraitAssocRuleEntry { trait_def, assoc_def, generic_count, for_ty, trait_args: trait_args.to_vec(), output_ty });
    }

    pub(super) fn query_inherent_methods(&self, target: ImplTargetKey, method_name: &str, out: &mut Vec<MethodCand>) {
        for m in &self.inherent_methods {
            if m.target == target && m.name == method_name {
                out.push(MethodCand {
                    method_def: m.method_def,
                    trait_def: GlobalDefId::NONE,
                    bound_owner_def: m.method_def,
                    container_generics: m.impl_generics,
                    method_generics: m.method_generics,
                    has_fixed_trait_subst: false,
                    fixed_self_ty: InferTyId::NONE,
                    fixed_trait_args: super::ty::TySliceRange::EMPTY,
                    is_blanket_impl: false,
                    impl_owner: GlobalDefId::NONE,
                    cst_fn_node: u32::MAX,
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn query_trait_impl_methods(
        &self,
        target: ImplTargetKey,
        trait_def: GlobalDefId,
        method_name: &str,
        step_ty: InferTyId,
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        out: &mut Vec<MethodCand>,
    ) {
        for (imp_idx, imp) in self.trait_impls.iter().enumerate() {
            if imp.trait_def != trait_def {
                continue;
            }
            if target == ImplTargetKey::Param {
                if !imp.is_blanket {
                    continue;
                }
            } else if imp.is_blanket || imp.target != target {
                continue;
            }
            let snap = vars.start_snapshot();
            let mut subst = InlineTyBuf::new();
            for _ in 0..imp.impl_generics {
                subst.push(vars.new_var(arena, TyVarKind::General).1);
            }
            let self_tpl = clone_ty_into(&self.templates, imp.self_ty, arena);
            let cand_self = instantiate_in_arena(arena, self_tpl, subst.as_slice());
            let compat = unify::equate(arena, vars, cand_self, step_ty).is_ok();
            vars.rollback_to(snap);
            if !compat {
                continue;
            }
            let imp_owner = GlobalDefId::new(CrateId(0), imp_idx as u32);
            if let Some(&(_, ov_def, ov_mg)) = imp.overrides.iter().find(|(n, _, _)| n == method_name) {
                let bound_owner = self.trait_methods.iter().find(|tm| tm.trait_def == trait_def && tm.name == method_name).map_or(ov_def, |tm| tm.method_def);
                out.push(MethodCand {
                    method_def: ov_def,
                    trait_def,
                    bound_owner_def: bound_owner,
                    container_generics: imp.impl_generics,
                    method_generics: ov_mg,
                    has_fixed_trait_subst: false,
                    fixed_self_ty: InferTyId::NONE,
                    fixed_trait_args: super::ty::TySliceRange::EMPTY,
                    is_blanket_impl: imp.is_blanket,
                    impl_owner: imp_owner,
                    cst_fn_node: u32::MAX,
                });
                continue;
            }
            let mut tr_args = InlineTyBuf::new();
            for &ta in &imp.trait_args {
                tr_args.push(clone_ty_into(&self.templates, ta, arena));
            }
            if let Some(mut dc) = self.query_trait_decl_method(trait_def, tr_args.as_slice(), self_tpl, method_name, arena, 0) {
                dc.container_generics = imp.impl_generics;
                dc.has_fixed_trait_subst = true;
                dc.fixed_self_ty = self_tpl;
                dc.fixed_trait_args = arena.intern_slice(tr_args.as_slice());
                dc.is_blanket_impl = imp.is_blanket;
                dc.impl_owner = imp_owner;
                out.push(dc);
            }
        }
    }

    pub(super) fn query_trait_decl_method(
        &self,
        trait_def: GlobalDefId,
        trait_args: &[InferTyId],
        self_ty: InferTyId,
        method_name: &str,
        arena: &mut InferTyArena,
        depth: u8,
    ) -> Option<MethodCand> {
        if depth > 6 {
            return None;
        }
        if let Some(tm) = self.trait_methods.iter().find(|m| m.trait_def == trait_def && m.name == method_name) {
            let mut padded = InlineTyBuf::new();
            for i in 0..tm.trait_generics as usize {
                padded.push(trait_args.get(i).copied().unwrap_or(InferTyId::NONE));
            }
            let args_r = arena.intern_slice(padded.as_slice());
            return Some(MethodCand {
                method_def: tm.method_def,
                trait_def,
                bound_owner_def: tm.method_def,
                container_generics: 0,
                method_generics: tm.method_generics,
                has_fixed_trait_subst: true,
                fixed_self_ty: self_ty,
                fixed_trait_args: args_r,
                is_blanket_impl: false,
                impl_owner: GlobalDefId::NONE,
                cst_fn_node: u32::MAX,
            });
        }
        for &(sub, sup) in &self.supertraits {
            if sub == trait_def {
                if let Some(c) = self.query_trait_decl_method(sup, &[], self_ty, method_name, arena, depth + 1) {
                    return Some(c);
                }
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn query_trait_assoc_rule(
        &self,
        trait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        self_ty: InferTyId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
    ) -> Option<InferTyId> {
        for r in &self.trait_assoc_rules {
            if (trait_def.is_some() && r.trait_def != trait_def && !self.has_supertrait_path(trait_def, r.trait_def))
                || r.assoc_def != assoc_def
                || r.trait_args.len() != trait_args.len()
            {
                continue;
            }
            let snap = vars.start_snapshot();
            let mut subst = InlineTyBuf::new();
            for _ in 0..r.generic_count {
                subst.push(vars.new_var(arena, TyVarKind::General).1);
            }
            let for_tpl = clone_ty_into(&self.templates, r.for_ty, arena);
            let cand_for = instantiate_in_arena(arena, for_tpl, subst.as_slice());
            let mut ok = unify::equate(arena, vars, self_ty, cand_for).is_ok();
            if ok {
                for (i, &aa) in trait_args.iter().enumerate() {
                    let atpl = clone_ty_into(&self.templates, r.trait_args[i], arena);
                    let ca = instantiate_in_arena(arena, atpl, subst.as_slice());
                    if unify::coerce(arena, vars, aa, ca).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                vars.commit(snap);
                let out_tpl = clone_ty_into(&self.templates, r.output_ty, arena);
                let inst = instantiate_in_arena(arena, out_tpl, subst.as_slice());
                return Some(vars.shallow_resolve(arena, inst));
            }
            vars.rollback_to(snap);
        }
        None
    }
}
