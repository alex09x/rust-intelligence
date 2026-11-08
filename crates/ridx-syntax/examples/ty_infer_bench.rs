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

use ridx_syntax::name_res::{CrateDefMap, CrateId, Edition, FileId, GlobalDefId, ImplTargetKey, IndexedImpl, ItemKind, MemItemDb, MemSourceDb, ModuleTree, OwnedDef, VisKind};
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

fn process_methods(items: &Vec<u64>, pkt: &Packet<Option<u64>>) -> Option<u64> {
    let n = items.len() as u64;
    let first = *items.first()?;
    let mapped = items.clone().into_iter().map(|v| v + first + n);
    let _ = mapped;
    pkt.payload
}

fn process_traits<I: Iterator<Item = u64>>(it: I, items: Vec<u64>) -> Vec<u64> {
    let head: Option<<I as Iterator>::Item> = Some(42);
    let extra = Vec::from([1u64, 2u64]);
    let _ = (head, extra, it);
    items.into_iter().collect::<Vec<_>>()
}

fn process_partial(items: &Vec<u64>, pkt: &Packet<Option<u64>>) -> Option<u64> {
    let unk = missing_symbol + ;
    let tup = (items, pkt);
    let _ = tup.0.;
    let _ = items.;
    let _ = unk;
    pkt.payload
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
    let vec_def = GlobalDefId::new(std_c, 15);
    let clone_tr = GlobalDefId::new(std_c, 16);
    let into_iter_tr = GlobalDefId::new(std_c, 17);
    let iter_tr = GlobalDefId::new(std_c, 18);
    let iter_item_assoc = GlobalDefId::new(std_c, 19);
    let into_iter_adt = GlobalDefId::new(std_c, 20);
    let map_adt = GlobalDefId::new(std_c, 21);
    let fn_mut_def = GlobalDefId::new(std_c, 22);
    let from_tr = GlobalDefId::new(std_c, 23);
    let from_iter_tr = GlobalDefId::new(std_c, 24);
    for (id, n, k, c) in [
        (opt_def, "Option", ItemKind::Enum, false),
        (some_def, "Some", ItemKind::Variant, true),
        (none_def, "None", ItemKind::Variant, true),
        (fn_once_def, "FnOnce", ItemKind::Trait, false),
        (fn_mut_def, "FnMut", ItemKind::Trait, false),
        (vec_def, "Vec", ItemKind::Struct, false),
        (clone_tr, "Clone", ItemKind::Trait, false),
        (into_iter_tr, "IntoIterator", ItemKind::Trait, false),
        (iter_tr, "Iterator", ItemKind::Trait, false),
        (from_tr, "From", ItemKind::Trait, false),
        (from_iter_tr, "FromIterator", ItemKind::Trait, false),
    ] {
        item_db.add_module_def(prelude, OwnedDef { id, name: n.into(), kind: k, vis: VisKind::Public, has_value_ctor: c });
    }
    item_db.add_assoc_item(iter_tr, OwnedDef { id: iter_item_assoc, name: "Item".into(), kind: ItemKind::AssocTy, vis: VisKind::Public, has_value_ctor: false });
    let mut ty_db = MemTyDefDb::new();
    ty_db.add_lang_item(TyLangItem::Option, opt_def);
    ty_db.add_lang_item(TyLangItem::OptionSome, some_def);
    ty_db.add_lang_item(TyLangItem::OptionNone, none_def);
    ty_db.add_lang_item(TyLangItem::FnOnce, fn_once_def);
    ty_db.add_lang_item(TyLangItem::FnMut, fn_mut_def);
    ty_db.add_lang_item(TyLangItem::Range, range_def);
    ty_db.add_lang_item(TyLangItem::Iterator, iter_tr);
    ty_db.add_lang_item(TyLangItem::IteratorItem, iter_item_assoc);
    let (p0, p1, p2) = (ty_db.param(0), ty_db.param(1), ty_db.param(2));
    ty_db.add_adt(opt_def, 1, CtorShapeKind::None);
    ty_db.add_variant(some_def, opt_def, 1, CtorShapeKind::Tuple);
    ty_db.set_tuple_fields(some_def, &[p0]);
    ty_db.add_variant(none_def, opt_def, 1, CtorShapeKind::Unit);
    ty_db.add_adt(range_def, 1, CtorShapeKind::Record);
    let vec_p0 = ty_db.add_adt(vec_def, 1, CtorShapeKind::None);
    let into_iter_p0 = ty_db.add_adt(into_iter_adt, 1, CtorShapeKind::None);
    let _ = ty_db.add_adt(map_adt, 2, CtorShapeKind::None);
    let sl_p0 = ty_db.templates.mk_slice(p0);
    ty_db.add_assoc_rule(TyLangItem::DerefTarget, 1, vec_p0, &[], sl_p0);
    let (ref_vec, ref_sl, ref_p0) = (ty_db.templates.mk_ref(false, vec_p0), ty_db.templates.mk_ref(false, sl_p0), ty_db.templates.mk_ref(false, p0));
    let opt_ref_p0 = ty_db.templates.mk_adt(opt_def, &[ref_p0]);
    let (vec_len_fn, sl_first_fn, vec_clone_fn, vec_ii_fn, iter_map_fn) =
        (GlobalDefId::new(std_c, 50), GlobalDefId::new(std_c, 51), GlobalDefId::new(std_c, 52), GlobalDefId::new(std_c, 53), GlobalDefId::new(std_c, 54));
    ty_db.add_fn(vec_len_fn, 1, &[ref_vec], InferTyId::USIZE);
    ty_db.add_inherent_method(ImplTargetKey::Adt(vec_def), "len", vec_len_fn, 1, 0);
    ty_db.add_fn(sl_first_fn, 1, &[ref_sl], opt_ref_p0);
    ty_db.add_inherent_method(ImplTargetKey::Slice, "first", sl_first_fn, 1, 0);
    ty_db.add_fn(vec_clone_fn, 1, &[ref_vec], vec_p0);
    ty_db.add_trait_method(clone_tr, 0, "clone", vec_clone_fn, 0);
    ty_db.add_trait_impl(clone_tr, 1, vec_p0, &[], &[("clone", vec_clone_fn, 0)]);
    ty_db.add_fn(vec_ii_fn, 1, &[vec_p0], into_iter_p0);
    ty_db.add_trait_method(into_iter_tr, 0, "into_iter", vec_ii_fn, 0);
    ty_db.add_trait_impl(into_iter_tr, 1, vec_p0, &[], &[("into_iter", vec_ii_fn, 0)]);
    let self_item = ty_db.templates.mk_alias(iter_tr, iter_item_assoc, p0, &[]);
    let map_ret = ty_db.templates.mk_adt(map_adt, &[p0, p2]);
    ty_db.add_fn(iter_map_fn, 3, &[p0, p2], map_ret);
    ty_db.add_param_fn_bound(iter_map_fn, 2, TyLangItem::FnMut, &[self_item], p1);
    ty_db.add_trait_method(iter_tr, 0, "map", iter_map_fn, 2);
    ty_db.add_trait_assoc_rule(iter_tr, iter_item_assoc, 1, into_iter_p0, &[], p0);
    ty_db.add_assoc_rule(TyLangItem::IteratorItem, 1, into_iter_p0, &[], p0);
    ty_db.add_trait_impl(iter_tr, 1, into_iter_p0, &[], &[]);
    let (from_fn, vec_from_arr_fn, iter_collect_fn, vec_from_imp) =
        (GlobalDefId::new(std_c, 55), GlobalDefId::new(std_c, 56), GlobalDefId::new(std_c, 57), GlobalDefId::new(std_c, 58));
    ty_db.add_fn(from_fn, 2, &[p1], p0);
    ty_db.add_trait_method(from_tr, 1, "from", from_fn, 0);
    item_db.add_assoc_item(from_tr, OwnedDef { id: from_fn, name: "from".into(), kind: ItemKind::AssocFn, vis: VisKind::Public, has_value_ctor: false });
    let arr2_p0 = ty_db.templates.mk_array(p0, 2);
    ty_db.add_fn(vec_from_arr_fn, 1, &[arr2_p0], vec_p0);
    ty_db.add_trait_impl(from_tr, 1, vec_p0, &[arr2_p0], &[("from", vec_from_arr_fn, 0)]);
    item_db.add_impl(IndexedImpl { id: vec_from_imp, file: FileId::NONE, target: ImplTargetKey::Adt(vec_def), trait_def: from_tr, is_negative: false });
    item_db.add_assoc_item(vec_from_imp, OwnedDef { id: vec_from_arr_fn, name: "from".into(), kind: ItemKind::AssocFn, vis: VisKind::Public, has_value_ctor: false });
    ty_db.add_fn(iter_collect_fn, 2, &[p0], p1);
    ty_db.add_where_trait_bound(iter_collect_fn, p1, from_iter_tr, &[self_item]);
    ty_db.add_trait_method(iter_tr, 0, "collect", iter_collect_fn, 1);
    ty_db.add_trait_impl(from_iter_tr, 1, vec_p0, &[p0], &[]);

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

    let m_node = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == "process_methods")).expect("process_methods fn");
    for _ in 0..16 {
        ctx.infer_fn_into(&st, &file_res, &ty_db, &local_defs, m_node, &mut res);
        black_box(res.body_ty);
    }
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    let t2 = Instant::now();
    for _ in 0..iters_body {
        ctx.infer_fn_into(&st, &file_res, &ty_db, &local_defs, m_node, &mut res);
        black_box(res.body_ty);
    }
    let ns_m = t2.elapsed().as_nanos() as f64;
    let allocs_m = ALLOCS.load(Ordering::Relaxed);
    let bytes_m = BYTES.load(Ordering::Relaxed);
    println!(
        "ty_infer_bench (3.3 methods+autoderef): iters={} total_ms={:.2} ns/fn={:.1} warm_allocs={} warm_bytes={}",
        iters_body,
        ns_m / 1_000_000.0,
        ns_m / (iters_body as f64),
        allocs_m,
        bytes_m
    );
    assert_eq!(allocs_m, 0, "expected 0 heap allocations in steady-state 3.3 method resolution loop");

    let tr_node = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == "process_traits")).expect("process_traits fn");
    for _ in 0..16 {
        ctx.infer_fn_into(&st, &file_res, &ty_db, &local_defs, tr_node, &mut res);
        black_box(res.body_ty);
    }
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    let t3 = Instant::now();
    for _ in 0..iters_body {
        ctx.infer_fn_into(&st, &file_res, &ty_db, &local_defs, tr_node, &mut res);
        black_box(res.body_ty);
    }
    let ns_tr = t3.elapsed().as_nanos() as f64;
    let allocs_tr = ALLOCS.load(Ordering::Relaxed);
    let bytes_tr = BYTES.load(Ordering::Relaxed);
    println!(
        "ty_infer_bench (3.4 trait-solve+proj): iters={} total_ms={:.2} ns/fn={:.1} warm_allocs={} warm_bytes={}",
        iters_body,
        ns_tr / 1_000_000.0,
        ns_tr / (iters_body as f64),
        allocs_tr,
        bytes_tr
    );
    assert_eq!(allocs_tr, 0, "expected 0 heap allocations in steady-state 3.4 trait solving loop");

    let p_node = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == "process_partial")).expect("process_partial fn");
    for _ in 0..16 {
        ctx.infer_fn_into(&st, &file_res, &ty_db, &local_defs, p_node, &mut res);
        black_box(res.incomplete_dots.len());
    }
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    let t4 = Instant::now();
    for _ in 0..iters_body {
        ctx.infer_fn_into(&st, &file_res, &ty_db, &local_defs, p_node, &mut res);
        black_box(res.incomplete_dots.len());
    }
    let ns_p = t4.elapsed().as_nanos() as f64;
    let allocs_p = ALLOCS.load(Ordering::Relaxed);
    let bytes_p = BYTES.load(Ordering::Relaxed);
    println!(
        "ty_infer_bench (3.5 partial-dot+recovery): iters={} total_ms={:.2} ns/fn={:.1} warm_allocs={} warm_bytes={}",
        iters_body,
        ns_p / 1_000_000.0,
        ns_p / (iters_body as f64),
        allocs_p,
        bytes_p
    );
    assert_eq!(allocs_p, 0, "expected 0 heap allocations in steady-state 3.5 partial-code recovery loop");
    assert_eq!(res.incomplete_dots.len(), 2, "expected both `tup.0.` and `items.` incomplete dots to be recorded");
}
