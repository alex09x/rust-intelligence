/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Phase `3.6` integration tests:
//! - `3.6.1`: `include!(concat!(env!("OUT_DIR"), ...))`, relative `include!("...")`, external `#[path = "..."]`,
//!   and macro-generated `mod` items in `ModuleTree` / `CrateDefMap` / `BodyInferCtx`.
//! - `3.6.2`: local `macro_rules!` body expansion (including nested macros & `pin!`-style local shadowing),
//!   dependency `$crate::...` `macro_rules!` with hygiene, `format!` / `vec!` / `assert!` / `write!` /
//!   `matches!` / `dbg!` / `panic!` / `env!` / `include_bytes!` / `std::pin::pin!` std macro families,
//!   default generic type arguments (`Vec<T, Global>`), nested closure `def_span`, and `0` warm allocations.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use ridx_syntax::name_res::{CrateId, CrateSnapshot, Edition, GlobalDefId, ImplTargetKey, ItemKind, MemItemDb, MemSourceDb, OwnedDef, OwnedUse, ResTarget, SourceDb, VisKind};
use ridx_syntax::ty_infer::{effective_local_def_id, BodyInferCtx, CtorShapeKind, FnInferResult, InferTyId, InferTyKind, LocalCstDefs, MemTyDefDb, TyLangItem};
use ridx_syntax::{parse, NodeId, SyntaxKind as K, SyntaxTree};

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<(bool, u64)> = const { Cell::new((false, 0)) };
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| {
            let (active, n) = c.get();
            if active {
                c.set((true, n + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn find_fn<'a>(st: &'a SyntaxTree<'a>, name: &str) -> NodeId {
    st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == name)).unwrap_or_else(|| panic!("missing fn `{name}`"))
}

fn local_ty_by_name(st: &SyntaxTree<'_>, file_res: &ridx_syntax::name_res::FileNameRes, res: &FnInferResult, name: &str) -> InferTyId {
    let lb = file_res
        .locals
        .iter()
        .find(|l| st.text(l.decl_node).trim() == name || st.child(l.decl_node, K::Name).is_some_and(|nn| st.text(nn) == name))
        .unwrap_or_else(|| panic!("missing local `{name}`"));
    res.local_ty(lb.id).unwrap_or_else(|| panic!("untyped local `{name}`"))
}

#[test]
fn include_out_dir_and_external_path_modules_resolve_and_typecheck() {
    let mut src = MemSourceDb::new();
    let root = src.add_file(
        "ws/my_crate/src/lib.rs",
        r#"
#[path = "../../shared/ext_mod.rs"]
pub mod ext_mod;

include!(concat!(env!("OUT_DIR"), "/generated.rs"));
include!("extra/inline_piece.rs");

pub fn check_all() -> (u32, &'static str, i64, u16) {
    let a = ext_mod::ext_answer();
    let b = gen_greeting();
    let c = inline_helper(10);
    let d = include!("extra/body_expr.rs");
    let raw = include_bytes!("assets/blob.bin");
    let _ = raw;
    (a, b, c, d)
}
"#,
    );
    let _ = src.add_file("ws/shared/ext_mod.rs", "pub fn ext_answer() -> u32 { 42 }\n");
    let _ = src.add_file("target/debug/build/my_crate-out/generated.rs", "pub fn gen_greeting() -> &'static str { \"ok\" }\n");
    let _ = src.add_file("ws/my_crate/src/extra/inline_piece.rs", "pub fn inline_helper(x: i64) -> i64 { x + 1 }\n");
    let body_inc_file = src.add_file("ws/my_crate/src/extra/body_expr.rs", "{ 77u16 + 1u16 }");

    let mut item_db = MemItemDb::new();
    let krate = item_db.add_crate("my_crate", Edition::Edition2021);
    item_db.set_env(krate, "OUT_DIR", "target/debug/build/my_crate-out");
    item_db.set_include_file(root, "extra/body_expr.rs", body_inc_file, "{ 77u16 + 1u16 }");
    item_db.set_include_bytes_len(root, "assets/blob.bin", 11);

    let snap = CrateSnapshot::build_clean(&item_db, &src, krate, root);
    let st = parse(src.file_text(root));
    let mut res_buf = Default::default();
    snap.resolve_file_paths_into(&item_db, root, &st, &mut res_buf);

    let resolve_fn_def = |ident: &str| -> GlobalDefId {
        res_buf
            .refs
            .iter()
            .find_map(|r| match r.target {
                ResTarget::Def { def, local, kind: ItemKind::Fn } if local.is_some() && &st.src[r.start as usize..r.end as usize] == ident => {
                    Some(effective_local_def_id(def, local))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected resolved cross-file Fn `{ident}`"))
    };
    let ext_def = resolve_fn_def("ext_answer");
    let gen_def = resolve_fn_def("gen_greeting");
    let inl_def = resolve_fn_def("inline_helper");

    let mut local_defs = LocalCstDefs::new();
    local_defs.index_file_with_item_db(&st, Some(&snap.clean_def_map), Some(&item_db), root);

    let mut ty_db = MemTyDefDb::new();
    ty_db.add_fn(ext_def, 0, &[], InferTyId::U32);
    ty_db.add_fn(gen_def, 0, &[], InferTyId::STR_REF);
    ty_db.add_fn(inl_def, 0, &[InferTyId::I64], InferTyId::I64);

    let mut ctx = BodyInferCtx::new();
    let fn_node = find_fn(&st, "check_all");
    let fn_res = ctx.infer_fn(&st, &res_buf, &ty_db, &local_defs, fn_node);

    assert_eq!(local_ty_by_name(&st, &res_buf, &fn_res, "a"), InferTyId::U32);
    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "b")), "&str");
    assert_eq!(local_ty_by_name(&st, &res_buf, &fn_res, "c"), InferTyId::I64);
    assert_eq!(local_ty_by_name(&st, &res_buf, &fn_res, "d"), InferTyId::U16);
    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "raw")), "&[u8; 11]");
}

#[test]
fn local_and_dep_macro_rules_hygiene_pin_and_std_macros_zero_alloc() {
    let mut src = MemSourceDb::new();
    let root = src.add_file(
        "src/lib.rs",
        r#"
macro_rules! inner_add {
    ($v:expr) => { $v + 1i32 };
}

macro_rules! outer_pair {
    ($x:expr) => {{
        let tmp: u8 = 7;
        (inner_add!($x), tmp)
    }};
}

macro_rules! rebind_mut_ref {
    ($id:ident) => {
        let mut $id = $id;
        #[allow(unused_mut)]
        let mut $id = &mut $id;
    };
}

pub fn exercise_macros(opt: Option<u64>, flag: bool) -> bool {
    let tmp: i32 = 41;
    let pair = outer_pair!(tmp);
    let from_dep = dep_crate::make_box_val!(99u16);
    let v = vec![10u32, 20u32];
    let v_rep = vec![false; 4];
    let msg = format!("v={:?}", pair);
    let echoed = dbg!(pair.0);
    let is_big = matches!(opt, Some(n) if n > 5);
    let env_val = env!("CARGO_PKG_NAME");
    let pinned_std = core::pin::pin!(123u64);
    let val = 55u32;
    rebind_mut_ref!(val);
    let re_borrow: &mut u32 = val;
    assert!(flag && is_big);
    assert_eq!(echoed, 42i32);
    if !flag {
        panic!("boom {}", msg);
    }
    let _ = (from_dep, v, v_rep, msg, env_val, pinned_std, re_borrow);
    is_big
}
"#,
    );

    let mut item_db = MemItemDb::new();
    let dep = item_db.add_crate("dep_crate", Edition::Edition2021);
    let dep_root = GlobalDefId::new(dep, 0);
    let dep_wrap_fn = GlobalDefId::new(dep, 1);
    let dep_macro = GlobalDefId::new(dep, 2);
    item_db.set_crate_root_def(dep, dep_root);
    item_db.add_module_def(dep_root, OwnedDef { id: dep_wrap_fn, name: "wrap_u16".into(), kind: ItemKind::Fn, vis: VisKind::Public, has_value_ctor: false });
    item_db.add_module_def(dep_root, OwnedDef { id: dep_macro, name: "make_box_val".into(), kind: ItemKind::Macro, vis: VisKind::Public, has_value_ctor: false });
    item_db.add_macro_def_body(dep_macro, "($e:expr) => { $crate::wrap_u16($e) }", "dep_crate");

    let core_k = item_db.add_crate("core", Edition::Edition2021);
    let core_root = GlobalDefId::new(core_k, 0);
    let opt_enum = GlobalDefId::new(core_k, 10);
    let opt_some = GlobalDefId::new(core_k, 11);
    let pin_struct = GlobalDefId::new(core_k, 12);
    let vec_struct = GlobalDefId::new(core_k, 13);
    let global_struct = GlobalDefId::new(core_k, 14);
    let string_struct = GlobalDefId::new(core_k, 15);
    item_db.set_crate_root_def(core_k, core_root);
    item_db.add_module_def(core_root, OwnedDef { id: opt_enum, name: "Option".into(), kind: ItemKind::Enum, vis: VisKind::Public, has_value_ctor: false });
    item_db.add_module_def(core_root, OwnedDef { id: opt_some, name: "Some".into(), kind: ItemKind::Variant, vis: VisKind::Public, has_value_ctor: true });

    let krate = item_db.add_crate("app", Edition::Edition2021);
    item_db.add_extern_crate(krate, "dep_crate", dep);
    item_db.add_extern_crate(krate, "core", core_k);
    item_db.set_prelude_module(krate, core_root);
    item_db.add_module_use(
        core_root,
        OwnedUse {
            name: "Option".into(),
            type_def: opt_enum,
            value_def: GlobalDefId::NONE,
            macro_def: GlobalDefId::NONE,
            vis: VisKind::Public,
            is_glob: false,
            is_reexport: true,
            is_prelude: true,
            is_underscore: false,
            prim_ty_code: 0,
        },
    );
    item_db.add_module_use(
        core_root,
        OwnedUse {
            name: "Some".into(),
            type_def: opt_some,
            value_def: opt_some,
            macro_def: GlobalDefId::NONE,
            vis: VisKind::Public,
            is_glob: false,
            is_reexport: true,
            is_prelude: true,
            is_underscore: false,
            prim_ty_code: 0,
        },
    );

    let mut ty_db = MemTyDefDb::new();
    let _ = ty_db.add_adt(opt_enum, 1, CtorShapeKind::None);
    let p0 = ty_db.param(0);
    let _ = ty_db.add_variant(opt_some, opt_enum, 1, CtorShapeKind::Tuple);
    ty_db.set_tuple_fields(opt_some, &[p0]);
    let _ = ty_db.add_adt(pin_struct, 1, CtorShapeKind::None);
    ty_db.add_lang_item(TyLangItem::Pin, pin_struct);
    let global_ty = ty_db.add_adt(global_struct, 0, CtorShapeKind::Unit);
    let _ = ty_db.add_adt(vec_struct, 2, CtorShapeKind::None);
    ty_db.add_generic_default(vec_struct, 1, global_ty);
    ty_db.vec_def = Some(vec_struct);
    ty_db.global_alloc_def = Some(global_struct);
    let _ = ty_db.add_adt(string_struct, 0, CtorShapeKind::None);
    ty_db.add_lang_item(TyLangItem::String, string_struct);
    ty_db.add_fn(dep_wrap_fn, 0, &[InferTyId::U16], InferTyId::U64);

    let snap = CrateSnapshot::build_clean(&item_db, &src, krate, root);
    let st = parse(src.file_text(root));
    let mut res_buf = Default::default();
    snap.resolve_file_paths_into(&item_db, root, &st, &mut res_buf);

    let mut local_defs = LocalCstDefs::new();
    local_defs.index_file_with_item_db(&st, Some(&snap.clean_def_map), Some(&item_db), root);

    let mut ctx = BodyInferCtx::new();
    let mut fn_res = FnInferResult::new();
    let fn_node = find_fn(&st, "exercise_macros");

    // Warmup pass so scratch vectors reach steady-state capacity.
    ctx.infer_fn_into(&st, &res_buf, &ty_db, &local_defs, fn_node, &mut fn_res);
    THREAD_ALLOCS.with(|c| c.set((true, 0)));
    ctx.infer_fn_into(&st, &res_buf, &ty_db, &local_defs, fn_node, &mut fn_res);
    let warm_allocs = THREAD_ALLOCS.with(|c| {
        let (_, n) = c.get();
        c.set((false, 0));
        n
    });
    assert_eq!(warm_allocs, 0, "expected 0 warm heap allocations during macro body inference, got {warm_allocs}");

    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "pair")), "(i32, u8)");
    assert_eq!(local_ty_by_name(&st, &res_buf, &fn_res, "from_dep"), InferTyId::U64);
    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "v")), "Adt(1:13)<u32, Adt(1:14)>");
    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "v_rep")), "Adt(1:13)<bool, Adt(1:14)>");
    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "msg")), "Adt(1:15)");
    assert_eq!(local_ty_by_name(&st, &res_buf, &fn_res, "echoed"), InferTyId::I32);
    assert_eq!(local_ty_by_name(&st, &res_buf, &fn_res, "is_big"), InferTyId::BOOL);
    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "env_val")), "&str");
    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "pinned_std")), "Adt(1:12)<&mut u64>");
    assert_eq!(ctx.table.arena.format_ty(local_ty_by_name(&st, &res_buf, &fn_res, "re_borrow")), "&mut u32");
    assert!(!fn_res.macro_expr_tys.is_empty(), "expected macro argument expressions mapped back to caller spans");
}

#[test]
fn default_generic_type_args_closure_spans_and_completion_methods() {
    let code = r#"
struct Alloc;
struct Bag<T, A = Alloc>(T, A);
impl<T, A> Bag<T, A> {
    fn len(&self) -> usize { 1 }
}
impl<A> Bag<u8, A> {
    fn as_bytes(&self) -> &[u8] { &[] }
}
fn probe(b_u8: Bag<u8>, b_i32: Bag<i32>) {
    let cls = |x: i32| {
        let inner = |y: i32| x + y;
        inner(1)
    };
    let _ = (b_u8, b_i32, cls);
}
"#;
    let mut src = MemSourceDb::new();
    let root = src.add_file("src/lib.rs", code);
    let mut item_db = MemItemDb::new();
    let krate = item_db.add_crate("app", Edition::Edition2021);
    let snap = CrateSnapshot::build_clean(&item_db, &src, krate, root);
    let st = parse(code);
    let mut res_buf = Default::default();
    snap.resolve_file_paths_into(&item_db, root, &st, &mut res_buf);
    let mut local_defs = LocalCstDefs::new();
    local_defs.index_file_with_item_db(&st, Some(&snap.clean_def_map), Some(&item_db), root);
    let ty_db = MemTyDefDb::new();
    let mut ctx = BodyInferCtx::new();
    let fn_node = find_fn(&st, "probe");
    let fn_res = ctx.infer_fn(&st, &res_buf, &ty_db, &local_defs, fn_node);

    let b_u8_ty = local_ty_by_name(&st, &res_buf, &fn_res, "b_u8");
    let b_i32_ty = local_ty_by_name(&st, &res_buf, &fn_res, "b_i32");
    let InferTyKind::Adt { def: bag_def, args: u8_args } = ctx.table.arena.kind(b_u8_ty) else {
        panic!("expected Bag ADT");
    };
    assert_eq!(u8_args.len, 2, "default allocator param A = Alloc must be filled");
    let alloc_ty = ctx.table.arena.slice(u8_args)[1];
    assert!(matches!(ctx.table.arena.kind(alloc_ty), InferTyKind::Adt { .. }));

    let cls_ty = local_ty_by_name(&st, &res_buf, &fn_res, "cls");
    let InferTyKind::Closure { def_start, def_end, .. } = ctx.table.arena.kind(cls_ty) else {
        panic!("expected Closure kind");
    };
    assert!(def_end > def_start && code[def_start as usize..def_end as usize].starts_with("|x: i32|"));

    let skolem = ctx.table.arena.mk_adt(GlobalDefId::new(CrateId(0xFFFE), 0), &[]);
    let bag_skolem = ctx.table.arena.mk_adt(bag_def, &[skolem, alloc_ty]);
    let cand_names = ["len", "as_bytes"];
    let (mut m_sk, mut m_u8, mut m_i32) = (Vec::new(), Vec::new(), Vec::new());
    ctx.collect_all_completion_methods(&st, &res_buf, &ty_db, &local_defs, fn_node, fn_node, bag_skolem, &cand_names, &mut m_sk);
    ctx.collect_all_completion_methods(&st, &res_buf, &ty_db, &local_defs, fn_node, fn_node, b_u8_ty, &cand_names, &mut m_u8);
    ctx.collect_all_completion_methods(&st, &res_buf, &ty_db, &local_defs, fn_node, fn_node, b_i32_ty, &cand_names, &mut m_i32);
    assert_ne!(m_sk, m_u8, "Bag<{{unknown}}> must NOT match specialized Bag<u8> method set (`as_bytes`)");
    assert_eq!(m_sk, m_i32, "Bag<{{unknown}}> must match unspecialized Bag<i32> method set (`len` only)");
    let _ = ImplTargetKey::Unknown;
}

#[test]
fn mutated_unclosed_nested_macro_delimiters_never_panic() {
    let cases = [
        r#"
macro_rules! inner { ($x:expr) => { $x } }
macro_rules! outer { ($x:expr) => { inner!($x }; }
fn broken() { let v = outer!("привет"); let _ = v; }
"#,
        r#"
macro_rules! wrap { ($x:expr) => { ($x, 1u32) }; }
fn truncated() { let a = wrap!("юникод"#,
    ];
    for code in cases {
        let mut src = MemSourceDb::new();
        let root = src.add_file("src/lib.rs", code);
        let mut item_db = MemItemDb::new();
        let krate = item_db.add_crate("app", Edition::Edition2021);
        let snap = CrateSnapshot::build_clean(&item_db, &src, krate, root);
        let st = parse(code);
        let mut res_buf = Default::default();
        snap.resolve_file_paths_into(&item_db, root, &st, &mut res_buf);
        let mut local_defs = LocalCstDefs::new();
        local_defs.index_file_with_item_db(&st, Some(&snap.clean_def_map), Some(&item_db), root);
        let ty_db = MemTyDefDb::new();
        let mut ctx = BodyInferCtx::new();
        for n in st.descendants(st.root()) {
            if st.kind(n) == K::Fn {
                let res = ctx.infer_fn(&st, &res_buf, &ty_db, &local_defs, n);
                assert!(!ctx.table.arena.flags(res.ret_ty).has_infer());
            }
        }
    }
}
