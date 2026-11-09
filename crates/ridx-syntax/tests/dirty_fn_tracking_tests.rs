/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for Checkbox 4.1 (`DirtyFileTracker`: incremental dirty-function
//! identification, single-function inference, `O(1)` span/NodeId shifting, environment
//! invalidation, and zero steady-state heap allocations).

use ridx_syntax::{
    name_res::PrimTy,
    ty_infer::{DirtyFileTracker, InferTyId, InferTyKind},
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static TRACK_ALLOCS: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK_ALLOCS.with(Cell::get) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if TRACK_ALLOCS.with(Cell::get) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn single_dirty_fn_inferred_and_cached_across_edits() {
    let baseline = "\
struct Counter { val: u64 }
impl Counter {
    fn new(v: u64) -> Self { Self { val: v } }
    fn get(&self) -> u64 { self.val }
}
fn first(a: i32) -> i32 {
    let x = a + 1;
    x
}
fn second(b: u64) -> u64 {
    let c = Counter::new(b);
    c.get()
}
fn third(flag: bool) -> &'static str {
    if flag { \"yes\" } else { \"no\" }
}
";
    let mut tracker = DirtyFileTracker::new();
    tracker.set_baseline(baseline);
    assert_eq!(tracker.last_stats().total_fns, 5);
    assert_eq!(tracker.last_stats().dirty_fns_vs_compiled, 0);
    assert!(tracker.dirty_fns().is_empty());

    // 1. Edit only `second`: add `let extra = c.get() + 10u64; extra`
    let edit1 = baseline.replace("    c.get()\n", "    let extra = c.get() + 10;\n    extra\n");
    let s1 = tracker.on_did_change(&edit1);
    assert_eq!(s1.total_fns, 5);
    assert_eq!(s1.dirty_fns_vs_compiled, 1);
    assert_eq!(s1.inferred_now, 1);
    assert_eq!(s1.reused_cached, 0);
    assert!(!s1.env_changed);

    let extra_off = edit1.find("extra =").unwrap() as u32;
    tracker.with_live_tree(&edit1, |tr, st1| {
        let dirty_sec = tr.dirty_fn_at_offset(extra_off).expect("second() must be dirty");
        let (_, extra_ty) = dirty_sec.pat_ty_at_offset(st1, extra_off).expect("extra pat ty");
        assert_eq!(dirty_sec.types.kind(extra_ty), InferTyKind::Prim(PrimTy::U64));
        assert_eq!(dirty_sec.types.kind(dirty_sec.infer.body_ty), InferTyKind::Prim(PrimTy::U64));
    });

    // Unedited functions (`first` and `third`) must NOT be in `dirty_fns()`
    let first_off = edit1.find("let x =").unwrap() as u32;
    let third_off = edit1.find("if flag").unwrap() as u32;
    assert!(tracker.dirty_fn_at_offset(first_off).is_none());
    assert!(tracker.dirty_fn_at_offset(third_off).is_none());

    // 2. Edit `first` above `second`: `second` must be shifted & reused in O(1) without re-inference!
    let edit2 = edit1.replace("    let x = a + 1;\n", "    let x = a + 100;\n    let y = x * 2;\n");
    let s2 = tracker.on_did_change(&edit2);
    assert_eq!(s2.total_fns, 5);
    assert_eq!(s2.dirty_fns_vs_compiled, 2);
    assert_eq!(s2.inferred_now, 1, "only first() should be inferred; second() is reused");
    assert_eq!(s2.reused_cached, 1, "second() must be reused from cache");
    assert!(!s2.env_changed);

    // Verify shifted offsets and NodeIds in `second()` still resolve accurately in `st2`!
    let extra_off2 = edit2.find("extra =").unwrap() as u32;
    assert!(extra_off2 > extra_off, "byte offset of second() shifted down");
    tracker.with_live_tree(&edit2, |tr, st2| {
        let dirty_sec2 = tr.dirty_fn_at_offset(extra_off2).expect("shifted second() must be found");
        let (pat_node2, extra_ty2) = dirty_sec2.pat_ty_at_offset(st2, extra_off2).expect("shifted extra pat ty");
        assert_eq!(dirty_sec2.types.kind(extra_ty2), InferTyKind::Prim(PrimTy::U64));
        assert_eq!(st2.range(pat_node2).start as u32, extra_off2);
    });

    // 3. Insert comment lines at the top of the file: 0 functions inferred, 2 reused!
    let edit3 = format!("// header line 1\n// header line 2\n{edit2}");
    let s3 = tracker.on_did_change(&edit3);
    assert_eq!(s3.dirty_fns_vs_compiled, 2);
    assert_eq!(s3.inferred_now, 0);
    assert_eq!(s3.reused_cached, 2);
    assert!(!s3.env_changed);

    // 4. Revert back to `baseline`: 0 dirty functions, 0 inferred!
    let s4 = tracker.on_did_change(baseline);
    assert_eq!(s4.dirty_fns_vs_compiled, 0);
    assert_eq!(s4.inferred_now, 0);
    assert_eq!(s4.reused_cached, 0);
    assert!(tracker.dirty_fns().is_empty());
}

#[test]
fn struct_or_signature_env_change_reinfers_dirty_functions() {
    let baseline = "\
struct Item { count: u32 }
fn read_item(it: Item) -> u32 {
    it.count
}
fn untouched() -> i32 { 42 }
";
    let mut tracker = DirtyFileTracker::new();
    tracker.set_baseline(baseline);

    // 1. Edit `read_item` body
    let edit1 = baseline.replace("    it.count\n", "    let v = it.count;\n    v\n");
    let s1 = tracker.on_did_change(&edit1);
    assert_eq!(s1.dirty_fns_vs_compiled, 1);
    assert_eq!(s1.inferred_now, 1);
    assert!(!s1.env_changed);

    // 2. Now change `struct Item { count: u32 }` -> `struct Item { count: u64 }` and return type `u64`
    let edit2 = edit1.replace("count: u32", "count: u64").replace("-> u32", "-> u64");
    let s2 = tracker.on_did_change(&edit2);
    assert!(s2.env_changed, "changing struct field and fn signature must flag env_changed");
    assert_eq!(s2.dirty_fns_vs_compiled, 1, "untouched() remains clean");
    assert_eq!(s2.inferred_now, 1, "dirty read_item() must be re-inferred against new struct field type");
    assert_eq!(s2.reused_cached, 0);

    let v_off = edit2.find("v = it.count").unwrap() as u32;
    tracker.with_live_tree(&edit2, |tr, st2| {
        let df = tr.dirty_fn_at_offset(v_off).unwrap();
        let (_, v_ty) = df.pat_ty_at_offset(st2, v_off).unwrap();
        assert_eq!(df.types.kind(v_ty), InferTyKind::Prim(PrimTy::U64));
    });
}

#[test]
fn incomplete_dot_in_dirty_fn_and_zero_steady_state_allocations() {
    let baseline = "\
struct Widget { width: usize }
impl Widget {
    fn area(&self) -> usize { self.width * self.width }
}
fn compute(w: Widget) -> usize {
    w.area()
}
fn helper(n: usize) -> usize {
    n + 1
}
";
    let mut tracker = DirtyFileTracker::new();
    tracker.set_baseline(baseline);

    let edit_a = "\
struct Widget { width: usize }
impl Widget {
    fn area(&self) -> usize { self.width * self.width }
}
fn compute(w: Widget) -> usize {
    let a = w.area();
    w.
}
fn helper(n: usize) -> usize {
    n + 1
}
";
    let edit_b = "\
struct Widget { width: usize }
impl Widget {
    fn area(&self) -> usize { self.width * self.width }
}
fn compute(w: Widget) -> usize {
    let a = w.area() + 1;
    a
}
fn helper(n: usize) -> usize {
    n + 1
}
";

    // Warm up internal scratch buffers on both edits
    let _ = tracker.on_did_change(edit_a);
    let dot_off = (edit_a.find("w.\n").unwrap() + 2) as u32;
    let df = tracker.dirty_fn_at_offset(dot_off).expect("compute() is dirty");
    let (_, recv_ty) = df.incomplete_dot_at_offset(dot_off).expect("incomplete dot recorded");
    assert_ne!(recv_ty, InferTyId::UNKNOWN);

    let _ = tracker.on_did_change(edit_b);
    let _ = tracker.on_did_change(baseline);

    // Measure allocations across warm alternating `on_did_change` cycles
    TRACK_ALLOCS.with(|c| c.set(true));
    let before = ALLOC_COUNT.load(Ordering::Relaxed);
    for _ in 0..20 {
        let sa = tracker.on_did_change(edit_a);
        assert_eq!(sa.inferred_now, 1);
        let sb = tracker.on_did_change(edit_b);
        assert_eq!(sb.inferred_now, 1);
    }
    let after = ALLOC_COUNT.load(Ordering::Relaxed);
    TRACK_ALLOCS.with(|c| c.set(false));
    assert_eq!(after - before, 0, "expected 0 heap allocations across 40 warm on_did_change calls");
    assert!(tracker.heap_bytes() < 512 * 1024, "tracker heap footprint must stay compact (< 512 KiB)");
}
