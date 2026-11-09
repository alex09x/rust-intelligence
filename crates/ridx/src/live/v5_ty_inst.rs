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
