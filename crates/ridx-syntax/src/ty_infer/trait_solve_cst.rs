/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! CST `where`-clause, inline generic bound, active parameter bound, and `<Param as Trait>::Assoc`
//! projection helpers for [`super::trait_solve`] (`Checkbox 3.4`).

use super::{
    autoderef::NormOwner,
    body_ctx::Runner,
    db::{TyDefDb, WhereObligation},
    local_defs::LocalCstDefs,
    lower_ty::{effective_target_def, LowerTyEnv},
    trait_solve::{solve_obligation_senv, SolveEnv},
    ty::{InferTyId, InferTyKind},
    uf_table::InlineTyBuf,
    InferenceTable,
};
use crate::{
    name_res::{FileNameRes, GlobalDefId, ImplTargetKey, ItemKind, LocalItemId, ResTarget},
    sema_fn::norm_ident,
    syntax_kind::SyntaxKind as K,
    token::TokenKind as T,
    tree::{NodeId, SyntaxTree},
};

#[allow(clippy::too_many_arguments)]
pub(super) fn check_cst_impl_bounds_in_table(
    local_defs: &LocalCstDefs,
    impl_node: NodeId,
    impl_gc: u16,
    subst: &[InferTyId],
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    db: &dyn TyDefDb,
    table: &mut InferenceTable,
    allow_ambig_self_var: bool,
    depth: u8,
) -> bool {
    let senv = SolveEnv { local_defs, st, file_res, db, owner: NormOwner::NONE };
    check_cst_impl_bounds_senv(senv, impl_node, impl_gc, subst, table, allow_ambig_self_var, depth)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn check_cst_impl_bounds_senv(senv: SolveEnv<'_, '_>, impl_node: NodeId, impl_gc: u16, subst: &[InferTyId], table: &mut InferenceTable, ambig: bool, d: u8) -> bool {
    if d > 8 {
        return false;
    }
    let env = LowerTyEnv {
        st: senv.st,
        file_res: senv.file_res,
        db: senv.db,
        local_defs: senv.local_defs,
        self_ty: None,
        impl_owner: Some(impl_node),
        impl_generics: impl_gc,
        item_owner: None,
        in_body: false,
        sub_map: None,
    };
    if let Some(gpl) = senv.st.child(impl_node, K::GenericParamList) {
        for (p_idx, gp) in senv.st.children(gpl).filter(|&c| matches!(senv.st.kind(c), K::TypeParam | K::ConstParam)).enumerate() {
            if senv.st.kind(gp) == K::TypeParam {
                if let (Some(&lhs), Some(tbl)) = (subst.get(p_idx), senv.st.child(gp, K::TypeBoundList)) {
                    if !eval_cst_bound_list(&env, senv, table, lhs, subst, tbl, ambig, d) {
                        return false;
                    }
                }
            }
        }
    }
    if let Some(wc) = senv.st.child(impl_node, K::WhereClause) {
        for wp in senv.st.children(wc).filter(|&c| senv.st.kind(c) == K::WherePred) {
            let (Some(tn), Some(tbl)) = (senv.st.children(wp).find(|&c| senv.st.kind(c).is_type()), senv.st.child(wp, K::TypeBoundList)) else {
                continue;
            };
            let raw_lhs = env.lower_cst_ty_rec(table, tn, d + 1);
            let lhs = table.instantiate(raw_lhs, subst);
            if !eval_cst_bound_list(&env, senv, table, lhs, subst, tbl, ambig, d) {
                return false;
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn eval_cst_bound_list(env: &LowerTyEnv<'_, '_>, senv: SolveEnv<'_, '_>, table: &mut InferenceTable, lhs: InferTyId, subst: &[InferTyId], tbl: NodeId, ambig: bool, d: u8) -> bool {
    let sub_senv = SolveEnv { owner: NormOwner::NONE, ..senv };
    for tb in env.st.children(tbl).filter(|&c| env.st.kind(c) == K::TypeBound) {
        if env.st.own_tokens(tb).any(|(_, t)| t.kind == T::Question) {
            continue;
        }
        let Some(p) = env.st.child(tb, K::Path) else {
            continue;
        };
        let btr = env.file_res.path_tail_ref_in_st(env.st, p).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target));
        if !btr.is_some() {
            continue;
        }
        let mut bargs = InlineTyBuf::new();
        if let Some(tail) = env.st.children(p).filter(|&c| env.st.kind(c) == K::PathSegment).last() {
            let mut raw_a = InlineTyBuf::new();
            env.collect_segment_args(table, tail, false, &mut raw_a, d + 1);
            for &ra in raw_a.as_slice() {
                bargs.push(table.instantiate(ra, subst));
            }
        }
        if !solve_obligation_senv(sub_senv, btr, lhs, bargs.as_slice(), table, ambig, d + 1) {
            return false;
        }
    }
    true
}

pub(super) fn collect_cst_fn_obligations(
    local_defs: &LocalCstDefs,
    fn_node: NodeId,
    st: &SyntaxTree<'_>,
    file_res: &FileNameRes,
    db: &dyn TyDefDb,
    table: &mut InferenceTable,
    out: &mut Vec<WhereObligation>,
) {
    let impl_owner = st.ancestors(fn_node).find(|&a| matches!(st.kind(a), K::Impl | K::Trait));
    let (impl_gc, self_ty) = local_defs.owner_generics_and_self_ty(impl_owner, st, file_res, db, table);
    let env = LowerTyEnv { st, file_res, db, local_defs, self_ty, impl_owner, impl_generics: impl_gc, item_owner: Some(fn_node), in_body: false, sub_map: None };
    if let Some(im) = impl_owner {
        let base = u16::from(st.kind(im) == K::Trait);
        scan_owner_cst_obligations(&env, table, im, base, out);
    }
    scan_owner_cst_obligations(&env, table, fn_node, impl_gc, out);
}

fn scan_owner_cst_obligations(env: &LowerTyEnv<'_, '_>, table: &mut InferenceTable, owner_node: NodeId, base_idx: u16, out: &mut Vec<WhereObligation>) {
    if let Some(gpl) = env.st.child(owner_node, K::GenericParamList) {
        for (local_idx, gp) in env.st.children(gpl).filter(|&c| matches!(env.st.kind(c), K::TypeParam | K::ConstParam)).enumerate() {
            if env.st.kind(gp) == K::TypeParam {
                if let Some(tbl) = env.st.child(gp, K::TypeBoundList) {
                    let lhs = table.arena.mk_param(base_idx + local_idx as u16, GlobalDefId::NONE);
                    emit_tbl_obligations(env, table, lhs, tbl, out);
                }
            }
        }
    }
    if let Some(wc) = env.st.child(owner_node, K::WhereClause) {
        for wp in env.st.children(wc).filter(|&c| env.st.kind(c) == K::WherePred) {
            if let (Some(tn), Some(tbl)) = (env.st.children(wp).find(|&c| env.st.kind(c).is_type()), env.st.child(wp, K::TypeBoundList)) {
                let lhs = env.lower_cst_ty_rec(table, tn, 0);
                emit_tbl_obligations(env, table, lhs, tbl, out);
            }
        }
    }
}

fn emit_tbl_obligations(env: &LowerTyEnv<'_, '_>, table: &mut InferenceTable, lhs_ty: InferTyId, tbl: NodeId, out: &mut Vec<WhereObligation>) {
    for tb in env.st.children(tbl).filter(|&c| env.st.kind(c) == K::TypeBound) {
        if env.st.own_tokens(tb).any(|(_, t)| t.kind == T::Question) {
            continue;
        }
        let Some(p) = env.st.child(tb, K::Path) else {
            continue;
        };
        let trait_def = env.file_res.path_tail_ref_in_st(env.st, p).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target));
        if !trait_def.is_some() {
            continue;
        }
        let mut targs = InlineTyBuf::new();
        let tail = env.st.children(p).filter(|&c| env.st.kind(c) == K::PathSegment).last();
        if let Some(tseg) = tail {
            env.collect_segment_args(table, tseg, false, &mut targs, 0);
        }
        let args_r = table.arena.intern_slice(targs.as_slice());
        out.push(WhereObligation::TraitBound { lhs_ty, trait_def, trait_args: args_r });
        if let Some(gal) = tail.and_then(|s| env.st.child(s, K::GenericArgList)) {
            for ga in env.st.children(gal).filter(|&c| env.st.kind(c) == K::AssocTypeArg) {
                if !env.st.own_tokens(ga).any(|(_, t)| t.kind == T::Eq) {
                    continue;
                }
                let (Some(nn), Some(tn)) = (env.st.child(ga, K::Name), env.st.children(ga).find(|&c| env.st.kind(c).is_type())) else {
                    continue;
                };
                let r = env.st.range(nn);
                let raw_adef = env.file_res.refs_at_span(r.start as u32, r.end as u32).first().map_or(GlobalDefId::NONE, |rf| effective_target_def(rf.target));
                let assoc_def = env.local_defs.resolve_local_trait_assoc_def(trait_def, raw_adef, norm_ident(env.st.text(nn)), env.st);
                let rhs_ty = env.lower_cst_ty_rec(table, tn, 0);
                out.push(WhereObligation::ProjectionEq { lhs_ty, trait_def, assoc_def, trait_args: args_r, rhs_ty });
            }
        }
    }
}

/// Projects `<Param(param_idx) as trait_def<trait_args>>::assoc_def` from CST inline/`where` bounds.
pub(super) fn project_cst_param_assoc(
    env: &LowerTyEnv<'_, '_>,
    table: &mut InferenceTable,
    param_idx: u16,
    trait_def: GlobalDefId,
    assoc_def: GlobalDefId,
    _trait_args: &[InferTyId],
    depth: u8,
) -> Option<InferTyId> {
    if depth > 10 {
        return None;
    }
    let assoc_name = env.local_defs.find_item_by_def(assoc_def).and_then(|i| env.st.child(i.node, K::Name)).map(|nn| norm_ident(env.st.text(nn)));
    if let Some(im) = env.impl_owner {
        let base = u16::from(env.st.kind(im) == K::Trait);
        if let Some(t) = scan_owner_param_assoc(env, table, im, base, param_idx, trait_def, assoc_def, assoc_name, depth) {
            return Some(t);
        }
    }
    if let Some(it) = env.item_owner {
        let base = if env.impl_owner.is_some() { env.impl_generics } else { 0 };
        if let Some(t) = scan_owner_param_assoc(env, table, it, base, param_idx, trait_def, assoc_def, assoc_name, depth) {
            return Some(t);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn scan_owner_param_assoc(
    env: &LowerTyEnv<'_, '_>,
    table: &mut InferenceTable,
    owner_node: NodeId,
    base_idx: u16,
    param_idx: u16,
    trait_def: GlobalDefId,
    assoc_def: GlobalDefId,
    assoc_name: Option<&str>,
    depth: u8,
) -> Option<InferTyId> {
    if let Some(gpl) = env.st.child(owner_node, K::GenericParamList) {
        for (lidx, gp) in env.st.children(gpl).filter(|&c| matches!(env.st.kind(c), K::TypeParam | K::ConstParam)).enumerate() {
            if env.st.kind(gp) == K::TypeParam && base_idx + lidx as u16 == param_idx {
                if let Some(tbl) = env.st.child(gp, K::TypeBoundList) {
                    if let Some(t) = match_tbl_assoc_eq(env, table, tbl, trait_def, assoc_def, assoc_name, depth) {
                        return Some(t);
                    }
                }
            }
        }
    }
    if let Some(wc) = env.st.child(owner_node, K::WhereClause) {
        for wp in env.st.children(wc).filter(|&c| env.st.kind(c) == K::WherePred) {
            let (Some(tn), Some(tbl)) = (env.st.children(wp).find(|&c| env.st.kind(c).is_type()), env.st.child(wp, K::TypeBoundList)) else {
                continue;
            };
            let lhs = env.lower_cst_ty_rec(table, tn, depth + 1);
            if matches!(table.arena.kind(lhs), InferTyKind::Param { idx, .. } if idx == param_idx) {
                if let Some(t) = match_tbl_assoc_eq(env, table, tbl, trait_def, assoc_def, assoc_name, depth) {
                    return Some(t);
                }
            }
        }
    }
    None
}

fn match_tbl_assoc_eq(
    env: &LowerTyEnv<'_, '_>,
    table: &mut InferenceTable,
    tbl: NodeId,
    trait_def: GlobalDefId,
    assoc_def: GlobalDefId,
    assoc_name: Option<&str>,
    depth: u8,
) -> Option<InferTyId> {
    let senv = SolveEnv { local_defs: env.local_defs, st: env.st, file_res: env.file_res, db: env.db, owner: NormOwner::NONE };
    for tb in env.st.children(tbl).filter(|&c| env.st.kind(c) == K::TypeBound) {
        let Some(p) = env.st.child(tb, K::Path) else {
            continue;
        };
        let btr = env.file_res.path_tail_ref_in_st(env.st, p).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target));
        if trait_def.is_some() && btr.is_some() && btr != trait_def && !trait_has_super(senv, table, btr, trait_def) {
            continue;
        }
        let Some(gal) = env.st.children(p).filter(|&c| env.st.kind(c) == K::PathSegment).last().and_then(|s| env.st.child(s, K::GenericArgList)) else {
            continue;
        };
        for ga in env.st.children(gal).filter(|&c| env.st.kind(c) == K::AssocTypeArg) {
            if !env.st.own_tokens(ga).any(|(_, t)| t.kind == T::Eq) {
                continue;
            }
            let (Some(nn), Some(tn)) = (env.st.child(ga, K::Name), env.st.children(ga).find(|&c| env.st.kind(c).is_type())) else {
                continue;
            };
            let r = env.st.range(nn);
            let ref_def = env.file_res.refs_at_span(r.start as u32, r.end as u32).first().map_or(GlobalDefId::NONE, |rf| effective_target_def(rf.target));
            let matched = (assoc_def.is_some() && ref_def == assoc_def) || assoc_name.is_some_and(|an| norm_ident(env.st.text(nn)) == an);
            if matched {
                return Some(env.lower_cst_ty_rec(table, tn, depth + 1));
            }
        }
    }
    None
}

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
impl<'a, 's> Runner<'a, 's> {
    pub(super) fn resolve_unindexed_ufcs_target(&mut self, path_node: NodeId, fallback: ResTarget) -> ResTarget {
        let mut segs = self.st.children(path_node).filter(|&c| self.st.kind(c) == K::PathSegment);
        let (Some(qual_seg), Some(tail_seg)) = (segs.next(), segs.last()) else { return fallback };
        let fn_name = self.st.own_tokens(tail_seg).find(|(_, t)| matches!(t.kind, T::Ident | T::RawIdent)).map_or("", |(_, t)| norm_ident(t.text(self.st.src)));
        if self.st.own_tokens(qual_seg).next().is_some_and(|(_, t)| t.kind == T::Lt) {
            let tr_def =
                self.st.child(qual_seg, K::Path).and_then(|tp| self.file_res.path_tail_ref_in_st(self.st, tp)).map_or(GlobalDefId::NONE, |r| effective_target_def(r.target));
            let d = self.local_defs.resolve_ufcs_assoc_fn(ImplTargetKey::Unknown, tr_def, fn_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table);
            return if d.is_some() { ResTarget::Def { def: d, local: LocalItemId::NONE, kind: ItemKind::AssocFn } } else { fallback };
        }
        let qr = self.st.range(qual_seg);
        let q_refs = self.file_res.refs_starting_in(qr.start as u32, qr.end as u32);
        let q_target = q_refs.iter().find(|r| (r.path_node == path_node || r.path_node == NodeId(u32::MAX)) && r.end <= qr.end as u32).map(|r| r.target);
        if let Some(ResTarget::Generic(gid)) = q_target {
            for &(bgid, btr) in &self.root_file_res.param_trait_bounds {
                let d = if bgid == gid && btr.is_some() {
                    self.local_defs.resolve_ufcs_assoc_fn(ImplTargetKey::Unknown, btr, fn_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table)
                } else {
                    GlobalDefId::NONE
                };
                if d.is_some() {
                    return ResTarget::Def { def: d, local: LocalItemId::NONE, kind: ItemKind::AssocFn };
                }
            }
            return fallback;
        }
        let (target_k, tr_def) = match q_target {
            Some(ResTarget::SelfTy { target, trait_def }) => (target, trait_def),
            Some(ResTarget::Prim(p)) => (ImplTargetKey::Prim(p), GlobalDefId::NONE),
            Some(rt @ ResTarget::Def { kind: ItemKind::Trait, .. }) => (ImplTargetKey::Unknown, effective_target_def(rt)),
            Some(rt @ ResTarget::Def { .. }) => (ImplTargetKey::Adt(effective_target_def(rt)), GlobalDefId::NONE),
            _ => return fallback,
        };
        let d = self.local_defs.resolve_ufcs_assoc_fn(target_k, tr_def, fn_name, self.root_st, self.root_file_res, self.db, &mut self.ctx.table);
        let assoc_res = ResTarget::Def { def: d, local: LocalItemId::NONE, kind: ItemKind::AssocFn };
        if d.is_some() {
            assoc_res
        } else {
            fallback
        }
    }
}
