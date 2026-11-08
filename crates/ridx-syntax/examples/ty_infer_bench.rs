/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Phase 3 (`3.1` kernel + `3.2` full CST `BodyInferCtx::infer_fn_into`) throughput & heap-allocation
//! benchmark (`ty_infer_bench`), verifying sub-microsecond / single-digit-microsecond latency and
//! **0 heap allocations** in warm steady state.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use ridx_syntax::name_res::{CrateDefMap, CrateId, Edition, GlobalDefId, ItemKind, MemItemDb, MemSourceDb, ModuleTree, OwnedDef, VisKind};
use ridx_syntax::parse;
use ridx_syntax::syntax_kind::SyntaxKind as K;
use ridx_syntax::ty_infer::{BodyInferCtx, BranchCoercer, CtorShapeKind, Expectation, FnInferResult, InferTyId, InferenceTable, LocalCstDefs, MemTyDefDb, TyLangItem};

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

    tab.fresh_subst(2, subst);
    let call_ret = tab.instantiate(sig_res, subst);

    let sl_u16 = tab.arena.mk_slice(InferTyId::U16);
    let ref_sl_u16 = tab.arena.mk_ref(false, sl_u16);
    let elem_exp = Expectation::HasType(ref_sl_u16).peel_for_borrow(tab, false).array_elem_expectation(tab);
    let exp_elem_ty = elem_exp.coercion_target(tab).unwrap_or(InferTyId::UNKNOWN);
    for _ in 0..4 {
        let lit = tab.new_int_var();
        let _ = tab.coerce(lit, exp_elem_ty);
    }

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

const BENCH_SRC: &str = r#"
struct Packet<T> {
    id: u32,
    payload: T,
}

fn map_opt<T, U, F: FnOnce(T) -> U>(opt: Option<T>, f: F) -> Option<U> {
    match opt {
        Some(v) => Some(f(v)),
        None => None,
    }
}

fn process(pkt: &Packet<Option<u64>>, slice: &[u64], flag: bool) -> Option<u64> {
    let base: &[u64] = &[1, 2, 3, 4];
    let sub = &slice[0..2];
    let picked = if flag { base[0] } else { sub[0] };
    let mapped = map_opt(pkt.payload, |x| x + picked)?;
    let acc = loop {
        if mapped > 0 {
            break mapped + 10;
        }
        return None;
    };
    Some(acc)
}
"#;

fn main() {
    let mut tab = InferenceTable::with_capacity(128, 64);
    let mut subst = Vec::with_capacity(8);
    for _ in 0..16 {
        black_box(simulate_fn_inference(&mut tab, &mut subst));
    }
    let iters_kernel: u64 = 100_000;
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    let t0 = Instant::now();
    for _ in 0..iters_kernel {
        black_box(simulate_fn_inference(&mut tab, &mut subst));
    }
    let ns_kernel = t0.elapsed().as_nanos() as f64;
    let allocs_kernel = ALLOCS.load(Ordering::Relaxed);
    let bytes_kernel = BYTES.load(Ordering::Relaxed);
    println!(
        "ty_infer_bench (3.1 kernel): iters={} total_ms={:.2} ns/fn={:.1} warm_allocs={} warm_bytes={}",
        iters_kernel,
        ns_kernel / 1_000_000.0,
        ns_kernel / (iters_kernel as f64),
        allocs_kernel,
        bytes_kernel
    );
    assert_eq!(allocs_kernel, 0, "expected 0 heap allocations in 3.1 kernel");

    // Full 3.2 CST body inference benchmark (`process`).
    let mut item_db = MemItemDb::new();
    let std_c = item_db.add_crate("std", Edition::Edition2021);
    let app_c = item_db.add_crate("app", Edition::Edition2021);
    let prelude = GlobalDefId::new(std_c, 1);
    item_db.set_crate_root_def(std_c, GlobalDefId::new(std_c, 0));
    item_db.set_prelude_module(app_c, prelude);
    item_db.add_extern_crate(app_c, "std", std_c);
    item_db.set_crate_root_def(app_c, GlobalDefId::new(app_c, 0));

    let opt_def = GlobalDefId::new(std_c, 10);
    let some_def = GlobalDefId::new(std_c, 11);
    let none_def = GlobalDefId::new(std_c, 12);
    let fn_once_def = GlobalDefId::new(std_c, 13);
    let range_def = GlobalDefId::new(std_c, 14);
    for (id, n, k, c) in [
        (opt_def, "Option", ItemKind::Enum, false),
        (some_def, "Some", ItemKind::Variant, true),
        (none_def, "None", ItemKind::Variant, true),
        (fn_once_def, "FnOnce", ItemKind::Trait, false),
    ] {
        item_db.add_module_def(prelude, OwnedDef { id, name: n.into(), kind: k, vis: VisKind::Public, has_value_ctor: c });
    }
    let mut ty_db = MemTyDefDb::new();
    ty_db.add_lang_item(TyLangItem::Option, opt_def);
    ty_db.add_lang_item(TyLangItem::OptionSome, some_def);
    ty_db.add_lang_item(TyLangItem::OptionNone, none_def);
    ty_db.add_lang_item(TyLangItem::FnOnce, fn_once_def);
    ty_db.add_lang_item(TyLangItem::Range, range_def);
    let p0 = ty_db.param(0);
    ty_db.add_adt(opt_def, 1, CtorShapeKind::None);
    ty_db.add_variant(some_def, opt_def, 1, CtorShapeKind::Tuple);
    ty_db.set_tuple_fields(some_def, &[p0]);
    ty_db.add_variant(none_def, opt_def, 1, CtorShapeKind::Unit);
    ty_db.add_adt(range_def, 1, CtorShapeKind::Record);

    let mut sources = MemSourceDb::new();
    let file = sources.add_file("src/lib.rs", BENCH_SRC);
    let mod_tree = ModuleTree::build(&item_db, &sources, app_c, file);
    let def_map = CrateDefMap::build(&item_db, &sources, &mod_tree);
    let st = parse(BENCH_SRC);
    let file_res = def_map.resolve_file_paths(&item_db, &sources, &mod_tree, file);
    let local_defs = LocalCstDefs::from_file(&st, Some(&def_map), file);
    let fn_node = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == "process")).expect("process fn");

    let mut ctx = BodyInferCtx::new();
    let mut res = FnInferResult::new();
    for _ in 0..16 {
        ctx.infer_fn_into(&st, &file_res, &ty_db, &local_defs, fn_node, &mut res);
        black_box(res.body_ty);
    }

    let iters_body: u64 = 50_000;
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    let t1 = Instant::now();
    for _ in 0..iters_body {
        ctx.infer_fn_into(&st, &file_res, &ty_db, &local_defs, fn_node, &mut res);
        black_box(res.body_ty);
    }
    let ns_body = t1.elapsed().as_nanos() as f64;
    let allocs_body = ALLOCS.load(Ordering::Relaxed);
    let bytes_body = BYTES.load(Ordering::Relaxed);
    println!(
        "ty_infer_bench (3.2 CST body): iters={} total_ms={:.2} ns/fn={:.1} warm_allocs={} warm_bytes={}",
        iters_body,
        ns_body / 1_000_000.0,
        ns_body / (iters_body as f64),
        allocs_body,
        bytes_body
    );
    assert_eq!(allocs_body, 0, "expected 0 heap allocations in steady-state BodyInferCtx::infer_fn_into loop");
}
