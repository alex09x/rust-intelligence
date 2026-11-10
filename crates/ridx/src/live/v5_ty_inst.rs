/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Stack-only type parameter substitution (`instantiate_v5`) for `.ridx` `v5` lowered types.

use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{InferTyArena, InferTyId, InferTyKind, TySliceRange};

pub fn instantiate_v5(arena: &mut InferTyArena, ty: InferTyId, subst: &[InferTyId], depth: u8) -> InferTyId {
    if depth > 48 || !arena.flags(ty).has_param() {
        return ty;
    }
    match arena.kind(ty) {
        InferTyKind::Param { idx, .. } => subst.get(idx as usize).copied().unwrap_or(InferTyId::UNKNOWN),
        InferTyKind::Ref { is_mut, inner } => {
            let s = instantiate_v5(arena, inner, subst, depth + 1);
            arena.mk_ref(is_mut, s)
        }
        InferTyKind::Ptr { is_mut, inner } => {
            let s = instantiate_v5(arena, inner, subst, depth + 1);
            arena.mk_ptr(is_mut, s)
        }
        InferTyKind::Slice(e) => {
            let s = instantiate_v5(arena, e, subst, depth + 1);
            arena.mk_slice(s)
        }
        InferTyKind::Array { elem, len } => {
            let s = instantiate_v5(arena, elem, subst, depth + 1);
            let l = arena.resolve_array_len(len, subst);
            arena.mk_array(s, l)
        }
        InferTyKind::Tuple(r) => {
            let b = inst_slice_v5(arena, r, subst, depth + 1);
            arena.mk_tuple(b.as_slice())
        }
        InferTyKind::Adt { def, args } => {
            let b = inst_slice_v5(arena, args, subst, depth + 1);
            arena.mk_adt(def, b.as_slice())
        }
        InferTyKind::FnPtr { sig } => {
            let b = inst_slice_v5(arena, sig, subst, depth + 1);
            let s = b.as_slice();
            arena.mk_fn_ptr(&s[..s.len() - 1], s[s.len() - 1])
        }
        InferTyKind::FnDef { def, args } => {
            let b = inst_slice_v5(arena, args, subst, depth + 1);
            arena.mk_fn_def(def, b.as_slice())
        }
        InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
            let (ns, b) = (instantiate_v5(arena, self_ty, subst, depth + 1), inst_slice_v5(arena, args, subst, depth + 1));
            arena.mk_alias(trait_def, assoc_def, ns, b.as_slice())
        }
        InferTyKind::Dyn { principal, args } | InferTyKind::Opaque { def: principal, args } => {
            let b = inst_slice_v5(arena, args, subst, depth + 1);
            if matches!(arena.kind(ty), InferTyKind::Dyn { .. }) {
                arena.mk_dyn(principal, b.as_slice())
            } else {
                arena.mk_opaque(principal, b.as_slice())
            }
        }
        _ => ty,
    }
}

fn inst_slice_v5(arena: &mut InferTyArena, r: TySliceRange, subst: &[InferTyId], depth: u8) -> InlineTyBuf {
    let mut b = InlineTyBuf::new();
    for idx in 0..r.len as usize {
        b.push(instantiate_v5(arena, arena.slice(r)[idx], subst, depth));
    }
    b
}

pub(crate) fn clone_reader_ty<A: super::query_ty_fmt::TyReader>(src: &A, id: InferTyId, dst: &mut InferTyArena) -> InferTyId {
    clone_reader_ty_rec(src, id, dst, 0)
}

fn clone_reader_ty_rec<A: super::query_ty_fmt::TyReader>(src: &A, id: InferTyId, dst: &mut InferTyArena, depth: u8) -> InferTyId {
    if !id.is_some() || (id.0 as usize) < InferTyId::BUILTIN_COUNT || depth > 48 {
        return id;
    }
    let map_s = |r, dst: &mut InferTyArena| {
        let mut b = InlineTyBuf::new();
        for &e in src.slice(r) {
            b.push(clone_reader_ty_rec(src, e, dst, depth + 1));
        }
        b
    };
    match src.kind(id) {
        InferTyKind::Unknown | InferTyKind::Infer(_) => InferTyId::UNKNOWN,
        InferTyKind::Never => InferTyId::NEVER,
        InferTyKind::Prim(p) => InferTyId::from_prim(p),
        InferTyKind::Ref { is_mut, inner } => {
            let s = clone_reader_ty_rec(src, inner, dst, depth + 1);
            dst.mk_ref(is_mut, s)
        }
        InferTyKind::Ptr { is_mut, inner } => {
            let s = clone_reader_ty_rec(src, inner, dst, depth + 1);
            dst.mk_ptr(is_mut, s)
        }
        InferTyKind::Slice(e) => {
            let s = clone_reader_ty_rec(src, e, dst, depth + 1);
            dst.mk_slice(s)
        }
        InferTyKind::Array { elem, len } => {
            let s = clone_reader_ty_rec(src, elem, dst, depth + 1);
            dst.mk_array(s, len)
        }
        InferTyKind::Param { idx, def } => dst.mk_param(idx, def),
        InferTyKind::Tuple(r) => {
            let b = map_s(r, dst);
            dst.mk_tuple(b.as_slice())
        }
        InferTyKind::Adt { def, args } => {
            let b = map_s(args, dst);
            dst.mk_adt(def, b.as_slice())
        }
        InferTyKind::FnPtr { sig } => {
            let b = map_s(sig, dst);
            let s = b.as_slice();
            dst.mk_fn_ptr(&s[..s.len() - 1], s[s.len() - 1])
        }
        InferTyKind::FnDef { def, args } => {
            let b = map_s(args, dst);
            dst.mk_fn_def(def, b.as_slice())
        }
        InferTyKind::Closure { def_start, def_end, sig } => {
            let b = map_s(sig, dst);
            let s = b.as_slice();
            dst.mk_closure(def_start, def_end, &s[..s.len() - 1], s[s.len() - 1])
        }
        InferTyKind::Alias { trait_def, assoc_def, self_ty, args } => {
            let st = clone_reader_ty_rec(src, self_ty, dst, depth + 1);
            let b = map_s(args, dst);
            dst.mk_alias(trait_def, assoc_def, st, b.as_slice())
        }
        InferTyKind::Dyn { principal, args } => {
            let b = map_s(args, dst);
            dst.mk_dyn(principal, b.as_slice())
        }
        InferTyKind::Opaque { def, args } => {
            let b = map_s(args, dst);
            dst.mk_opaque(def, b.as_slice())
        }
    }
}

pub fn count_def_non_lt_non_self<B: AsRef<[u8]>>(reader: &ridx_format::IndexReader<B>, didx: u32) -> u16 {
    reader.generic_params_for_def(didx).filter(|p| !matches!(p.kind, ridx_format::GenericParamKind::Lifetime) && !p.is_self()).count() as u16
}

pub fn count_impl_non_lt<B: AsRef<[u8]>>(reader: &ridx_format::IndexReader<B>, impl_idx: u32) -> u16 {
    reader.generic_params_for_impl(impl_idx).filter(|p| !matches!(p.kind, ridx_format::GenericParamKind::Lifetime)).count() as u16
}

impl super::v5_db::RidxV5ItemDb<'_> {
    pub(crate) fn detect_impl_self_fn_bound_arity<B: AsRef<[u8]>>(
        &self,
        r_idx: usize,
        reader: &ridx_format::IndexReader<B>,
        arena: ridx_format::TyArenaView<'_>,
        impl_idx: u32,
        self_ty: ridx_format::TyId,
    ) -> u8 {
        let ridx_format::TyKindView::Param { index: self_pidx, .. } = arena.kind(self_ty) else { return u8::MAX };
        for wp in reader.where_preds_for_impl(impl_idx) {
            if wp.kind() != ridx_format::WherePredKind::TraitBound || wp.is_negative() {
                continue;
            }
            let lhs_ok = match arena.kind(wp.lhs_ty()) {
                ridx_format::TyKindView::Param { index, .. } => index == self_pidx,
                ridx_format::TyKindView::Ref { inner, .. } => matches!(arena.kind(inner), ridx_format::TyKindView::Param { index, .. } if index == self_pidx),
                _ => false,
            };
            let Some(td) = wp.trait_def().filter(|_| lhs_ok) else { continue };
            if !self.ty_tables.gid_to_lang.get(&self.canonical_def(self.ty_def_gid(r_idx, td))).is_some_and(|l| l.is_fn_trait()) {
                continue;
            }
            if let Some(tid) = wp.trait_args().find_map(|ga| if let ridx_format::GenericArgView::Type(t) = ga { Some(t) } else { None }) {
                match arena.kind(tid) {
                    ridx_format::TyKindView::Unit => return 0,
                    ridx_format::TyKindView::Tuple(elems) => return elems.len() as u8,
                    _ => {}
                }
            }
        }
        u8::MAX
    }
}

pub(crate) fn extend_flat_groups<K: Eq + std::hash::Hash, T: Copy>(
    arena: &mut Vec<T>,
    map: &mut std::collections::HashMap<K, (u32, u32)>,
    grouped: std::collections::HashMap<K, Vec<T>>,
) {
    let add_total: usize = grouped.values().map(Vec::len).sum();
    arena.reserve(add_total);
    map.reserve(grouped.len());
    for (k, items) in grouped {
        if items.is_empty() {
            continue;
        }
        if let Some(&(old_s, old_l)) = map.get(&k) {
            let new_s = arena.len() as u32;
            let new_l = old_l + items.len() as u32;
            arena.reserve((old_l as usize) + items.len());
            for idx in (old_s as usize)..((old_s + old_l) as usize) {
                let v = arena[idx];
                arena.push(v);
            }
            arena.extend_from_slice(&items);
            map.insert(k, (new_s, new_l));
        } else {
            let start = arena.len() as u32;
            let len = items.len() as u32;
            arena.extend_from_slice(&items);
            map.insert(k, (start, len));
        }
    }
}

#[inline]
pub(crate) fn slice_flat_group<'a, K: Eq + std::hash::Hash, T>(arena: &'a [T], map: &std::collections::HashMap<K, (u32, u32)>, key: &K) -> &'a [T] {
    map.get(key).and_then(|&(s, l)| arena.get((s as usize)..((s + l) as usize))).unwrap_or(&[])
}

impl super::v5_db::RidxV5ItemDb<'_> {
    pub fn lower_ty_to_target_key(&self, r_idx: usize, arena: ridx_format::TyArenaView<'_>, ty_id: ridx_format::TyId, depth: u8) -> ridx_syntax::name_res::ImplTargetKey {
        use ridx_format::{FloatTyWidth, IntTyWidth, TyKindView, TyMutability, UintTyWidth};
        use ridx_syntax::name_res::{ImplTargetKey, PrimTy};
        if depth > 6 {
            return ImplTargetKey::Unknown;
        }
        match arena.kind(ty_id) {
            TyKindView::Unit => ImplTargetKey::Tuple(0),
            TyKindView::Bool => ImplTargetKey::Prim(PrimTy::Bool),
            TyKindView::Char => ImplTargetKey::Prim(PrimTy::Char),
            TyKindView::Str => ImplTargetKey::Prim(PrimTy::Str),
            TyKindView::Never => ImplTargetKey::Never,
            TyKindView::Int(w) => match w {
                IntTyWidth::I8 => ImplTargetKey::Prim(PrimTy::I8),
                IntTyWidth::I16 => ImplTargetKey::Prim(PrimTy::I16),
                IntTyWidth::I32 => ImplTargetKey::Prim(PrimTy::I32),
                IntTyWidth::I64 => ImplTargetKey::Prim(PrimTy::I64),
                IntTyWidth::I128 => ImplTargetKey::Prim(PrimTy::I128),
                IntTyWidth::Isize => ImplTargetKey::Prim(PrimTy::Isize),
            },
            TyKindView::Uint(w) => match w {
                UintTyWidth::U8 => ImplTargetKey::Prim(PrimTy::U8),
                UintTyWidth::U16 => ImplTargetKey::Prim(PrimTy::U16),
                UintTyWidth::U32 => ImplTargetKey::Prim(PrimTy::U32),
                UintTyWidth::U64 => ImplTargetKey::Prim(PrimTy::U64),
                UintTyWidth::U128 => ImplTargetKey::Prim(PrimTy::U128),
                UintTyWidth::Usize => ImplTargetKey::Prim(PrimTy::Usize),
            },
            TyKindView::Float(FloatTyWidth::F32) => ImplTargetKey::Prim(PrimTy::F32),
            TyKindView::Float(FloatTyWidth::F64) => ImplTargetKey::Prim(PrimTy::F64),
            TyKindView::Adt { def, .. } => Some(self.resolve_ty_def(r_idx, arena, def)).filter(|g| g.is_some()).map_or(ImplTargetKey::Unknown, ImplTargetKey::Adt),
            TyKindView::Ref { mutability, .. } => ImplTargetKey::Ref { is_mut: mutability == TyMutability::Mut },
            TyKindView::Ptr { mutability, .. } => ImplTargetKey::Ptr { is_mut: mutability == TyMutability::Mut },
            TyKindView::Slice(_) => ImplTargetKey::Slice,
            TyKindView::Array { .. } => ImplTargetKey::Array,
            TyKindView::Tuple(list) => ImplTargetKey::Tuple(list.count() as u16),
            TyKindView::FnPtr { inputs, .. } => ImplTargetKey::FnPtr { arity: inputs.count() as u16 },
            TyKindView::Dyn { mut bounds, .. } => {
                bounds.next().map(|f| self.resolve_ty_def(r_idx, arena, f.trait_def)).filter(|g| g.is_some()).map_or(ImplTargetKey::Unknown, ImplTargetKey::Dyn)
            }
            TyKindView::Param { .. } => ImplTargetKey::Param,
            _ => ImplTargetKey::Unknown,
        }
    }

    pub(crate) fn probe_inherent_impl_self(
        &self,
        r_idx: usize,
        impl_idx: u32,
        cur: InferTyId,
        arena: &mut InferTyArena,
        vars: &mut ridx_syntax::ty_infer::uf_table::UnionFindTable,
    ) -> bool {
        let reader = &self.ws.readers()[r_idx];
        let Some(iv) = reader.impl_view(impl_idx) else { return false };
        let remap = super::v5_ty_lower::ParamRemap::for_impl(self, r_idx, impl_idx, arena);
        let raw_self = super::v5_ty_lower::lower_v5_ty(self, r_idx, reader.ty_arena(), iv.self_ty(), &remap, arena);
        let snap = vars.start_snapshot();
        let mut subst = InlineTyBuf::new();
        for _ in 0..remap.non_lt_count {
            subst.push(vars.new_var(arena, ridx_syntax::ty_infer::TyVarKind::General).1);
        }
        let inst_self = instantiate_v5(arena, raw_self, subst.as_slice(), 0);
        let ok = ridx_syntax::ty_infer::unify::equate(arena, vars, cur, inst_self).is_ok();
        vars.rollback_to(snap);
        ok
    }
}
