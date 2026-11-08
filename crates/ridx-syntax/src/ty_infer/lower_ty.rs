/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! CST type node (`lower_cst_ty`) and `TokenTree` slice (`lower_tok_slice_ty`) lowering into
//! [`InferTyId`], supporting primitives, generic parameters, `Self`, ADTs, type aliases,
//! `Fn(A, B) -> R` parenthesized sugar, `impl Trait` / `dyn Trait`, and `<T as Trait>::Assoc`.

use super::db::{TyDefDb, TyLangItem};
use super::local_defs::{effective_local_def_id, LocalCstDefs};
use super::mem_ty_db::instantiate_in_arena;
use super::tok_ty_scan::{eval_const_uint_expr, next_sig_idx, rfind_sig_idx};
use super::ty::InferTyId;
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{FileNameRes, GenericBindId, GlobalDefId, ImplTargetKey, ItemKind, ResTarget};
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

/// Contextual bindings while lowering a CST or token-slice type.
pub struct LowerTyEnv<'a, 's> {
    pub st: &'a SyntaxTree<'s>,
    pub file_res: &'a FileNameRes,
    pub db: &'a dyn TyDefDb,
    pub local_defs: &'a LocalCstDefs,
    pub self_ty: Option<InferTyId>,
    /// Owner `Impl` node (if lowering inside an `impl` block) whose generics occupy `0..impl_generics`.
    pub impl_owner: Option<NodeId>,
    pub impl_generics: u16,
    /// Owner item node (`Fn`, `Struct`, `Enum`, `TypeAlias`, ...) whose generics start at `impl_generics`.
    pub item_owner: Option<NodeId>,
    /// When `true` (inside a function body), `_` and elided generic args allocate fresh `TyVar`s.
    pub in_body: bool,
}

impl<'a, 's> LowerTyEnv<'a, 's> {
    #[inline]
    fn resolve_generic_param(&self, table: &mut InferenceTable, gid: GenericBindId) -> InferTyId {
        let g = self.file_res.generic_data(gid);
        if g.is_const {
            return InferTyId::UNKNOWN;
        }
        let idx = if self.impl_owner == Some(g.owner_node) {
            g.index
        } else if self.item_owner == Some(g.owner_node) {
            self.impl_generics + g.index
        } else {
            g.index
        };
        table.arena.mk_param(idx, g.def)
    }

    #[inline]
    pub(super) fn placeholder_ty(&self, table: &mut InferenceTable) -> InferTyId {
        if self.in_body {
            table.new_ty_var()
        } else {
            InferTyId::UNKNOWN
        }
    }

    /// Lowers a CST type `node` (`k.is_type()`) into `table`.
    pub fn lower_cst_ty(&self, table: &mut InferenceTable, node: NodeId) -> InferTyId {
        self.lower_cst_ty_rec(table, node, 0)
    }

    fn lower_cst_ty_rec(&self, table: &mut InferenceTable, node: NodeId, depth: u8) -> InferTyId {
        if depth > 48 {
            return InferTyId::UNKNOWN;
        }
        match self.st.kind(node) {
            K::NeverType => InferTyId::NEVER,
            K::InferType => self.placeholder_ty(table),
            K::ParenType => self.st.children(node).find(|&c| self.st.kind(c).is_type()).map_or(InferTyId::UNIT, |c| self.lower_cst_ty_rec(table, c, depth + 1)),
            K::TupleType => {
                let mut elems = InlineTyBuf::new();
                for ch in self.st.children(node).filter(|&c| self.st.kind(c).is_type()) {
                    elems.push(self.lower_cst_ty_rec(table, ch, depth + 1));
                }
                table.arena.mk_tuple(elems.as_slice())
            }
            K::RefType => {
                let is_mut = self.st.own_tokens(node).any(|(_, t)| t.kind == T::Mut);
                let inner = self.st.children(node).find(|&c| self.st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |c| self.lower_cst_ty_rec(table, c, depth + 1));
                table.arena.mk_ref(is_mut, inner)
            }
            K::PtrType => {
                let is_mut = self.st.own_tokens(node).any(|(_, t)| t.kind == T::Mut);
                let inner = self.st.children(node).find(|&c| self.st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |c| self.lower_cst_ty_rec(table, c, depth + 1));
                table.arena.mk_ptr(is_mut, inner)
            }
            K::SliceType => {
                let elem = self.st.children(node).find(|&c| self.st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |c| self.lower_cst_ty_rec(table, c, depth + 1));
                table.arena.mk_slice(elem)
            }
            K::ArrayType => {
                let elem = self.st.children(node).find(|&c| self.st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |c| self.lower_cst_ty_rec(table, c, depth + 1));
                let len = self.st.children(node).find(|&c| self.st.kind(c).is_expr()).and_then(|e| eval_const_uint_expr(self.st, e)).unwrap_or(u64::MAX);
                table.arena.mk_array(elem, len)
            }
            K::FnPtrType => {
                let mut inputs = InlineTyBuf::new();
                if let Some(tt) = self.st.child(node, K::TokenTree) {
                    let toks = self.st.tokens_of(tt);
                    if let (Some(open), Some(close)) = (next_sig_idx(toks, 0), rfind_sig_idx(toks)) {
                        if toks[open].kind == T::LParen && open < close {
                            self.lower_fn_ptr_params(table, &toks[open + 1..close], &mut inputs, depth + 1);
                        }
                    }
                }
                let out = self
                    .st
                    .child(node, K::RetType)
                    .and_then(|rt| self.st.children(rt).find(|&c| self.st.kind(c).is_type()))
                    .map_or(InferTyId::UNIT, |c| self.lower_cst_ty_rec(table, c, depth + 1));
                table.arena.mk_fn_ptr(inputs.as_slice(), out)
            }
            K::PathType => {
                let Some(p) = self.st.child(node, K::Path) else {
                    return InferTyId::UNKNOWN;
                };
                self.lower_cst_path_ty(table, p, false, false, depth + 1)
            }
            K::DynTraitType | K::ImplTraitType => {
                let is_opaque = self.st.kind(node) == K::ImplTraitType;
                for ch in self.st.children(node) {
                    if self.st.kind(ch) == K::PathType {
                        if let Some(p) = self.st.child(ch, K::Path) {
                            return self.lower_cst_path_ty(table, p, !is_opaque, is_opaque, depth + 1);
                        }
                    } else if self.st.kind(ch) == K::TypeBoundList {
                        for tb in self.st.children(ch).filter(|&b| self.st.kind(b) == K::TypeBound) {
                            if self.st.own_tokens(tb).any(|(_, t)| t.kind == T::Question) {
                                continue;
                            }
                            if let Some(p) = self.st.child(tb, K::Path) {
                                return self.lower_cst_path_ty(table, p, !is_opaque, is_opaque, depth + 1);
                            }
                        }
                    }
                }
                InferTyId::UNKNOWN
            }
            _ => InferTyId::UNKNOWN,
        }
    }

    pub(super) fn lower_cst_path_ty(&self, table: &mut InferenceTable, path_node: NodeId, as_dyn: bool, as_opaque: bool, depth: u8) -> InferTyId {
        let mut qself_ty: Option<InferTyId> = None;
        let mut qtrait_def = GlobalDefId::NONE;
        if let Some(first_seg) = self.st.child(path_node, K::PathSegment) {
            if self.st.own_tokens(first_seg).next().is_some_and(|(_, t)| t.kind == T::Lt) {
                if let Some(tn) = self.st.children(first_seg).find(|&c| self.st.kind(c).is_type()) {
                    qself_ty = Some(self.lower_cst_ty_rec(table, tn, depth + 1));
                }
                if let Some(tp) = self.st.child(first_seg, K::Path) {
                    if let Some(tr_ref) = self.file_res.path_tail_ref(tp) {
                        qtrait_def = effective_target_def(tr_ref.target);
                    }
                }
            }
        }
        let tail_seg = self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment).last();
        let mut args = InlineTyBuf::new();
        if let Some(seg) = tail_seg {
            self.collect_segment_args(table, seg, &mut args, depth + 1);
        }

        let Some(rref) = self.file_res.path_tail_ref(path_node) else {
            return InferTyId::UNKNOWN;
        };
        self.finish_resolved_type(table, rref.target, args.as_slice(), qself_ty, qtrait_def, as_dyn, as_opaque, depth)
    }

    pub(super) fn collect_segment_args(&self, table: &mut InferenceTable, seg: NodeId, out: &mut InlineTyBuf, depth: u8) {
        if let Some(gal) = self.st.child(seg, K::GenericArgList) {
            for ga in self.st.children(gal) {
                match self.st.kind(ga) {
                    K::TypeArg | K::AssocTypeArg => {
                        if let Some(tn) = self.st.children(ga).find(|&c| self.st.kind(c).is_type()) {
                            out.push(self.lower_cst_ty_rec(table, tn, depth));
                        }
                    }
                    K::ConstArg => out.push(InferTyId::UNKNOWN),
                    _ => {}
                }
            }
            return;
        }
        // Parenthesized `Fn(A, B) -> R` trait sugar -> `[Tuple([A, B]), R]`.
        let paren_or_tup = self.st.children(seg).find(|&c| matches!(self.st.kind(c), K::TupleType | K::ParenType));
        if let Some(pt) = paren_or_tup {
            let mut fn_ins = InlineTyBuf::new();
            for ch in self.st.children(pt).filter(|&c| self.st.kind(c).is_type()) {
                fn_ins.push(self.lower_cst_ty_rec(table, ch, depth));
            }
            let tup = table.arena.mk_tuple(fn_ins.as_slice());
            let ret = self
                .st
                .child(seg, K::RetType)
                .and_then(|rt| self.st.children(rt).find(|&c| self.st.kind(c).is_type()))
                .map_or(InferTyId::UNIT, |c| self.lower_cst_ty_rec(table, c, depth));
            out.push(tup);
            out.push(ret);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_resolved_type(
        &self,
        table: &mut InferenceTable,
        target: ResTarget,
        explicit_args: &[InferTyId],
        qself_ty: Option<InferTyId>,
        qtrait_def: GlobalDefId,
        as_dyn: bool,
        as_opaque: bool,
        depth: u8,
    ) -> InferTyId {
        match target {
            ResTarget::Prim(p) => InferTyId::from_prim(p),
            ResTarget::Generic(gid) => self.resolve_generic_param(table, gid),
            ResTarget::SelfTy { target: imp_t, .. } => {
                if let Some(sty) = self.self_ty {
                    return sty;
                }
                match imp_t {
                    ImplTargetKey::Adt(d) if d.is_some() => table.arena.mk_adt(d, &[]),
                    ImplTargetKey::Prim(p) => InferTyId::from_prim(p),
                    _ => InferTyId::UNKNOWN,
                }
            }
            ResTarget::Def { def, local, kind } => {
                let eff_def = effective_local_def_id(def, local);
                if as_opaque {
                    return table.arena.mk_opaque(eff_def, explicit_args);
                }
                if as_dyn || kind == ItemKind::Trait {
                    return table.arena.mk_dyn(eff_def, explicit_args);
                }
                match kind {
                    ItemKind::Struct | ItemKind::Enum | ItemKind::Union => {
                        let g_count =
                            self.db.def_ty(eff_def, &mut table.arena).map(|(g, ..)| g).or_else(|| self.local_defs.adt_generic_count(eff_def)).unwrap_or(explicit_args.len() as u16);
                        let mut full = InlineTyBuf::new();
                        for i in 0..g_count as usize {
                            full.push(explicit_args.get(i).copied().unwrap_or_else(|| self.placeholder_ty(table)));
                        }
                        table.arena.mk_adt(eff_def, full.as_slice())
                    }
                    ItemKind::TypeAlias => {
                        if let Some((g_count, _, target_ty)) = self.db.def_ty(eff_def, &mut table.arena) {
                            let mut full = InlineTyBuf::new();
                            for i in 0..g_count as usize {
                                full.push(explicit_args.get(i).copied().unwrap_or_else(|| self.placeholder_ty(table)));
                            }
                            return instantiate_in_arena(&mut table.arena, target_ty, full.as_slice());
                        }
                        if let Some((g_count, target_ty)) = self.local_defs.lower_type_alias(eff_def, self.st, self.file_res, self.db, table, depth + 1) {
                            let mut full = InlineTyBuf::new();
                            for i in 0..g_count as usize {
                                full.push(explicit_args.get(i).copied().unwrap_or_else(|| self.placeholder_ty(table)));
                            }
                            return instantiate_in_arena(&mut table.arena, target_ty, full.as_slice());
                        }
                        InferTyId::UNKNOWN
                    }
                    ItemKind::AssocTy => {
                        let sty = qself_ty.or(self.self_ty).unwrap_or(InferTyId::UNKNOWN);
                        if let Some(li) = self.db.lang_item_of_def(eff_def).or_else(|| self.db.lang_item_of_def(qtrait_def).and_then(trait_default_assoc)) {
                            if let Some(proj) = self.db.project_assoc_ty(li, sty, explicit_args, &mut table.arena, &mut table.vars) {
                                return proj;
                            }
                        }
                        table.arena.mk_alias(qtrait_def, eff_def, sty, explicit_args)
                    }
                    _ => InferTyId::UNKNOWN,
                }
            }
            ResTarget::Local(_) => InferTyId::UNKNOWN,
        }
    }
}

fn trait_default_assoc(item: TyLangItem) -> Option<TyLangItem> {
    match item {
        TyLangItem::Try => Some(TyLangItem::TryOutput),
        TyLangItem::Future => Some(TyLangItem::FutureOutput),
        TyLangItem::IntoFuture => Some(TyLangItem::IntoFutureOutput),
        TyLangItem::Deref => Some(TyLangItem::DerefTarget),
        TyLangItem::Index => Some(TyLangItem::IndexOutput),
        TyLangItem::IntoIterator => Some(TyLangItem::IntoIteratorItem),
        TyLangItem::Iterator => Some(TyLangItem::IteratorItem),
        _ => None,
    }
}

#[inline]
pub(super) fn effective_target_def(target: ResTarget) -> GlobalDefId {
    match target {
        ResTarget::Def { def, local, .. } => effective_local_def_id(def, local),
        ResTarget::SelfTy { target: ImplTargetKey::Adt(d), .. } => d,
        _ => GlobalDefId::NONE,
    }
}
