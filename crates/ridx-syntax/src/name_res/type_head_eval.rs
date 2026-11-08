/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! CST type-head classifier (`ImplTargetKey`) and type/trait associated-item lookup helpers
//! shared by `impl` header setup, `<SelfTy as Trait>::Assoc`, and `T::Assoc` path resolution.

use super::body_walk::FileResolver;
use super::db::{GlobalDefId, ItemKind, ModuleId, NsMask};
use super::db_impls::ImplTargetKey;
use super::def_path::resolve_segments;
use super::path_res::{GenericBindId, PathCtx, ResTarget};
use super::per_ns::{LocalItemId, PerNs, PrimTy, ResolvedBinding};
use super::use_tree::PathAnchor;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

/// Intermediate container / receiver state while stepping across `Seg0::Seg1::...`.
#[derive(Clone, Copy, Debug)]
pub(super) enum StepState {
    LocalMod(ModuleId),
    ExtMod(GlobalDefId),
    LocalEnum(LocalItemId, GlobalDefId),
    ExtEnum(GlobalDefId),
    BlockEnum(GlobalDefId),
    TypeTarget(ImplTargetKey),
    TraitTarget(GlobalDefId),
    TypeParamTarget(GenericBindId),
}

#[inline]
pub(super) const fn ctx_ns(ctx: PathCtx) -> NsMask {
    match ctx {
        PathCtx::Expr | PathCtx::PatValue => NsMask::VALUE,
        PathCtx::Type | PathCtx::TypeBound | PathCtx::RecordExpr | PathCtx::PatRecord => NsMask::TYPE,
        PathCtx::TypeArg => NsMask(NsMask::TYPE.0 | NsMask::VALUE.0),
        PathCtx::MacroCall => NsMask::MACRO,
        PathCtx::UseTree => NsMask::ALL,
    }
}

#[inline]
pub(super) fn path_has_leading_colon2(st: &crate::tree::SyntaxTree<'_>, path_node: NodeId) -> bool {
    let n = st.node(path_node);
    n.first < n.end && st.tokens[n.first as usize].kind == T::Colon
}

#[inline]
pub(super) fn binding_to_res_target(b: ResolvedBinding) -> Option<ResTarget> {
    if b.prim_ty != 0 {
        return PrimTy::from_code(b.prim_ty).map(ResTarget::Prim);
    }
    if b.def.is_some() || b.local.is_some() || b.local_mod.is_some() {
        return Some(ResTarget::Def { def: b.def, local: b.local, kind: b.kind });
    }
    None
}

impl<'a, 's> FileResolver<'a, 's> {
    /// Classifies a CST type node into its canonical [`ImplTargetKey`] without emitting references.
    pub(super) fn type_node_to_target_key(&mut self, ty_node: NodeId) -> ImplTargetKey {
        match self.st.kind(ty_node) {
            K::ParenType => self.st.children(ty_node).find(|&c| self.st.kind(c).is_type()).map_or(ImplTargetKey::Unknown, |inner| self.type_node_to_target_key(inner)),
            K::NeverType => ImplTargetKey::Never,
            K::SliceType => ImplTargetKey::Slice,
            K::ArrayType => ImplTargetKey::Array,
            K::TupleType => ImplTargetKey::Tuple(self.st.children(ty_node).filter(|&c| self.st.kind(c).is_type()).count().min(u16::MAX as usize) as u16),
            K::RefType => ImplTargetKey::Ref { is_mut: self.st.own_tokens(ty_node).any(|(_, t)| t.kind == T::Mut) },
            K::PtrType => ImplTargetKey::Ptr { is_mut: self.st.own_tokens(ty_node).any(|(_, t)| t.kind == T::Mut) },
            K::FnPtrType => {
                let arity = self.st.child(ty_node, K::TokenTree).map_or(0, |tt| count_fn_ptr_arity(self.st.tokens_of(tt)));
                ImplTargetKey::FnPtr { arity }
            }
            K::DynTraitType => {
                if let Some(p_ty) = self.st.child(ty_node, K::PathType) {
                    if let Some(p) = self.st.child(p_ty, K::Path) {
                        if let Some(b) = self.peek_path_type_binding(p) {
                            if b.kind == ItemKind::Trait && b.def.is_some() {
                                return ImplTargetKey::Dyn(b.def);
                            }
                        }
                    }
                }
                if let Some(tbl) = self.st.child(ty_node, K::TypeBoundList) {
                    for tb in self.st.children(tbl).filter(|&c| self.st.kind(c) == K::TypeBound) {
                        if self.st.own_tokens(tb).any(|(_, t)| t.kind == T::Question) {
                            continue;
                        }
                        if let Some(p) = self.st.child(tb, K::Path) {
                            if let Some(b) = self.peek_path_type_binding(p) {
                                if b.kind == ItemKind::Trait && b.def.is_some() {
                                    return ImplTargetKey::Dyn(b.def);
                                }
                            }
                        }
                    }
                }
                ImplTargetKey::Unknown
            }
            K::PathType => {
                let Some(path_node) = self.st.child(ty_node, K::Path) else {
                    return ImplTargetKey::Unknown;
                };
                if let Some((single_kind, single_name)) = self.single_bare_segment_info(path_node) {
                    if single_kind == T::SelfType {
                        return self.ribs.self_ctx.map_or(ImplTargetKey::Unknown, |sc| sc.target);
                    }
                    if self.ribs.lookup_generic(self.st.src, single_name).is_some_and(|(_, is_const)| !is_const) {
                        return ImplTargetKey::Param;
                    }
                    if let Some(tb) = self.ribs.lookup_block_ribs(single_name).types {
                        return self.binding_to_target_key(tb);
                    }
                }
                self.peek_path_type_binding(path_node).map_or(ImplTargetKey::Unknown, |tb| self.binding_to_target_key(tb))
            }
            _ => ImplTargetKey::Unknown,
        }
    }

    pub(super) fn binding_to_target_key(&self, tb: ResolvedBinding) -> ImplTargetKey {
        if tb.prim_ty != 0 {
            return PrimTy::from_code(tb.prim_ty).map_or(ImplTargetKey::Unknown, ImplTargetKey::Prim);
        }
        match tb.kind {
            ItemKind::Struct | ItemKind::Enum | ItemKind::Union if tb.def.is_some() => ImplTargetKey::Adt(tb.def),
            ItemKind::TypeAlias if tb.def.is_some() => self.map.resolve_alias_target(self.db, tb.def).unwrap_or(ImplTargetKey::Unknown),
            _ => ImplTargetKey::Unknown,
        }
    }

    pub(super) fn single_bare_segment_info(&self, path_node: NodeId) -> Option<(T, &'s str)> {
        if path_has_leading_colon2(self.st, path_node) {
            return None;
        }
        let mut segs = self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment);
        let first = segs.next()?;
        if segs.next().is_some() {
            return None;
        }
        let (_, tok) = self.st.own_tokens(first).next()?;
        if tok.kind == T::Lt {
            return None;
        }
        Some((tok.kind, norm_ident(tok.text(self.st.src))))
    }

    pub(super) fn peek_path_type_binding(&mut self, path_node: NodeId) -> Option<ResolvedBinding> {
        let leading_colon2 = path_has_leading_colon2(self.st, path_node);
        let mut seg_names: [&str; 16] = [""; 16];
        let mut len = 0usize;
        let mut anchor = if leading_colon2 { PathAnchor::ExternRoot } else { PathAnchor::Bare };
        let mut super_cnt = 0u16;
        for (idx, seg) in self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment).enumerate() {
            let (_, tok) = self.st.own_tokens(seg).next()?;
            if tok.kind == T::Lt || len >= 16 {
                return None;
            }
            match tok.kind {
                T::Crate if idx == 0 && !leading_colon2 => anchor = PathAnchor::CrateRoot,
                T::SelfKw if idx == 0 && !leading_colon2 => anchor = PathAnchor::SelfMod,
                T::Super if !leading_colon2 && (idx == 0 || matches!(anchor, PathAnchor::Super(_))) => {
                    super_cnt += 1;
                    anchor = PathAnchor::Super(super_cnt);
                }
                _ => {
                    seg_names[len] = norm_ident(tok.text(self.st.src));
                    len += 1;
                }
            }
        }
        if anchor == PathAnchor::Bare && len == 1 {
            if let Some(tb) = self.ribs.lookup_block_ribs(seg_names[0]).types {
                return Some(tb);
            }
        }
        resolve_segments(self.map, self.db, self.mod_tree, self.ext_cache, self.cur_mod, anchor, &seg_names[..len], false).types
    }

    /// Converts a resolved TYPE-namespace binding into the [`StepState`] used for `::NextSeg`.
    pub(super) fn type_binding_to_step_state(&self, tb: ResolvedBinding) -> Option<StepState> {
        if tb.prim_ty != 0 {
            return PrimTy::from_code(tb.prim_ty).map(|p| StepState::TypeTarget(ImplTargetKey::Prim(p)));
        }
        if tb.local_mod.is_some() {
            return Some(StepState::LocalMod(tb.local_mod));
        }
        match tb.kind {
            ItemKind::Mod if tb.def.is_some() => Some(StepState::ExtMod(tb.def)),
            ItemKind::Enum if tb.local.is_some() => Some(StepState::LocalEnum(tb.local, tb.def)),
            ItemKind::Enum if tb.def.is_overlay() => Some(StepState::BlockEnum(tb.def)),
            ItemKind::Enum if tb.def.is_some() => Some(StepState::ExtEnum(tb.def)),
            ItemKind::Struct | ItemKind::Union if tb.def.is_some() => Some(StepState::TypeTarget(ImplTargetKey::Adt(tb.def))),
            ItemKind::Trait if tb.def.is_some() => Some(StepState::TraitTarget(tb.def)),
            ItemKind::TypeAlias if tb.def.is_some() => {
                let target = self.map.resolve_alias_target(self.db, tb.def)?;
                if let ImplTargetKey::Adt(adt_def) = target {
                    if let Some(lid) = self.find_local_enum_by_def(adt_def) {
                        return Some(StepState::LocalEnum(lid, adt_def));
                    }
                    if self.db.def_kind(adt_def) == Some(ItemKind::Enum) {
                        return Some(StepState::ExtEnum(adt_def));
                    }
                }
                Some(StepState::TypeTarget(target))
            }
            _ => None,
        }
    }

    pub(super) fn find_local_enum_by_def(&self, enum_def: GlobalDefId) -> Option<LocalItemId> {
        self.map.items.iter().find(|it| it.kind == ItemKind::Enum && it.index_def == enum_def).map(|it| it.id)
    }

    pub(super) fn lookup_local_enum_variant(&self, elid: LocalItemId, seg: &str) -> PerNs {
        let eit = self.map.item(elid);
        let s = eit.var_start as usize;
        for vit in &self.map.items[s..s + eit.var_len as usize] {
            if self.map.item_name(vit.id) == seg {
                let b = ResolvedBinding {
                    def: vit.index_def,
                    local: vit.id,
                    local_mod: ModuleId::NONE,
                    vis_mod: vit.vis_mod,
                    kind: ItemKind::Variant,
                    vis: vit.vis,
                    origin: super::per_ns::BindingOrigin::Explicit,
                    prim_ty: 0,
                    ambiguous: false,
                };
                return PerNs::from_binding(b, ItemKind::Variant.ns_mask(vit.has_value_ctor));
            }
        }
        PerNs::EMPTY
    }

    /// Resolves `Target::seg` across block-local impls, inherent impls, and in-scope traits.
    pub(super) fn resolve_on_type_target(&mut self, target: ImplTargetKey, seg: &str, ns: NsMask) -> Option<(ResTarget, GlobalDefId)> {
        if let Some((bdef, bkind)) = self.ribs.lookup_block_assoc(self.st.src, target, GlobalDefId::NONE, seg, ns) {
            return Some((ResTarget::Def { def: bdef, local: LocalItemId::NONE, kind: bkind }, GlobalDefId::NONE));
        }
        if let Some(inh) = self.map.resolve_inherent_assoc(self.db, self.mod_tree, self.cur_mod, target, seg, ns) {
            return Some((ResTarget::Def { def: inh.def, local: LocalItemId::NONE, kind: inh.kind }, GlobalDefId::NONE));
        }
        let extra = if self.ribs.block_ribs.is_empty() {
            &[][..]
        } else {
            self.ribs.refresh_unnamed_traits_scratch();
            &self.ribs.extra_traits_scratch[..]
        };
        if let Some((tdef, tr_assoc)) = self.map.resolve_in_scope_trait_assoc(self.db, self.mod_tree, self.cur_mod, extra, target, seg, ns) {
            return Some((ResTarget::Def { def: tr_assoc.def, local: LocalItemId::NONE, kind: tr_assoc.kind }, tdef));
        }
        None
    }
}

fn count_fn_ptr_arity(toks: &[crate::token::Token]) -> u16 {
    let Some(first) = toks.iter().position(|t| !t.is_trivia()) else {
        return 0;
    };
    let Some(last) = toks.iter().rposition(|t| !t.is_trivia()) else {
        return 0;
    };
    if first >= last {
        return 0;
    }
    let mut depth = 0u32;
    let mut commas = 0u16;
    let mut any_tok = false;
    let mut trailing = true;
    for t in &toks[first + 1..last] {
        if t.is_trivia() {
            continue;
        }
        any_tok = true;
        match t.kind {
            T::LParen | T::LBracket | T::LBrace | T::Lt => {
                depth += 1;
                trailing = false;
            }
            T::RParen | T::RBracket | T::RBrace | T::Gt => {
                depth = depth.saturating_sub(1);
                trailing = false;
            }
            T::Comma if depth == 0 => {
                commas = commas.saturating_add(1);
                trailing = true;
            }
            _ => trailing = false,
        }
    }
    if !any_tok {
        0
    } else if trailing {
        commas
    } else {
        commas.saturating_add(1)
    }
}
