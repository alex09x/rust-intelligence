/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for Phase 3 Checkbox `3.5` (error tolerance & partial-code receiver recovery):
//! 1. Unresolved paths, missing operands, unclosed parens, and incomplete expressions infer `{unknown}`
//!    (`InferTyId::UNKNOWN`) without panicking, blocking, or leaking `Infer(_)` variables, while
//!    unaffected expressions in the same function keep exact types.
//! 2. Partial trailing `.` (`x.`, `self.0.`, `(a, b).1.`, inside unclosed calls/closures/arms/blocks)
//!    recovers the exact receiver type via `FnInferResult::receiver_ty_at_dot`.
//! 3. Deterministic token-deletion & random-truncation fuzz stress across rich Rust syntax (`0` panics).
//! 4. Equivalence between `resolve_fn_paths_into` + `index_target_fn_with_item_db` and full-file resolution.

use ridx_syntax::name_res::{CrateDefMap, CrateId, CrateSnapshot, Edition, FileNameRes, GlobalDefId, ItemKind, MemItemDb, MemSourceDb, ModuleTree, OwnedDef, VisKind};
use ridx_syntax::syntax_kind::SyntaxKind as K;
use ridx_syntax::ty_infer::{BodyInferCtx, CtorShapeKind, FnInferResult, InferTyId, InferTyKind, LocalCstDefs, MemTyDefDb, TyLangItem};
use ridx_syntax::{lex, parse};

struct PartialFixture {
    item_db: MemItemDb,
    ty_db: MemTyDefDb,
    krate: CrateId,
    vec_def: GlobalDefId,
    opt_def: GlobalDefId,
    into_def: GlobalDefId,
}

impl PartialFixture {
    fn new() -> Self {
        let mut item_db = MemItemDb::new();
        let std_c = item_db.add_crate("std", Edition::Edition2021);
        let krate = item_db.add_crate("app", Edition::Edition2021);
        let prelude = GlobalDefId::new(std_c, 1);
        item_db.set_crate_root_def(std_c, GlobalDefId::new(std_c, 0));
        item_db.set_prelude_module(krate, prelude);
        item_db.add_extern_crate(krate, "std", std_c);
        item_db.set_crate_root_def(krate, GlobalDefId::new(krate, 0));

        let (opt_def, some_def, vec_def, fn_once_def) = (GlobalDefId::new(std_c, 10), GlobalDefId::new(std_c, 11), GlobalDefId::new(std_c, 12), GlobalDefId::new(std_c, 13));
        let (into_iter_def, into_def) = (GlobalDefId::new(std_c, 14), GlobalDefId::new(std_c, 15));
        for (id, name, kind, ctor) in [
            (opt_def, "Option", ItemKind::Enum, false),
            (some_def, "Some", ItemKind::Variant, true),
            (vec_def, "Vec", ItemKind::Struct, false),
            (fn_once_def, "FnOnce", ItemKind::Trait, false),
            (into_iter_def, "IntoIterator", ItemKind::Trait, false),
            (into_def, "Into", ItemKind::Trait, false),
        ] {
            item_db.add_module_def(prelude, OwnedDef { id, name: name.into(), kind, vis: VisKind::Public, has_value_ctor: ctor });
        }

        let mut ty_db = MemTyDefDb::new();
        let p0 = ty_db.param(0);
        ty_db.add_adt(opt_def, 1, CtorShapeKind::None);
        ty_db.add_variant(some_def, opt_def, 1, CtorShapeKind::Tuple);
        ty_db.set_tuple_fields(some_def, &[p0]);
        let vec_p0 = ty_db.add_adt(vec_def, 1, CtorShapeKind::None);
        ty_db.add_lang_item(TyLangItem::Option, opt_def);
        ty_db.add_lang_item(TyLangItem::FnOnce, fn_once_def);
        ty_db.add_lang_item(TyLangItem::IntoIterator, into_iter_def);

        let vec_len = GlobalDefId::new(std_c, 20);
        let ref_vec_p0 = ty_db.templates.mk_ref(false, vec_p0);
        ty_db.add_fn(vec_len, 1, &[ref_vec_p0], InferTyId::USIZE);
        ty_db.add_inherent_method(ridx_syntax::name_res::ImplTargetKey::Adt(vec_def), "len", vec_len, 1, 0);

        Self { item_db, ty_db, krate, vec_def, opt_def, into_def }
    }

    fn infer_fn_named(&self, src: &str, fn_name: &str) -> (BodyInferCtx, FnInferResult, ridx_syntax::SyntaxTree<'static>) {
        let leaked: &'static str = Box::leak(src.to_owned().into_boxed_str());
        let mut sources = MemSourceDb::new();
        let fid = sources.add_file("src/lib.rs", leaked);
        let st = parse(leaked);
        let mod_tree = ModuleTree::build(&self.item_db, &sources, self.krate, fid);
        let def_map = CrateDefMap::build(&self.item_db, &sources, &mod_tree);
        let file_res = def_map.resolve_file_paths(&self.item_db, &sources, &mod_tree, fid);
        let local_defs = LocalCstDefs::from_file(&st, Some(&def_map), fid);
        let fn_node = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == fn_name)).expect("target fn");
        let mut ctx = BodyInferCtx::new();
        let res = ctx.infer_fn(&st, &file_res, &self.ty_db, &local_defs, fn_node);
        assert_no_leaked_infer_vars(&ctx, &res);
        (ctx, res, st)
    }
}

fn assert_no_leaked_infer_vars(ctx: &BodyInferCtx, res: &FnInferResult) {
    for &(_, ty) in res.expr_tys.iter().chain(res.pat_tys.iter()) {
        assert!(!matches!(ctx.table.arena.kind(ty), InferTyKind::Infer(_)), "leaked Infer var in node ty: {:?}", ctx.table.arena.kind(ty));
    }
    for &(_, ty) in &res.local_tys {
        assert!(!matches!(ctx.table.arena.kind(ty), InferTyKind::Infer(_)), "leaked Infer var in local ty: {:?}", ctx.table.arena.kind(ty));
    }
    for &(_, ty) in &res.incomplete_dots {
        assert!(!matches!(ctx.table.arena.kind(ty), InferTyKind::Infer(_)), "leaked Infer var in incomplete_dots: {:?}", ctx.table.arena.kind(ty));
    }
}

#[test]
fn unknown_formatting_and_error_isolation() {
    let fx = PartialFixture::new();
    let src = r#"
        struct Pair { left: u32, right: bool }

        fn broken(v: Vec<u64>, p: Pair) -> usize {
            let bad1 = non_existent_fn(1, 2);
            let bad2 = p.left + ;
            let bad3 = (;
            let bad4 = if p.right ;
            let keep_len = v.len();
            let keep_flag = p.right;
            let _ = (bad1, bad2, bad3, bad4, keep_flag);
            keep_len
        }
    "#;
    let (ctx, res, st) = fx.infer_fn_named(src, "broken");
    assert_eq!(ctx.table.arena.format_ty(InferTyId::UNKNOWN), "{unknown}");
    assert_eq!(ctx.table.arena.format_ty(InferTyId::NEVER), "!");
    assert_eq!(ctx.table.arena.format_ty(InferTyId::UNIT), "()");

    let mut local_by_name = std::collections::HashMap::new();
    for n in st.descendants(st.root()).filter(|&n| st.kind(n) == K::IdentPat) {
        let txt = st.text(n).trim().to_string();
        if let Some(ty) = res.pat_ty(n) {
            local_by_name.insert(txt, ty);
        }
    }
    assert_eq!(local_by_name["bad1"], InferTyId::UNKNOWN);
    assert_eq!(local_by_name["bad2"], InferTyId::UNKNOWN);
    assert_eq!(local_by_name["bad3"], InferTyId::UNKNOWN);
    assert_eq!(local_by_name["bad4"], InferTyId::UNKNOWN);
    assert_eq!(local_by_name["keep_len"], InferTyId::USIZE);
    assert_eq!(local_by_name["keep_flag"], InferTyId::BOOL);
    assert_eq!(res.body_ty, InferTyId::USIZE);
}

#[test]
fn trailing_dot_and_chained_tuple_dot_recover_receiver_types() {
    let fx = PartialFixture::new();
    // 1. Simple trailing `v.` at EOF (unclosed function block).
    let s1 = "fn f(v: Vec<u64>) { v.";
    let (ctx1, res1, st1) = fx.infer_fn_named(s1, "f");
    let recv1 = res1.receiver_ty_at_dot(&st1, s1.len() as u32).expect("receiver at EOF v.");
    let exp_vec_u64 = {
        let mut a = ctx1.table;
        a.arena.mk_adt(fx.vec_def, &[InferTyId::U64])
    };
    assert_eq!(recv1, exp_vec_u64);

    // 2. Chained tuple field `self.0.` and nested `(1u8, (true, v)).1.1.` (lexed as Float `0.` / `1.`).
    let s2 = r#"
        struct Wrap((u32, bool));
        impl Wrap {
            fn check(&self, v: Vec<u8>) {
                let _ = self.0.;
                let nested = (1u16, (true, v));
                nested.1.1.
            }
        }
    "#;
    let (mut ctx2, res2, st2) = fx.infer_fn_named(s2, "check");
    let dot_pos_1 = (s2.find("self.0.").unwrap() + "self.0.".len()) as u32;
    let dot_pos_2 = (s2.find("nested.1.1.").unwrap() + "nested.1.1.".len()) as u32;
    let exp_tup = ctx2.table.arena.mk_tuple(&[InferTyId::U32, InferTyId::BOOL]);
    let exp_vec_u8 = ctx2.table.arena.mk_adt(fx.vec_def, &[InferTyId::U8]);
    assert_eq!(res2.receiver_ty_at_dot(&st2, dot_pos_1), Some(exp_tup));
    assert_eq!(res2.receiver_ty_at_dot(&st2, dot_pos_2), Some(exp_vec_u8));

    // 3. Partial `x.` inside an unclosed call argument + closure parameter inferred from `Option`.
    let s3 = r#"
        fn take_fn<F: FnOnce(Option<u32>) -> bool>(f: F) {}
        fn g(items: &Vec<i64>) {
            take_fn(|opt| {
                let _ = items.;
                opt.
    "#;
    let (mut ctx3, res3, st3) = fx.infer_fn_named(s3, "g");
    let items_dot = (s3.find("items.").unwrap() + "items.".len()) as u32;
    let opt_dot = (s3.rfind("opt.").unwrap() + "opt.".len()) as u32;
    let vec_i64 = ctx3.table.arena.mk_adt(fx.vec_def, &[InferTyId::I64]);
    let ref_vec_i64 = ctx3.table.arena.mk_ref(false, vec_i64);
    let opt_u32 = ctx3.table.arena.mk_adt(fx.opt_def, &[InferTyId::U32]);
    assert_eq!(res3.receiver_ty_at_dot(&st3, items_dot), Some(ref_vec_i64));
    assert_eq!(res3.receiver_ty_at_dot(&st3, opt_dot), Some(opt_u32));
}

#[test]
fn parser_minus_without_literal_and_mutation_stress_never_panics() {
    let fx = PartialFixture::new();
    // Guard against `-` with deleted literal in patterns and const generic args (`match x { - => 1 }`, `Foo<->`).
    let tricky = r#"
        struct Arr<const N: usize = ->([u8; 1]);
        fn edge(x: i32) -> i32 {
            let _: Arr<->;
            match x {
                - => 1,
                0..- => 2,
                _ => 3,
            }
        }
    "#;
    let (_, edge_res, _) = fx.infer_fn_named(tricky, "edge");
    assert_eq!(edge_res.body_ty, InferTyId::I32);

    let rich = r#"
        struct Node<T> { val: T, count: usize }
        impl<T> Node<T> {
            fn demo(&self, xs: &[i32], opt: Option<u64>) -> usize {
                let mut total = self.count;
                for &item in xs {
                    let delta = match opt {
                        Some(n) if item > -1 => (n as usize) + (item as usize),
                        _ => 0,
                    };
                    total += delta;
                }
                let f = |z: usize| z.wrapping_add(total);
                let _ = f(1);
                total
            }
        }
    "#;

    let toks: Vec<_> = lex(rich).into_iter().filter(|t| !t.is_trivia()).collect();
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next_u64 = || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };

    let mut ctx = BodyInferCtx::new();
    let mut out = FnInferResult::new();
    let mut file_res = FileNameRes::new();
    let mut local_defs = LocalCstDefs::new();

    for iter in 0..240usize {
        let mutated = if iter % 2 == 0 {
            let pick = &toks[(next_u64() as usize) % toks.len()];
            format!("{}{}", &rich[..pick.start as usize], &rich[pick.end as usize..])
        } else {
            let mut cut = (next_u64() as usize) % (rich.len() + 1);
            while !rich.is_char_boundary(cut) {
                cut -= 1;
            }
            rich[..cut].to_string()
        };
        let mut sources = MemSourceDb::new();
        let fid = sources.add_file("src/lib.rs", &mutated);
        let st = parse(&mutated);
        let mod_tree = ModuleTree::build(&fx.item_db, &sources, fx.krate, fid);
        let def_map = CrateDefMap::build(&fx.item_db, &sources, &mod_tree);
        let mut ext_c = ridx_syntax::name_res::ExtScopeCache::new();
        def_map.resolve_file_paths_into(&fx.item_db, &mod_tree, &mut ext_c, fid, &st, &mut file_res);
        local_defs.index_file(&st, Some(&def_map), fid);
        for n in st.descendants(st.root()).filter(|&n| matches!(st.kind(n), K::Fn | K::Const | K::Static)) {
            ctx.infer_fn_into(&st, &file_res, &fx.ty_db, &local_defs, n, &mut out);
            assert_no_leaked_infer_vars(&ctx, &out);
        }
    }
}

#[test]
fn single_fn_targeted_resolution_matches_full_file_resolution() {
    let fx = PartialFixture::new();
    let src = r#"
        fn sibling_before(a: u32) -> u32 { a + 1 }
        struct Boxed { n: usize }
        impl Boxed {
            fn skipped_method(&self) -> usize { self.n }
            fn target_method(&self, v: Vec<u64>) -> usize {
                v.len() + self.n
            }
        }
        fn sibling_after(b: bool) -> bool { !b }
    "#;
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", src);
    let snap = CrateSnapshot::build_clean(&fx.item_db, &sources, fx.krate, fid);
    let st = parse(src);
    let target_fn = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == "target_method")).unwrap();

    let mut full_res = FileNameRes::new();
    snap.resolve_file_paths_into(&fx.item_db, fid, &st, &mut full_res);
    let mut full_ld = LocalCstDefs::new();
    full_ld.index_file_with_item_db(&st, Some(&snap.clean_def_map), Some(&fx.item_db), fid);
    let mut ctx_a = BodyInferCtx::new();
    let out_a = ctx_a.infer_fn(&st, &full_res, &fx.ty_db, &full_ld, target_fn);

    let mut single_res = FileNameRes::new();
    snap.resolve_fn_paths_into(&fx.item_db, fid, &st, target_fn, &mut single_res);
    let mut single_ld = LocalCstDefs::new();
    single_ld.index_target_fn_with_item_db(&st, Some(&snap.clean_def_map), Some(&fx.item_db), fid, target_fn);
    let mut ctx_b = BodyInferCtx::new();
    let out_b = ctx_b.infer_fn(&st, &single_res, &fx.ty_db, &single_ld, target_fn);

    assert_eq!(out_a.expr_tys, out_b.expr_tys);
    assert_eq!(out_a.pat_tys, out_b.pat_tys);
    assert_eq!(out_a.dot_refs, out_b.dot_refs);
    assert!(single_res.refs.len() < full_res.refs.len(), "expected sibling functions to be pruned");
}

#[test]
fn impl_trait_for_loop_partial_dot_receiver_recovery() {
    let fx = PartialFixture::new();
    let src = r#"
        fn collect_names(names: impl IntoIterator<Item = impl Into<u64>>) {
            for item in names {
                item.
    "#;
    let (mut ctx, res, st) = fx.infer_fn_named(src, "collect_names");
    let dot_pos = (src.rfind("item.").unwrap() + "item.".len()) as u32;
    let exp_opaque = ctx.table.arena.mk_opaque(fx.into_def, &[InferTyId::U64]);
    assert_eq!(res.receiver_ty_at_dot(&st, dot_pos), Some(exp_opaque));
}

#[test]
fn closure_user_display_and_slice_pat_const_array_len_inference() {
    let fx = PartialFixture::new();
    let src = r#"
        fn check_closures() {
            let add_one = |x: u32| -> u32 { x + 1 };
            let noop = || {};
            let _ = (add_one, noop);
        }
    "#;
    let (mut ctx, res, st) = fx.infer_fn_named(src, "check_closures");
    let closures: Vec<_> = st.descendants(st.root()).filter(|&n| st.kind(n) == K::ClosureExpr).collect();
    assert_eq!(closures.len(), 2);
    let ty0 = res.expr_tys.iter().find(|&&(n, _)| n == closures[0]).unwrap().1;
    let ty1 = res.expr_tys.iter().find(|&&(n, _)| n == closures[1]).unwrap().1;
    assert_eq!(ctx.table.arena.format_ty(ty0), "impl Fn(u32) -> u32");
    assert_eq!(ctx.table.arena.format_ty(ty1), "impl Fn()");

    let n_var = ctx.table.new_ty_var();
    let InferTyKind::Infer(v) = ctx.table.arena.kind(n_var) else { panic!() };
    let arr_ty = ctx.table.arena.mk_array(InferTyId::U32, ridx_syntax::ty_infer::arena::ARRAY_LEN_VAR_TAG | u64::from(v.0));
    let carrier = ctx.table.arena.mk_array(InferTyId::NEVER, 2);
    let _ = ctx.table.equate(n_var, carrier);
    let resolved = ctx.table.deep_resolve(arr_ty);
    assert_eq!(ctx.table.arena.kind(resolved), InferTyKind::Array { elem: InferTyId::U32, len: 2 });
}
