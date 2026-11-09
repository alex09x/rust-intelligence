/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! In-memory [`MemTyDefDb`] implementation of [`TyDefDb`] for unit tests, benchmarks, and standalone
//! std/core prelude fixtures without external `.ridx` files (`0` heap allocations per query).

use super::arena::InferTyArena;
use super::db::{clone_ty_into, CtorShapeKind, MethodCand, StdMacroTy, TraitAssocFnMeta, TyDefDb, TyLangItem, WhereObligation};
use super::mem_ty_db_methods::{InherentMethodEntry, TraitAssocRuleEntry, TraitImplEntry, TraitMethodDeclEntry};
pub(super) use super::mem_ty_db_traits::instantiate_in_arena;
use super::mem_ty_db_traits::MemWhereEntry;
use super::ty::{InferTyId, TyVarKind};
use super::uf_table::{InlineTyBuf, UnionFindTable};
use super::unify;
use crate::name_res::{GlobalDefId, ImplTargetKey};

#[derive(Clone, Debug)]
struct FnEntry {
    def: GlobalDefId,
    generic_count: u16,
    inputs: Vec<InferTyId>,
    output: InferTyId,
}

#[derive(Clone, Debug)]
struct DefEntry {
    def: GlobalDefId,
    generic_count: u16,
    shape: CtorShapeKind,
    ty: InferTyId,
    tuple_fields: Vec<InferTyId>,
    named_fields: Vec<(String, InferTyId, GlobalDefId)>,
}

#[derive(Clone, Debug)]
struct AssocRule {
    assoc_item: TyLangItem,
    generic_count: u16,
    for_ty: InferTyId,
    trait_args: Vec<InferTyId>,
    output_ty: InferTyId,
}

#[derive(Clone, Debug)]
struct FnBoundEntry {
    owner_def: GlobalDefId,
    param_idx: u16,
    trait_item: TyLangItem,
    inputs: Vec<InferTyId>,
    output: InferTyId,
}

/// Fast in-memory [`TyDefDb`] storing template [`InferTyId`]s in an internal [`InferTyArena`].
#[derive(Debug, Default)]
pub struct MemTyDefDb {
    pub templates: InferTyArena,
    fns: Vec<FnEntry>,
    defs: Vec<DefEntry>,
    lang_items: Vec<(TyLangItem, GlobalDefId)>,
    assoc_rules: Vec<AssocRule>,
    fn_bounds: Vec<FnBoundEntry>,
    pub(super) inherent_methods: Vec<InherentMethodEntry>,
    pub(super) trait_methods: Vec<TraitMethodDeclEntry>,
    pub(super) trait_impls: Vec<TraitImplEntry>,
    pub(super) supertraits: Vec<(GlobalDefId, GlobalDefId)>,
    pub(super) param_bounds: Vec<(GlobalDefId, u16, GlobalDefId)>,
    pub(super) trait_assoc_rules: Vec<TraitAssocRuleEntry>,
    pub(super) where_entries: Vec<MemWhereEntry>,
    pub(super) generic_defaults: Vec<(GlobalDefId, u16, InferTyId)>,
    pub cstr: Option<GlobalDefId>,
    pub vec_def: Option<GlobalDefId>,
    pub global_alloc_def: Option<GlobalDefId>,
    pub fmt_args_def: Option<GlobalDefId>,
    pub fmt_err_def: Option<GlobalDefId>,
}

impl MemTyDefDb {
    pub fn new() -> Self {
        Self { templates: InferTyArena::with_capacity(128), ..Self::default() }
    }

    pub fn add_lang_item(&mut self, item: TyLangItem, def: GlobalDefId) {
        self.lang_items.push((item, def));
    }

    pub fn add_fn(&mut self, def: GlobalDefId, generic_count: u16, inputs: &[InferTyId], output: InferTyId) {
        self.fns.push(FnEntry { def, generic_count, inputs: inputs.to_vec(), output });
    }

    pub fn add_adt(&mut self, def: GlobalDefId, generic_count: u16, shape: CtorShapeKind) -> InferTyId {
        let mut params = InlineTyBuf::new();
        for idx in 0..generic_count {
            params.push(self.templates.mk_param(idx, GlobalDefId::NONE));
        }
        let adt_ty = self.templates.mk_adt(def, params.as_slice());
        self.defs.push(DefEntry { def, generic_count, shape, ty: adt_ty, tuple_fields: Vec::new(), named_fields: Vec::new() });
        adt_ty
    }

    pub fn add_variant(&mut self, var_def: GlobalDefId, enum_def: GlobalDefId, generic_count: u16, shape: CtorShapeKind) -> InferTyId {
        let mut params = InlineTyBuf::new();
        for idx in 0..generic_count {
            params.push(self.templates.mk_param(idx, GlobalDefId::NONE));
        }
        let enum_ty = self.templates.mk_adt(enum_def, params.as_slice());
        self.defs.push(DefEntry { def: var_def, generic_count, shape, ty: enum_ty, tuple_fields: Vec::new(), named_fields: Vec::new() });
        enum_ty
    }

    pub fn add_def_ty(&mut self, def: GlobalDefId, generic_count: u16, ty: InferTyId) {
        self.defs.push(DefEntry { def, generic_count, shape: CtorShapeKind::None, ty, tuple_fields: Vec::new(), named_fields: Vec::new() });
    }

    pub fn add_generic_default(&mut self, def: GlobalDefId, param_idx: u16, default_ty: InferTyId) {
        self.generic_defaults.push((def, param_idx, default_ty));
    }

    pub fn set_tuple_fields(&mut self, def: GlobalDefId, fields: &[InferTyId]) {
        if let Some(d) = self.defs.iter_mut().find(|e| e.def == def) {
            d.tuple_fields = fields.to_vec();
            for (idx, &fty) in fields.iter().enumerate() {
                d.named_fields.push((idx.to_string(), fty, GlobalDefId::NONE));
            }
        }
    }

    pub fn add_field(&mut self, owner_def: GlobalDefId, name: &str, ty: InferTyId) {
        self.add_field_with_def(owner_def, name, ty, GlobalDefId::NONE);
    }

    pub fn add_field_with_def(&mut self, owner_def: GlobalDefId, name: &str, ty: InferTyId, field_def: GlobalDefId) {
        if let Some(d) = self.defs.iter_mut().find(|e| e.def == owner_def) {
            d.named_fields.push((name.to_string(), ty, field_def));
        }
    }

    /// Registers a parameterized `impl<T..> Trait<Args..> for SelfTy { type Assoc = OutputTy; }` rule.
    pub fn add_assoc_rule(&mut self, assoc_item: TyLangItem, generic_count: u16, for_ty: InferTyId, trait_args: &[InferTyId], output_ty: InferTyId) {
        self.assoc_rules.push(AssocRule { assoc_item, generic_count, for_ty, trait_args: trait_args.to_vec(), output_ty });
    }

    pub fn add_param_fn_bound(&mut self, owner_def: GlobalDefId, param_idx: u16, trait_item: TyLangItem, inputs: &[InferTyId], output: InferTyId) {
        self.fn_bounds.push(FnBoundEntry { owner_def, param_idx, trait_item, inputs: inputs.to_vec(), output });
    }

    #[inline]
    pub fn param(&mut self, idx: u16) -> InferTyId {
        self.templates.mk_param(idx, GlobalDefId::NONE)
    }
}

impl TyDefDb for MemTyDefDb {
    fn fn_sig(&self, def: GlobalDefId, arena: &mut InferTyArena, inputs_out: &mut Vec<InferTyId>) -> Option<(u16, InferTyId)> {
        let entry = self.fns.iter().find(|f| f.def == def)?;
        inputs_out.clear();
        for &inp in &entry.inputs {
            inputs_out.push(clone_ty_into(&self.templates, inp, arena));
        }
        let out = clone_ty_into(&self.templates, entry.output, arena);
        Some((entry.generic_count, out))
    }

    fn def_ty(&self, def: GlobalDefId, arena: &mut InferTyArena) -> Option<(u16, CtorShapeKind, InferTyId)> {
        let entry = self.defs.iter().find(|d| d.def == def)?;
        Some((entry.generic_count, entry.shape, clone_ty_into(&self.templates, entry.ty, arena)))
    }

    fn ctor_tuple_fields(&self, def: GlobalDefId, arena: &mut InferTyArena, fields_out: &mut Vec<InferTyId>) -> Option<u16> {
        let entry = self.defs.iter().find(|d| d.def == def)?;
        if entry.shape != CtorShapeKind::Tuple {
            return None;
        }
        fields_out.clear();
        for &f in &entry.tuple_fields {
            fields_out.push(clone_ty_into(&self.templates, f, arena));
        }
        Some(entry.generic_count)
    }

    fn field_ty(&self, owner_def: GlobalDefId, name: &str, arena: &mut InferTyArena) -> Option<(u16, InferTyId)> {
        self.field_ty_and_def(owner_def, name, arena).map(|(gc, ty, _)| (gc, ty))
    }

    fn field_ty_and_def(&self, owner_def: GlobalDefId, name: &str, arena: &mut InferTyArena) -> Option<(u16, InferTyId, GlobalDefId)> {
        let entry = self.defs.iter().find(|d| d.def == owner_def)?;
        let &(_, fty, fdef) = entry.named_fields.iter().find(|(n, _, _)| n == name)?;
        Some((entry.generic_count, clone_ty_into(&self.templates, fty, arena), fdef))
    }

    fn lang_item(&self, item: TyLangItem) -> Option<GlobalDefId> {
        self.lang_items.iter().find(|&&(k, _)| k == item).map(|&(_, d)| d)
    }

    fn lang_item_of_def(&self, def: GlobalDefId) -> Option<TyLangItem> {
        self.lang_items.iter().find(|&&(_, d)| d == def).map(|&(k, _)| k)
    }

    fn project_assoc_ty(&self, assoc_item: TyLangItem, self_ty: InferTyId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> Option<InferTyId> {
        for rule in &self.assoc_rules {
            if rule.assoc_item != assoc_item || rule.trait_args.len() != trait_args.len() {
                continue;
            }
            let snap = vars.start_snapshot();
            let mut subst = InlineTyBuf::new();
            for _ in 0..rule.generic_count {
                subst.push(vars.new_var(arena, TyVarKind::General).1);
            }
            let for_tpl = clone_ty_into(&self.templates, rule.for_ty, arena);
            let cand_for = instantiate_in_arena(arena, for_tpl, subst.as_slice());
            let mut ok = unify::equate(arena, vars, self_ty, cand_for).is_ok();
            if ok {
                for (i, &actual_arg) in trait_args.iter().enumerate() {
                    let arg_tpl = clone_ty_into(&self.templates, rule.trait_args[i], arena);
                    let cand_arg = instantiate_in_arena(arena, arg_tpl, subst.as_slice());
                    if unify::coerce(arena, vars, actual_arg, cand_arg).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                vars.commit(snap);
                let out_tpl = clone_ty_into(&self.templates, rule.output_ty, arena);
                let inst = instantiate_in_arena(arena, out_tpl, subst.as_slice());
                return Some(vars.shallow_resolve(arena, inst));
            }
            vars.rollback_to(snap);
        }
        None
    }

    fn param_fn_bound(&self, owner_def: GlobalDefId, param_idx: u16, arena: &mut InferTyArena, inputs_out: &mut Vec<InferTyId>) -> Option<(TyLangItem, InferTyId)> {
        let b = self.fn_bounds.iter().find(|e| e.owner_def == owner_def && e.param_idx == param_idx)?;
        inputs_out.clear();
        for &inp in &b.inputs {
            inputs_out.push(clone_ty_into(&self.templates, inp, arena));
        }
        Some((b.trait_item, clone_ty_into(&self.templates, b.output, arena)))
    }

    fn cstr_def(&self) -> Option<GlobalDefId> {
        self.cstr
    }

    fn project_trait_assoc(
        &self,
        trait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        self_ty: InferTyId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
    ) -> Option<InferTyId> {
        self.query_trait_assoc_rule(trait_def, assoc_def, self_ty, trait_args, arena, vars)
    }

    fn inherent_method_cands(&self, target: ImplTargetKey, method_name: &str, _arena: &mut InferTyArena, out: &mut Vec<MethodCand>) {
        self.query_inherent_methods(target, method_name, out);
    }

    fn trait_impl_method_cands(
        &self,
        target: ImplTargetKey,
        trait_def: GlobalDefId,
        method_name: &str,
        step_ty: InferTyId,
        arena: &mut InferTyArena,
        vars: &mut UnionFindTable,
        out: &mut Vec<MethodCand>,
    ) {
        self.query_trait_impl_methods(target, trait_def, method_name, step_ty, arena, vars, out);
    }

    fn trait_decl_method_cand(&self, trait_def: GlobalDefId, trait_args: &[InferTyId], self_ty: InferTyId, method_name: &str, arena: &mut InferTyArena) -> Option<MethodCand> {
        self.query_trait_decl_method(trait_def, trait_args, self_ty, method_name, arena, 0)
    }

    fn trait_supertraits(&self, trait_def: GlobalDefId, out: &mut Vec<GlobalDefId>) {
        for &(sub, sup) in &self.supertraits {
            if sub == trait_def && !out.contains(&sup) {
                out.push(sup);
            }
        }
    }

    fn param_trait_bounds(&self, owner_def: GlobalDefId, param_idx: u16, out: &mut Vec<GlobalDefId>) {
        for &(od, pi, tdef) in &self.param_bounds {
            if od == owner_def && pi == param_idx && !out.contains(&tdef) {
                out.push(tdef);
            }
        }
    }

    fn check_blanket_method_bounds(&self, cand: &MethodCand, impl_subst: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        self.query_blanket_method_bounds(cand, impl_subst, arena, vars)
    }

    fn trait_of_assoc_fn(&self, assoc_fn: GlobalDefId) -> Option<TraitAssocFnMeta> {
        self.query_trait_of_assoc_fn(assoc_fn)
    }

    fn trait_assoc_fn_by_name(&self, trait_def: GlobalDefId, fn_name: &str) -> Option<TraitAssocFnMeta> {
        self.query_trait_assoc_fn_by_name(trait_def, fn_name)
    }

    fn owner_where_obligations(&self, owner_def: GlobalDefId, arena: &mut InferTyArena, out: &mut Vec<WhereObligation>) {
        self.query_owner_where_obligations(owner_def, arena, out);
    }

    fn solve_trait_obligation(&self, trait_def: GlobalDefId, self_ty: InferTyId, trait_args: &[InferTyId], arena: &mut InferTyArena, vars: &mut UnionFindTable) -> bool {
        self.query_solve_trait_obligation(trait_def, self_ty, trait_args, arena, vars, 0)
    }

    fn project_param_assoc(
        &self,
        owner_def: GlobalDefId,
        param_idx: u16,
        trait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        trait_args: &[InferTyId],
        arena: &mut InferTyArena,
    ) -> Option<InferTyId> {
        self.query_project_param_assoc(owner_def, param_idx, trait_def, assoc_def, trait_args, arena)
    }

    fn generic_param_default_ty(&self, def: GlobalDefId, param_idx: u16, arena: &mut InferTyArena) -> Option<InferTyId> {
        self.generic_defaults.iter().find(|&&(d, idx, _)| d == def && idx == param_idx).map(|&(_, _, t)| clone_ty_into(&self.templates, t, arena))
    }

    fn std_macro_ty(&self, kind: StdMacroTy, arena: &mut InferTyArena) -> Option<InferTyId> {
        match kind {
            StdMacroTy::Vec(elem) => {
                let vd = self.vec_def?;
                if let Some(gd) = self.global_alloc_def {
                    let gty = arena.mk_adt(gd, &[]);
                    Some(arena.mk_adt(vd, &[elem, gty]))
                } else {
                    Some(arena.mk_adt(vd, &[elem]))
                }
            }
            StdMacroTy::String => self.lang_item(TyLangItem::String).map(|d| arena.mk_adt(d, &[])),
            StdMacroTy::FmtArguments => self.fmt_args_def.map(|d| arena.mk_adt(d, &[])),
            StdMacroTy::FmtResult => {
                let res_d = self.lang_item(TyLangItem::Result)?;
                let err_ty = self.fmt_err_def.map_or(InferTyId::UNKNOWN, |d| arena.mk_adt(d, &[]));
                Some(arena.mk_adt(res_d, &[InferTyId::UNIT, err_ty]))
            }
            StdMacroTy::OptionStr => self.lang_item(TyLangItem::Option).map(|d| arena.mk_adt(d, &[InferTyId::STR_REF])),
            StdMacroTy::ThreadLocalKey(_) => None,
        }
    }
}
