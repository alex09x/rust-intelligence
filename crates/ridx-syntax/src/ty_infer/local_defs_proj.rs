/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Local CST associated-type projection (`Deref::Target` & `<T as Trait>::Assoc`) and unindexed
//! UFCS associated-function fallback on [`LocalCstDefs`].

use super::autoderef::impl_target_key_of_ty;
use super::db::{TyDefDb, TyLangItem};
use super::local_defs::LocalCstDefs;
use super::local_defs_impl::find_assoc_fn;
use super::lower_ty::{effective_target_def, LowerTyEnv};
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{FileNameRes, GlobalDefId, ImplTargetKey, ItemKind};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

impl LocalCstDefs {
    pub(super) fn resolve_local_trait_assoc_def(&self, trait_def: GlobalDefId, raw_assoc_def: GlobalDefId, assoc_name: &str, st: &SyntaxTree<'_>) -> GlobalDefId {
        if !assoc_name.is_empty() && trait_def.is_some() {
            if let Some(tr) = self.traits.iter().find(|t| t.def == trait_def) {
                if let Some(it) = self
                    .items
                    .iter()
                    .find(|i| i.impl_node == Some(tr.node) && i.kind == ItemKind::TypeAlias && st.child(i.node, K::Name).is_some_and(|n| norm_ident(st.text(n)) == assoc_name))
                {
                    return it.def;
                }
            }
        }
        raw_assoc_def
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_assoc_trait_and_args(
        &self,
        qtrait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        assoc_name: &str,
        sty: InferTyId,
        self_ty: Option<InferTyId>,
        impl_owner: Option<NodeId>,
        explicit_args: &[InferTyId],
        st: &SyntaxTree<'_>,
        table: &mut InferenceTable,
    ) -> (GlobalDefId, GlobalDefId, InlineTyBuf) {
        let mut args = InlineTyBuf::new();
        for &a in explicit_args {
            args.push(a);
        }
        let mut tdef = qtrait_def;
        if !tdef.is_some() {
            if let Some(owner) = impl_owner.filter(|&o| st.kind(o) == K::Trait) {
                if self_ty == Some(sty) {
                    if let Some(tr) = self.traits.iter().find(|t| t.node == owner) {
                        tdef = tr.def;
                        if args.as_slice().is_empty() {
                            for i in 0..tr.trait_generics {
                                args.push(table.arena.mk_param(1 + i, GlobalDefId::NONE));
                            }
                        }
                    }
                }
            }
            if !tdef.is_some() && self.traits.iter().any(|t| t.def == assoc_def) {
                tdef = assoc_def;
            }
            if !tdef.is_some() {
                if let Some(tr_node) = self.find_item_by_def(assoc_def).and_then(|i| i.impl_node) {
                    if let Some(tr) = self.traits.iter().find(|t| t.node == tr_node) {
                        tdef = tr.def;
                    }
                }
            }
        }
        let canon_assoc = self.resolve_local_trait_assoc_def(tdef, assoc_def, assoc_name, st);
        (tdef, canon_assoc, args)
    }

    pub fn project_local_deref(&self, self_ty: InferTyId, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, table: &mut InferenceTable) -> Option<InferTyId> {
        let deref_tr = db.lang_item(TyLangItem::Deref).unwrap_or(GlobalDefId::NONE);
        self.project_local_assoc_by_name(deref_tr, "Deref", "Target", self_ty, &[], st, file_res, db, table, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn project_trait_assoc(
        &self,
        trait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        self_ty: InferTyId,
        trait_args: &[InferTyId],
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
    ) -> Option<InferTyId> {
        self.project_trait_assoc_rec(trait_def, assoc_def, self_ty, trait_args, st, file_res, db, table, 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn project_trait_assoc_rec(
        &self,
        trait_def: GlobalDefId,
        assoc_def: GlobalDefId,
        self_ty: InferTyId,
        trait_args: &[InferTyId],
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        depth: u8,
    ) -> Option<InferTyId> {
        let assoc_name = self.find_item_by_def(assoc_def).and_then(|i| st.child(i.node, K::Name)).map(|nn| norm_ident(st.text(nn)));
        let aname = assoc_name?;
        self.project_local_assoc_by_name(trait_def, "", aname, self_ty, trait_args, st, file_res, db, table, depth)
    }

    #[allow(clippy::too_many_arguments)]
    fn project_local_assoc_by_name(
        &self,
        trait_def: GlobalDefId,
        fallback_trait_name: &str,
        assoc_name: &str,
        raw_self: InferTyId,
        actual_args: &[InferTyId],
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        depth: u8,
    ) -> Option<InferTyId> {
        if depth > 10 {
            return None;
        }
        let actual_self = table.shallow_resolve(raw_self);
        if matches!(table.arena.kind(actual_self), InferTyKind::Unknown | InferTyKind::Infer(_) | InferTyKind::Param { .. } | InferTyKind::Alias { .. }) {
            return None;
        }
        let args_have_infer = actual_args.iter().any(|&a| {
            let ra = table.shallow_resolve(a);
            matches!(table.arena.kind(ra), InferTyKind::Infer(_))
        });
        for want_blanket in [false, true] {
            let mut match_count = 0u8;
            let mut single_hit: Option<(NodeId, NodeId, u16, InferTyId, InlineTyBuf)> = None;
            for imp in &self.impls {
                if !imp.has_for || imp.is_negative || (imp.indexed_in_clean_db && trait_def.is_some() && !trait_def.is_overlay()) {
                    continue;
                }
                if let Some(tp) = imp.trait_path {
                    let tp_def = file_res.path_tail_ref_in_st(st, tp).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target));
                    let fast_ok =
                        (trait_def.is_some() && tp_def == trait_def) || (!fallback_trait_name.is_empty() && impl_trait_tail_name(st, imp.node) == Some(fallback_trait_name));
                    if !fast_ok {
                        continue;
                    }
                }
                let Some((imp_tr, imp_args, imp_self)) = self.lower_trait_impl_header(imp.node, imp.impl_generics, st, file_res, db, table) else {
                    continue;
                };
                let is_blanket = matches!(table.arena.kind(imp_self), InferTyKind::Param { .. });
                if is_blanket != want_blanket {
                    continue;
                }
                let tr_match = (trait_def.is_some() && imp_tr == trait_def) || (!fallback_trait_name.is_empty() && impl_trait_tail_name(st, imp.node) == Some(fallback_trait_name));
                if !tr_match {
                    continue;
                }
                let Some(il) = st.child(imp.node, K::ItemList) else {
                    continue;
                };
                let Some(ta_node) = st.children(il).find(|&c| st.kind(c) == K::TypeAlias && st.child(c, K::Name).is_some_and(|n| norm_ident(st.text(n)) == assoc_name)) else {
                    continue;
                };
                let Some(rhs_ty_node) = st.children(ta_node).find(|&c| st.kind(c).is_type()) else {
                    continue;
                };
                let snap = table.start_snapshot();
                let mut subst = InlineTyBuf::new();
                for _ in 0..imp.impl_generics {
                    subst.push(table.new_ty_var());
                }
                let inst_self = table.instantiate(imp_self, subst.as_slice());
                let mut ok = table.equate(actual_self, inst_self).is_ok();
                if ok && !actual_args.is_empty() {
                    if imp_args.as_slice().len() != actual_args.len() {
                        ok = false;
                    } else {
                        for (i, &aa) in actual_args.iter().enumerate() {
                            let ia = table.instantiate(imp_args.as_slice()[i], subst.as_slice());
                            if table.coerce(aa, ia).is_err() {
                                ok = false;
                                break;
                            }
                        }
                    }
                }
                if ok && is_blanket {
                    ok = super::trait_solve::check_cst_impl_bounds_in_table(self, imp.node, imp.impl_generics, subst.as_slice(), st, file_res, db, table, false, depth + 1);
                }
                if ok {
                    if !args_have_infer {
                        table.commit(snap);
                        let env = LowerTyEnv {
                            st,
                            file_res,
                            db,
                            local_defs: self,
                            self_ty: Some(imp_self),
                            impl_owner: Some(imp.node),
                            impl_generics: imp.impl_generics,
                            item_owner: Some(ta_node),
                            in_body: false,
                            sub_map: None,
                        };
                        let raw_rhs = env.lower_cst_ty_rec(table, rhs_ty_node, depth + 1);
                        let inst_rhs = table.instantiate(raw_rhs, subst.as_slice());
                        return Some(table.shallow_resolve(inst_rhs));
                    }
                    match_count += 1;
                    let mut res_subst = InlineTyBuf::new();
                    for &s in subst.as_slice() {
                        res_subst.push(table.shallow_resolve(s));
                    }
                    table.rollback_to(snap);
                    if match_count > 1 {
                        return None;
                    }
                    single_hit = Some((imp.node, ta_node, imp.impl_generics, imp_self, res_subst));
                    continue;
                }
                table.rollback_to(snap);
            }
            if let Some((imp_node, ta_node, impl_gc, imp_self, res_subst)) = single_hit {
                let rhs_ty_node = st.children(ta_node).find(|&c| st.kind(c).is_type())?;
                let env = LowerTyEnv {
                    st,
                    file_res,
                    db,
                    local_defs: self,
                    self_ty: Some(imp_self),
                    impl_owner: Some(imp_node),
                    impl_generics: impl_gc,
                    item_owner: Some(ta_node),
                    in_body: false,
                    sub_map: None,
                };
                let raw_rhs = env.lower_cst_ty_rec(table, rhs_ty_node, depth + 1);
                let inst_rhs = table.instantiate(raw_rhs, res_subst.as_slice());
                return Some(table.shallow_resolve(inst_rhs));
            }
        }
        if trait_def.is_some() {
            if let Some(tr) = self.traits.iter().find(|t| t.def == trait_def) {
                if let Some(tbl) = st.child(tr.node, K::TypeBoundList) {
                    for tb in st.children(tbl).filter(|&c| st.kind(c) == K::TypeBound) {
                        let sup =
                            st.child(tb, K::Path).and_then(|p| file_res.path_tail_ref_in_st(st, p)).map_or(GlobalDefId::NONE, |r| super::lower_ty::effective_target_def(r.target));
                        if sup.is_some() {
                            if let Some(p) = self.project_local_assoc_by_name(sup, "", assoc_name, actual_self, &[], st, file_res, db, table, depth + 1) {
                                return Some(p);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve_ufcs_assoc_fn(
        &self,
        target: ImplTargetKey,
        trait_def: GlobalDefId,
        fn_name: &str,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
    ) -> GlobalDefId {
        if trait_def.is_some() {
            if let Some(tr) = self.traits.iter().find(|t| t.def == trait_def) {
                if let Some(fn_node) = find_assoc_fn(st, tr.node, fn_name, false) {
                    return self.fn_def_for_node(fn_node);
                }
            }
            if let Some(meta) = db.trait_assoc_fn_by_name(trait_def, fn_name) {
                return meta.trait_fn_def;
            }
        }
        for imp in &self.impls {
            if imp.is_negative {
                continue;
            }
            let Some(fn_node) = find_assoc_fn(st, imp.node, fn_name, false) else {
                continue;
            };
            let (_, self_ty) = self.lower_impl_self_ty(imp.node, st, file_res, db, table);
            if impl_target_key_of_ty(&table.arena, self_ty) == target {
                return self.fn_def_for_node(fn_node);
            }
        }
        GlobalDefId::NONE
    }

    pub(super) fn assoc_ty_cst_bounds(&self, assoc_def: GlobalDefId, st: &SyntaxTree<'_>, file_res: &FileNameRes, out: &mut Vec<GlobalDefId>) {
        let Some(it) = self.find_item_by_def(assoc_def) else { return };
        for ch in st.children(it.node) {
            if st.kind(ch) == K::TypeBoundList {
                for tb in st.children(ch).filter(|&b| st.kind(b) == K::TypeBound) {
                    if let Some(p) = st.child(tb, K::Path).or_else(|| st.child(tb, K::PathType).and_then(|pt| st.child(pt, K::Path))) {
                        if let Some(r) = file_res.path_tail_ref_in_st(st, p) {
                            let td = effective_target_def(r.target);
                            if td.is_some() && !out.contains(&td) {
                                out.push(td);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn impl_trait_tail_name<'s>(st: &'s SyntaxTree<'s>, impl_node: NodeId) -> Option<&'s str> {
    let tr_ty = st.children(impl_node).find(|&c| st.kind(c).is_type())?;
    let p = st.child(tr_ty, K::Path)?;
    let tail = st.children(p).filter(|&c| st.kind(c) == K::PathSegment).last()?;
    let (_, tok) = st.own_tokens(tail).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent))?;
    Some(norm_ident(tok.text(st.src)))
}
