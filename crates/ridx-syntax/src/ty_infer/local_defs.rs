/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! CST item index ([`LocalCstDefs`]) providing zero-index / dirty-overlay type, signature,
//! `impl Self`, type-alias, and `Fn*` generic-bound fallback directly from [`SyntaxTree`] + [`FileNameRes`].

use super::db::{CtorShapeKind, TyDefDb, TyLangItem};
use super::lower_ty::LowerTyEnv;
use super::ty::InferTyId;
use super::InferenceTable;
use crate::name_res::{CrateDefMap, CrateId, FileId, FileNameRes, GlobalDefId, ItemDb, ItemKind, LocalItemId};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

/// Produces a stable [`GlobalDefId`] for both indexed defs and unindexed local CST items.
#[inline]
pub fn effective_local_def_id(def: GlobalDefId, local: LocalItemId) -> GlobalDefId {
    if def.is_some() {
        def
    } else if local.is_some() {
        GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x8000_0000 | local.0)
    } else {
        GlobalDefId::NONE
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalItemNode {
    pub def: GlobalDefId,
    pub node: NodeId,
    pub impl_node: Option<NodeId>,
    pub kind: ItemKind,
    pub generic_count: u16,
    pub shape: CtorShapeKind,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalVariantNode {
    pub def: GlobalDefId,
    pub enum_def: GlobalDefId,
    pub enum_node: NodeId,
    pub enum_generics: u16,
    pub shape: CtorShapeKind,
    pub tok_start: u32,
    pub tok_end: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalImplNode {
    pub node: NodeId,
    pub impl_generics: u16,
    pub has_for: bool,
    pub is_negative: bool,
    pub trait_path: Option<NodeId>,
    pub indexed_in_clean_db: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalTraitNode {
    pub def: GlobalDefId,
    pub node: NodeId,
    pub trait_generics: u16,
}

/// Reusable per-file CST definition table for unindexed or dirty-overlay items.
#[derive(Clone, Debug, Default)]
pub struct LocalCstDefs {
    pub(super) items: Vec<LocalItemNode>,
    pub(super) variants: Vec<LocalVariantNode>,
    pub(super) impls: Vec<LocalImplNode>,
    pub(super) traits: Vec<LocalTraitNode>,
}

impl LocalCstDefs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_file(st: &SyntaxTree<'_>, def_map: Option<&CrateDefMap>, file: FileId) -> Self {
        let mut out = Self::new();
        out.index_file(st, def_map, file);
        out
    }

    #[inline]
    pub fn clear(&mut self) {
        self.items.clear();
        self.variants.clear();
        self.impls.clear();
        self.traits.clear();
    }

    /// Indexes all items in `st` into reusable buffers (`0` heap allocations when warm).
    pub fn index_file(&mut self, st: &SyntaxTree<'_>, def_map: Option<&CrateDefMap>, file: FileId) {
        self.index_file_with_item_db(st, def_map, None, file);
    }

    /// Indexes all items in `st`, resolving clean `.ridx` definitions through `item_db` when provided.
    pub fn index_file_with_item_db(&mut self, st: &SyntaxTree<'_>, def_map: Option<&CrateDefMap>, item_db: Option<&dyn ItemDb>, file: FileId) {
        self.clear();
        let clean_db = item_db.is_some() && def_map.is_some_and(|dm| !dm.is_dirty_file(file));
        self.scan_container(st, st.root(), None, def_map, item_db, file, clean_db);
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_container(
        &mut self,
        st: &SyntaxTree<'_>,
        container: NodeId,
        impl_node: Option<NodeId>,
        def_map: Option<&CrateDefMap>,
        item_db: Option<&dyn ItemDb>,
        file: FileId,
        clean_mod_scope: bool,
    ) {
        for ch in st.children(container) {
            match st.kind(ch) {
                K::Module | K::ExternBlock => {
                    if let Some(il) = st.child(ch, K::ItemList) {
                        self.scan_container(st, il, None, def_map, item_db, file, clean_mod_scope);
                    }
                }
                K::Impl => {
                    let has_for = st.own_tokens(ch).any(|(_, t)| t.kind == T::For);
                    let is_negative = has_for && st.own_tokens(ch).any(|(_, t)| t.kind == T::Bang);
                    let trait_path = if has_for { st.children(ch).find(|&c| st.kind(c).is_type()).and_then(|tn| st.child(tn, K::Path)) } else { None };
                    self.impls.push(LocalImplNode { node: ch, impl_generics: count_cst_generics(st, ch), has_for, is_negative, trait_path, indexed_in_clean_db: clean_mod_scope });
                    if let Some(il) = st.child(ch, K::ItemList) {
                        self.scan_container(st, il, Some(ch), def_map, item_db, file, clean_mod_scope);
                    }
                }
                K::Trait => {
                    let def = resolve_item_node_def(st, ch, false, def_map, item_db, file);
                    if def.is_some() {
                        self.traits.push(LocalTraitNode { def, node: ch, trait_generics: count_cst_generics(st, ch) });
                    }
                    if let Some(il) = st.child(ch, K::ItemList) {
                        self.scan_container(st, il, Some(ch), def_map, item_db, file, clean_mod_scope);
                    }
                }
                K::Fn => {
                    let def = resolve_item_node_def(st, ch, impl_node.is_some(), def_map, item_db, file);
                    if def.is_some() {
                        let parent_gc = impl_node.map_or(0, |im| if st.kind(im) == K::Trait { 1 + count_cst_generics(st, im) } else { count_cst_generics(st, im) });
                        let gc = parent_gc + count_cst_generics(st, ch);
                        let kind = if impl_node.is_some() { ItemKind::AssocFn } else { ItemKind::Fn };
                        self.items.push(LocalItemNode { def, node: ch, impl_node, kind, generic_count: gc, shape: CtorShapeKind::None });
                    }
                    if let Some(body) = st.child(ch, K::BlockExpr) {
                        self.scan_block_items(st, body, def_map, item_db, file, 0);
                    }
                }
                K::Struct | K::Union => {
                    let def = resolve_item_node_def(st, ch, false, def_map, item_db, file);
                    if def.is_some() {
                        let gc = count_cst_generics(st, ch);
                        let kind = if st.kind(ch) == K::Struct { ItemKind::Struct } else { ItemKind::Union };
                        let shape = super::local_defs_adt::detect_struct_shape(st, ch);
                        self.items.push(LocalItemNode { def, node: ch, impl_node: None, kind, generic_count: gc, shape });
                    }
                }
                K::Enum => {
                    let def = resolve_item_node_def(st, ch, false, def_map, item_db, file);
                    if def.is_some() {
                        let gc = count_cst_generics(st, ch);
                        self.items.push(LocalItemNode { def, node: ch, impl_node: None, kind: ItemKind::Enum, generic_count: gc, shape: CtorShapeKind::None });
                        self.index_enum_variants(st, ch, def, gc, def_map, file);
                    }
                }
                K::TypeAlias => {
                    let def = resolve_item_node_def(st, ch, impl_node.is_some(), def_map, item_db, file);
                    if def.is_some() {
                        let gc = count_cst_generics(st, ch);
                        self.items.push(LocalItemNode { def, node: ch, impl_node, kind: ItemKind::TypeAlias, generic_count: gc, shape: CtorShapeKind::None });
                    }
                }
                K::Const | K::Static => {
                    let def = resolve_item_node_def(st, ch, impl_node.is_some(), def_map, item_db, file);
                    if def.is_some() {
                        let kind = if st.kind(ch) == K::Static { ItemKind::Static } else { ItemKind::Const };
                        self.items.push(LocalItemNode { def, node: ch, impl_node, kind, generic_count: 0, shape: CtorShapeKind::None });
                    }
                }
                _ => {}
            }
        }
    }

    fn scan_block_items(&mut self, st: &SyntaxTree<'_>, block: NodeId, def_map: Option<&CrateDefMap>, item_db: Option<&dyn ItemDb>, file: FileId, depth: u16) {
        if depth > 96 {
            return;
        }
        self.scan_container(st, block, None, def_map, item_db, file, false);
        for ch in st.children(block) {
            if !st.kind(ch).is_item() {
                self.scan_block_items(st, ch, def_map, item_db, file, depth + 1);
            }
        }
    }

    #[inline]
    pub fn adt_generic_count(&self, def: GlobalDefId) -> Option<u16> {
        self.items.iter().find(|i| i.def == def).map(|i| i.generic_count)
    }

    #[inline]
    pub fn fn_def_for_node(&self, fn_node: NodeId) -> GlobalDefId {
        self.items.iter().find(|i| i.node == fn_node).map_or(GlobalDefId::NONE, |i| i.def)
    }

    pub fn owner_generics_and_self_ty(
        &self,
        owner: Option<NodeId>,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
    ) -> (u16, Option<InferTyId>) {
        match owner {
            Some(im) if st.kind(im) == K::Trait => (1 + count_cst_generics(st, im), Some(table.arena.mk_param(0, GlobalDefId::NONE))),
            Some(im) => {
                let (gc, sty) = self.lower_impl_self_ty(im, st, file_res, db, table);
                (gc, Some(sty))
            }
            None => (0, None),
        }
    }

    pub fn lower_impl_self_ty(&self, impl_node: NodeId, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, table: &mut InferenceTable) -> (u16, InferTyId) {
        let impl_gc = count_cst_generics(st, impl_node);
        let has_for = st.own_tokens(impl_node).any(|(_, t)| t.kind == T::For);
        let mut tys = st.children(impl_node).filter(|&c| st.kind(c).is_type());
        let self_node = if has_for {
            let _ = tys.next();
            tys.next()
        } else {
            tys.next()
        };
        let Some(sn) = self_node else {
            return (impl_gc, InferTyId::UNKNOWN);
        };
        let env = LowerTyEnv { st, file_res, db, local_defs: self, self_ty: None, impl_owner: Some(impl_node), impl_generics: impl_gc, item_owner: None, in_body: false };
        (impl_gc, env.lower_cst_ty(table, sn))
    }

    pub fn lower_type_alias(
        &self,
        def: GlobalDefId,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        depth: u8,
    ) -> Option<(u16, InferTyId)> {
        if depth > 16 {
            return None;
        }
        let item = self.items.iter().find(|i| i.def == def && i.kind == ItemKind::TypeAlias)?;
        let ty_node = st.children(item.node).find(|&c| st.kind(c).is_type())?;
        let env = LowerTyEnv { st, file_res, db, local_defs: self, self_ty: None, impl_owner: None, impl_generics: 0, item_owner: Some(item.node), in_body: false };
        Some((item.generic_count, env.lower_cst_ty_rec(table, ty_node, depth + 1)))
    }

    pub fn const_or_static_ty(&self, def: GlobalDefId, st: &SyntaxTree<'_>, file_res: &FileNameRes, db: &dyn TyDefDb, table: &mut InferenceTable) -> Option<InferTyId> {
        let item = self.items.iter().find(|i| i.def == def && matches!(i.kind, ItemKind::Const | ItemKind::Static))?;
        let self_ty = item.impl_node.map(|im| self.lower_impl_self_ty(im, st, file_res, db, table).1);
        let ty_node = st.children(item.node).find(|&c| st.kind(c).is_type())?;
        let env = LowerTyEnv { st, file_res, db, local_defs: self, self_ty, impl_owner: item.impl_node, impl_generics: 0, item_owner: Some(item.node), in_body: false };
        Some(env.lower_cst_ty(table, ty_node))
    }

    pub fn fn_sig(
        &self,
        def: GlobalDefId,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        inputs_out: &mut Vec<InferTyId>,
    ) -> Option<(u16, InferTyId)> {
        let item = self.items.iter().find(|i| i.def == def && matches!(i.kind, ItemKind::Fn | ItemKind::AssocFn))?;
        let (impl_gc, self_ty) = self.owner_generics_and_self_ty(item.impl_node, st, file_res, db, table);
        let env = LowerTyEnv { st, file_res, db, local_defs: self, self_ty, impl_owner: item.impl_node, impl_generics: impl_gc, item_owner: Some(item.node), in_body: false };
        inputs_out.clear();
        if let Some(pl) = st.child(item.node, K::ParamList) {
            for p in st.children(pl) {
                match st.kind(p) {
                    K::SelfParam => inputs_out.push(lower_self_param_ty(st, p, &env, table)),
                    K::Param => {
                        let pty = st.children(p).find(|&c| st.kind(c).is_type()).map_or(InferTyId::UNKNOWN, |tn| env.lower_cst_ty(table, tn));
                        inputs_out.push(pty);
                    }
                    _ => {}
                }
            }
        }
        let mut ret = st.child(item.node, K::RetType).and_then(|rt| st.children(rt).find(|&c| st.kind(c).is_type())).map_or(InferTyId::UNIT, |tn| env.lower_cst_ty(table, tn));
        if st.own_tokens(item.node).any(|(_, t)| t.kind == T::Async) {
            if let Some(fut_def) = db.lang_item(TyLangItem::Future) {
                ret = table.arena.mk_opaque(fut_def, &[ret]);
            }
        }
        Some((item.generic_count, ret))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn param_fn_bound(
        &self,
        owner_def: GlobalDefId,
        param_idx: u16,
        st: &SyntaxTree<'_>,
        file_res: &FileNameRes,
        db: &dyn TyDefDb,
        table: &mut InferenceTable,
        inputs_out: &mut Vec<InferTyId>,
    ) -> Option<(TyLangItem, InferTyId)> {
        let item = self.items.iter().find(|i| i.def == owner_def)?;
        let (impl_gc, self_ty) = self.owner_generics_and_self_ty(item.impl_node, st, file_res, db, table);
        let env = LowerTyEnv { st, file_res, db, local_defs: self, self_ty, impl_owner: item.impl_node, impl_generics: impl_gc, item_owner: Some(item.node), in_body: false };
        super::local_defs_adt::extract_cst_fn_bound(st, file_res, db, item.node, impl_gc, param_idx, &env, table, inputs_out)
    }
}

pub(super) fn lower_self_param_ty(st: &SyntaxTree<'_>, sp: NodeId, env: &LowerTyEnv<'_, '_>, table: &mut InferenceTable) -> InferTyId {
    if let Some(tn) = st.children(sp).find(|&c| st.kind(c).is_type()) {
        return env.lower_cst_ty(table, tn);
    }
    let base = env.self_ty.unwrap_or(InferTyId::UNKNOWN);
    if st.own_tokens(sp).any(|(_, t)| t.kind == T::Amp) {
        let is_mut = st.own_tokens(sp).any(|(_, t)| t.kind == T::Mut);
        table.arena.mk_ref(is_mut, base)
    } else {
        base
    }
}

pub(super) fn count_cst_generics(st: &SyntaxTree<'_>, owner: NodeId) -> u16 {
    let Some(gpl) = st.child(owner, K::GenericParamList) else {
        return 0;
    };
    st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)).count() as u16
}

fn resolve_item_node_def(st: &SyntaxTree<'_>, node: NodeId, is_assoc: bool, def_map: Option<&CrateDefMap>, item_db: Option<&dyn ItemDb>, file: FileId) -> GlobalDefId {
    let Some(nn) = st.child(node, K::Name) else {
        return GlobalDefId::NONE;
    };
    let r = st.range(nn);
    let (s, e) = (r.start as u32, r.end as u32);
    if let Some(dm) = def_map {
        if let Some(idb) = item_db {
            if !dm.is_dirty_file(file) {
                let d = idb.def_at_byte_span(dm.krate, file, s, e);
                if d.is_some() {
                    return d;
                }
            }
        }
        if is_assoc {
            if let Some(ai) = dm.local_impls.assoc_items.iter().find(|a| a.file == file && a.span_start == s && a.span_end == e) {
                if ai.def.is_some() {
                    return ai.def;
                }
            }
        }
        if let Some(it) = dm.items.iter().find(|it| it.file == file && it.span_start == s && it.span_end == e) {
            return effective_local_def_id(it.index_def, it.id);
        }
    }
    let _ = norm_ident(st.text(nn));
    GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x9000_0000 | node.0)
}
