/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Active generic parameter bound checks, supertrait BFS traversal, local CST `impl`
//! solving, and unindexed UFCS path target fallback for [`super::trait_solve`].

use super::autoderef::impl_target_key_of_ty;
use super::body_ctx::Runner;
use super::lower_ty::effective_target_def;
use super::trait_solve::SolveEnv;
use super::trait_solve_cst::check_cst_impl_bounds_senv;
use super::ty::{InferTyId, InferTyKind};
use super::uf_table::InlineTyBuf;
use super::InferenceTable;
use crate::name_res::{GlobalDefId, ImplTargetKey, ItemKind, LocalItemId, ResTarget};
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::NodeId;

pub(super) fn param_has_active_bound(senv: SolveEnv<'_, '_>, table: &mut InferenceTable, idx: u16, pdef: GlobalDefId, target_trait: GlobalDefId) -> bool {
    let mut scratch = core::mem::take(&mut table.gid_scratch);
    scratch.clear();
    let owner = senv.owner;
    for &(gid, tdef) in &senv.file_res.param_trait_bounds {
        let g = senv.file_res.generic_data(gid);
        let same = (pdef.is_some() && g.def == pdef)
            || (owner.impl_node == Some(g.owner_node) && g.index == idx)
            || (owner.fn_node == Some(g.owner_node) && owner.impl_gc + g.index == idx);
        if same && tdef.is_some() && !scratch.contains(&tdef) {
            scratch.push(tdef);
        }
    }
    if owner.fn_def.is_some() {
        senv.db.param_trait_bounds(owner.fn_def, idx, &mut scratch);
    }
    bfs_supertraits_restore(senv, table, scratch, target_trait, 48)
}

pub(super) fn trait_has_super(senv: SolveEnv<'_, '_>, table: &mut InferenceTable, sub: GlobalDefId, target: GlobalDefId) -> bool {
    if !sub.is_some() || !target.is_some() {
        return false;
    }
    let mut scratch = core::mem::take(&mut table.gid_scratch);
    scratch.clear();
    scratch.push(sub);
    bfs_supertraits_restore(senv, table, scratch, target, 32)
}

fn bfs_supertraits_restore(senv: SolveEnv<'_, '_>, table: &mut InferenceTable, mut scratch: Vec<GlobalDefId>, target: GlobalDefId, limit: usize) -> bool {
    let mut cursor = 0usize;
    while cursor < scratch.len() && scratch.len() < limit {
        let t = scratch[cursor];
        if t == target {
            table.gid_scratch = scratch;
            return true;
        }
        cursor += 1;
        senv.local_defs.collect_supertraits_into(t, senv.st, senv.file_res, senv.db, &mut scratch);
    }
    let ok = scratch.contains(&target);
    table.gid_scratch = scratch;
    ok
}

pub(super) fn try_solve_local_cst_impls(
    senv: SolveEnv<'_, '_>,
    trait_def: GlobalDefId,
    self_ty: InferTyId,
    trait_args: &[InferTyId],
    table: &mut InferenceTable,
    depth: u8,
) -> bool {
    let self_head = impl_target_key_of_ty(&table.arena, self_ty);
    let self_is_var = matches!(table.arena.kind(self_ty), InferTyKind::Infer(_));
    let has_infer = table.has_unresolved_infer(self_ty) || trait_args.iter().any(|&t| table.has_unresolved_infer(t));
    let mut match_count = 0u32;
    let mut unique_imp: Option<(NodeId, u16)> = None;
    for imp in &senv.local_defs.impls {
        if imp.indexed_in_clean_db || !imp.has_for || imp.is_negative {
            continue;
        }
        let Some(tp) = imp.trait_path else {
            continue;
        };
        if senv.file_res.path_tail_ref_in_st(senv.st, tp).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target)) != trait_def {
            continue;
        }
        let Some((imp_tr, imp_args, imp_self)) = senv.local_defs.lower_trait_impl_header(imp.node, imp.impl_generics, senv.st, senv.file_res, senv.db, table) else { continue };
        if imp_tr != trait_def {
            continue;
        }
        let is_blanket = matches!(table.arena.kind(imp_self), InferTyKind::Param { .. });
        if !self_is_var && !is_blanket && impl_target_key_of_ty(&table.arena, imp_self) != self_head {
            continue;
        }
        let snap = table.start_snapshot();
        let ok = bind_and_check_cst_impl(senv, (imp.node, imp.impl_generics, imp_self, is_blanket), imp_args.as_slice(), self_ty, trait_args, table, depth);
        if ok && !has_infer {
            table.commit(snap);
            return true;
        }
        table.rollback_to(snap);
        if ok {
            match_count += 1;
            unique_imp = Some((imp.node, imp.impl_generics));
            if match_count > 1 {
                break;
            }
        }
    }
    if match_count == 1 {
        if let Some((node, igc, (_, imp_args, imp_self))) =
            unique_imp.and_then(|(n, g)| senv.local_defs.lower_trait_impl_header(n, g, senv.st, senv.file_res, senv.db, table).map(|h| (n, g, h)))
        {
            let is_b = matches!(table.arena.kind(imp_self), InferTyKind::Param { .. });
            let snap = table.start_snapshot();
            if bind_and_check_cst_impl(senv, (node, igc, imp_self, is_b), imp_args.as_slice(), self_ty, trait_args, table, depth) {
                table.commit(snap);
            } else {
                table.rollback_to(snap);
            }
        }
    }
    match_count >= 1
}

fn bind_and_check_cst_impl(
    senv: SolveEnv<'_, '_>,
    (imp_node, imp_gc, imp_self, is_blanket): (NodeId, u16, InferTyId, bool),
    imp_args: &[InferTyId],
    self_ty: InferTyId,
    trait_args: &[InferTyId],
    table: &mut InferenceTable,
    depth: u8,
) -> bool {
    let mut subst = InlineTyBuf::new();
    for _ in 0..imp_gc {
        subst.push(table.new_ty_var());
    }
    let inst_self = table.instantiate(imp_self, subst.as_slice());
    if table.equate(self_ty, inst_self).is_err() {
        return false;
    }
    if !trait_args.is_empty() {
        if imp_args.len() == trait_args.len() {
            for (&ta, &ia_raw) in trait_args.iter().zip(imp_args) {
                let ia = table.instantiate(ia_raw, subst.as_slice());
                if table.equate(ta, ia).is_err() {
                    return false;
                }
            }
        } else if !imp_args.is_empty() || trait_args.len() != 1 || table.equate(trait_args[0], inst_self).is_err() {
            return false;
        }
    }
    !is_blanket || check_cst_impl_bounds_senv(senv, imp_node, imp_gc, subst.as_slice(), table, false, depth)
}

impl<'a, 's> Runner<'a, 's> {
    pub(super) fn collect_ty_direct_bounds(&self, st_ty: InferTyId, out: &mut Vec<GlobalDefId>) {
        out.clear();
        match self.ctx.table.arena.kind(st_ty) {
            InferTyKind::Param { idx, def: pdef } => {
                if idx == 0 {
                    if let Some(tdef) = self
                        .root_impl_node
                        .filter(|&n| self.root_st.kind(n) == K::Trait)
                        .and_then(|im| self.local_defs.traits.iter().find(|t| t.node == im))
                        .map(|t| t.def)
                        .filter(|d| d.is_some())
                    {
                        if !out.contains(&tdef) {
                            out.push(tdef);
                        }
                    }
                }
                for &(gid, tdef) in self.root_file_res.param_trait_bounds.iter() {
                    let g = self.root_file_res.generic_data(gid);
                    let same_param = (pdef.is_some() && g.def == pdef)
                        || (self.root_impl_node == Some(g.owner_node) && g.index == idx)
                        || (Some(self.root_fn_node) == Some(g.owner_node) && self.impl_gc + g.index == idx);
                    if same_param && tdef.is_some() && !out.contains(&tdef) {
                        out.push(tdef);
                    }
                }
                if self.fn_def.is_some() {
                    self.db.param_trait_bounds(self.fn_def, idx, out);
                }
            }
            InferTyKind::Alias { assoc_def, .. } if assoc_def.is_some() => {
                self.local_defs.assoc_ty_cst_bounds(assoc_def, self.root_st, self.root_file_res, out);
                self.db.assoc_ty_trait_bounds(assoc_def, out);
            }
            _ => {}
        }
        self.expand_supertraits(out, 48);
    }

    pub(super) fn expand_supertraits(&self, out: &mut Vec<GlobalDefId>, limit: usize) {
        let mut cursor = 0usize;
        while cursor < out.len() && out.len() < limit {
            let tdef = out[cursor];
            cursor += 1;
            self.local_defs.collect_supertraits_into(tdef, self.root_st, self.root_file_res, self.db, out);
        }
    }

    pub(super) fn resolve_ufcs_in_scope_traits(&mut self, at_node: NodeId, fn_name: &str) -> GlobalDefId {
        let mut scope_traits = core::mem::take(&mut self.ctx.scope_traits_buf);
        scope_traits.clear();
        self.root_file_res.collect_traits_in_scope_at(at_node, &mut scope_traits);
        self.expand_supertraits(&mut scope_traits, 64);
        let mut found = GlobalDefId::NONE;
        for &tdef in &scope_traits {
            let cand =
                self.db.trait_assoc_fn_by_name(tdef, fn_name).map(|m| m.trait_fn_def).unwrap_or_else(|| {
                    self.local_defs.resolve_ufcs_assoc_fn(ImplTargetKey::Unknown, tdef, fn_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table)
                });
            if cand.is_some() {
                found = cand;
                break;
            }
        }
        self.ctx.scope_traits_buf = scope_traits;
        found
    }

    pub(super) fn resolve_unindexed_ufcs_target(&mut self, path_node: NodeId, fallback: ResTarget) -> ResTarget {
        let mut segs = [NodeId(0); 8];
        let mut slen = 0usize;
        for s in self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment) {
            if slen < segs.len() {
                segs[slen] = s;
                slen += 1;
            }
        }
        let (Some(qual_seg), Some(tail_seg)) = ((slen >= 2).then(|| segs[slen - 2]), (slen >= 1).then(|| segs[slen - 1])) else { return fallback };
        let fn_name = self.st.own_tokens(tail_seg).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent)).map_or("", |(_, t)| norm_ident(t.text(self.st.src)));
        if self.st.own_tokens(segs[0]).next().is_some_and(|(_, t)| t.kind == T::Lt) {
            let tr_def =
                self.st.child(segs[0], K::Path).and_then(|tp| self.file_res.path_tail_ref_in_st(self.st, tp)).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target));
            let mut d = self.local_defs.resolve_ufcs_assoc_fn(ImplTargetKey::Unknown, tr_def, fn_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table);
            if !d.is_some() {
                d = self.resolve_ufcs_in_scope_traits(path_node, fn_name);
            }
            return if d.is_some() { ResTarget::Def { def: d, local: LocalItemId::NONE, kind: ItemKind::AssocFn } } else { fallback };
        }
        let qr = self.st.range(qual_seg);
        let q_target = self
            .st
            .own_tokens(qual_seg)
            .find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent | T::SelfType))
            .and_then(|(_, t)| self.file_res.refs_at_span(t.start, t.end).first().map(|r| r.target))
            .or_else(|| {
                self.file_res
                    .refs_starting_in(qr.start as u32, qr.end as u32)
                    .iter()
                    .find(|r| (r.path_node == path_node || r.path_node == NodeId(u32::MAX)) && r.end <= qr.end as u32)
                    .map(|r| r.target)
            });
        if let Some(ResTarget::Generic(gid)) = q_target {
            for &(bgid, btr) in &self.root_file_res.param_trait_bounds {
                if bgid == gid && btr.is_some() {
                    let d = self.db.trait_assoc_fn_by_name(btr, fn_name).map(|m| m.trait_fn_def).unwrap_or_else(|| {
                        self.local_defs.resolve_ufcs_assoc_fn(ImplTargetKey::Unknown, btr, fn_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table)
                    });
                    if d.is_some() {
                        return ResTarget::Def { def: d, local: LocalItemId::NONE, kind: ItemKind::AssocFn };
                    }
                }
            }
            return fallback;
        }
        if let Some(rt @ ResTarget::Def { kind: ItemKind::AssocTy, .. }) = q_target {
            let adef = effective_target_def(rt);
            let mut bounds = core::mem::take(&mut self.ctx.param_bounds_buf);
            bounds.clear();
            self.db.assoc_ty_trait_bounds(adef, &mut bounds);
            self.local_defs.assoc_ty_cst_bounds(adef, self.root_st, self.root_file_res, &mut bounds);
            for &btr in &bounds {
                let d =
                    self.db.trait_assoc_fn_by_name(btr, fn_name).map(|m| m.trait_fn_def).unwrap_or_else(|| {
                        self.local_defs.resolve_ufcs_assoc_fn(ImplTargetKey::Unknown, btr, fn_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table)
                    });
                if d.is_some() {
                    self.ctx.param_bounds_buf = bounds;
                    return ResTarget::Def { def: d, local: LocalItemId::NONE, kind: ItemKind::AssocFn };
                }
            }
            self.ctx.param_bounds_buf = bounds;
            let d = self.resolve_ufcs_in_scope_traits(path_node, fn_name);
            return if d.is_some() { ResTarget::Def { def: d, local: LocalItemId::NONE, kind: ItemKind::AssocFn } } else { fallback };
        }
        let (target_k, tr_def) = match q_target {
            Some(ResTarget::SelfTy { target, trait_def }) => (target, trait_def),
            Some(ResTarget::Prim(p)) => (ImplTargetKey::Prim(p), GlobalDefId::NONE),
            Some(rt @ ResTarget::Def { kind: ItemKind::Trait, .. }) => (ImplTargetKey::Unknown, effective_target_def(rt)),
            Some(rt @ ResTarget::Def { .. }) => (ImplTargetKey::Adt(effective_target_def(rt)), GlobalDefId::NONE),
            _ => return fallback,
        };
        let mut d = self.local_defs.resolve_ufcs_assoc_fn(target_k, tr_def, fn_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table);
        if !d.is_some() && !tr_def.is_some() {
            d = self.resolve_ufcs_in_scope_traits(path_node, fn_name);
        }
        if d.is_some() {
            ResTarget::Def { def: d, local: LocalItemId::NONE, kind: ItemKind::AssocFn }
        } else {
            fallback
        }
    }
}
