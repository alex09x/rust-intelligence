/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Multi-segment path stepping (`Seg0::Seg1::...`), `Self::Assoc`, and generic `T::Assoc`
//! resolution methods for [`super::body_walk::FileResolver`].

use super::body_walk::FileResolver;
use super::db::{GlobalDefId, ItemKind, ModuleId, NsMask, VisKind};
use super::db_impls::ImplTargetKey;
use super::path_ctx_resolve::RawSeg;
use super::path_res::{GenericBindId, PathCtx, ResTarget};
use super::per_ns::{BindingOrigin, LocalItemId, PerNs, ResolvedBinding};
use super::type_head_eval::{binding_to_res_target, ctx_ns, StepState};
use crate::tree::NodeId;

impl<'a, 's> FileResolver<'a, 's> {
    pub(super) fn resolve_self_qualified_tail(&mut self, path_node: NodeId, target: ImplTargetKey, trait_def: GlobalDefId, rest: &[RawSeg<'_>], ctx: PathCtx) -> Option<ResTarget> {
        let is_last = rest.len() == 1;
        let ns = if is_last { ctx_ns(ctx) } else { NsMask::TYPE };
        let seg0 = rest[0];
        let res = self.resolve_on_self_or_target(target, trait_def, seg0.name, ns);
        self.walk_segment_args(seg0.seg_node, GlobalDefId::NONE);
        let (rt0, _) = res?;
        self.emit_ref(seg0.start, seg0.end, path_node, rt0, ctx, is_last);
        if is_last {
            return Some(rt0);
        }
        let next_st = self.res_target_to_step_state(rt0)?;
        self.resolve_remaining_segments(path_node, next_st, &rest[1..], ctx)
    }

    pub(super) fn resolve_remaining_segments(&mut self, path_node: NodeId, mut cur: StepState, rest: &[RawSeg<'_>], ctx: PathCtx) -> Option<ResTarget> {
        for (idx, &seg) in rest.iter().enumerate() {
            let is_last = idx + 1 == rest.len();
            let ns = if is_last { ctx_ns(ctx) } else { NsMask::TYPE };
            let (rt, next_state, owner_tr) = self.step_one_segment(cur, seg.name, ns, is_last, ctx)?;
            self.walk_segment_args(seg.seg_node, owner_tr);
            self.emit_ref(seg.start, seg.end, path_node, rt, ctx, is_last);
            if is_last {
                return Some(rt);
            }
            cur = next_state?;
        }
        None
    }

    fn step_one_segment(&mut self, state: StepState, seg: &str, ns: NsMask, is_last: bool, ctx: PathCtx) -> Option<(ResTarget, Option<StepState>, GlobalDefId)> {
        match state {
            StepState::LocalMod(m) => {
                let pns = self.map.scopes[m.0 as usize].get(seg).filter_visible(self.mod_tree, self.cur_mod);
                self.finish_pns_step(pns, is_last, ctx)
            }
            StepState::ExtMod(mdef) => {
                let pns = self.ext_cache.module_scope(self.db, mdef).get(seg);
                self.finish_pns_step(pns, is_last, ctx)
            }
            StepState::LocalEnum(elid, edef) => {
                let pns = self.lookup_local_enum_variant(elid, seg);
                if !pns.is_empty() {
                    return self.finish_pns_step(pns, is_last, ctx);
                }
                let (rt, _) = self.resolve_on_type_target(ImplTargetKey::Adt(edef), seg, ns)?;
                let nst = if is_last { None } else { self.res_target_to_step_state(rt) };
                Some((rt, nst, GlobalDefId::NONE))
            }
            StepState::ExtEnum(edef) => {
                let pns = self.ext_cache.enum_scope(self.db, edef).get(seg);
                if !pns.is_empty() {
                    return self.finish_pns_step(pns, is_last, ctx);
                }
                let (rt, _) = self.resolve_on_type_target(ImplTargetKey::Adt(edef), seg, ns)?;
                let nst = if is_last { None } else { self.res_target_to_step_state(rt) };
                Some((rt, nst, GlobalDefId::NONE))
            }
            StepState::BlockEnum(edef) => {
                let pns = self.ribs.lookup_block_enum_variant(edef, seg);
                if !pns.is_empty() {
                    return self.finish_pns_step(pns, is_last, ctx);
                }
                let (rt, _) = self.resolve_on_type_target(ImplTargetKey::Adt(edef), seg, ns)?;
                let nst = if is_last { None } else { self.res_target_to_step_state(rt) };
                Some((rt, nst, GlobalDefId::NONE))
            }
            StepState::TypeTarget(target) => {
                let (rt, _) = self.resolve_on_type_target(target, seg, ns)?;
                let nst = if is_last { None } else { self.res_target_to_step_state(rt) };
                Some((rt, nst, GlobalDefId::NONE))
            }
            StepState::TraitTarget(tdef) => {
                let rt = self
                    .ribs
                    .lookup_block_assoc(self.st.src, ImplTargetKey::Unknown, tdef, seg, ns)
                    .map(|(d, k)| ResTarget::Def { def: d, local: LocalItemId::NONE, kind: k })
                    .or_else(|| self.map.resolve_trait_assoc(self.db, tdef, seg, ns).map(|a| ResTarget::Def { def: a.def, local: LocalItemId::NONE, kind: a.kind }))?;
                let nst = if is_last { None } else { self.res_target_to_step_state(rt) };
                Some((rt, nst, GlobalDefId::NONE))
            }
            StepState::TypeParamTarget(gid) => {
                let (rt, _) = self.resolve_on_type_param(gid, seg, ns)?;
                let nst = if is_last { None } else { self.res_target_to_step_state(rt) };
                Some((rt, nst, GlobalDefId::NONE))
            }
        }
    }

    fn finish_pns_step(&self, pns: PerNs, is_last: bool, ctx: PathCtx) -> Option<(ResTarget, Option<StepState>, GlobalDefId)> {
        if !is_last {
            let tb = pns.types?;
            let owner_tr = if tb.kind == ItemKind::Trait { tb.def } else { GlobalDefId::NONE };
            return Some((binding_to_res_target(tb)?, self.type_binding_to_step_state(tb), owner_tr));
        }
        let chosen = match ctx {
            PathCtx::Expr => pns.values.or_else(|| pns.types.filter(|b| b.kind == ItemKind::TypeAlias)),
            PathCtx::PatValue => pns.values.or_else(|| pns.types.filter(|b| matches!(b.kind, ItemKind::Variant | ItemKind::Struct | ItemKind::TypeAlias))),
            PathCtx::Type | PathCtx::TypeBound | PathCtx::RecordExpr | PathCtx::PatRecord => pns.types,
            PathCtx::TypeArg => pns.types.or_else(|| pns.values.filter(|b| matches!(b.kind, ItemKind::Const | ItemKind::AssocConst))),
            PathCtx::MacroCall => pns.macros,
            PathCtx::UseTree => pns.types.or(pns.values).or(pns.macros),
        }?;
        let owner_tr = if chosen.kind == ItemKind::Trait { chosen.def } else { GlobalDefId::NONE };
        Some((binding_to_res_target(chosen)?, None, owner_tr))
    }

    pub(super) fn resolve_on_self_or_target(&mut self, target: ImplTargetKey, trait_def: GlobalDefId, seg: &str, ns: NsMask) -> Option<(ResTarget, GlobalDefId)> {
        if let ImplTargetKey::Adt(adt_def) = target {
            if let Some(lid) = self.find_local_enum_by_def(adt_def) {
                let pns = self.lookup_local_enum_variant(lid, seg);
                if let Some(b) = pns.values.filter(|_| ns.contains(NsMask::VALUE)).or_else(|| pns.types.filter(|_| ns.contains(NsMask::TYPE))) {
                    return binding_to_res_target(b).map(|rt| (rt, GlobalDefId::NONE));
                }
            } else if self.db.def_kind(adt_def) == Some(ItemKind::Enum) {
                let pns = self.ext_cache.enum_scope(self.db, adt_def).get(seg);
                if let Some(b) = pns.values.filter(|_| ns.contains(NsMask::VALUE)).or_else(|| pns.types.filter(|_| ns.contains(NsMask::TYPE))) {
                    return binding_to_res_target(b).map(|rt| (rt, GlobalDefId::NONE));
                }
            }
        }
        if target != ImplTargetKey::Unknown {
            if let Some(hit) = self.resolve_on_type_target(target, seg, ns) {
                return Some(hit);
            }
        }
        if trait_def.is_some() {
            if let Some(a) = self.map.resolve_trait_impl_assoc(self.db, target, trait_def, seg, ns) {
                return Some((ResTarget::Def { def: a.def, local: LocalItemId::NONE, kind: a.kind }, trait_def));
            }
        }
        None
    }

    pub(super) fn resolve_on_type_param(&mut self, gid: GenericBindId, seg: &str, ns: NsMask) -> Option<(ResTarget, GlobalDefId)> {
        let mut bound_traits: [GlobalDefId; 16] = [GlobalDefId::NONE; 16];
        let mut b_len = 0usize;
        for &(pg, tdef) in self.ribs.param_bounds.iter().rev() {
            if pg == gid && tdef.is_some() && b_len < 16 {
                bound_traits[b_len] = tdef;
                b_len += 1;
            }
        }
        for &tdef in &bound_traits[..b_len] {
            if let Some(a) = self.map.resolve_trait_assoc(self.db, tdef, seg, ns) {
                return Some((ResTarget::Def { def: a.def, local: LocalItemId::NONE, kind: a.kind }, tdef));
            }
        }
        let extra = if self.ribs.block_ribs.is_empty() {
            &[][..]
        } else {
            self.ribs.refresh_unnamed_traits_scratch();
            &self.ribs.extra_traits_scratch[..]
        };
        self.map
            .resolve_in_scope_trait_assoc(self.db, self.mod_tree, self.cur_mod, extra, ImplTargetKey::Param, seg, ns)
            .map(|(tdef, a)| (ResTarget::Def { def: a.def, local: LocalItemId::NONE, kind: a.kind }, tdef))
    }

    pub(super) fn res_target_to_step_state(&self, rt: ResTarget) -> Option<StepState> {
        match rt {
            ResTarget::Def { def, local, kind } => {
                let b = ResolvedBinding {
                    def,
                    local,
                    local_mod: ModuleId::NONE,
                    vis_mod: ModuleId::NONE,
                    kind,
                    vis: VisKind::Public,
                    origin: BindingOrigin::Explicit,
                    prim_ty: 0,
                    ambiguous: false,
                };
                self.type_binding_to_step_state(b)
            }
            ResTarget::Prim(p) => Some(StepState::TypeTarget(ImplTargetKey::Prim(p))),
            ResTarget::Generic(gid) => Some(StepState::TypeParamTarget(gid)),
            ResTarget::SelfTy { target, .. } if target != ImplTargetKey::Unknown => Some(StepState::TypeTarget(target)),
            ResTarget::SelfTy { trait_def, .. } if trait_def.is_some() => Some(StepState::TraitTarget(trait_def)),
            _ => None,
        }
    }
}
