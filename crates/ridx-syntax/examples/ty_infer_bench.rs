/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Phase 3 Checkbox `3.1` throughput & heap-allocation benchmark (`ty_infer_bench`).
//! Simulates 100,000 realistic function-body inference passes (`InferenceTable::clear` reuse,
//! generic ADT instantiation, `{integer}`/`{float}` unification, speculative snapshot/rollback,
//! downward `Expectation` propagation, `BranchCoercer` arm joining, and `resolve_with_fallback`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use ridx_syntax::name_res::{CrateId, GlobalDefId};
use ridx_syntax::ty_infer::{BranchCoercer, Expectation, InferTyId, InferenceTable};

struct CountingAlloc;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn simulate_fn_inference(tab: &mut InferenceTable, subst: &mut Vec<InferTyId>) -> InferTyId {
    tab.clear();
    let vec_def = GlobalDefId::new(CrateId(1), 10);
    let res_def = GlobalDefId::new(CrateId(1), 11);
    let p0 = tab.arena.mk_param(0, GlobalDefId::new(CrateId(1), 100));
    let p1 = tab.arena.mk_param(1, GlobalDefId::new(CrateId(1), 101));
    let sig_vec = tab.arena.mk_adt(vec_def, &[p0]);
    let sig_res = tab.arena.mk_adt(res_def, &[sig_vec, p1]);

    // 1. Generic call instantiation (`Result<Vec<?0>, ?1>`).
    tab.fresh_subst(2, subst);
    let call_ret = tab.instantiate(sig_res, subst);

    // 2. Expected slice borrow propagation (`let bytes: &[u16] = &[lit0, lit1, lit2];`).
    let sl_u16 = tab.arena.mk_slice(InferTyId::U16);
    let ref_sl_u16 = tab.arena.mk_ref(false, sl_u16);
    let elem_exp = Expectation::HasType(ref_sl_u16).peel_for_borrow(tab, false).array_elem_expectation(tab);
    let exp_elem_ty = elem_exp.coercion_target(tab).unwrap_or(InferTyId::UNKNOWN);
    for _ in 0..4 {
        let lit = tab.new_int_var();
        let _ = tab.coerce(lit, exp_elem_ty);
    }

    // 3. Speculative overload probe (first candidate fails and rolls back, second succeeds).
    let snap = tab.start_snapshot();
    let wrong_target = tab.arena.mk_adt(res_def, &[InferTyId::BOOL, InferTyId::STR_REF]);
    if tab.equate(call_ret, wrong_target).is_err() {
        tab.rollback_to(snap);
    } else {
        tab.commit(snap);
    }

    let vec_u16 = tab.arena.mk_adt(vec_def, &[exp_elem_ty]);
    let ok_target = tab.arena.mk_adt(res_def, &[vec_u16, InferTyId::STR_REF]);
    let _ = tab.equate(call_ret, ok_target);

    // 4. Multi-arm `match` join (`!`, `&mut Vec<u16>`, `&Vec<u16>`) + unconstrained literal fallback.
    let mut branches = BranchCoercer::new(tab, Expectation::None);
    let rm = tab.arena.mk_ref(true, call_ret);
    let rs = tab.arena.mk_ref(false, call_ret);
    branches.merge_arm(tab, InferTyId::NEVER);
    branches.merge_arm(tab, rm);
    branches.merge_arm(tab, rs);
    let joined = branches.finish(tab);

    let free_int = tab.new_int_var();
    let free_flt = tab.new_float_var();
    let out_tup = tab.arena.mk_tuple(&[joined, free_int, free_flt]);
    tab.resolve_with_fallback(out_tup)
}

fn main() {
    let mut tab = InferenceTable::with_capacity(128, 64);
    let mut subst = Vec::with_capacity(8);

    // Warmup once so internal vectors reach steady-state capacity.
    for _ in 0..16 {
        black_box(simulate_fn_inference(&mut tab, &mut subst));
    }

    let iters: u64 = 100_000;
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    let t0 = Instant::now();
    for _ in 0..iters {
        black_box(simulate_fn_inference(&mut tab, &mut subst));
    }
    let elapsed_ns = t0.elapsed().as_nanos() as f64;
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);

    println!("ty_infer_bench (3.1): iters={} total_ms={:.2} ns/fn={:.1} warm_allocs={} warm_bytes={}", iters, elapsed_ns / 1_000_000.0, elapsed_ns / (iters as f64), allocs, bytes);
    assert_eq!(allocs, 0, "expected 0 heap allocations in steady-state InferenceTable::clear loop");
}
