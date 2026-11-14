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
pub(super) use super::local_defs_macros::{count_cst_generics, PreExpandedMacro};
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
    pub(super) by_def: Vec<(u64, u32)>,
    pub(super) by_node: Vec<(u32, u32)>,
    pub(super) body_macros: Vec<PreExpandedMacro>,
    pub(super) macro_cache: Vec<PreExpandedMacro>,
    pub(super) failed_macro_keys: Vec<u64>,
    pub(super) include_bytes_lens: Vec<(u32, u64)>,
    pub(super) file_krate: CrateId,
    pub(super) clean_indexed: bool,
}

#[inline]
pub(super) fn def_sort_key(def: GlobalDefId) -> u64 {
    ((def.krate.0 as u64) << 32) | (def.index as u64)
}

impl LocalCstDefs {
    #[inline]
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
        self.by_def.clear();
        self.by_node.clear();
        if self.macro_cache.len() > 256 {
            self.macro_cache.clear();
        }
        if self.failed_macro_keys.len() > 2048 {
            self.failed_macro_keys.clear();
        }
        self.macro_cache.append(&mut self.body_macros);
        self.include_bytes_lens.clear();
        self.file_krate = CrateId::NONE;
        self.clean_indexed = false;
    }

    #[inline]
    pub fn is_clean_indexed(&self) -> bool {
        self.clean_indexed
    }

    #[inline]
    pub(super) fn find_item_by_def(&self, def: GlobalDefId) -> Option<&LocalItemNode> {
        if !def.is_some() || (self.clean_indexed && !def.is_overlay() && def.krate != self.file_krate) {
            return None;
        }
        let idx = self.by_def.binary_search_by_key(&def_sort_key(def), |&(k, _)| k).ok()?;
        self.items.get(self.by_def[idx].1 as usize)
    }

    /// Indexes all items in `st` into reusable buffers (`0` heap allocations when warm).
    pub fn index_file(&mut self, st: &SyntaxTree<'_>, def_map: Option<&CrateDefMap>, file: FileId) {
        self.index_file_with_item_db(st, def_map, None, file);
    }

    /// Indexes all items in `st`, resolving clean `.ridx` definitions through `item_db` when provided.
    pub fn index_file_with_item_db(&mut self, st: &SyntaxTree<'_>, def_map: Option<&CrateDefMap>, item_db: Option<&dyn ItemDb>, file: FileId) {
        self.clear();
        let clean_db = match (item_db, def_map) {
            (Some(idb), Some(dm)) => idb.crate_root_def(dm.krate).is_some() && !dm.is_dirty_file(file),
            _ => false,
        };
        self.file_krate = def_map.map_or(CrateId::NONE, |dm| dm.krate);
        self.clean_indexed = clean_db;
        self.scan_container(st, st.root(), None, def_map, item_db, file, clean_db, None);
        for (idx, it) in self.items.iter().enumerate() {
            let i32_idx = idx as u32;
            self.by_def.push((def_sort_key(it.def), i32_idx));
            self.by_node.push((it.node.0, i32_idx));
        }
        self.by_def.sort_unstable_by_key(|&(k, _)| k);
        self.by_node.sort_unstable_by_key(|&(n, _)| n);
        self.finalize_macro_tables();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn scan_container(
        &mut self,
        st: &SyntaxTree<'_>,
        container: NodeId,
        impl_node: Option<NodeId>,
        def_map: Option<&CrateDefMap>,
        item_db: Option<&dyn ItemDb>,
        file: FileId,
        clean_mod_scope: bool,
        target: Option<(u32, u32)>,
    ) {
        let skip_type_items = clean_mod_scope && def_map.is_some_and(|dm| dm.indexed_clean);
        for ch in st.children(container) {
            let overlaps = target.is_none_or(|(ts, te)| ch.0 < te && ts < st.node(ch).subtree_end);
            if clean_mod_scope
                && !overlaps
                && (matches!(st.kind(ch), K::Fn | K::Const | K::Static | K::Impl | K::Module | K::ExternBlock)
                    || (skip_type_items && matches!(st.kind(ch), K::Struct | K::Union | K::Enum | K::Trait | K::TypeAlias)))
            {
                continue;
            }
            match st.kind(ch) {
                K::Module | K::ExternBlock => {
                    if let Some(il) = st.child(ch, K::ItemList) {
                        self.scan_container(st, il, None, def_map, item_db, file, clean_mod_scope, target);
                    }
                }
                K::Impl => {
                    let has_for = st.own_tokens(ch).any(|(_, t)| t.kind == T::For);
                    let is_negative = has_for && st.own_tokens(ch).any(|(_, t)| t.kind == T::Bang);
                    let trait_path = if has_for { st.children(ch).find(|&c| st.kind(c).is_type()).and_then(|tn| st.child(tn, K::Path)) } else { None };
                    self.impls.push(LocalImplNode { node: ch, impl_generics: count_cst_generics(st, ch), has_for, is_negative, trait_path, indexed_in_clean_db: clean_mod_scope });
                    if let Some(il) = st.child(ch, K::ItemList) {
                        self.scan_container(st, il, Some(ch), def_map, item_db, file, clean_mod_scope, target);
                    }
                }
                K::Trait => {
                    let def = resolve_item_node_def(st, ch, false, def_map, item_db, file);
                    if def.is_some() {
                        self.traits.push(LocalTraitNode { def, node: ch, trait_generics: count_cst_generics(st, ch) });
                    }
                    if overlaps {
                        if let Some(il) = st.child(ch, K::ItemList) {
                            self.scan_container(st, il, Some(ch), def_map, item_db, file, clean_mod_scope, target);
                        }
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
                    if overlaps {
                        if let Some(body) = st.child(ch, K::BlockExpr).filter(|&b| target.is_none_or(|(ts, te)| b.0 < te && ts < st.node(b).subtree_end)) {
                            self.scan_block_items(st, body, def_map, item_db, file, 0);
                            self.scan_body_macros(st, body, def_map, item_db, file);
                        }
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
                        self.index_enum_variants(st, ch, def, gc, def_map, item_db, file);
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
                    if overlaps {
                        self.scan_body_macros(st, ch, def_map, item_db, file);
                    }
                }
                _ => {}
            }
        }
    }

    fn scan_block_items(&mut self, st: &SyntaxTree<'_>, block: NodeId, def_map: Option<&CrateDefMap>, item_db: Option<&dyn ItemDb>, file: FileId, depth: u16) {
        if depth > 96 || !st.nodes[block.0 as usize + 1..st.node(block).subtree_end as usize].iter().any(|n| n.kind.is_item()) {
            return;
        }
        self.scan_container(st, block, None, def_map, item_db, file, false, None);
        for ch in st.children(block) {
            if !st.kind(ch).is_item() {
                self.scan_block_items(st, ch, def_map, item_db, file, depth + 1);
            }
        }
    }

    #[inline]
    pub fn adt_generic_count(&self, def: GlobalDefId) -> Option<u16> {
        self.find_item_by_def(def).map(|i| i.generic_count)
    }

    #[inline]
    pub fn fn_def_for_node(&self, fn_node: NodeId) -> GlobalDefId {
        let Ok(idx) = self.by_node.binary_search_by_key(&fn_node.0, |&(n, _)| n) else {
            return GlobalDefId::NONE;
        };
        self.items.get(self.by_node[idx].1 as usize).map_or(GlobalDefId::NONE, |i| i.def)
    }

    #[inline]
    pub fn fn_node_for_def(&self, def: GlobalDefId) -> u32 {
        self.find_item_by_def(def).map_or(u32::MAX, |i| i.node.0)
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
        let Some(sn) = (if has_for { tys.nth(1) } else { tys.next() }) else { return (impl_gc, InferTyId::UNKNOWN) };
        let env =
            LowerTyEnv { st, file_res, db, local_defs: self, self_ty: None, impl_owner: Some(impl_node), impl_generics: impl_gc, item_owner: None, in_body: false, sub_map: None };
        (impl_gc, env.lower_cst_ty(table, sn))
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
        let item = self.find_item_by_def(def).filter(|i| matches!(i.kind, ItemKind::Fn | ItemKind::AssocFn))?;
        let (igc, self_ty) = self.owner_generics_and_self_ty(item.impl_node, st, file_res, db, table);
        let env =
            LowerTyEnv { st, file_res, db, local_defs: self, self_ty, impl_owner: item.impl_node, impl_generics: igc, item_owner: Some(item.node), in_body: false, sub_map: None };
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
        table.arena.record_overlay_fn_sig(def, inputs_out, ret);
        Some((item.generic_count, ret))
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

fn resolve_item_node_def(st: &SyntaxTree<'_>, node: NodeId, is_assoc: bool, def_map: Option<&CrateDefMap>, item_db: Option<&dyn ItemDb>, file: FileId) -> GlobalDefId {
    let Some(nn) = st.child(node, K::Name) else { return GlobalDefId::NONE };
    let r = st.range(nn);
    let (s, e) = (r.start as u32, r.end as u32);
    if let Some(dm) = def_map {
        if let Some(d) = item_db.filter(|_| !dm.is_dirty_file(file)).map(|idb| idb.def_at_byte_span(dm.krate, file, s, e)).filter(|d| d.is_some()) {
            return d;
        }
        if is_assoc {
            if let Some(ai) = dm.local_impls.assoc_items.iter().find(|a| a.file == file && a.span_start == s && a.span_end == e && a.def.is_some()) {
                return ai.def;
            }
        }
        if let Some(it) = dm.items.iter().find(|it| it.file == file && it.span_start == s && it.span_end == e) {
            return effective_local_def_id(it.index_def, it.id);
        }
    }
    let _ = norm_ident(st.text(nn));
    GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x9000_0000 | node.0)
}
