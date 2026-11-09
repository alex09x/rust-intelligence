/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Local CST `impl` / `trait` method candidates, supertraits, associated-type projections
//! (`Deref::Target` & `<T as Trait>::Assoc`), and unindexed UFCS lookup on [`LocalCstDefs`].

use super::autoderef::impl_target_key_of_ty;
use super::db::{MethodCand, TraitAssocFnMeta, TyDefDb};
use super::local_defs::{count_cst_generics, LocalCstDefs};
use super::lower_ty::{effective_target_def, LowerTyEnv};
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{CrateId, FileNameRes, GlobalDefId, ImplTargetKey, ItemKind};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::tree::{NodeId, SyntaxTree};

impl LocalCstDefs {
    #[allow(clippy::too_many_arguments)]
    pub fn inherent_method_cands(
        &self,
        target: ImplTargetKey,
        method_name: &str,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        out: &mut Vec<MethodCand>,
    ) {
        for imp in &self.impls {
            if imp.indexed_in_clean_db || imp.has_for || imp.is_negative {
                continue;
            }
            let Some(fn_node) = find_assoc_fn(st, imp.node, method_name, true) else {
                continue;
            };
            let (impl_gc, self_ty) = self.lower_impl_self_ty(imp.node, st, file_res, db, table);
            if impl_target_key_of_ty(&table.arena, self_ty) != target {
                continue;
            }
            let mdef = self.fn_def_for_node(fn_node);
            if !mdef.is_some() || out.iter().any(|c| c.method_def == mdef) {
                continue;
            }
            out.push(MethodCand {
                method_def: mdef,
                trait_def: GlobalDefId::NONE,
                bound_owner_def: mdef,
                container_generics: impl_gc,
                method_generics: count_cst_generics(st, fn_node),
                has_fixed_trait_subst: false,
                fixed_self_ty: InferTyId::NONE,
                fixed_trait_args: super::ty::TySliceRange::EMPTY,
                is_blanket_impl: false,
                impl_owner: GlobalDefId::new(CrateId::LOCAL_OVERLAY, imp.node.0),
                cst_fn_node: fn_node.0,
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn trait_impl_method_cands(
        &self,
        target: ImplTargetKey,
        trait_def: GlobalDefId,
        method_name: &str,
        step_ty: InferTyId,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        out: &mut Vec<MethodCand>,
    ) {
        for imp in &self.impls {
            if imp.indexed_in_clean_db || !imp.has_for || imp.is_negative {
                continue;
            }
            let Some(tp) = imp.trait_path else {
                continue;
            };
            if file_res.path_tail_ref_in_st(st, tp).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target)) != trait_def {
                continue;
            }
            let Some((imp_tr, tr_args, self_ty)) = self.lower_trait_impl_header(imp.node, imp.impl_generics, st, file_res, db, table) else {
                continue;
            };
            if imp_tr != trait_def {
                continue;
            }
            let is_blanket = matches!(table.arena.kind(self_ty), InferTyKind::Param { .. });
            if target == ImplTargetKey::Param {
                if !is_blanket {
                    continue;
                }
            } else if is_blanket || impl_target_key_of_ty(&table.arena, self_ty) != target {
                continue;
            }
            let imp_owner = GlobalDefId::new(CrateId::LOCAL_OVERLAY, imp.node.0);
            if is_blanket && out.iter().any(|c| c.impl_owner == imp_owner) {
                continue;
            }
            if !probe_impl_self_compat(table, imp.impl_generics, self_ty, step_ty) {
                continue;
            }
            if let Some(fn_node) = find_assoc_fn(st, imp.node, method_name, true) {
                let mdef = self.fn_def_for_node(fn_node);
                if mdef.is_some() && !out.iter().any(|c| c.method_def == mdef && !c.has_fixed_trait_subst) {
                    out.push(MethodCand {
                        method_def: mdef,
                        trait_def,
                        bound_owner_def: mdef,
                        container_generics: imp.impl_generics,
                        method_generics: count_cst_generics(st, fn_node),
                        has_fixed_trait_subst: false,
                        fixed_self_ty: InferTyId::NONE,
                        fixed_trait_args: super::ty::TySliceRange::EMPTY,
                        is_blanket_impl: is_blanket,
                        impl_owner: imp_owner,
                        cst_fn_node: fn_node.0,
                    });
                }
                continue;
            }
            let base = self
                .trait_decl_method_cand_direct(trait_def, tr_args.as_slice(), self_ty, method_name, st, table)
                .or_else(|| db.trait_decl_method_cand(trait_def, tr_args.as_slice(), self_ty, method_name, &mut table.arena));
            if let Some(mut dc) = base {
                dc.container_generics = imp.impl_generics;
                dc.has_fixed_trait_subst = true;
                dc.fixed_self_ty = self_ty;
                dc.fixed_trait_args = table.arena.intern_slice(tr_args.as_slice());
                dc.is_blanket_impl = is_blanket;
                dc.impl_owner = imp_owner;
                out.push(dc);
            }
        }
    }

    pub fn dedup_inherent_cands(before: usize, cands: &mut Vec<MethodCand>) {
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

    #[allow(clippy::too_many_arguments)]
    pub fn collapse_overloaded_trait_cands(
        &self,
        start: usize,
        tdef: GlobalDefId,
        method_name: &str,
        probe_ty: InferTyId,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        cands: &mut Vec<MethodCand>,
    ) {
        if cands.len() - start <= 1 {
            return;
        }
        let any_non_blanket = cands[start..].iter().any(|c| !c.is_blanket_impl);
        if let Some(mut poly) = self.trait_decl_method_cand(tdef, &[], probe_ty, method_name, st, file_res, db, table) {
            if poly.fixed_trait_args.len > 0 {
                poly.is_blanket_impl = !any_non_blanket;
                cands.truncate(start);
                cands.push(poly);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn trait_decl_method_cand(
        &self,
        trait_def: GlobalDefId,
        trait_args: &[InferTyId],
        self_ty: InferTyId,
        method_name: &str,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
    ) -> Option<MethodCand> {
        self.trait_decl_method_rec(trait_def, trait_args, self_ty, method_name, st, file_res, db, table, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn trait_decl_method_rec(
        &self,
        trait_def: GlobalDefId,
        trait_args: &[InferTyId],
        self_ty: InferTyId,
        method_name: &str,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        depth: u8,
    ) -> Option<MethodCand> {
        if depth > 6 || !trait_def.is_some() {
            return None;
        }
        if let Some(c) = self.trait_decl_method_cand_direct(trait_def, trait_args, self_ty, method_name, st, table) {
            return Some(c);
        }
        if let Some(c) = db.trait_decl_method_cand(trait_def, trait_args, self_ty, method_name, &mut table.arena) {
            return Some(c);
        }
        let mut supers = [GlobalDefId::NONE; 8];
        let slen = self.collect_supertraits_fixed(trait_def, st, file_res, db, &mut supers);
        for &sup in &supers[..slen] {
            if let Some(c) = self.trait_decl_method_rec(sup, &[], self_ty, method_name, st, file_res, db, table, depth + 1) {
                return Some(c);
            }
        }
        None
    }

    pub(super) fn trait_decl_method_cand_direct(
        &self,
        trait_def: GlobalDefId,
        trait_args: &[InferTyId],
        self_ty: InferTyId,
        method_name: &str,
        st: &SyntaxTree<'_>,
        table: &mut InferenceTable,
    ) -> Option<MethodCand> {
        let tr = self.traits.iter().find(|t| t.def == trait_def)?;
        let fn_node = find_assoc_fn(st, tr.node, method_name, true)?;
        let mdef = self.fn_def_for_node(fn_node);
        if !mdef.is_some() {
            return None;
        }
        let mut padded = InlineTyBuf::new();
        for i in 0..tr.trait_generics as usize {
            padded.push(trait_args.get(i).copied().unwrap_or(InferTyId::NONE));
        }
        let args_range = table.arena.intern_slice(padded.as_slice());
        Some(MethodCand {
            method_def: mdef,
            trait_def,
            bound_owner_def: mdef,
            container_generics: 0,
            method_generics: count_cst_generics(st, fn_node),
            has_fixed_trait_subst: true,
            fixed_self_ty: self_ty,
            fixed_trait_args: args_range,
            is_blanket_impl: false,
            impl_owner: GlobalDefId::NONE,
            cst_fn_node: fn_node.0,
        })
    }

    pub fn trait_of_assoc_fn(&self, assoc_fn: GlobalDefId, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb) -> Option<TraitAssocFnMeta> {
        let item = self.find_item_by_def(assoc_fn).filter(|i| i.kind == ItemKind::AssocFn)?;
        let owner = item.impl_node?;
        let mgc = count_cst_generics(st, item.node);
        if st.kind(owner) == K::Trait {
            let tr = self.traits.iter().find(|t| t.node == owner)?;
            return Some(TraitAssocFnMeta { trait_def: tr.def, trait_fn_def: assoc_fn, trait_generics: tr.trait_generics, method_generics: mgc });
        }
        let imp = self.impls.iter().find(|im| im.node == owner && im.has_for && !im.is_negative)?;
        let tp = imp.trait_path?;
        let trait_def = file_res.path_tail_ref_in_st(st, tp).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target));
        if !trait_def.is_some() {
            return None;
        }
        let fn_name = norm_ident(st.text(st.child(item.node, K::Name)?));
        if let Some(tr) = self.traits.iter().find(|t| t.def == trait_def) {
            let tr_fn = find_assoc_fn(st, tr.node, fn_name, false)?;
            let tr_fn_def = self.fn_def_for_node(tr_fn);
            if tr_fn_def.is_some() {
                return Some(TraitAssocFnMeta { trait_def, trait_fn_def: tr_fn_def, trait_generics: tr.trait_generics, method_generics: mgc });
            }
        }
        db.trait_assoc_fn_by_name(trait_def, fn_name)
    }

    pub fn collect_supertraits_into(&self, trait_def: GlobalDefId, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, out: &mut Vec<GlobalDefId>) {
        db.trait_supertraits(trait_def, out);
        let Some(tr) = self.traits.iter().find(|t| t.def == trait_def) else {
            return;
        };
        if let Some(tbl) = st.child(tr.node, K::TypeBoundList) {
            for tb in st.children(tbl).filter(|&c| st.kind(c) == K::TypeBound) {
                if let Some(p) = st.child(tb, K::Path).and_then(|p| file_res.path_tail_ref_in_st(st, p)) {
                    let sdef = effective_target_def(p.target);
                    if sdef.is_some() && !out.contains(&sdef) {
                        out.push(sdef);
                    }
                }
            }
        }
    }

    fn collect_supertraits_fixed(&self, trait_def: GlobalDefId, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, out: &mut [GlobalDefId; 8]) -> usize {
        let mut len = 0usize;
        if let Some(tr) = self.traits.iter().find(|t| t.def == trait_def) {
            if let Some(tbl) = st.child(tr.node, K::TypeBoundList) {
                for tb in st.children(tbl).filter(|&c| st.kind(c) == K::TypeBound) {
                    if let Some(p) = st.child(tb, K::Path).and_then(|p| file_res.path_tail_ref_in_st(st, p)) {
                        let sdef = effective_target_def(p.target);
                        if sdef.is_some() && !out[..len].contains(&sdef) && len < out.len() {
                            out[len] = sdef;
                            len += 1;
                        }
                    }
                }
            }
        }
        let _ = db;
        len
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_trait_impl_header(
        &self,
        impl_node: NodeId,
        impl_gc: u16,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
    ) -> Option<(GlobalDefId, InlineTyBuf, InferTyId)> {
        let mut tys = st.children(impl_node).filter(|&c| st.kind(c).is_type());
        let tr_ty_node = tys.next()?;
        let self_node = tys.next()?;
        let tr_path = st.child(tr_ty_node, K::Path)?;
        let tr_def = file_res.path_tail_ref_in_st(st, tr_path).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target));
        let env =
            LowerTyEnv { st, file_res, db, local_defs: self, self_ty: None, impl_owner: Some(impl_node), impl_generics: impl_gc, item_owner: None, in_body: false, sub_map: None };
        let mut tr_args = InlineTyBuf::new();
        if let Some(tail) = st.children(tr_path).filter(|&c| st.kind(c) == K::PathSegment).last() {
            env.collect_segment_args(table, tail, false, &mut tr_args, 0);
        }
        let self_ty = env.lower_cst_ty(table, self_node);
        Some((tr_def, tr_args, self_ty))
    }
}

pub(super) fn find_assoc_fn(st: &SyntaxTree<'_>, container: NodeId, name: &str, require_self: bool) -> Option<NodeId> {
    let il = st.child(container, K::ItemList)?;
    for ch in st.children(il) {
        if st.kind(ch) != K::Fn {
            continue;
        }
        let Some(nn) = st.child(ch, K::Name) else {
            continue;
        };
        if norm_ident(st.text(nn)) != name {
            continue;
        }
        if require_self && !st.child(ch, K::ParamList).is_some_and(|pl| st.child(pl, K::SelfParam).is_some()) {
            continue;
        }
        return Some(ch);
    }
    None
}

fn probe_impl_self_compat(table: &mut InferenceTable, impl_gc: u16, impl_self_ty: InferTyId, step_ty: InferTyId) -> bool {
    let snap = table.start_snapshot();
    let mut subst = InlineTyBuf::new();
    for _ in 0..impl_gc {
        subst.push(table.new_ty_var());
    }
    let inst = table.instantiate(impl_self_ty, subst.as_slice());
    let ok = table.equate(inst, step_ty).is_ok();
    table.rollback_to(snap);
    ok
}
