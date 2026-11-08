/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Invariant structural unification (`equate`), directional Rust coercions (`coerce`: `! -> T`,
//! `&mut T -> &T`, `&[T; N] -> &[T]`, `&T -> *const T`, `Closure/FnDef -> FnPtr`), and multi-arm
//! least-upper-bound (`lub`) over [`InferTyArena`] + [`UnionFindTable`].

use super::arena::InferTyArena;
use super::ty::{is_float_prim, is_int_prim, InferTyId, InferTyKind, TySliceRange, TyVarId, TyVarKind};
use super::uf_table::{InlineTyBuf, UnionFindTable};

/// Structural unification or coercion mismatch reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeError {
    Mismatch { expected: InferTyId, found: InferTyId },
    CyclicType { var: TyVarId, ty: InferTyId },
    IntMismatch { found: InferTyId },
    FloatMismatch { found: InferTyId },
    ArityMismatch { expected: u16, found: u16 },
    MutabilityMismatch,
}

pub type UnifyResult = Result<(), TypeError>;

/// Equates `a` and `b` invariantly, committing variable bindings on success and rolling back all
/// partial bindings on mismatch so a failed `equate` never leaves half-unified composite state.
pub fn equate(arena: &mut InferTyArena, vars: &mut UnionFindTable, a: InferTyId, b: InferTyId) -> UnifyResult {
    let a = vars.shallow_resolve(arena, a);
    let b = vars.shallow_resolve(arena, b);
    if a == b {
        return Ok(());
    }
    let snap = vars.start_snapshot();
    match equate_rec(arena, vars, a, b, 0) {
        Ok(()) => {
            vars.commit(snap);
            Ok(())
        }
        Err(e) => {
            vars.rollback_to(snap);
            Err(e)
        }
    }
}

fn equate_rec(arena: &InferTyArena, vars: &mut UnionFindTable, raw_a: InferTyId, raw_b: InferTyId, depth: u8) -> UnifyResult {
    if depth > 64 {
        return Ok(());
    }
    let a = vars.shallow_resolve(arena, raw_a);
    let b = vars.shallow_resolve(arena, raw_b);
    if a == b {
        return Ok(());
    }
    let ka = arena.kind(a);
    let kb = arena.kind(b);

    // Error-recovery wildcard (`3.5`): `{unknown}` unifies permissively and propagates to unbound vars.
    if ka == InferTyKind::Unknown {
        if let InferTyKind::Infer(vb) = kb {
            if let Err((rb, ..)) = vars.probe_var(vb) {
                vars.bind_root(rb, InferTyId::UNKNOWN);
            }
        }
        return Ok(());
    }
    if kb == InferTyKind::Unknown {
        if let InferTyKind::Infer(va) = ka {
            if let Err((ra, ..)) = vars.probe_var(va) {
                vars.bind_root(ra, InferTyId::UNKNOWN);
            }
        }
        return Ok(());
    }

    match (ka, kb) {
        (InferTyKind::Infer(va), InferTyKind::Infer(vb)) => unify_var_var(vars, va, vb, a, b),
        (InferTyKind::Infer(va), _) => unify_var_ty(arena, vars, va, b),
        (_, InferTyKind::Infer(vb)) => unify_var_ty(arena, vars, vb, a),

        (InferTyKind::Ref { is_mut: m1, inner: i1 }, InferTyKind::Ref { is_mut: m2, inner: i2 })
        | (InferTyKind::Ptr { is_mut: m1, inner: i1 }, InferTyKind::Ptr { is_mut: m2, inner: i2 }) => {
            if m1 != m2 {
                return Err(TypeError::MutabilityMismatch);
            }
            equate_rec(arena, vars, i1, i2, depth + 1)
        }
        (InferTyKind::Slice(e1), InferTyKind::Slice(e2)) => equate_rec(arena, vars, e1, e2, depth + 1),
        (InferTyKind::Array { elem: e1, len: l1 }, InferTyKind::Array { elem: e2, len: l2 }) => {
            if l1 != l2 && l1 != u64::MAX && l2 != u64::MAX {
                return Err(TypeError::Mismatch { expected: a, found: b });
            }
            equate_rec(arena, vars, e1, e2, depth + 1)
        }
        (InferTyKind::Tuple(r1), InferTyKind::Tuple(r2)) => equate_slices(arena, vars, r1, r2, depth + 1),
        (InferTyKind::Adt { def: d1, args: r1 }, InferTyKind::Adt { def: d2, args: r2 }) if d1 == d2 => equate_slices(arena, vars, r1, r2, depth + 1),
        (InferTyKind::FnPtr { sig: r1 }, InferTyKind::FnPtr { sig: r2 }) => equate_slices(arena, vars, r1, r2, depth + 1),
        (InferTyKind::FnDef { def: d1, args: r1 }, InferTyKind::FnDef { def: d2, args: r2 }) if d1 == d2 => equate_slices(arena, vars, r1, r2, depth + 1),
        (InferTyKind::Closure { id: i1, sig: r1 }, InferTyKind::Closure { id: i2, sig: r2 }) if i1 == i2 => equate_slices(arena, vars, r1, r2, depth + 1),
        (InferTyKind::Alias { trait_def: t1, assoc_def: a1, self_ty: s1, args: r1 }, InferTyKind::Alias { trait_def: t2, assoc_def: a2, self_ty: s2, args: r2 })
            if t1 == t2 && a1 == a2 =>
        {
            equate_rec(arena, vars, s1, s2, depth + 1)?;
            equate_slices(arena, vars, r1, r2, depth + 1)
        }
        (InferTyKind::Dyn { principal: p1, args: r1 }, InferTyKind::Dyn { principal: p2, args: r2 }) if p1 == p2 => equate_slices(arena, vars, r1, r2, depth + 1),
        (InferTyKind::Opaque { def: d1, args: r1 }, InferTyKind::Opaque { def: d2, args: r2 }) if d1 == d2 => equate_slices(arena, vars, r1, r2, depth + 1),
        _ => Err(TypeError::Mismatch { expected: a, found: b }),
    }
}

fn equate_slices(arena: &InferTyArena, vars: &mut UnionFindTable, r1: TySliceRange, r2: TySliceRange, depth: u8) -> UnifyResult {
    if r1.len != r2.len {
        return Err(TypeError::ArityMismatch { expected: r1.len, found: r2.len });
    }
    for idx in 0..r1.len as usize {
        let e1 = arena.slice(r1)[idx];
        let e2 = arena.slice(r2)[idx];
        equate_rec(arena, vars, e1, e2, depth)?;
    }
    Ok(())
}

fn unify_var_var(vars: &mut UnionFindTable, va: TyVarId, vb: TyVarId, ty_a: InferTyId, ty_b: InferTyId) -> UnifyResult {
    let Err((ra, ka, rank_a, root_ty_a)) = vars.probe_var(va) else {
        unreachable!();
    };
    let Err((rb, kb, rank_b, root_ty_b)) = vars.probe_var(vb) else {
        unreachable!();
    };
    let merged = match (ka, kb) {
        (TyVarKind::General, other) | (other, TyVarKind::General) => other,
        (TyVarKind::IntVar, TyVarKind::IntVar) => TyVarKind::IntVar,
        (TyVarKind::FloatVar, TyVarKind::FloatVar) => TyVarKind::FloatVar,
        (TyVarKind::IntVar, TyVarKind::FloatVar) => return Err(TypeError::IntMismatch { found: ty_b }),
        (TyVarKind::FloatVar, TyVarKind::IntVar) => return Err(TypeError::FloatMismatch { found: ty_a }),
    };
    vars.union_roots((ra, rank_a, root_ty_a), (rb, rank_b, root_ty_b), merged);
    Ok(())
}

fn unify_var_ty(arena: &InferTyArena, vars: &mut UnionFindTable, var: TyVarId, concrete: InferTyId) -> UnifyResult {
    let Err((root, vkind, ..)) = vars.probe_var(var) else {
        unreachable!();
    };
    match vkind {
        TyVarKind::General => {
            if vars.occurs_in(arena, root, concrete) {
                return Err(TypeError::CyclicType { var: root, ty: concrete });
            }
            vars.bind_root(root, concrete);
            Ok(())
        }
        TyVarKind::IntVar => match arena.kind(concrete) {
            InferTyKind::Prim(p) if is_int_prim(p) => {
                vars.bind_root(root, concrete);
                Ok(())
            }
            _ => Err(TypeError::IntMismatch { found: concrete }),
        },
        TyVarKind::FloatVar => match arena.kind(concrete) {
            InferTyKind::Prim(p) if is_float_prim(p) => {
                vars.bind_root(root, concrete);
                Ok(())
            }
            _ => Err(TypeError::FloatMismatch { found: concrete }),
        },
    }
}

/// Coerces `actual` into `expected` (supporting `! -> T`, `&mut T -> &T`, `&[T; N] -> &[T]`,
/// `&T/&mut T -> *const T/*mut T`, and `Closure -> FnPtr`), rolling back cleanly on failure.
pub fn coerce(arena: &mut InferTyArena, vars: &mut UnionFindTable, actual: InferTyId, expected: InferTyId) -> UnifyResult {
    let actual = vars.shallow_resolve(arena, actual);
    let expected = vars.shallow_resolve(arena, expected);
    if actual == expected || actual == InferTyId::UNKNOWN || expected == InferTyId::UNKNOWN {
        return equate(arena, vars, actual, expected);
    }
    if actual == InferTyId::NEVER {
        // Diverging expressions (`return`, `break`, `panic!`) coerce to any concrete or variable target
        // without constraining an unbound inference variable (`let x = if c { return } else { 5u8 };`).
        return Ok(());
    }
    let ka = arena.kind(actual);
    let ke = arena.kind(expected);
    match (ka, ke) {
        // `&mut [T; N]` or `&[T; N]` -> `&[U]` (and `&mut [T; N]` -> `&mut [U]`).
        (InferTyKind::Ref { is_mut: ma, inner: ia }, InferTyKind::Ref { is_mut: me, inner: ie }) if ma || !me => {
            let ia_res = vars.shallow_resolve(arena, ia);
            let ie_res = vars.shallow_resolve(arena, ie);
            if let (InferTyKind::Array { elem: arr_e, .. }, InferTyKind::Slice(sl_e)) = (arena.kind(ia_res), arena.kind(ie_res)) {
                return equate(arena, vars, arr_e, sl_e);
            }
            if ma && !me {
                return equate(arena, vars, ia_res, ie_res);
            }
            equate(arena, vars, actual, expected)
        }
        // `&mut T -> *mut U / *const U` and `&T -> *const U`.
        (InferTyKind::Ref { is_mut: ma, inner: ia }, InferTyKind::Ptr { is_mut: me, inner: ie }) if ma || !me => equate(arena, vars, ia, ie),
        // `*mut T -> *const U`.
        (InferTyKind::Ptr { is_mut: true, inner: ia }, InferTyKind::Ptr { is_mut: false, inner: ie }) => equate(arena, vars, ia, ie),
        // Non-capturing `Closure { sig } -> FnPtr { sig }`.
        (InferTyKind::Closure { sig: s_act, .. }, InferTyKind::FnPtr { sig: s_exp }) => {
            let snap = vars.start_snapshot();
            match equate_slices(arena, vars, s_act, s_exp, 0) {
                Ok(()) => {
                    vars.commit(snap);
                    Ok(())
                }
                Err(e) => {
                    vars.rollback_to(snap);
                    Err(e)
                }
            }
        }
        _ => equate(arena, vars, actual, expected),
    }
}

/// Computes the least upper bound (`LUB`) of two branch types (`if`/`else`, `match` arms).
pub fn lub(arena: &mut InferTyArena, vars: &mut UnionFindTable, a: InferTyId, b: InferTyId) -> Result<InferTyId, TypeError> {
    let a = vars.shallow_resolve(arena, a);
    let b = vars.shallow_resolve(arena, b);
    if a == b {
        return Ok(a);
    }
    if a == InferTyId::NEVER {
        return Ok(b);
    }
    if b == InferTyId::NEVER {
        return Ok(a);
    }
    if a == InferTyId::UNKNOWN || b == InferTyId::UNKNOWN {
        let _ = equate(arena, vars, a, b);
        return Ok(InferTyId::UNKNOWN);
    }
    let ka = arena.kind(a);
    let kb = arena.kind(b);
    match (ka, kb) {
        (InferTyKind::Ref { is_mut: ma, inner: ia }, InferTyKind::Ref { is_mut: mb, inner: ib }) => {
            let ia_res = vars.shallow_resolve(arena, ia);
            let ib_res = vars.shallow_resolve(arena, ib);
            let joined_mut = ma && mb;
            match (arena.kind(ia_res), arena.kind(ib_res)) {
                (InferTyKind::Array { elem: ea, .. }, InferTyKind::Slice(eb)) | (InferTyKind::Slice(eb), InferTyKind::Array { elem: ea, .. }) => {
                    equate(arena, vars, ea, eb)?;
                    let sl = arena.mk_slice(eb);
                    Ok(arena.mk_ref(joined_mut, sl))
                }
                (InferTyKind::Array { elem: ea, len: la }, InferTyKind::Array { elem: eb, len: lb }) if la != lb && la != u64::MAX && lb != u64::MAX => {
                    equate(arena, vars, ea, eb)?;
                    let sl = arena.mk_slice(ea);
                    Ok(arena.mk_ref(joined_mut, sl))
                }
                _ => {
                    equate(arena, vars, ia_res, ib_res)?;
                    Ok(arena.mk_ref(joined_mut, ia_res))
                }
            }
        }
        (InferTyKind::Tuple(r1), InferTyKind::Tuple(r2)) if r1.len == r2.len => {
            let snap = vars.start_snapshot();
            let mut joined = InlineTyBuf::new();
            for idx in 0..r1.len as usize {
                let e1 = arena.slice(r1)[idx];
                let e2 = arena.slice(r2)[idx];
                match lub(arena, vars, e1, e2) {
                    Ok(j) => joined.push(j),
                    Err(err) => {
                        vars.rollback_to(snap);
                        return Err(err);
                    }
                }
            }
            vars.commit(snap);
            Ok(arena.mk_tuple(joined.as_slice()))
        }
        (InferTyKind::Closure { sig: sc, .. }, InferTyKind::FnPtr { sig: sf }) | (InferTyKind::FnPtr { sig: sf }, InferTyKind::Closure { sig: sc, .. }) => {
            let snap = vars.start_snapshot();
            if let Err(e) = equate_slices(arena, vars, sc, sf, 0) {
                vars.rollback_to(snap);
                return Err(e);
            }
            vars.commit(snap);
            Ok(if matches!(ka, InferTyKind::FnPtr { .. }) { a } else { b })
        }
        _ => {
            equate(arena, vars, a, b)?;
            Ok(vars.shallow_resolve(arena, a))
        }
    }
}
