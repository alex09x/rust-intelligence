/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Module and item CST walker (`FileResolver`) for [`FileNameRes`].

use super::attr::{eval_inner_attrs, eval_outer_attrs, EffectiveAttrs};
use super::body_ribs::{norm_byte_span, BlockAssocEntry, RibStack, SelfCtx};
use super::db::{FileId, GlobalDefId, ItemDb, ItemKind, ModuleId};
use super::db_impls::ImplTargetKey;
use super::def_map::CrateDefMap;
use super::ext_scope::ExtScopeCache;
use super::mod_tree::ModuleTree;
use super::path_res::{FileNameRes, GenericBindId, LocalBindData, LocalBindId, PathCtx, ResTarget};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

pub(super) struct FileResolver<'a, 's> {
    pub map: &'a CrateDefMap,
    pub db: &'a dyn ItemDb,
    pub mod_tree: &'a ModuleTree,
    pub ext_cache: &'a mut ExtScopeCache,
    pub file: FileId,
    pub cur_mod: ModuleId,
    pub st: &'a SyntaxTree<'s>,
    pub out: &'a mut FileNameRes,
    pub ribs: RibStack,
}

impl<'a, 's> FileResolver<'a, 's> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        map: &'a CrateDefMap,
        db: &'a dyn ItemDb,
        mod_tree: &'a ModuleTree,
        ext_cache: &'a mut ExtScopeCache,
        file: FileId,
        st: &'a SyntaxTree<'s>,
        out: &'a mut FileNameRes,
        ribs: RibStack,
    ) -> Self {
        let cur_mod =
            mod_tree.modules.iter().find(|m| m.file == file && !m.is_inline()).or_else(|| mod_tree.modules.iter().find(|m| m.file == file)).map_or(ModuleId::ROOT, |m| m.id);
        Self { map, db, mod_tree, ext_cache, file, cur_mod, st, out, ribs }
    }

    pub fn walk_file(&mut self) {
        let mut inner = EffectiveAttrs::default();
        eval_inner_attrs(self.st, self.st.root(), self.db, self.map.krate, &mut inner);
        if !inner.enabled {
            return;
        }
        self.walk_container_items(self.st.root(), false);
    }

    #[inline]
    pub(super) fn node_cfg_enabled(&self, node: NodeId) -> bool {
        if self.st.children(node).next().is_none_or(|c| self.st.kind(c) != K::Attr) {
            return true;
        }
        eval_outer_attrs(self.st, node, self.db, self.map.krate).enabled
    }

    fn walk_container_items(&mut self, container: NodeId, in_body: bool) {
        for ch in self.st.children(container) {
            if self.st.kind(ch).is_item() || self.st.kind(ch) == K::MacroCall {
                self.walk_item(ch, in_body);
            }
        }
    }

    #[inline]
    pub(super) fn walk_block_scoped_item(&mut self, item: NodeId) {
        self.walk_item(item, true);
    }

    fn walk_item(&mut self, item: NodeId, in_body: bool) {
        if !self.node_cfg_enabled(item) {
            return;
        }
        let saved = in_body.then(|| self.ribs.enter_standalone_item_barrier());
        match self.st.kind(item) {
            K::Module => {
                if let Some(il) = self.st.child(item, K::ItemList) {
                    let mut inner = EffectiveAttrs::default();
                    eval_inner_attrs(self.st, il, self.db, self.map.krate, &mut inner);
                    if inner.enabled {
                        if !in_body {
                            if let Some(nn) = self.st.child(item, K::Name) {
                                let target = norm_ident(self.st.text(nn));
                                if let Some(edge) = self.mod_tree.children(self.cur_mod).iter().find(|e| self.mod_tree.child_name(e) == target) {
                                    let prev_m = self.cur_mod;
                                    self.cur_mod = edge.child;
                                    self.walk_container_items(il, false);
                                    self.cur_mod = prev_m;
                                }
                            }
                        } else {
                            self.ribs.push_block_rib(self.map, self.db, self.mod_tree, self.ext_cache, self.file, self.cur_mod, self.st, il);
                            self.walk_container_items(il, true);
                            self.ribs.pop_block_rib();
                        }
                    }
                }
            }
            K::Use => self.walk_use_item(item),
            K::Fn => self.walk_fn_item(item),
            K::Impl => self.walk_impl_item(item, in_body),
            K::Trait => self.walk_trait_item(item),
            K::Struct | K::Union => {
                let (gm, bm) = self.push_item_generics(item, self.lookup_item_def(item));
                if let Some(wc) = self.st.child(item, K::WhereClause) {
                    self.walk_where_clause(wc);
                }
                if let Some(tt) = self.st.child(item, K::TokenTree) {
                    self.walk_struct_or_union_tt(tt);
                }
                self.pop_generics(gm, bm);
            }
            K::Enum => {
                let (gm, bm) = self.push_item_generics(item, self.lookup_item_def(item));
                if let Some(wc) = self.st.child(item, K::WhereClause) {
                    self.walk_where_clause(wc);
                }
                if let Some(tt) = self.st.child(item, K::TokenTree) {
                    self.walk_enum_tt(tt);
                }
                self.pop_generics(gm, bm);
            }
            K::TypeAlias => self.walk_type_alias(item),
            K::Const | K::Static => self.walk_const_or_static(item),
            K::ExternBlock => {
                if let Some(il) = self.st.child(item, K::ItemList) {
                    self.walk_container_items(il, false);
                }
            }
            K::MacroCall => {
                if let Some(p) = self.st.child(item, K::Path) {
                    self.resolve_cst_path(p, PathCtx::MacroCall);
                }
            }
            _ => {}
        }
        if let Some(s) = saved {
            self.ribs.restore_barrier(s);
        }
    }

    fn walk_fn_item(&mut self, fn_node: NodeId) {
        let owner_def = self.lookup_item_def(fn_node);
        let (gm, bm) = self.push_item_generics(fn_node, owner_def);
        let loc_mark = self.ribs.locals_active.len();
        if let Some(pl) = self.st.child(fn_node, K::ParamList) {
            for param in self.st.children(pl) {
                if !self.node_cfg_enabled(param) {
                    continue;
                }
                match self.st.kind(param) {
                    K::SelfParam => {
                        if let Some(tn) = self.st.children(param).find(|&c| self.st.kind(c).is_type()) {
                            self.walk_type(tn);
                        }
                        if let Some((_, stok)) = self.st.own_tokens(param).find(|(_, t)| t.kind == T::SelfKw) {
                            let is_mut = self.st.own_tokens(param).any(|(_, t)| t.kind == T::Mut);
                            let lid = self.alloc_local(param, stok.start, stok.end, true, is_mut);
                            self.ribs.push_local(stok.start, stok.end, lid);
                        }
                    }
                    K::Param => {
                        if let Some(tn) = self.st.children(param).find(|&c| self.st.kind(c).is_type()) {
                            self.walk_type(tn);
                        }
                        if let Some(pn) = self.st.children(param).find(|&c| self.st.kind(c).is_pat()) {
                            self.walk_pat(pn, None, false);
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(rt) = self.st.child(fn_node, K::RetType) {
            if let Some(tn) = self.st.children(rt).find(|&c| self.st.kind(c).is_type()) {
                self.walk_type(tn);
            }
        }
        if let Some(wc) = self.st.child(fn_node, K::WhereClause) {
            self.walk_where_clause(wc);
        }
        if let Some(body) = self.st.child(fn_node, K::BlockExpr) {
            self.walk_block_expr(body, true);
        }
        self.ribs.locals_active.truncate(loc_mark);
        self.pop_generics(gm, bm);
    }

    fn walk_impl_item(&mut self, impl_node: NodeId, in_block: bool) {
        let (gm, bm) = self.push_item_generics(impl_node, GlobalDefId::NONE);
        let prev_self = self.ribs.self_ctx;
        let has_for = self.st.own_tokens(impl_node).any(|(_, t)| t.kind == T::For);
        let mut type_kids = self.st.children(impl_node).filter(|&c| self.st.kind(c).is_type());
        let mut trait_def = GlobalDefId::NONE;
        let self_ty_opt = if has_for {
            if let Some(tr_ty) = type_kids.next() {
                trait_def = self.walk_impl_trait_type(tr_ty);
            }
            type_kids.next()
        } else {
            type_kids.next()
        };
        let target = self_ty_opt.map_or(ImplTargetKey::Unknown, |stn| self.type_node_to_target_key(stn));
        if let Some(stn) = self_ty_opt {
            self.walk_type(stn);
        }
        if let Some(wc) = self.st.child(impl_node, K::WhereClause) {
            self.walk_where_clause(wc);
        }
        self.ribs.self_ctx = Some(SelfCtx { target, trait_def, self_has_value_ctor: true });
        if let Some(il) = self.st.child(impl_node, K::ItemList) {
            if in_block {
                self.register_block_impl_assocs(il, target, trait_def);
            }
            self.walk_assoc_item_list(il);
        }
        self.ribs.self_ctx = prev_self;
        self.pop_generics(gm, bm);
    }

    fn walk_impl_trait_type(&mut self, tr_ty: NodeId) -> GlobalDefId {
        if self.st.kind(tr_ty) == K::PathType {
            if let Some(p) = self.st.child(tr_ty, K::Path) {
                if let Some(ResTarget::Def { def, kind: ItemKind::Trait, .. }) = self.resolve_cst_path(p, PathCtx::TypeBound) {
                    return def;
                }
                return GlobalDefId::NONE;
            }
        }
        self.walk_type(tr_ty);
        GlobalDefId::NONE
    }

    fn walk_trait_item(&mut self, trait_node: NodeId) {
        let trait_def = self.lookup_item_def(trait_node);
        let (gm, bm) = self.push_item_generics(trait_node, trait_def);
        let prev_self = self.ribs.self_ctx;
        self.ribs.self_ctx = Some(SelfCtx { target: ImplTargetKey::Unknown, trait_def, self_has_value_ctor: false });
        for tbl in self.st.children(trait_node).filter(|&c| self.st.kind(c) == K::TypeBoundList) {
            self.walk_type_bound_list(tbl, GenericBindId::NONE);
        }
        if let Some(wc) = self.st.child(trait_node, K::WhereClause) {
            self.walk_where_clause(wc);
        }
        if let Some(il) = self.st.child(trait_node, K::ItemList) {
            self.walk_assoc_item_list(il);
        }
        self.ribs.self_ctx = prev_self;
        self.pop_generics(gm, bm);
    }

    fn walk_assoc_item_list(&mut self, il: NodeId) {
        for ch in self.st.children(il) {
            if !self.node_cfg_enabled(ch) {
                continue;
            }
            let lb = self.ribs.enter_assoc_item_barrier();
            match self.st.kind(ch) {
                K::Fn => self.walk_fn_item(ch),
                K::TypeAlias => self.walk_type_alias(ch),
                K::Const => self.walk_const_or_static(ch),
                K::MacroCall => {
                    if let Some(p) = self.st.child(ch, K::Path) {
                        self.resolve_cst_path(p, PathCtx::MacroCall);
                    }
                }
                _ => {}
            }
            self.ribs.local_barrier = lb;
        }
    }

    fn walk_type_alias(&mut self, node: NodeId) {
        let (gm, bm) = self.push_item_generics(node, self.lookup_item_def(node));
        for ch in self.st.children(node) {
            match self.st.kind(ch) {
                K::TypeBoundList => self.walk_type_bound_list(ch, GenericBindId::NONE),
                K::WhereClause => self.walk_where_clause(ch),
                k if k.is_type() => self.walk_type(ch),
                _ => {}
            }
        }
        self.pop_generics(gm, bm);
    }

    fn walk_const_or_static(&mut self, node: NodeId) {
        let (gm, bm) = self.push_item_generics(node, self.lookup_item_def(node));
        let lb = self.ribs.enter_assoc_item_barrier();
        for ch in self.st.children(node) {
            match self.st.kind(ch) {
                k if k.is_type() => self.walk_type(ch),
                K::WhereClause => self.walk_where_clause(ch),
                k if k.is_expr() => self.walk_expr(ch),
                _ => {}
            }
        }
        self.ribs.local_barrier = lb;
        self.pop_generics(gm, bm);
    }

    pub(super) fn lookup_item_def(&self, item_node: NodeId) -> GlobalDefId {
        let Some(nn) = self.st.child(item_node, K::Name) else {
            return GlobalDefId::NONE;
        };
        let (ns, ne) = norm_byte_span(self.st.src, self.st.range(nn));
        let by_span = self.db.def_at_byte_span(self.map.krate, self.file, ns, ne);
        if by_span.is_some() {
            return by_span;
        }
        let r = self.st.range(nn);
        self.map.items.iter().find(|it| it.file == self.file && it.span_start == r.start as u32 && it.span_end == r.end as u32).map_or(GlobalDefId::NONE, |it| it.index_def)
    }

    fn register_block_impl_assocs(&mut self, il: NodeId, target: ImplTargetKey, trait_def: GlobalDefId) {
        if self.ribs.block_ribs.is_empty() {
            return;
        }
        let mut entries = Vec::new();
        for ch in self.st.children(il) {
            if !self.node_cfg_enabled(ch) {
                continue;
            }
            let kind = match self.st.kind(ch) {
                K::Fn => ItemKind::AssocFn,
                K::Const => ItemKind::AssocConst,
                K::TypeAlias => ItemKind::AssocTy,
                _ => continue,
            };
            let Some(nn) = self.st.child(ch, K::Name) else {
                continue;
            };
            let (ns, ne) = norm_byte_span(self.st.src, self.st.range(nn));
            let mut def = self.db.def_at_byte_span(self.map.krate, self.file, ns, ne);
            if !def.is_some() {
                def = self.ribs.alloc_synthetic_def();
            }
            entries.push(BlockAssocEntry { target, trait_def, name_start: ns, name_end: ne, def, kind });
        }
        if let Some(rib) = self.ribs.block_ribs.last_mut() {
            rib.assocs.extend(entries);
        }
    }

    #[inline]
    pub(super) fn alloc_local(&mut self, decl_node: NodeId, name_start: u32, name_end: u32, is_self: bool, is_mut: bool) -> LocalBindId {
        let id = LocalBindId(self.out.locals.len() as u32);
        self.out.locals.push(LocalBindData { id, decl_node, name_start, name_end, is_self, is_mut });
        id
    }
}
