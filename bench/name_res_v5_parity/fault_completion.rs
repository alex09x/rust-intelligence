/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Exact completion method-set equivalence verifier for `PrefixHead` partial-`x.` receivers (`m=15061`).
//!
//! Replaces every `{unknown}` (and any unnormalized projection containing `{unknown}`) in `our_ty`
//! with a fresh rigid skolem ADT (`CrateId(0xFFFE)`) that implements zero traits and has zero
//! inherent methods, then compares the full `(method_name, canonical_method_def)` completion set
//! against `rustc`'s concrete receiver type (`comp_ty`). Specialization impls (`impl Vec<u8>`,
//! `[u8]::to_ascii_lowercase`) and conditional trait/method bounds (`T: Clone`, `E: Debug`,
//! `Iterator::cloned` where `Item = &T, T: Clone`) therefore fail on the skolemized type and are
//! excluded from `dot_recv_prefix_eq_methods`.

use std::collections::HashMap;

use ridx_syntax::name_res::{CrateId, FileNameRes, GlobalDefId, ImplTargetKey};
use ridx_syntax::ty_infer::autoderef::{build_autoderef_steps_with_owner, impl_target_key_of_ty, NormOwner, MAX_AUTODEREF_STEPS};
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{BodyInferCtx, InferTyArena, InferTyId, InferTyKind, LocalCstDefs};
use ridx_syntax::{NodeId, SyntaxTree};

use crate::ty_report::ty_contains_unknown;
use ridx::live::v5_db::RidxV5ItemDb;

const SKOLEM_CRATE: CrateId = CrateId(0xFFFE);

pub struct CompletionMethodIndex<'w> {
    inherent_names_by_head: HashMap<ImplTargetKey, Vec<&'w str>>,
    trait_method_names: HashMap<GlobalDefId, Vec<&'w str>>,
}

impl<'w> CompletionMethodIndex<'w> {
    pub fn build(db: &RidxV5ItemDb<'w>) -> Self {
        let mut inherent_names_by_head: HashMap<ImplTargetKey, Vec<&'w str>> = HashMap::new();
        for (&(head, _), list) in &db.ty_tables.inherent_methods {
            let entry = inherent_names_by_head.entry(head).or_default();
            for info in list {
                entry.push(info.name);
            }
        }
        for names in inherent_names_by_head.values_mut() {
            names.sort_unstable();
            names.dedup();
        }
        let mut trait_method_names: HashMap<GlobalDefId, Vec<&'w str>> = HashMap::new();
        for (&(tr, _), info) in &db.ty_tables.trait_methods {
            trait_method_names.entry(tr).or_default().push(info.name);
        }
        for names in trait_method_names.values_mut() {
            names.sort_unstable();
            names.dedup();
        }
        Self { inherent_names_by_head, trait_method_names }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prefix_head_has_identical_methods(
        &self,
        db: &RidxV5ItemDb<'w>,
        mst: &SyntaxTree<'_>,
        res_buf: &FileNameRes,
        local_defs: &LocalCstDefs,
        infer_ctx: &mut BodyInferCtx,
        fn_gid: GlobalDefId,
        m_fn: NodeId,
        dot_end: u32,
        our_ty: InferTyId,
        comp_ty: InferTyId,
    ) -> bool {
        let mut sk_idx = 0u32;
        let skolem_our = skolemize_unknowns(&mut infer_ctx.table.arena, our_ty, &mut sk_idx, 0);
        let at_node = find_deepest_node_at(mst, m_fn, dot_end.saturating_sub(1));

        let snap = infer_ctx.table.start_snapshot();
        let owner = NormOwner { fn_def: fn_gid, impl_node: None, impl_gc: 0, fn_node: Some(m_fn), self_ty: None };
        let mut steps_our = [InferTyId::UNKNOWN; MAX_AUTODEREF_STEPS];
        let mut steps_comp = [InferTyId::UNKNOWN; MAX_AUTODEREF_STEPS];
        let len_our = build_autoderef_steps_with_owner(&mut infer_ctx.table, db, local_defs, mst, res_buf, owner, skolem_our, &mut steps_our);
        let len_comp = build_autoderef_steps_with_owner(&mut infer_ctx.table, db, local_defs, mst, res_buf, owner, comp_ty, &mut steps_comp);
        infer_ctx.table.rollback_to(snap);

        let mut cand_names: Vec<&str> = Vec::with_capacity(128);
        local_defs.collect_cst_method_names(mst, &mut cand_names);

        let mut traits: Vec<GlobalDefId> = Vec::with_capacity(64);
        res_buf.collect_traits_in_scope_at(at_node, &mut traits);

        for &st_ty in steps_our[..len_our].iter().chain(steps_comp[..len_comp].iter()) {
            let head = canon_head(db, impl_target_key_of_ty(&infer_ctx.table.arena, st_ty));
            if let Some(ns) = self.inherent_names_by_head.get(&head) {
                cand_names.extend_from_slice(ns);
            }
            if matches!(infer_ctx.table.arena.kind(st_ty), InferTyKind::Array { .. }) {
                if let Some(ns) = self.inherent_names_by_head.get(&ImplTargetKey::Slice) {
                    cand_names.extend_from_slice(ns);
                }
            }
            for rhead in [ImplTargetKey::Ref { is_mut: false }, ImplTargetKey::Ref { is_mut: true }] {
                if let Some(ns) = self.inherent_names_by_head.get(&rhead) {
                    cand_names.extend_from_slice(ns);
                }
            }
            match infer_ctx.table.arena.kind(st_ty) {
                InferTyKind::Dyn { principal, .. } | InferTyKind::Opaque { def: principal, .. } if principal.is_some() => {
                    if !traits.contains(&principal) {
                        traits.push(principal);
                    }
                }
                InferTyKind::Param { idx, .. } if fn_gid.is_some() => {
                    use ridx_syntax::ty_infer::TyDefDb;
                    db.param_trait_bounds(fn_gid, idx, &mut traits);
                }
                _ => {}
            }
        }
        let mut cursor = 0usize;
        while cursor < traits.len() && traits.len() < 128 {
            let tdef = traits[cursor];
            cursor += 1;
            local_defs.collect_supertraits_into(tdef, mst, res_buf, db, &mut traits);
        }
        for tdef in traits {
            if let Some(ns) = self.trait_method_names.get(&db.canonical_def(tdef)) {
                cand_names.extend_from_slice(ns);
            }
        }
        cand_names.sort_unstable();
        cand_names.dedup();

        let mut defs_our: Vec<(u32, GlobalDefId)> = Vec::with_capacity(cand_names.len());
        let mut defs_comp: Vec<(u32, GlobalDefId)> = Vec::with_capacity(cand_names.len());
        infer_ctx.collect_all_completion_methods(mst, res_buf, db, local_defs, m_fn, at_node, skolem_our, &cand_names, &mut defs_our);
        infer_ctx.collect_all_completion_methods(mst, res_buf, db, local_defs, m_fn, at_node, comp_ty, &cand_names, &mut defs_comp);
        for (_, d) in &mut defs_our {
            *d = db.canonical_def(*d);
        }
        for (_, d) in &mut defs_comp {
            *d = db.canonical_def(*d);
        }
        defs_our.sort_unstable_by_key(|&(ni, d)| (ni, d.krate.0, d.index));
        defs_our.dedup();
        defs_comp.sort_unstable_by_key(|&(ni, d)| (ni, d.krate.0, d.index));
        defs_comp.dedup();
        defs_our == defs_comp
    }
}

#[inline]
fn canon_head(db: &RidxV5ItemDb<'_>, key: ImplTargetKey) -> ImplTargetKey {
    match key {
        ImplTargetKey::Adt(d) => ImplTargetKey::Adt(db.canonical_def(d)),
        ImplTargetKey::Dyn(d) => ImplTargetKey::Dyn(db.canonical_def(d)),
        other => other,
    }
}

fn find_deepest_node_at(st: &SyntaxTree<'_>, root: NodeId, byte_off: u32) -> NodeId {
    let b = byte_off as usize;
    let mut cur = root;
    loop {
        let Some(next) = st.children(cur).find(|&ch| {
            let r = st.range(ch);
            r.start <= b && b < r.end
        }) else {
            return cur;
        };
        cur = next;
    }
}

fn skolemize_unknowns(arena: &mut InferTyArena, ty: InferTyId, next_skolem: &mut u32, depth: u8) -> InferTyId {
    if !ty.is_some() || depth > 24 || !ty_contains_unknown(arena, ty, depth) {
        return ty;
    }
    let fresh_skolem = |a: &mut InferTyArena, c: &mut u32| -> InferTyId {
        let id = *c;
        *c += 1;
        a.mk_adt(GlobalDefId::new(SKOLEM_CRATE, id), &[])
    };
    match arena.kind(ty) {
        InferTyKind::Unknown | InferTyKind::Infer(_) | InferTyKind::Alias { .. } => fresh_skolem(arena, next_skolem),
        InferTyKind::Ref { is_mut, inner } => {
            let ni = skolemize_unknowns(arena, inner, next_skolem, depth + 1);
            arena.mk_ref(is_mut, ni)
        }
        InferTyKind::Ptr { is_mut, inner } => {
            let ni = skolemize_unknowns(arena, inner, next_skolem, depth + 1);
            arena.mk_ptr(is_mut, ni)
        }
        InferTyKind::Slice(elem) => {
            let ne = skolemize_unknowns(arena, elem, next_skolem, depth + 1);
            arena.mk_slice(ne)
        }
        InferTyKind::Array { elem, len } => {
            let ne = skolemize_unknowns(arena, elem, next_skolem, depth + 1);
            arena.mk_array(ne, len)
        }
        InferTyKind::Tuple(r) => {
            let mut buf = InlineTyBuf::new();
            for i in 0..r.len as usize {
                let x = arena.slice(r)[i];
                buf.push(skolemize_unknowns(arena, x, next_skolem, depth + 1));
            }
            arena.mk_tuple(buf.as_slice())
        }
        InferTyKind::Adt { def, args } => {
            let mut buf = InlineTyBuf::new();
            for i in 0..args.len as usize {
                let x = arena.slice(args)[i];
                buf.push(skolemize_unknowns(arena, x, next_skolem, depth + 1));
            }
            arena.mk_adt(def, buf.as_slice())
        }
        InferTyKind::FnDef { def, args } => {
            let mut buf = InlineTyBuf::new();
            for i in 0..args.len as usize {
                let x = arena.slice(args)[i];
                buf.push(skolemize_unknowns(arena, x, next_skolem, depth + 1));
            }
            arena.mk_fn_def(def, buf.as_slice())
        }
        _ => fresh_skolem(arena, next_skolem),
    }
}
