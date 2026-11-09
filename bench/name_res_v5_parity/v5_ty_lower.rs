/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Zero-copy `.ridx` `v5` `TyArenaView` -> `InferTyArena` type lowerer with lifetime-erased
//! `ParamRemap` alignment across standalone definitions, `impl` blocks, traits, and `.exprty` bodies.

use ridx_format::{
    ConstArg, FloatTyWidth, FnAbiKind, GenericArgIter, GenericArgView, GenericParamIter, GenericParamKind, IntTyWidth, LangItemKind, TyArenaView, TyDefId, TyId, TyKindView,
    TyMutability, UintTyWidth, WherePredIter, WherePredKind,
};
use ridx_syntax::name_res::{CrateId, GlobalDefId, PrimTy};
use ridx_syntax::ty_infer::uf_table::InlineTyBuf;
use ridx_syntax::ty_infer::{InferTyArena, InferTyId};

use crate::v5_db::RidxV5ItemDb;

#[derive(Clone, Copy)]
struct ParamSlot {
    raw_idx: u16,
    mapped_ty: InferTyId,
}

/// Maps `rustc` `TyKind::Param.index` (`0..total_generics`, including lifetimes) onto dense
/// non-lifetime `InferTyKind::Param { idx, def }` (or `Opaque` for body `impl Trait` args).
pub struct ParamRemap {
    slots: [ParamSlot; 32],
    len: usize,
    pub non_lt_count: u16,
}

impl ParamRemap {
    #[inline]
    pub const fn empty() -> Self {
        Self { slots: [ParamSlot { raw_idx: u16::MAX, mapped_ty: InferTyId::UNKNOWN }; 32], len: 0, non_lt_count: 0 }
    }

    pub fn for_impl(db: &RidxV5ItemDb<'_>, r_idx: usize, impl_idx: u32, arena: &mut InferTyArena) -> Self {
        let mut out = Self::empty();
        let reader = &db.ws.readers()[r_idx];
        out.append_params(db, r_idx, reader.generic_params_for_impl(impl_idx), None, false, arena);
        out
    }

    pub fn for_def(db: &RidxV5ItemDb<'_>, r_idx: usize, didx: u32, for_body_exprs: bool, arena: &mut InferTyArena) -> Self {
        let mut out = Self::empty();
        if didx == u32::MAX {
            return out;
        }
        let reader = &db.ws.readers()[r_idx];
        let gid = GlobalDefId::new(CrateId(r_idx as u32), didx);
        if let Some(&impl_idx) = db.ty_tables.assoc_to_impl.get(&gid) {
            out.append_params(db, r_idx, reader.generic_params_for_impl(impl_idx), None, false, arena);
        } else if let Some(&trait_didx) = db.shards[r_idx].trait_parent_of_assoc.get(&didx) {
            out.append_params(db, r_idx, reader.generic_params_for_def(trait_didx), None, false, arena);
        }
        let preds = for_body_exprs.then(|| reader.where_preds_for_def(didx));
        out.append_params(db, r_idx, reader.generic_params_for_def(didx), preds, for_body_exprs, arena);
        out
    }

    fn append_params(
        &mut self,
        db: &RidxV5ItemDb<'_>,
        r_idx: usize,
        params: GenericParamIter<'_>,
        preds: Option<WherePredIter<'_>>,
        synth_as_opaque: bool,
        arena: &mut InferTyArena,
    ) {
        let krate = CrateId(r_idx as u32);
        let reader_arena = db.ws.readers()[r_idx].ty_arena();
        for p in params {
            if matches!(p.kind, GenericParamKind::Lifetime) {
                continue;
            }
            if self.len >= self.slots.len() {
                break;
            }
            if synth_as_opaque && p.is_synthetic() && !p.is_self() {
                let opq = preds.and_then(|wp_iter| synth_param_to_opaque(db, r_idx, reader_arena, wp_iter, u32::from(p.param_idx), self, arena)).unwrap_or(InferTyId::UNKNOWN);
                self.slots[self.len] = ParamSlot { raw_idx: p.param_idx, mapped_ty: opq };
                self.len += 1;
                continue;
            }
            let dense = self.non_lt_count;
            self.non_lt_count = self.non_lt_count.saturating_add(1);
            let pdef = p.def_index.map_or(GlobalDefId::NONE, |di| GlobalDefId::new(krate, di));
            let mty = arena.mk_param(dense, pdef);
            self.slots[self.len] = ParamSlot { raw_idx: p.param_idx, mapped_ty: mty };
            self.len += 1;
        }
    }

    #[inline]
    pub fn resolve(&self, raw_idx: u32) -> InferTyId {
        let Ok(r16) = u16::try_from(raw_idx) else {
            return InferTyId::UNKNOWN;
        };
        for i in 0..self.len {
            if self.slots[i].raw_idx == r16 {
                return self.slots[i].mapped_ty;
            }
        }
        InferTyId::UNKNOWN
    }

    #[inline]
    pub fn dense_idx_of(&self, raw_idx: u32, arena: &InferTyArena) -> Option<u16> {
        let ty = self.resolve(raw_idx);
        match arena.kind(ty) {
            ridx_syntax::ty_infer::InferTyKind::Param { idx, .. } => Some(idx),
            _ => None,
        }
    }
}

fn synth_param_to_opaque(
    db: &RidxV5ItemDb<'_>,
    r_idx: usize,
    va: TyArenaView<'_>,
    preds: WherePredIter<'_>,
    raw_param: u32,
    remap: &ParamRemap,
    arena: &mut InferTyArena,
) -> Option<InferTyId> {
    let sized_gid = db.ty_tables.lang_to_gid.get(&(LangItemKind::Sized as u16)).copied().unwrap_or(GlobalDefId::NONE);
    for wp in preds {
        if wp.kind() != WherePredKind::TraitBound || wp.is_negative() {
            continue;
        }
        if !matches!(va.kind(wp.lhs_ty()), TyKindView::Param { index, .. } if index == raw_param) {
            continue;
        }
        let Some(td) = wp.trait_def() else {
            continue;
        };
        let tgid = db.ty_def_gid(r_idx, td);
        if !tgid.is_some() || tgid == sized_gid {
            continue;
        }
        let mut args = InlineTyBuf::new();
        lower_generic_args(db, r_idx, va, wp.trait_args(), remap, arena, &mut args, 1);
        for p2 in preds {
            if p2.kind() == WherePredKind::ProjectionEq
                && matches!(va.kind(p2.lhs_ty()), TyKindView::Param { index, .. } if index == raw_param)
                && p2.trait_def().is_some_and(|td2| db.ty_def_gid(r_idx, td2) == tgid)
            {
                if let Some(rhs) = p2.rhs_ty() {
                    args.push(lower_v5_ty_rec(db, r_idx, va, rhs, remap, arena, 1));
                }
            }
        }
        return Some(arena.mk_opaque(tgid, args.as_slice()));
    }
    None
}

pub fn lower_v5_ty(db: &RidxV5ItemDb<'_>, r_idx: usize, va: TyArenaView<'_>, ty_id: TyId, remap: &ParamRemap, arena: &mut InferTyArena) -> InferTyId {
    lower_v5_ty_rec(db, r_idx, va, ty_id, remap, arena, 0)
}

fn lower_v5_ty_rec(db: &RidxV5ItemDb<'_>, r_idx: usize, va: TyArenaView<'_>, ty_id: TyId, remap: &ParamRemap, arena: &mut InferTyArena, depth: u8) -> InferTyId {
    if !ty_id.is_some() || depth > 48 {
        return InferTyId::UNKNOWN;
    }
    match va.kind(ty_id) {
        TyKindView::Unit => InferTyId::UNIT,
        TyKindView::Bool => InferTyId::BOOL,
        TyKindView::Char => InferTyId::CHAR,
        TyKindView::Str => InferTyId::STR,
        TyKindView::Never => InferTyId::NEVER,
        TyKindView::Int(w) => InferTyId::from_prim(match w {
            IntTyWidth::I8 => PrimTy::I8,
            IntTyWidth::I16 => PrimTy::I16,
            IntTyWidth::I32 => PrimTy::I32,
            IntTyWidth::I64 => PrimTy::I64,
            IntTyWidth::I128 => PrimTy::I128,
            IntTyWidth::Isize => PrimTy::Isize,
        }),
        TyKindView::Uint(w) => InferTyId::from_prim(match w {
            UintTyWidth::U8 => PrimTy::U8,
            UintTyWidth::U16 => PrimTy::U16,
            UintTyWidth::U32 => PrimTy::U32,
            UintTyWidth::U64 => PrimTy::U64,
            UintTyWidth::U128 => PrimTy::U128,
            UintTyWidth::Usize => PrimTy::Usize,
        }),
        TyKindView::Float(w) => match w {
            FloatTyWidth::F32 => InferTyId::F32,
            FloatTyWidth::F64 => InferTyId::F64,
            _ => InferTyId::UNKNOWN,
        },
        TyKindView::Ref { mutability, inner, .. } => {
            let sub = lower_v5_ty_rec(db, r_idx, va, inner, remap, arena, depth + 1);
            arena.mk_ref(mutability == TyMutability::Mut, sub)
        }
        TyKindView::Ptr { mutability, inner } => {
            let sub = lower_v5_ty_rec(db, r_idx, va, inner, remap, arena, depth + 1);
            arena.mk_ptr(mutability == TyMutability::Mut, sub)
        }
        TyKindView::Slice(elem) => {
            let sub = lower_v5_ty_rec(db, r_idx, va, elem, remap, arena, depth + 1);
            arena.mk_slice(sub)
        }
        TyKindView::Array { elem, len } => {
            let sub = lower_v5_ty_rec(db, r_idx, va, elem, remap, arena, depth + 1);
            let n = match len {
                ConstArg::Value(v) => u64::from(v),
                _ => u64::MAX,
            };
            arena.mk_array(sub, n)
        }
        TyKindView::Tuple(list) => {
            let mut elems = InlineTyBuf::new();
            for item in list {
                elems.push(lower_v5_ty_rec(db, r_idx, va, item, remap, arena, depth + 1));
            }
            arena.mk_tuple(elems.as_slice())
        }
        TyKindView::Adt { def, args } => {
            let g = db.ty_def_gid(r_idx, def);
            if !g.is_some() {
                return InferTyId::UNKNOWN;
            }
            let mut abuf = InlineTyBuf::new();
            lower_generic_args(db, r_idx, va, args, remap, arena, &mut abuf, depth + 1);
            arena.mk_adt(g, abuf.as_slice())
        }
        TyKindView::FnDef { def, args } => {
            if let Some((cs, ce)) = parse_closure_def_span(va, def) {
                let mut abuf = InlineTyBuf::new();
                lower_generic_args(db, r_idx, va, args, remap, arena, &mut abuf, depth + 1);
                let s = abuf.as_slice();
                return match s.split_last() {
                    Some((&out, ins)) => arena.mk_closure(cs, ce, ins, out),
                    None => arena.mk_closure(cs, ce, &[], InferTyId::UNIT),
                };
            }
            let g = db.ty_def_gid(r_idx, def);
            if !g.is_some() {
                return InferTyId::UNKNOWN;
            }
            let mut abuf = InlineTyBuf::new();
            lower_generic_args(db, r_idx, va, args, remap, arena, &mut abuf, depth + 1);
            arena.mk_fn_def(g, abuf.as_slice())
        }
        TyKindView::FnPtr { inputs, output, abi, .. } => {
            let mut ins = InlineTyBuf::new();
            for inp in inputs {
                ins.push(lower_v5_ty_rec(db, r_idx, va, inp, remap, arena, depth + 1));
            }
            let out = lower_v5_ty_rec(db, r_idx, va, output, remap, arena, depth + 1);
            if abi == FnAbiKind::RustCall {
                arena.mk_closure(0, 0, ins.as_slice(), out)
            } else {
                arena.mk_fn_ptr(ins.as_slice(), out)
            }
        }
        TyKindView::Param { index, .. } => remap.resolve(index),
        TyKindView::Assoc { trait_def, name, self_ty, trait_args } => {
            let tgid = db.ty_def_gid(r_idx, trait_def);
            let agid = db.lookup_trait_assoc_ty_def(tgid, name);
            let sty = lower_v5_ty_rec(db, r_idx, va, self_ty, remap, arena, depth + 1);
            let mut abuf = InlineTyBuf::new();
            lower_generic_args(db, r_idx, va, trait_args, remap, arena, &mut abuf, depth + 1);
            arena.mk_alias(tgid, agid, sty, abuf.as_slice())
        }
        TyKindView::Dyn { bounds, .. } => lower_existential_bounds(db, r_idx, va, bounds, true, remap, arena, depth + 1),
        TyKindView::ImplTrait { bounds } => lower_existential_bounds(db, r_idx, va, bounds, false, remap, arena, depth + 1),
        TyKindView::Unknown => InferTyId::UNKNOWN,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn lower_generic_args(
    db: &RidxV5ItemDb<'_>,
    r_idx: usize,
    va: TyArenaView<'_>,
    args: GenericArgIter<'_>,
    remap: &ParamRemap,
    arena: &mut InferTyArena,
    out: &mut InlineTyBuf,
    depth: u8,
) {
    for ga in args {
        match ga {
            GenericArgView::Lifetime(_) => {}
            GenericArgView::Type(tid) => {
                out.push(lower_v5_ty_rec(db, r_idx, va, tid, remap, arena, depth));
            }
            GenericArgView::Const(c) => {
                let cty = match c {
                    ConstArg::Param(idx) => remap.resolve(idx),
                    _ => InferTyId::UNKNOWN,
                };
                out.push(cty);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_existential_bounds(
    db: &RidxV5ItemDb<'_>,
    r_idx: usize,
    va: TyArenaView<'_>,
    bounds: ridx_format::TyBoundIter<'_>,
    is_dyn: bool,
    remap: &ParamRemap,
    arena: &mut InferTyArena,
    depth: u8,
) -> InferTyId {
    let sized_gid = db.ty_tables.lang_to_gid.get(&(LangItemKind::Sized as u16)).copied().unwrap_or(GlobalDefId::NONE);
    let mut chosen = None;
    for b in bounds {
        if b.is_maybe {
            continue;
        }
        let g = db.ty_def_gid(r_idx, b.trait_def);
        if !g.is_some() || g == sized_gid {
            continue;
        }
        let is_auto = matches!(
            db.ty_tables.gid_to_lang.get(&g),
            Some(ridx_syntax::ty_infer::TyLangItem::Sync | ridx_syntax::ty_infer::TyLangItem::Freeze | ridx_syntax::ty_infer::TyLangItem::Unpin)
        );
        if chosen.is_none() || !is_auto {
            chosen = Some((g, b));
            if !is_auto {
                break;
            }
        }
    }
    let Some((principal, b)) = chosen else {
        return InferTyId::UNKNOWN;
    };
    let mut args = InlineTyBuf::new();
    lower_generic_args(db, r_idx, va, b.args, remap, arena, &mut args, depth);
    for assoc in b.assocs {
        args.push(lower_v5_ty_rec(db, r_idx, va, assoc.ty, remap, arena, depth));
    }
    if is_dyn {
        arena.mk_dyn(principal, args.as_slice())
    } else {
        arena.mk_opaque(principal, args.as_slice())
    }
}

#[inline]
fn parse_closure_def_span(va: TyArenaView<'_>, def: TyDefId) -> Option<(u32, u32)> {
    let td = va.def(def)?;
    let rest = td.path.strip_prefix("{closure}@")?;
    let (s_str, e_str) = rest.split_once("..")?;
    Some((s_str.parse().ok()?, e_str.parse().ok()?))
}
