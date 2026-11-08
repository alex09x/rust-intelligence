/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for Phase 3 Checkbox `3.2` (`BodyInferCtx` / `FnInferResult`):
//! literals, `let` with/without annotation + `let-else`, paths, generic calls, struct literals,
//! tuples, refs/derefs, blocks, `if`/`match` coercion + match ergonomics, `loop`/`break` values,
//! `?` via `Try`, `.await` via `Future`/`IntoFuture`, closures (`Fn*` bounds), ranges, and `Index`.

use ridx_syntax::name_res::{CrateDefMap, CrateId, Edition, FileId, FileNameRes, GlobalDefId, ItemKind, MemItemDb, MemSourceDb, ModuleTree, OwnedDef, VisKind};
use ridx_syntax::syntax_kind::SyntaxKind as K;
use ridx_syntax::ty_infer::{BodyInferCtx, CtorShapeKind, FnInferResult, InferTyId, InferTyKind, LocalCstDefs, MemTyDefDb, TyLangItem};
use ridx_syntax::{parse, NodeId, SyntaxTree};

struct StdFixture {
    item_db: MemItemDb,
    ty_db: MemTyDefDb,
    krate: CrateId,
    opt_def: GlobalDefId,
    fut_trait: GlobalDefId,
    range_def: GlobalDefId,
    range_inc_def: GlobalDefId,
    range_from_def: GlobalDefId,
    range_to_def: GlobalDefId,
    range_to_inc_def: GlobalDefId,
    range_full_def: GlobalDefId,
}

fn gdef(c: u32, i: u32) -> GlobalDefId {
    GlobalDefId::new(CrateId(c), i)
}

fn odef(id: GlobalDefId, name: &str, kind: ItemKind, ctor: bool) -> OwnedDef {
    OwnedDef { id, name: name.into(), kind, vis: VisKind::Public, has_value_ctor: ctor }
}

impl StdFixture {
    fn new() -> Self {
        let mut item_db = MemItemDb::new();
        let std_c = item_db.add_crate("std", Edition::Edition2021);
        let krate = item_db.add_crate("app", Edition::Edition2021);
        let prelude = gdef(std_c.0, 1);
        item_db.set_crate_root_def(std_c, gdef(std_c.0, 0));
        item_db.set_prelude_module(krate, prelude);
        item_db.add_extern_crate(krate, "std", std_c);
        item_db.set_crate_root_def(krate, gdef(krate.0, 0));

        let (opt_def, some_def, none_def) = (gdef(std_c.0, 10), gdef(std_c.0, 11), gdef(std_c.0, 12));
        let (res_def, ok_def, err_def) = (gdef(std_c.0, 13), gdef(std_c.0, 14), gdef(std_c.0, 15));
        let (cf_def, vec_def, box_def, str_adt_def, pin_def) = (gdef(std_c.0, 16), gdef(std_c.0, 17), gdef(std_c.0, 18), gdef(std_c.0, 19), gdef(std_c.0, 20));
        let (fut_trait, fn_trait, fn_mut_trait, fn_once_trait) = (gdef(std_c.0, 21), gdef(std_c.0, 22), gdef(std_c.0, 23), gdef(std_c.0, 24));
        let (range_def, range_inc_def, range_from_def, range_to_def, range_to_inc_def, range_full_def) =
            (gdef(std_c.0, 30), gdef(std_c.0, 31), gdef(std_c.0, 32), gdef(std_c.0, 33), gdef(std_c.0, 34), gdef(std_c.0, 35));

        for d in [
            odef(opt_def, "Option", ItemKind::Enum, false),
            odef(some_def, "Some", ItemKind::Variant, true),
            odef(none_def, "None", ItemKind::Variant, true),
            odef(res_def, "Result", ItemKind::Enum, false),
            odef(ok_def, "Ok", ItemKind::Variant, true),
            odef(err_def, "Err", ItemKind::Variant, true),
            odef(cf_def, "ControlFlow", ItemKind::Enum, false),
            odef(vec_def, "Vec", ItemKind::Struct, false),
            odef(box_def, "Box", ItemKind::Struct, false),
            odef(str_adt_def, "String", ItemKind::Struct, false),
            odef(pin_def, "Pin", ItemKind::Struct, false),
            odef(fut_trait, "Future", ItemKind::Trait, false),
            odef(fn_trait, "Fn", ItemKind::Trait, false),
            odef(fn_mut_trait, "FnMut", ItemKind::Trait, false),
            odef(fn_once_trait, "FnOnce", ItemKind::Trait, false),
        ] {
            item_db.add_module_def(prelude, d);
        }

        let mut ty_db = MemTyDefDb::new();
        for (li, def) in [
            (TyLangItem::Option, opt_def),
            (TyLangItem::OptionSome, some_def),
            (TyLangItem::OptionNone, none_def),
            (TyLangItem::Result, res_def),
            (TyLangItem::ResultOk, ok_def),
            (TyLangItem::ResultErr, err_def),
            (TyLangItem::ControlFlow, cf_def),
            (TyLangItem::OwnedBox, box_def),
            (TyLangItem::String, str_adt_def),
            (TyLangItem::Pin, pin_def),
            (TyLangItem::Future, fut_trait),
            (TyLangItem::Fn, fn_trait),
            (TyLangItem::FnMut, fn_mut_trait),
            (TyLangItem::FnOnce, fn_once_trait),
            (TyLangItem::Range, range_def),
            (TyLangItem::RangeInclusive, range_inc_def),
            (TyLangItem::RangeFrom, range_from_def),
            (TyLangItem::RangeTo, range_to_def),
            (TyLangItem::RangeToInclusive, range_to_inc_def),
            (TyLangItem::RangeFull, range_full_def),
        ] {
            ty_db.add_lang_item(li, def);
        }

        let (p0, p1) = (ty_db.param(0), ty_db.param(1));
        ty_db.add_adt(opt_def, 1, CtorShapeKind::None);
        ty_db.add_variant(some_def, opt_def, 1, CtorShapeKind::Tuple);
        ty_db.set_tuple_fields(some_def, &[p0]);
        ty_db.add_variant(none_def, opt_def, 1, CtorShapeKind::Unit);

        ty_db.add_adt(res_def, 2, CtorShapeKind::None);
        ty_db.add_variant(ok_def, res_def, 2, CtorShapeKind::Tuple);
        ty_db.set_tuple_fields(ok_def, &[p0]);
        ty_db.add_variant(err_def, res_def, 2, CtorShapeKind::Tuple);
        ty_db.set_tuple_fields(err_def, &[p1]);

        ty_db.add_adt(cf_def, 2, CtorShapeKind::None);
        let vec_t = ty_db.add_adt(vec_def, 1, CtorShapeKind::Record);
        ty_db.add_adt(box_def, 1, CtorShapeKind::Record);
        ty_db.add_adt(str_adt_def, 0, CtorShapeKind::Record);
        ty_db.add_adt(pin_def, 1, CtorShapeKind::Record);

        let sl_p0 = ty_db.templates.mk_slice(p0);
        ty_db.add_assoc_rule(TyLangItem::DerefTarget, 1, vec_t, &[], sl_p0);
        ty_db.add_assoc_rule(TyLangItem::IntoIteratorItem, 1, vec_t, &[], p0);

        Self { item_db, ty_db, krate, opt_def, fut_trait, range_def, range_inc_def, range_from_def, range_to_def, range_to_inc_def, range_full_def }
    }

    fn infer_fn_by_name<'s>(&self, src: &'s str, fn_name: &str) -> (SyntaxTree<'s>, FileNameRes, BodyInferCtx, FnInferResult) {
        let mut sources = MemSourceDb::new();
        let file: FileId = sources.add_file("src/lib.rs", src);
        let mod_tree = ModuleTree::build(&self.item_db, &sources, self.krate, file);
        let def_map = CrateDefMap::build(&self.item_db, &sources, &mod_tree);
        let st = parse(src);
        let file_res = def_map.resolve_file_paths(&self.item_db, &sources, &mod_tree, file);
        let local_defs = LocalCstDefs::from_file(&st, Some(&def_map), file);
        let fn_node = find_fn_node(&st, fn_name);
        let mut ctx = BodyInferCtx::new();
        let res = ctx.infer_fn(&st, &file_res, &self.ty_db, &local_defs, fn_node);
        (st, file_res, ctx, res)
    }
}

fn find_fn_node(st: &SyntaxTree<'_>, name: &str) -> NodeId {
    st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == name)).unwrap_or_else(|| panic!("fn `{name}` not found"))
}

fn local_ty_by_name(src: &str, file_res: &FileNameRes, res: &FnInferResult, name: &str) -> InferTyId {
    let &(lid, ty) =
        res.local_tys.iter().find(|&&(lid, _)| file_res.local_data(lid).name(src) == name).unwrap_or_else(|| panic!("local `{name}` not found in {:?}", res.local_tys));
    let _ = lid;
    ty
}

#[test]
fn literals_let_annotations_hex_suffix_and_fallbacks() {
    let fx = StdFixture::new();
    let src = r##"
        fn demo() -> (i32, f64, u64, u32) {
            let a = 42;
            let b = 3.5;
            let c: u64 = 7;
            let hex_unsuf = 0x1f32;
            let d = 1f32;
            let bs = b"hi\n";
            let sl: &[u8] = b"abcd";
            let flag = true && !false;
            let ch = 'z';
            let s = r#"raw"#;
            let _ = (d, bs, sl, flag, ch, s);
            (a, b, c, hex_unsuf)
        }
    "##;
    let (_, fres, mut ctx, res) = fx.infer_fn_by_name(src, "demo");
    assert_eq!(local_ty_by_name(src, &fres, &res, "a"), InferTyId::I32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "b"), InferTyId::F64);
    assert_eq!(local_ty_by_name(src, &fres, &res, "c"), InferTyId::U64);
    assert_eq!(local_ty_by_name(src, &fres, &res, "hex_unsuf"), InferTyId::U32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "d"), InferTyId::F32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "flag"), InferTyId::BOOL);
    assert_eq!(local_ty_by_name(src, &fres, &res, "ch"), InferTyId::CHAR);
    assert_eq!(local_ty_by_name(src, &fres, &res, "s"), InferTyId::STR_REF);
    let bs_expected = {
        let arr = ctx.table.arena.mk_array(InferTyId::U8, 3);
        ctx.table.arena.mk_ref(false, arr)
    };
    let sl_expected = {
        let sl = ctx.table.arena.mk_slice(InferTyId::U8);
        ctx.table.arena.mk_ref(false, sl)
    };
    assert_eq!(local_ty_by_name(src, &fres, &res, "bs"), bs_expected);
    assert_eq!(local_ty_by_name(src, &fres, &res, "sl"), sl_expected);
}

#[test]
fn generic_calls_struct_literals_tuples_and_chained_tuple_fields() {
    let fx = StdFixture::new();
    let src = r#"
        struct Pair<A, B> {
            first: A,
            second: B,
        }
        fn make_pair<X, Y>(x: X, y: Y) -> Pair<X, Y> {
            Pair { first: x, second: y }
        }
        fn check() -> u16 {
            let first = 10u16;
            let p = Pair { first, second: ("ok", (true, 99u64)) };
            let p2 = Pair { first: 20, ..p };
            let nested_num = p.second.1.1;
            let q = make_pair(p2.first, nested_num);
            q.first
        }
    "#;
    let (_, fres, _, res) = fx.infer_fn_by_name(src, "check");
    assert_eq!(local_ty_by_name(src, &fres, &res, "nested_num"), InferTyId::U64);
    assert_eq!(res.body_ty, InferTyId::U16);
}

#[test]
fn refs_derefs_if_match_coercion_ergonomics_and_loops() {
    let fx = StdFixture::new();
    let src = r#"
        fn control(cond: bool, opt: &Option<u32>, mut x: u32, y: u32) -> u32 {
            let joined_ref = if cond { &mut x } else { &y };
            let deref_val = *joined_ref;
            let m = match opt {
                Some(v) => *v + deref_val,
                None => return 0,
            };
            let loop_val = 'outer: loop {
                if m > 10 {
                    break 'outer 5u8;
                }
                break 9;
            };
            let inf = loop {};
            let _ = (loop_val, inf);
            m
        }
    "#;
    let (_, fres, mut ctx, res) = fx.infer_fn_by_name(src, "control");
    let ref_u32 = ctx.table.arena.mk_ref(false, InferTyId::U32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "joined_ref"), ref_u32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "deref_val"), InferTyId::U32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "v"), ref_u32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "m"), InferTyId::U32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "loop_val"), InferTyId::U8);
    assert_eq!(local_ty_by_name(src, &fres, &res, "inf"), InferTyId::NEVER);
}

#[test]
fn try_operator_await_future_and_async_blocks() {
    let fx = StdFixture::new();
    let src = r#"
        async fn fetch_len(flag: bool) -> usize {
            if flag { 12 } else { 0 }
        }
        async fn pipeline(r: Result<Option<u64>, &str>, cf: ControlFlow<bool, i16>) -> Option<u64> {
            let inner_opt = r.ok();
            let _ = inner_opt;
            let unwrapped_res = match Ok::<Option<u64>, &str>(Some(5)) {
                x => x?,
            };
            let val = unwrapped_res?;
            let cf_cont = cf?;
            let n = fetch_len(true).await;
            let fut_blk = async { val };
            let awaited_blk = fut_blk.await;
            let _ = (cf_cont, n);
            Some(awaited_blk)
        }
    "#;
    let (_, fres, mut ctx, res) = fx.infer_fn_by_name(src, "pipeline");
    let opt_u64 = ctx.table.arena.mk_adt(fx.opt_def, &[InferTyId::U64]);
    let fut_u64 = ctx.table.arena.mk_opaque(fx.fut_trait, &[InferTyId::U64]);
    assert_eq!(local_ty_by_name(src, &fres, &res, "unwrapped_res"), opt_u64);
    assert_eq!(local_ty_by_name(src, &fres, &res, "val"), InferTyId::U64);
    assert_eq!(local_ty_by_name(src, &fres, &res, "cf_cont"), InferTyId::I16);
    assert_eq!(local_ty_by_name(src, &fres, &res, "n"), InferTyId::USIZE);
    assert_eq!(local_ty_by_name(src, &fres, &res, "fut_blk"), fut_u64);
    assert_eq!(local_ty_by_name(src, &fres, &res, "awaited_blk"), InferTyId::U64);
    assert_eq!(res.ret_ty, opt_u64);
}

#[test]
fn closures_infer_params_from_fn_bounds_in_both_arg_orders_and_dyn_fn() {
    let fx = StdFixture::new();
    let src = r#"
        fn apply<T, U, F: FnOnce(T) -> U>(val: T, f: F) -> U {
            f(val)
        }
        fn apply_rev<T, U, F>(f: F, val: T) -> U
        where
            F: Fn(T) -> U,
        {
            f(val)
        }
        fn test_closures(bytes: &[u8]) {
            let a = apply(10u32, |x| x + 5);
            let b = apply_rev(|s| s[0], bytes);
            let fp: fn(i64) -> i64 = |z| {
                if z < 0 { return 0; }
                z + 1
            };
            let dyn_f: &dyn Fn(u16) -> bool = &|k| k == 3;
            let ok = dyn_f(3);
            let _ = (a, b, fp, ok);
        }
    "#;
    let (_, fres, mut ctx, res) = fx.infer_fn_by_name(src, "test_closures");
    let sl_u8 = ctx.table.arena.mk_slice(InferTyId::U8);
    let ref_sl_u8 = ctx.table.arena.mk_ref(false, sl_u8);
    assert_eq!(local_ty_by_name(src, &fres, &res, "x"), InferTyId::U32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "a"), InferTyId::U32);
    assert_eq!(local_ty_by_name(src, &fres, &res, "s"), ref_sl_u8);
    assert_eq!(local_ty_by_name(src, &fres, &res, "b"), InferTyId::U8);
    assert_eq!(local_ty_by_name(src, &fres, &res, "z"), InferTyId::I64);
    assert_eq!(local_ty_by_name(src, &fres, &res, "k"), InferTyId::U16);
    assert_eq!(local_ty_by_name(src, &fres, &res, "ok"), InferTyId::BOOL);
}

#[test]
fn ranges_slice_indexing_vec_autoderef_indexing_and_for_loops() {
    let fx = StdFixture::new();
    let src = r#"
        fn indexing(v: Vec<u16>, s: &str, bx: Box<String>) {
            let r1 = 0..10usize;
            let r2 = 1u8..=5;
            let r3 = 4u64..;
            let r4 = ..8i16;
            let r5 = ..=3u32;
            let r6 = ..;
            let elem = v[0];
            let sub_sl = &v[1..3];
            let sub_str = &s[..2];
            let deref_str = &**bx;
            for item in v {
                let copy_item = item;
                let _ = copy_item;
            }
            for idx in 0..4u32 {
                let copy_idx = idx;
                let _ = copy_idx;
            }
            let _ = (r1, r2, r3, r4, r5, r6, elem, sub_sl, sub_str, deref_str);
        }
    "#;
    let (_, fres, mut ctx, res) = fx.infer_fn_by_name(src, "indexing");
    let t_r1 = ctx.table.arena.mk_adt(fx.range_def, &[InferTyId::USIZE]);
    let t_r2 = ctx.table.arena.mk_adt(fx.range_inc_def, &[InferTyId::U8]);
    let t_r3 = ctx.table.arena.mk_adt(fx.range_from_def, &[InferTyId::U64]);
    let t_r4 = ctx.table.arena.mk_adt(fx.range_to_def, &[InferTyId::I16]);
    let t_r5 = ctx.table.arena.mk_adt(fx.range_to_inc_def, &[InferTyId::U32]);
    let t_r6 = ctx.table.arena.mk_adt(fx.range_full_def, &[]);
    let sl_u16 = ctx.table.arena.mk_slice(InferTyId::U16);
    let ref_sl_u16 = ctx.table.arena.mk_ref(false, sl_u16);
    assert_eq!(local_ty_by_name(src, &fres, &res, "r1"), t_r1);
    assert_eq!(local_ty_by_name(src, &fres, &res, "r2"), t_r2);
    assert_eq!(local_ty_by_name(src, &fres, &res, "r3"), t_r3);
    assert_eq!(local_ty_by_name(src, &fres, &res, "r4"), t_r4);
    assert_eq!(local_ty_by_name(src, &fres, &res, "r5"), t_r5);
    assert_eq!(local_ty_by_name(src, &fres, &res, "r6"), t_r6);
    assert_eq!(local_ty_by_name(src, &fres, &res, "elem"), InferTyId::U16);
    assert_eq!(local_ty_by_name(src, &fres, &res, "sub_sl"), ref_sl_u16);
    assert_eq!(local_ty_by_name(src, &fres, &res, "sub_str"), InferTyId::STR_REF);
    assert_eq!(local_ty_by_name(src, &fres, &res, "deref_str"), InferTyId::STR_REF);
    assert_eq!(local_ty_by_name(src, &fres, &res, "copy_item"), InferTyId::U16);
    assert_eq!(local_ty_by_name(src, &fres, &res, "copy_idx"), InferTyId::U32);
    assert!(matches!(ctx.table.arena.kind(ref_sl_u16), InferTyKind::Ref { .. }));
}
