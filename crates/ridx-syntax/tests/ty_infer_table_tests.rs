/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for Phase 3 Checkbox `3.1` (`ridx_syntax::ty_infer`):
//! union-find over type variables, invariant/coercive unification, snapshot rollback,
//! occurs-check, literal fallbacks, generic instantiation, and expected-type propagation.

use ridx_syntax::name_res::{CrateId, GlobalDefId, PrimTy};
use ridx_syntax::ty_infer::{BranchCoercer, Expectation, InferTyId, InferTyKind, InferenceTable, TyFlags, TyVarKind, TypeError};

fn adt_id(idx: u32) -> GlobalDefId {
    GlobalDefId::new(CrateId(1), idx)
}

#[test]
fn hash_consing_builtins_and_summary_flags() {
    let mut tab = InferenceTable::new();
    for p in PrimTy::ALL {
        let id = InferTyId::from_prim(p);
        assert_eq!(tab.arena.kind(id), InferTyKind::Prim(p));
        assert_eq!(tab.arena.flags(id), TyFlags::EMPTY);
    }
    assert_eq!(tab.arena.mk_tuple(&[]), InferTyId::UNIT);
    assert_eq!(tab.arena.mk_ref(false, InferTyId::STR), InferTyId::STR_REF);

    let vec_def = adt_id(10);
    let v1 = tab.arena.mk_adt(vec_def, &[InferTyId::U32]);
    let v2 = tab.arena.mk_adt(vec_def, &[InferTyId::U32]);
    let v3 = tab.arena.mk_adt(vec_def, &[InferTyId::U64]);
    assert_eq!(v1, v2);
    assert_ne!(v1, v3);
    assert!(!tab.arena.flags(v1).has_infer());

    let t_var = tab.new_ty_var();
    let v_infer = tab.arena.mk_adt(vec_def, &[t_var]);
    assert!(tab.arena.flags(v_infer).has_infer());
    assert!(!tab.arena.flags(v_infer).has_param());

    let p0 = tab.arena.mk_param(0, adt_id(99));
    let v_param = tab.arena.mk_adt(vec_def, &[p0]);
    assert!(tab.arena.flags(v_param).has_param());
    assert!(!tab.arena.flags(v_param).has_infer());
}

#[test]
fn union_find_chains_numeric_domains_and_default_fallbacks() {
    let mut tab = InferenceTable::new();

    // Chain of general vars tightened by `IntVar`, then unified with `u16`.
    let g1 = tab.new_ty_var();
    let g2 = tab.new_ty_var();
    let g3 = tab.new_ty_var();
    let iv = tab.new_int_var();
    assert!(tab.equate(g1, g2).is_ok());
    assert!(tab.equate(g2, g3).is_ok());
    assert!(tab.equate(g3, iv).is_ok());

    // `g1` now carries `IntVar` domain -> rejects `f32` and `bool`, accepts `u16`.
    assert_eq!(tab.equate(g1, InferTyId::F32), Err(TypeError::IntMismatch { found: InferTyId::F32 }));
    assert_eq!(tab.equate(g1, InferTyId::BOOL), Err(TypeError::IntMismatch { found: InferTyId::BOOL }));
    assert!(tab.equate(g1, InferTyId::U16).is_ok());
    assert_eq!(tab.shallow_resolve(g1), InferTyId::U16);
    assert_eq!(tab.shallow_resolve(iv), InferTyId::U16);

    // Unconstrained `{integer}`, `{float}`, and `_` fall back to `i32`, `f64`, and `{unknown}`.
    let u_int = tab.new_int_var();
    let u_flt = tab.new_float_var();
    let u_gen = tab.new_ty_var();
    let tup = tab.arena.mk_tuple(&[u_int, u_flt, u_gen]);
    let resolved = tab.resolve_with_fallback(tup);
    let expected = tab.arena.mk_tuple(&[InferTyId::I32, InferTyId::F64, InferTyId::UNKNOWN]);
    assert_eq!(resolved, expected);

    // `{integer}` vs `{float}` fails cleanly without corrupting either variable.
    let i2 = tab.new_int_var();
    let f2 = tab.new_float_var();
    assert!(matches!(tab.equate(i2, f2), Err(TypeError::IntMismatch { .. })));
    assert_eq!(tab.resolve_with_fallback(i2), InferTyId::I32);
    assert_eq!(tab.resolve_with_fallback(f2), InferTyId::F64);
}

#[test]
fn occurs_check_rejects_cyclic_types_and_recovers_cleanly() {
    let mut tab = InferenceTable::new();
    let box_def = adt_id(20);
    let (vid, vty) = tab.new_var_with_id(TyVarKind::General);
    let boxed_v = tab.arena.mk_adt(box_def, &[vty]);

    assert_eq!(tab.equate(vty, boxed_v), Err(TypeError::CyclicType { var: vid, ty: boxed_v }));
    // Variable remains unbound after failed cyclic unification!
    assert!(tab.vars.probe_var_imm(vid).is_err());

    // Error-tolerant recovery returns `{unknown}` without hanging.
    assert_eq!(tab.equate_or_recover(vty, boxed_v), InferTyId::UNKNOWN);
}

#[test]
fn snapshot_rollback_restores_path_compressed_and_partial_composite_bindings() {
    let mut tab = InferenceTable::new();
    let pair_def = adt_id(30);

    let a = tab.new_ty_var();
    let b = tab.new_ty_var();
    let c = tab.new_ty_var();
    assert!(tab.equate(a, b).is_ok());
    assert!(tab.equate(b, c).is_ok());

    // Partial composite mismatch: `Pair<?a, u32>` vs `Pair<i64, bool>` must roll back `?a = i64`!
    let lhs = tab.arena.mk_adt(pair_def, &[a, InferTyId::U32]);
    let rhs = tab.arena.mk_adt(pair_def, &[InferTyId::I64, InferTyId::BOOL]);
    assert!(tab.equate(lhs, rhs).is_err());

    // `a`, `b`, `c` must still be unbound and free to unify with `&str`!
    assert!(tab.equate(c, InferTyId::STR_REF).is_ok());
    assert_eq!(tab.shallow_resolve(a), InferTyId::STR_REF);

    // Explicit nested snapshot for overload probing.
    let d = tab.new_int_var();
    let snap = tab.start_snapshot();
    assert!(tab.equate(d, InferTyId::U8).is_ok());
    assert_eq!(tab.shallow_resolve(d), InferTyId::U8);
    tab.rollback_to(snap);
    assert_eq!(tab.resolve_with_fallback(d), InferTyId::I32);
}

#[test]
fn directional_coercions_and_unknown_recovery() {
    let mut tab = InferenceTable::new();

    // `&mut u32 -> &u32` succeeds, while `&u32 -> &mut u32` fails.
    let ref_mut_u32 = tab.arena.mk_ref(true, InferTyId::U32);
    let ref_shr_u32 = tab.arena.mk_ref(false, InferTyId::U32);
    assert!(tab.coerce(ref_mut_u32, ref_shr_u32).is_ok());
    assert_eq!(tab.coerce(ref_shr_u32, ref_mut_u32), Err(TypeError::MutabilityMismatch));

    // Unsized array-to-slice coercion: `&[?int; 4] -> &[u64]` binds `?int = u64`.
    let iv = tab.new_int_var();
    let arr = tab.arena.mk_array(iv, 4);
    let ref_arr = tab.arena.mk_ref(false, arr);
    let sl_u64 = tab.arena.mk_slice(InferTyId::U64);
    let ref_sl_u64 = tab.arena.mk_ref(false, sl_u64);
    assert!(tab.coerce(ref_arr, ref_sl_u64).is_ok());
    assert_eq!(tab.shallow_resolve(iv), InferTyId::U64);

    // Closure -> `fn(i32) -> bool` coercion propagates parameter and return types!
    let c_arg = tab.new_ty_var();
    let c_ret = tab.new_ty_var();
    let clos = tab.arena.mk_closure(0, &[c_arg], c_ret);
    let fn_ptr = tab.arena.mk_fn_ptr(&[InferTyId::I32], InferTyId::BOOL);
    assert!(tab.coerce(clos, fn_ptr).is_ok());
    assert_eq!(tab.shallow_resolve(c_arg), InferTyId::I32);
    assert_eq!(tab.shallow_resolve(c_ret), InferTyId::BOOL);

    // `{unknown}` unifies permissively and propagates to fresh vars without cascading mismatches.
    let unk_var = tab.new_ty_var();
    assert!(tab.equate(unk_var, InferTyId::UNKNOWN).is_ok());
    assert_eq!(tab.shallow_resolve(unk_var), InferTyId::UNKNOWN);
    assert!(tab.equate(unk_var, InferTyId::USIZE).is_ok());
}

#[test]
fn expectation_propagation_and_branch_coercer_lub() {
    let mut tab = InferenceTable::new();

    // `let xs: &[u8] = &[1, 2, 3];` downward expectation propagation:
    let sl_u8 = tab.arena.mk_slice(InferTyId::U8);
    let ref_sl_u8 = tab.arena.mk_ref(false, sl_u8);
    let outer_exp = Expectation::HasType(ref_sl_u8);
    let inner_exp = outer_exp.peel_for_borrow(&mut tab, false);
    assert_eq!(inner_exp, Expectation::RvalueLikeUnsized(sl_u8));
    let elem_exp = inner_exp.array_elem_expectation(&mut tab);
    assert_eq!(elem_exp, Expectation::HasType(InferTyId::U8));

    let lit0 = tab.new_int_var();
    if let Some(target) = elem_exp.coercion_target(&mut tab) {
        assert!(tab.coerce(lit0, target).is_ok());
    }
    assert_eq!(tab.shallow_resolve(lit0), InferTyId::U8);

    // Tuple destructuring expectations: `let (a, b): (i16, f32) = (10, 2.5);`
    let tup_ty = tab.arena.mk_tuple(&[InferTyId::I16, InferTyId::F32]);
    let mut field_exps = Vec::new();
    Expectation::HasType(tup_ty).tuple_elem_expectations(&mut tab, 2, &mut field_exps);
    assert_eq!(field_exps, vec![Expectation::HasType(InferTyId::I16), Expectation::HasType(InferTyId::F32)]);

    // `match` arms with no outer expectation: `return` (`!`), `&mut u32`, `&u32` -> LUB is `&u32`!
    let mut coercer = BranchCoercer::new(&mut tab, Expectation::None);
    let rm = tab.arena.mk_ref(true, InferTyId::U32);
    let rs = tab.arena.mk_ref(false, InferTyId::U32);
    coercer.merge_arm(&mut tab, InferTyId::NEVER);
    coercer.merge_arm(&mut tab, rm);
    coercer.merge_arm(&mut tab, rs);
    assert_eq!(coercer.first_error, None);
    assert_eq!(coercer.finish(&mut tab), rs);

    // All-diverging `match` (`A => return, B => break`) finishes as `!`.
    let mut div_coercer = BranchCoercer::new(&mut tab, Expectation::None);
    div_coercer.merge_arm(&mut tab, InferTyId::NEVER);
    div_coercer.merge_arm(&mut tab, InferTyId::NEVER);
    assert_eq!(div_coercer.finish(&mut tab), InferTyId::NEVER);
}

#[test]
fn generic_instantiation_and_clear_reuse() {
    let mut tab = InferenceTable::new();
    let res_def = adt_id(50);
    let p_t = tab.arena.mk_param(0, adt_id(51));
    let p_e = tab.arena.mk_param(1, adt_id(52));
    let generic_res = tab.arena.mk_adt(res_def, &[p_t, p_e]);

    let mut subst = Vec::new();
    tab.fresh_subst(2, &mut subst);
    let call_res = tab.instantiate(generic_res, &subst);
    assert!(tab.arena.flags(call_res).has_infer());
    assert!(!tab.arena.flags(call_res).has_param());

    let expected = tab.arena.mk_adt(res_def, &[InferTyId::USIZE, InferTyId::STR_REF]);
    assert!(tab.equate(call_res, expected).is_ok());
    assert_eq!(tab.deep_resolve(call_res), expected);
    assert_eq!(tab.shallow_resolve(subst[0]), InferTyId::USIZE);
    assert_eq!(tab.shallow_resolve(subst[1]), InferTyId::STR_REF);

    // `clear()` resets back to exact builtin state while preserving capacity.
    tab.clear();
    assert_eq!(tab.arena.len(), InferTyId::BUILTIN_COUNT);
    assert_eq!(tab.vars.var_count(), 0);
}
