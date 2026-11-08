/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Context-aware CST `Path`, qualified `<SelfTy as Trait>::Assoc`, anchored-prefix, and single-bare
//! segment resolvers for [`super::body_walk::FileResolver`].

use super::body_walk::FileResolver;
use super::db::{GlobalDefId, ItemKind, ModuleId, NsMask, VisKind};
use super::db_impls::ImplTargetKey;
use super::def_map::{extern_crate_binding, local_mod_binding, CrateDefMap};
use super::def_path::step_super;
use super::path_res::{GenericBindId, PathCtx, ResTarget, ResolvedRef};
use super::per_ns::{BindingOrigin, LocalItemId, PerNs, PrimTy, ResolvedBinding};
use super::type_head_eval::{binding_to_res_target, ctx_ns, path_has_leading_colon2, StepState};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

/// Lightweight borrowed descriptor of one path segment token.
#[derive(Clone, Copy)]
pub(super) struct RawSeg<'s> {
    pub start: u32,
    pub end: u32,
    pub tok_kind: T,
    pub name: &'s str,
    pub seg_node: NodeId,
}

impl RawSeg<'static> {
    pub const EMPTY: Self = Self { start: 0, end: 0, tok_kind: T::Ident, name: "", seg_node: NodeId(u32::MAX) };
}

impl<'a, 's> FileResolver<'a, 's> {
    #[inline]
    pub(super) fn emit_ref(&mut self, start: u32, end: u32, path_node: NodeId, target: ResTarget, ctx: PathCtx, is_segment_tail: bool) {
        if start < end {
            let tail = &self.out.refs[self.out.refs.len().saturating_sub(64)..];
            if tail.iter().any(|r| r.start == start && r.end == end && r.target == target) {
                return;
            }
            self.out.refs.push(ResolvedRef { start, end, path_node, target, ctx, is_segment_tail });
        }
    }

    pub(super) fn emit_use_leaf_pns(&mut self, start: u32, end: u32, path_node: NodeId, pns: PerNs) -> Option<ResTarget> {
        let mut first = None;
        for b in [pns.types, pns.values, pns.macros].into_iter().flatten() {
            if let Some(rt) = binding_to_res_target(b) {
                self.emit_ref(start, end, path_node, rt, PathCtx::UseTree, true);
                if first.is_none() {
                    first = Some(rt);
                }
            }
        }
        first
    }

    /// Checks whether a bare `K::IdentPat` resolves to an in-scope unit variant, unit struct, or `const`.
    pub(super) fn resolve_bare_ident_pat_item(&self, norm_name: &str) -> Option<ResTarget> {
        let vb = self.ribs.lookup_block_ribs(norm_name).values.or_else(|| self.map.resolve_name_in_module(self.mod_tree, self.cur_mod, norm_name).values)?;
        if matches!(vb.kind, ItemKind::Variant | ItemKind::Struct | ItemKind::Const | ItemKind::AssocConst) {
            binding_to_res_target(vb)
        } else {
            None
        }
    }

    /// Resolves a CST `K::Path` node (ordinary, anchored, or `<SelfTy [as Trait]>::Assoc`) and its generic arguments.
    pub(super) fn resolve_cst_path(&mut self, path_node: NodeId, ctx: PathCtx) -> Option<ResTarget> {
        let mut seg_iter = self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment);
        let first_seg = seg_iter.next()?;
        let first_tok = self.st.own_tokens(first_seg).next().map(|(_, t)| t);

        if first_tok.is_some_and(|t| t.kind == T::Lt) {
            return self.resolve_qualified_cst_path(path_node, first_seg, seg_iter, ctx);
        }

        let leading_colon2 = path_has_leading_colon2(self.st, path_node);
        let mut stack_segs = [RawSeg::EMPTY; 16];
        let mut count = 0usize;
        for seg_node in std::iter::once(first_seg).chain(seg_iter) {
            let Some((_, tok)) = self.st.own_tokens(seg_node).next() else {
                continue;
            };
            if count < 16 {
                stack_segs[count] = RawSeg { start: tok.start, end: tok.end, tok_kind: tok.kind, name: norm_ident(tok.text(self.st.src)), seg_node };
                count += 1;
            }
        }
        if count == 0 {
            return None;
        }
        self.resolve_raw_path(path_node, leading_colon2, &stack_segs[..count], ctx)
    }

    fn resolve_qualified_cst_path(&mut self, path_node: NodeId, qseg: NodeId, rest: impl Iterator<Item = NodeId>, ctx: PathCtx) -> Option<ResTarget> {
        let self_ty_node = self.st.children(qseg).find(|&c| self.st.kind(c).is_type());
        let mut self_param_gid = GenericBindId::NONE;
        if let Some(stn) = self_ty_node {
            if self.st.kind(stn) == K::PathType {
                if let Some(pn) = self.st.child(stn, K::Path) {
                    if let Some((_, sname)) = self.single_bare_segment_info(pn) {
                        if let Some((gid, false)) = self.ribs.lookup_generic(self.st.src, sname) {
                            self_param_gid = gid;
                        }
                    }
                }
            }
            self.walk_type(stn);
        }
        let self_key = self_ty_node.map_or(ImplTargetKey::Unknown, |stn| self.type_node_to_target_key(stn));
        let trait_def = self
            .st
            .children(qseg)
            .find(|&c| self.st.kind(c) == K::Path)
            .and_then(|tp| self.resolve_cst_path(tp, PathCtx::TypeBound))
            .and_then(|rt| match rt {
                ResTarget::Def { def, kind: ItemKind::Trait, .. } if def.is_some() => Some(def),
                _ => None,
            })
            .unwrap_or(GlobalDefId::NONE);

        let mut tail_segs = [RawSeg::EMPTY; 16];
        let mut len = 0usize;
        for sn in rest {
            if let Some((_, tok)) = self.st.own_tokens(sn).next() {
                if len < 16 {
                    tail_segs[len] = RawSeg { start: tok.start, end: tok.end, tok_kind: tok.kind, name: norm_ident(tok.text(self.st.src)), seg_node: sn };
                    len += 1;
                }
            }
        }
        if len == 0 {
            return None;
        }
        let is_last = len == 1;
        let ns = if is_last { ctx_ns(ctx) } else { NsMask::TYPE };
        let seg0 = tail_segs[0];
        let resolved0 = if trait_def.is_some() {
            self.ribs
                .lookup_block_assoc(self.st.src, self_key, trait_def, seg0.name, ns)
                .map(|(d, k)| (ResTarget::Def { def: d, local: LocalItemId::NONE, kind: k }, trait_def))
                .or_else(|| {
                    self.map
                        .resolve_trait_impl_assoc_cached(self.db, self.ext_cache, self_key, trait_def, seg0.name, ns)
                        .map(|a| (ResTarget::Def { def: a.def, local: LocalItemId::NONE, kind: a.kind }, trait_def))
                })
        } else if self_param_gid.is_some() {
            self.resolve_on_type_param(self_param_gid, seg0.name, ns)
        } else {
            self.resolve_on_self_or_target(self_key, GlobalDefId::NONE, seg0.name, ns)
        };
        self.walk_segment_args(seg0.seg_node, GlobalDefId::NONE);
        let (target0, _) = resolved0?;
        self.emit_ref(seg0.start, seg0.end, path_node, target0, ctx, is_last);
        if is_last {
            return Some(target0);
        }
        let state = self.res_target_to_step_state(target0)?;
        self.resolve_remaining_segments(path_node, state, &tail_segs[1..len], ctx)
    }

    /// Resolves `[::]? segs[0]::segs[1]...`, emitting intermediate and tail [`ResolvedRef`]s.
    pub(super) fn resolve_raw_path(&mut self, path_node: NodeId, leading_colon2: bool, segs: &[RawSeg<'_>], ctx: PathCtx) -> Option<ResTarget> {
        let s0 = segs[0];
        let is_single = segs.len() == 1;

        if leading_colon2 && self.map.edition.has_uniform_paths() {
            self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
            let &(cid, rdef) = self.map.extern_prelude.get(s0.name)?;
            let b = extern_crate_binding(cid, self.map.krate, rdef);
            if let Some(rt) = binding_to_res_target(b) {
                self.emit_ref(s0.start, s0.end, path_node, rt, ctx, is_single);
            }
            if is_single {
                return binding_to_res_target(b);
            }
            let st0 = if cid == self.map.krate { StepState::LocalMod(ModuleId::ROOT) } else { StepState::ExtMod(rdef) };
            return self.resolve_remaining_segments(path_node, st0, &segs[1..], ctx);
        }

        match s0.tok_kind {
            T::Crate => {
                self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
                let rt = local_mod_res_target(self.map, ModuleId::ROOT);
                if let Some(r) = rt {
                    self.emit_ref(s0.start, s0.end, path_node, r, ctx, is_single);
                }
                return if is_single { rt } else { self.resolve_remaining_segments(path_node, StepState::LocalMod(ModuleId::ROOT), &segs[1..], ctx) };
            }
            T::SelfKw if is_single && ctx == PathCtx::Expr => {
                self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
                let lid = self.ribs.lookup_local(self.st.src, "self")?;
                let rt = ResTarget::Local(lid);
                self.emit_ref(s0.start, s0.end, path_node, rt, ctx, true);
                return Some(rt);
            }
            T::SelfKw => {
                self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
                let rt = local_mod_res_target(self.map, self.cur_mod);
                if let Some(r) = rt {
                    self.emit_ref(s0.start, s0.end, path_node, r, ctx, is_single);
                }
                return if is_single { rt } else { self.resolve_remaining_segments(path_node, StepState::LocalMod(self.cur_mod), &segs[1..], ctx) };
            }
            T::Super => {
                let (mut cur_m, mut idx, mut last_rt) = (self.cur_mod, 0usize, None);
                while idx < segs.len() && segs[idx].tok_kind == T::Super {
                    self.walk_segment_args(segs[idx].seg_node, GlobalDefId::NONE);
                    cur_m = step_super(self.mod_tree, cur_m, 1);
                    last_rt = local_mod_res_target(self.map, cur_m);
                    if let Some(r) = last_rt {
                        self.emit_ref(segs[idx].start, segs[idx].end, path_node, r, ctx, idx + 1 == segs.len());
                    }
                    idx += 1;
                }
                return if idx == segs.len() { last_rt } else { self.resolve_remaining_segments(path_node, StepState::LocalMod(cur_m), &segs[idx..], ctx) };
            }
            T::SelfType => {
                self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
                let sc = self.ribs.self_ctx?;
                let rt = ResTarget::SelfTy { target: sc.target, trait_def: sc.trait_def };
                self.emit_ref(s0.start, s0.end, path_node, rt, ctx, is_single);
                return if is_single { Some(rt) } else { self.resolve_self_qualified_tail(path_node, sc.target, sc.trait_def, &segs[1..], ctx) };
            }
            _ => {}
        }

        if is_single {
            return self.resolve_single_bare_seg(path_node, s0, ctx);
        }

        if let Some((gid, false)) = self.ribs.lookup_generic(self.st.src, s0.name) {
            self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
            self.emit_ref(s0.start, s0.end, path_node, ResTarget::Generic(gid), ctx, false);
            return self.resolve_remaining_segments(path_node, StepState::TypeParamTarget(gid), &segs[1..], ctx);
        }
        let pns0 = if !self.map.edition.has_uniform_paths() && ctx == PathCtx::UseTree {
            self.map.scopes[ModuleId::ROOT.0 as usize].get(s0.name).filter_visible(self.mod_tree, self.cur_mod)
        } else {
            self.lookup_bare_all_ns(s0.name)
        };
        let tb0 = pns0.types?;
        let owner_tr = if tb0.kind == ItemKind::Trait { tb0.def } else { GlobalDefId::NONE };
        self.walk_segment_args(s0.seg_node, owner_tr);
        if let Some(rt0) = binding_to_res_target(tb0) {
            self.emit_ref(s0.start, s0.end, path_node, rt0, ctx, false);
        }
        let state = self.type_binding_to_step_state(tb0)?;
        self.resolve_remaining_segments(path_node, state, &segs[1..], ctx)
    }

    fn resolve_single_bare_seg(&mut self, path_node: NodeId, s0: RawSeg<'_>, ctx: PathCtx) -> Option<ResTarget> {
        let rt = match ctx {
            PathCtx::Expr => {
                if let Some(lid) = self.ribs.lookup_local(self.st.src, s0.name) {
                    Some(ResTarget::Local(lid))
                } else if let Some((gid, true)) = self.ribs.lookup_generic(self.st.src, s0.name) {
                    Some(ResTarget::Generic(gid))
                } else {
                    let pns = self.lookup_bare_all_ns(s0.name);
                    pns.values.or_else(|| pns.types.filter(|b| b.kind == ItemKind::TypeAlias)).and_then(binding_to_res_target)
                }
            }
            PathCtx::Type | PathCtx::TypeBound | PathCtx::RecordExpr | PathCtx::PatRecord => {
                if ctx != PathCtx::TypeBound {
                    if let Some((gid, false)) = self.ribs.lookup_generic(self.st.src, s0.name) {
                        self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
                        let rt = ResTarget::Generic(gid);
                        self.emit_ref(s0.start, s0.end, path_node, rt, ctx, true);
                        return Some(rt);
                    }
                }
                let pns = self.lookup_bare_all_ns(s0.name);
                let owner_tr = pns.types.filter(|b| b.kind == ItemKind::Trait).map_or(GlobalDefId::NONE, |b| b.def);
                self.walk_segment_args(s0.seg_node, owner_tr);
                let rt = pns.types.and_then(binding_to_res_target)?;
                self.emit_ref(s0.start, s0.end, path_node, rt, ctx, true);
                return Some(rt);
            }
            PathCtx::TypeArg => {
                if let Some((gid, _)) = self.ribs.lookup_generic(self.st.src, s0.name) {
                    Some(ResTarget::Generic(gid))
                } else {
                    let pns = self.lookup_bare_all_ns(s0.name);
                    let owner_tr = pns.types.filter(|b| b.kind == ItemKind::Trait).map_or(GlobalDefId::NONE, |b| b.def);
                    self.walk_segment_args(s0.seg_node, owner_tr);
                    let chosen = pns.types.or_else(|| pns.values.filter(|b| matches!(b.kind, ItemKind::Const | ItemKind::AssocConst)));
                    let rt = chosen.and_then(binding_to_res_target)?;
                    self.emit_ref(s0.start, s0.end, path_node, rt, ctx, true);
                    return Some(rt);
                }
            }
            PathCtx::PatValue => {
                if let Some((gid, true)) = self.ribs.lookup_generic(self.st.src, s0.name) {
                    Some(ResTarget::Generic(gid))
                } else {
                    let pns = self.lookup_bare_all_ns(s0.name);
                    pns.values.or_else(|| pns.types.filter(|b| matches!(b.kind, ItemKind::Variant | ItemKind::Struct | ItemKind::TypeAlias))).and_then(binding_to_res_target)
                }
            }
            PathCtx::MacroCall => self.lookup_bare_all_ns(s0.name).macros.and_then(binding_to_res_target),
            PathCtx::UseTree => {
                let pns = if !self.map.edition.has_uniform_paths() {
                    self.map.scopes[ModuleId::ROOT.0 as usize].get(s0.name).filter_visible(self.mod_tree, self.cur_mod)
                } else {
                    self.lookup_bare_all_ns(s0.name)
                };
                self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
                return self.emit_use_leaf_pns(s0.start, s0.end, path_node, pns);
            }
        };
        self.walk_segment_args(s0.seg_node, GlobalDefId::NONE);
        if let Some(r) = rt {
            self.emit_ref(s0.start, s0.end, path_node, r, ctx, true);
        }
        rt
    }

    pub(super) fn lookup_bare_all_ns(&self, seg: &str) -> PerNs {
        let mut pns = self.ribs.lookup_block_ribs(seg);
        pns = pns.or_else_ns(self.map.resolve_name_in_module(self.mod_tree, self.cur_mod, seg));
        if pns.types.is_none() {
            if let Some(&(cid, rdef)) = self.map.extern_prelude.get(seg) {
                pns.types = Some(extern_crate_binding(cid, self.map.krate, rdef));
            }
        }
        if pns.types.is_none() {
            if let Some(p) = PrimTy::from_name(seg) {
                pns.types = Some(ResolvedBinding::primitive(p, VisKind::Public, ModuleId::NONE, BindingOrigin::Builtin));
            }
        }
        pns
    }
}

#[inline]
fn local_mod_res_target(map: &CrateDefMap, m: ModuleId) -> Option<ResTarget> {
    let b = local_mod_binding(m, map.scopes[m.0 as usize].index_def, VisKind::Public, ModuleId::NONE).types?;
    binding_to_res_target(b)
}
