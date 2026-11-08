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
use super::lower_ty::LowerTyEnv;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{FileNameRes, GlobalDefId, ImplTargetKey};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

impl LocalCstDefs {
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
        let assoc_name = self.items.iter().find(|i| i.def == assoc_def).and_then(|i| st.child(i.node, K::Name)).map(|nn| norm_ident(st.text(nn)));
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
        for imp in &self.impls {
            if !imp.has_for || imp.is_negative {
                continue;
            }
            let Some((imp_tr, imp_args, imp_self)) = self.lower_trait_impl_header(imp.node, imp.impl_generics, st, file_res, db, table) else {
                continue;
            };
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
            if ok && !actual_args.is_empty() && imp_args.as_slice().len() == actual_args.len() {
                for (i, &aa) in actual_args.iter().enumerate() {
                    let ia = table.instantiate(imp_args.as_slice()[i], subst.as_slice());
                    if table.coerce(aa, ia).is_err() {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
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
                };
                let raw_rhs = env.lower_cst_ty_rec(table, rhs_ty_node, depth + 1);
                let inst_rhs = table.instantiate(raw_rhs, subst.as_slice());
                return Some(table.shallow_resolve(inst_rhs));
            }
            table.rollback_to(snap);
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
        }
        for imp in &self.impls {
            if imp.is_negative {
                continue;
            }
            let (_, self_ty) = self.lower_impl_self_ty(imp.node, st, file_res, db, table);
            if impl_target_key_of_ty(&table.arena, self_ty) == target {
                if let Some(fn_node) = find_assoc_fn(st, imp.node, fn_name, false) {
                    return self.fn_def_for_node(fn_node);
                }
            }
        }
        GlobalDefId::NONE
    }
}

fn impl_trait_tail_name<'s>(st: &'s SyntaxTree<'s>, impl_node: NodeId) -> Option<&'s str> {
    let tr_ty = st.children(impl_node).find(|&c| st.kind(c).is_type())?;
    let p = st.child(tr_ty, K::Path)?;
    let tail = st.children(p).filter(|&c| st.kind(c) == K::PathSegment).last()?;
    let (_, tok) = st.own_tokens(tail).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent))?;
    Some(norm_ident(tok.text(st.src)))
}
