/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Type, generic parameter, `where`-clause, and generic argument walker methods for [`FileResolver`].

use super::body_ribs::norm_byte_span;
use super::body_walk::FileResolver;
use super::db::{GlobalDefId, ItemKind, NsMask};
use super::path_res::{GenericBindData, GenericBindId, PathCtx, ResTarget};
use super::per_ns::LocalItemId;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

impl<'a, 's> FileResolver<'a, 's> {
    pub(super) fn walk_type(&mut self, ty_node: NodeId) {
        match self.st.kind(ty_node) {
            K::PathType => {
                if let Some(p) = self.st.child(ty_node, K::Path) {
                    self.resolve_cst_path(p, PathCtx::Type);
                }
            }
            K::DynTraitType | K::ImplTraitType => {
                for ch in self.st.children(ty_node) {
                    match self.st.kind(ch) {
                        K::PathType => {
                            if let Some(p) = self.st.child(ch, K::Path) {
                                self.resolve_cst_path(p, PathCtx::TypeBound);
                            }
                        }
                        K::TypeBoundList => self.walk_type_bound_list(ch, GenericBindId::NONE),
                        _ => {}
                    }
                }
            }
            K::FnPtrType => {
                if let Some(tt) = self.st.child(ty_node, K::TokenTree) {
                    self.walk_fn_ptr_tt(tt);
                }
                if let Some(rt) = self.st.child(ty_node, K::RetType) {
                    if let Some(tn) = self.st.children(rt).find(|&c| self.st.kind(c).is_type()) {
                        self.walk_type(tn);
                    }
                }
            }
            K::ArrayType => {
                for ch in self.st.children(ty_node) {
                    if self.st.kind(ch).is_type() {
                        self.walk_type(ch);
                    } else if self.st.kind(ch).is_expr() {
                        let lb = self.ribs.enter_assoc_item_barrier();
                        self.walk_expr(ch);
                        self.ribs.local_barrier = lb;
                    }
                }
            }
            K::MacroCall => self.walk_macro_call_node(ty_node, false),
            _ => {
                for ch in self.st.children(ty_node) {
                    if self.st.kind(ch).is_type() {
                        self.walk_type(ch);
                    }
                }
            }
        }
    }

    pub(super) fn push_item_generics(&mut self, owner_node: NodeId, owner_def: GlobalDefId) -> (usize, usize) {
        let gm = self.ribs.generics_active.len();
        let bm = self.ribs.param_bounds.len();
        let Some(gpl) = self.st.child(owner_node, K::GenericParamList) else {
            return (gm, bm);
        };
        let mut stack_gps = [(NodeId(0), GenericBindId::NONE, false); 16];
        let mut overflow: Option<Vec<(NodeId, GenericBindId, bool)>> = None;
        let mut glen = 0usize;
        let mut idx = 0u16;
        for gp in self.st.children(gpl) {
            let gk = self.st.kind(gp);
            if !matches!(gk, K::TypeParam | K::ConstParam) || !self.node_cfg_enabled(gp) {
                continue;
            }
            if let Some(nn) = self.st.child(gp, K::Name) {
                let (ns, ne) = norm_byte_span(self.st.src, self.st.range(nn));
                if &self.st.src[ns as usize..ne as usize] != "_" {
                    let is_const = gk == K::ConstParam;
                    let def = self.db.def_at_byte_span(self.map.krate, self.file, ns, ne);
                    let gid = GenericBindId(self.out.generics.len() as u32);
                    let r = self.st.range(nn);
                    self.out.generics.push(GenericBindData {
                        id: gid,
                        def,
                        owner_node,
                        owner_def,
                        decl_node: gp,
                        name_start: r.start as u32,
                        name_end: r.end as u32,
                        index: idx,
                        is_const,
                    });
                    self.ribs.push_generic(ns, ne, gid, is_const);
                    if let Some(ov) = overflow.as_mut() {
                        ov.push((gp, gid, is_const));
                    } else if glen < 16 {
                        stack_gps[glen] = (gp, gid, is_const);
                        glen += 1;
                    } else {
                        let mut ov = stack_gps.to_vec();
                        ov.push((gp, gid, is_const));
                        overflow = Some(ov);
                    }
                }
            }
            idx = idx.saturating_add(1);
        }
        let slice = overflow.as_deref().unwrap_or(&stack_gps[..glen]);
        for &(gp, gid, is_const) in slice {
            for ch in self.st.children(gp) {
                match self.st.kind(ch) {
                    K::TypeBoundList => self.walk_type_bound_list(ch, if is_const { GenericBindId::NONE } else { gid }),
                    k if k.is_type() => self.walk_type(ch),
                    k if k.is_expr() => {
                        let lb = self.ribs.enter_assoc_item_barrier();
                        self.walk_expr(ch);
                        self.ribs.local_barrier = lb;
                    }
                    _ => {}
                }
            }
        }
        (gm, bm)
    }

    #[inline]
    pub(super) fn pop_generics(&mut self, gm: usize, bm: usize) {
        self.ribs.generics_active.truncate(gm);
        self.ribs.param_bounds.truncate(bm);
    }

    pub(super) fn walk_type_bound_list(&mut self, tbl: NodeId, attach_param: GenericBindId) {
        for tb in self.st.children(tbl).filter(|&c| self.st.kind(c) == K::TypeBound) {
            let is_maybe = self.st.own_tokens(tb).any(|(_, t)| t.kind == T::Question);
            if let Some(p) = self.st.child(tb, K::Path) {
                if let Some(rt @ ResTarget::Def { kind: ItemKind::Trait, .. }) = self.resolve_cst_path(p, PathCtx::TypeBound) {
                    let eff = match rt {
                        ResTarget::Def { def, local, .. } => super::dirty_overlay::effective_local_or_def(def, local),
                        _ => GlobalDefId::NONE,
                    };
                    if !is_maybe && attach_param.is_some() && eff.is_some() {
                        self.ribs.param_bounds.push((attach_param, eff));
                        self.out.param_trait_bounds.push((attach_param, eff));
                    }
                }
            }
        }
    }

    pub(super) fn walk_where_clause(&mut self, wc: NodeId) {
        for wp in self.st.children(wc).filter(|&c| self.st.kind(c) == K::WherePred) {
            let mut attach = GenericBindId::NONE;
            if let Some(tn) = self.st.children(wp).find(|&c| self.st.kind(c).is_type()) {
                if self.st.kind(tn) == K::PathType {
                    if let Some(p) = self.st.child(tn, K::Path) {
                        if let Some((_, sname)) = self.single_bare_segment_info(p) {
                            if let Some((gid, false)) = self.ribs.lookup_generic(self.st.src, sname) {
                                attach = gid;
                            }
                        }
                    }
                }
                self.walk_type(tn);
            }
            if let Some(tbl) = self.st.child(wp, K::TypeBoundList) {
                self.walk_type_bound_list(tbl, attach);
            }
        }
    }

    pub(super) fn walk_segment_args(&mut self, seg_node: NodeId, trait_def: GlobalDefId) {
        if seg_node.0 == u32::MAX {
            return;
        }
        for ch in self.st.children(seg_node) {
            match self.st.kind(ch) {
                K::GenericArgList => self.walk_generic_arg_list(ch, trait_def),
                k if k.is_type() => self.walk_type(ch),
                K::RetType => {
                    if let Some(tn) = self.st.children(ch).find(|&c| self.st.kind(c).is_type()) {
                        self.walk_type(tn);
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn walk_generic_arg_list(&mut self, gal: NodeId, trait_def: GlobalDefId) {
        for arg in self.st.children(gal) {
            match self.st.kind(arg) {
                K::TypeArg => {
                    if let Some(tn) = self.st.children(arg).find(|&c| self.st.kind(c).is_type()) {
                        if self.st.kind(tn) == K::PathType {
                            if let Some(p) = self.st.child(tn, K::Path) {
                                self.resolve_cst_path(p, PathCtx::TypeArg);
                                continue;
                            }
                        }
                        self.walk_type(tn);
                    }
                }
                K::ConstArg => {
                    for sub in self.st.children(arg) {
                        if self.st.kind(sub).is_expr() {
                            self.walk_expr(sub);
                        }
                    }
                }
                K::AssocTypeArg => {
                    if trait_def.is_some() {
                        if let Some(nn) = self.st.child(arg, K::Name) {
                            let r = self.st.range(nn);
                            if let Some(assoc) = self.map.resolve_trait_assoc_cached(self.db, self.ext_cache, trait_def, norm_ident(self.st.text(nn)), NsMask::TYPE) {
                                let rt = ResTarget::Def { def: assoc.def, local: LocalItemId::NONE, kind: assoc.kind };
                                self.emit_ref(r.start as u32, r.end as u32, NodeId(u32::MAX), rt, PathCtx::Type, false);
                            }
                        }
                    }
                    for sub in self.st.children(arg) {
                        match self.st.kind(sub) {
                            K::GenericArgList => self.walk_generic_arg_list(sub, GlobalDefId::NONE),
                            K::TypeBoundList => self.walk_type_bound_list(sub, GenericBindId::NONE),
                            k if k.is_type() => self.walk_type(sub),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
