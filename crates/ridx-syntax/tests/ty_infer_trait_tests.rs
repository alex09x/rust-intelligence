/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for Phase 3 Checkbox `3.4` (minimal trait solving):
//! 1. `impl` selection by `Self` type with generic arguments (`Vec::from([1, 2])`, overloaded `From<T>`,
//!    `Default::default()`, `Iterator::collect::<Vec<_>>()`, and `<Type as Trait<Args>>::fn`).
//! 2. Associated type projection (`<I as Iterator>::Item`, `<T as MyAdd<Rhs>>::Output`, inline & `where`
//!    parameter bounds, and supertrait associated types).
//! 3. Blanket `impl`s over bounds (`impl<T: Marker> Ext for T` rejecting unmarked types cleanly, disambiguating
//!    competing traits in scope, and return-directed `impl<T, U: MyFrom<T>> MyInto<U> for T`).

use ridx_syntax::name_res::{
    CrateDefMap, CrateId, Edition, FileId, FileNameRes, GlobalDefId, ImplTargetKey, IndexedImpl, ItemKind, MemItemDb, MemSourceDb, ModuleTree, OwnedDef, PrimTy, VisKind,
};
use ridx_syntax::syntax_kind::SyntaxKind as K;
use ridx_syntax::ty_infer::{BodyInferCtx, CtorShapeKind, EmptyTyDefDb, FnInferResult, InferTyId, InferTyKind, LocalCstDefs, MemTyDefDb, TyLangItem};
use ridx_syntax::{parse, SyntaxTree};

fn gdef(c: u32, i: u32) -> GlobalDefId {
    GlobalDefId::new(CrateId(c), i)
}

fn odef(id: GlobalDefId, name: &str, kind: ItemKind, ctor: bool) -> OwnedDef {
    OwnedDef { id, name: name.into(), kind, vis: VisKind::Public, has_value_ctor: ctor }
}

struct TraitFixture {
    item_db: MemItemDb,
    ty_db: MemTyDefDb,
    krate: CrateId,
    opt_def: GlobalDefId,
    vec_def: GlobalDefId,
    good_ext_def: GlobalDefId,
    bad_ext_def: GlobalDefId,
}

impl TraitFixture {
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
        let (vec_def, into_iter_adt) = (gdef(std_c.0, 13), gdef(std_c.0, 14));
        let (good_ext_def, bad_ext_def) = (gdef(std_c.0, 15), gdef(std_c.0, 16));
        let (iter_tr, iter_item_assoc, into_iter_tr, from_iter_tr) = (gdef(std_c.0, 20), gdef(std_c.0, 21), gdef(std_c.0, 22), gdef(std_c.0, 23));
        let (from_tr, default_tr, sub_iter_tr) = (gdef(std_c.0, 24), gdef(std_c.0, 25), gdef(std_c.0, 26));
        let (copy_tag_tr, marked_ext_tr) = (gdef(std_c.0, 27), gdef(std_c.0, 28));

        for d in [
            odef(opt_def, "Option", ItemKind::Enum, false),
            odef(some_def, "Some", ItemKind::Variant, true),
            odef(none_def, "None", ItemKind::Variant, true),
            odef(vec_def, "Vec", ItemKind::Struct, false),
            odef(into_iter_adt, "IntoIter", ItemKind::Struct, false),
            odef(good_ext_def, "GoodExt", ItemKind::Struct, true),
            odef(bad_ext_def, "BadExt", ItemKind::Struct, true),
            odef(iter_tr, "Iterator", ItemKind::Trait, false),
            odef(into_iter_tr, "IntoIterator", ItemKind::Trait, false),
            odef(from_iter_tr, "FromIterator", ItemKind::Trait, false),
            odef(from_tr, "From", ItemKind::Trait, false),
            odef(default_tr, "Default", ItemKind::Trait, false),
            odef(sub_iter_tr, "ExactIter", ItemKind::Trait, false),
            odef(copy_tag_tr, "CopyTag", ItemKind::Trait, false),
            odef(marked_ext_tr, "MarkedExt", ItemKind::Trait, false),
        ] {
            item_db.add_module_def(prelude, d);
        }
        item_db.add_assoc_item(iter_tr, odef(iter_item_assoc, "Item", ItemKind::AssocTy, false));
        item_db.add_trait_supertrait(sub_iter_tr, iter_tr);

        let mut ty_db = MemTyDefDb::new();
        ty_db.add_lang_item(TyLangItem::Option, opt_def);
        ty_db.add_lang_item(TyLangItem::OptionSome, some_def);
        ty_db.add_lang_item(TyLangItem::OptionNone, none_def);
        ty_db.add_lang_item(TyLangItem::Iterator, iter_tr);
        ty_db.add_lang_item(TyLangItem::IteratorItem, iter_item_assoc);
        ty_db.add_lang_item(TyLangItem::IntoIterator, into_iter_tr);
        ty_db.add_supertrait(sub_iter_tr, iter_tr);

        let (p0, p1) = (ty_db.param(0), ty_db.param(1));
        ty_db.add_adt(opt_def, 1, CtorShapeKind::None);
        ty_db.add_variant(some_def, opt_def, 1, CtorShapeKind::Tuple);
        ty_db.set_tuple_fields(some_def, &[p0]);
        ty_db.add_variant(none_def, opt_def, 1, CtorShapeKind::Unit);
        let vec_p0 = ty_db.add_adt(vec_def, 1, CtorShapeKind::None);
        let ii_p0 = ty_db.add_adt(into_iter_adt, 1, CtorShapeKind::None);
        let good_ty = ty_db.add_adt(good_ext_def, 0, CtorShapeKind::Unit);
        let _bad_ty = ty_db.add_adt(bad_ext_def, 0, CtorShapeKind::Unit);

        let (vec_ii_fn, iter_next_fn, iter_collect_fn, from_fn, vec_from_arr_fn, def_fn, vec_def_fn, ext_val_fn) =
            (gdef(std_c.0, 50), gdef(std_c.0, 51), gdef(std_c.0, 52), gdef(std_c.0, 53), gdef(std_c.0, 54), gdef(std_c.0, 55), gdef(std_c.0, 56), gdef(std_c.0, 57));
        ty_db.add_fn(vec_ii_fn, 1, &[vec_p0], ii_p0);
        ty_db.add_trait_method(into_iter_tr, 0, "into_iter", vec_ii_fn, 0);
        ty_db.add_trait_impl(into_iter_tr, 1, vec_p0, &[], &[("into_iter", vec_ii_fn, 0)]);

        let self_item = ty_db.templates.mk_alias(iter_tr, iter_item_assoc, p0, &[]);
        let mut_self = ty_db.templates.mk_ref(true, p0);
        let opt_item = ty_db.templates.mk_adt(opt_def, &[self_item]);
        ty_db.add_fn(iter_next_fn, 1, &[mut_self], opt_item);
        ty_db.add_trait_method(iter_tr, 0, "next", iter_next_fn, 0);
        ty_db.add_assoc_rule(TyLangItem::IteratorItem, 1, ii_p0, &[], p0);
        ty_db.add_trait_assoc_rule(iter_tr, iter_item_assoc, 1, ii_p0, &[], p0);
        ty_db.add_trait_impl(iter_tr, 1, ii_p0, &[], &[]);

        ty_db.add_fn(iter_collect_fn, 2, &[p0], p1);
        ty_db.add_where_trait_bound(iter_collect_fn, p1, from_iter_tr, &[self_item]);
        ty_db.add_trait_method(iter_tr, 0, "collect", iter_collect_fn, 1);
        ty_db.add_trait_impl(from_iter_tr, 1, vec_p0, &[p0], &[]);

        ty_db.add_fn(from_fn, 2, &[p1], p0);
        ty_db.add_trait_method(from_tr, 1, "from", from_fn, 0);
        item_db.add_assoc_item(from_tr, odef(from_fn, "from", ItemKind::AssocFn, false));
        let arr2_p0 = ty_db.templates.mk_array(p0, 2);
        ty_db.add_fn(vec_from_arr_fn, 1, &[arr2_p0], vec_p0);
        ty_db.add_trait_impl(from_tr, 1, vec_p0, &[arr2_p0], &[("from", vec_from_arr_fn, 0)]);
        let vec_from_imp = gdef(std_c.0, 70);
        item_db.add_impl(IndexedImpl { id: vec_from_imp, file: FileId::NONE, target: ImplTargetKey::Adt(vec_def), trait_def: from_tr, is_negative: false });
        item_db.add_assoc_item(vec_from_imp, odef(vec_from_arr_fn, "from", ItemKind::AssocFn, false));

        ty_db.add_fn(def_fn, 1, &[], p0);
        ty_db.add_trait_method(default_tr, 0, "default", def_fn, 0);
        item_db.add_assoc_item(default_tr, odef(def_fn, "default", ItemKind::AssocFn, false));
        ty_db.add_fn(vec_def_fn, 1, &[], vec_p0);
        ty_db.add_trait_impl(default_tr, 1, vec_p0, &[], &[("default", vec_def_fn, 0)]);
        let vec_def_imp = gdef(std_c.0, 71);
        item_db.add_impl(IndexedImpl { id: vec_def_imp, file: FileId::NONE, target: ImplTargetKey::Adt(vec_def), trait_def: default_tr, is_negative: false });
        item_db.add_assoc_item(vec_def_imp, odef(vec_def_fn, "default", ItemKind::AssocFn, false));

        // External blanket impl `impl<T: CopyTag> MarkedExt for T`: `GoodExt` implements `CopyTag`, `BadExt` does not.
        ty_db.add_trait_impl(copy_tag_tr, 0, good_ty, &[], &[]);
        let ref_self = ty_db.templates.mk_ref(false, p0);
        ty_db.add_fn(ext_val_fn, 1, &[ref_self], InferTyId::U64);
        ty_db.add_trait_method(marked_ext_tr, 0, "ext_val", ext_val_fn, 0);
        ty_db.add_trait_impl_with_bounds(marked_ext_tr, 1, p0, &[], &[(p0, copy_tag_tr, &[])], &[]);

        Self { item_db, ty_db, krate, opt_def, vec_def, good_ext_def, bad_ext_def }
    }

    fn infer(&self, src: &str, fn_name: &str) -> (SyntaxTree<'static>, FileNameRes, LocalCstDefs, BodyInferCtx, FnInferResult) {
        let leaked: &'static str = Box::leak(src.to_string().into_boxed_str());
        let mut sources = MemSourceDb::new();
        let fid = sources.add_file("src/lib.rs", leaked);
        let mod_tree = ModuleTree::build(&self.item_db, &sources, self.krate, fid);
        let def_map = CrateDefMap::build(&self.item_db, &sources, &mod_tree);
        let st = parse(leaked);
        let file_res = def_map.resolve_file_paths(&self.item_db, &sources, &mod_tree, fid);
        let local_defs = LocalCstDefs::from_file(&st, Some(&def_map), fid);
        let fn_node = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == fn_name)).expect("fn not found");
        let mut ctx = BodyInferCtx::new();
        let mut res = FnInferResult::new();
        ctx.infer_fn_into(&st, &file_res, &self.ty_db, &local_defs, fn_node, &mut res);
        (st, file_res, local_defs, ctx, res)
    }
}

fn infer_pure_local(src: &str, fn_name: &str) -> (SyntaxTree<'static>, FileNameRes, BodyInferCtx, FnInferResult) {
    let leaked: &'static str = Box::leak(src.to_string().into_boxed_str());
    let mut item_db = MemItemDb::new();
    let krate = item_db.add_crate("local", Edition::Edition2021);
    item_db.set_crate_root_def(krate, gdef(krate.0, 0));
    let mut sources = MemSourceDb::new();
    let fid = sources.add_file("src/lib.rs", leaked);
    let mod_tree = ModuleTree::build(&item_db, &sources, krate, fid);
    let def_map = CrateDefMap::build(&item_db, &sources, &mod_tree);
    let st = parse(leaked);
    let file_res = def_map.resolve_file_paths(&item_db, &sources, &mod_tree, fid);
    let local_defs = LocalCstDefs::from_file(&st, Some(&def_map), fid);
    let fn_node = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == fn_name)).expect("fn not found");
    let mut ctx = BodyInferCtx::new();
    let mut res = FnInferResult::new();
    ctx.infer_fn_into(&st, &file_res, &EmptyTyDefDb, &local_defs, fn_node, &mut res);
    (st, file_res, ctx, res)
}

fn local_ty(st: &SyntaxTree<'_>, file_res: &FileNameRes, res: &FnInferResult, name: &str) -> InferTyId {
    let lid = file_res.locals.iter().enumerate().find(|(_, l)| st.text(l.decl_node) == name).map(|(i, _)| ridx_syntax::name_res::LocalBindId(i as u32)).expect("local not found");
    res.local_ty(lid).unwrap_or(InferTyId::UNKNOWN)
}

#[test]
fn impl_selection_by_self_ty_with_generic_args_and_collect() {
    let fx = TraitFixture::new();
    let src = r#"
        fn build(seed: Vec<u64>) -> Vec<u64> {
            let a = Vec::from([10u64, 20u64]);
            let b = <Vec<u32> as From<[u32; 2]>>::from([1, 2]);
            let c: Vec<bool> = Vec::default();
            let d = seed.into_iter().collect::<Vec<_>>();
            let _ = (b, c, d);
            a
        }
    "#;
    let (st, fres, _, ctx, res) = fx.infer(src, "build");
    for (vname, expected_elem) in [("a", PrimTy::U64), ("b", PrimTy::U32), ("c", PrimTy::Bool), ("d", PrimTy::U64)] {
        let ty = local_ty(&st, &fres, &res, vname);
        let InferTyKind::Adt { def, args } = ctx.table.arena.kind(ty) else {
            panic!("expected Vec Adt for `{vname}`, got {:?}", ctx.table.arena.kind(ty));
        };
        assert_eq!(def, fx.vec_def);
        assert_eq!(ctx.table.arena.kind(ctx.table.arena.slice(args)[0]), InferTyKind::Prim(expected_elem));
    }
}

#[test]
fn overloaded_local_trait_impl_selection_with_generic_args() {
    let src = r#"
        trait MyFrom<T> {
            fn my_from(val: T) -> Self;
        }
        struct NumBox(u32);
        impl MyFrom<u32> for NumBox {
            fn my_from(val: u32) -> Self { NumBox(val) }
        }
        impl MyFrom<&str> for NumBox {
            fn my_from(_val: &str) -> Self { NumBox(0) }
        }
        trait MyAdd<Rhs = Self> {
            type Output;
            fn my_add(self, rhs: Rhs) -> Self::Output;
        }
        struct Meter(u32);
        struct Area(u64);
        impl MyAdd<u32> for Meter {
            type Output = Meter;
            fn my_add(self, rhs: u32) -> Meter { Meter(self.0 + rhs) }
        }
        impl MyAdd<Meter> for Meter {
            type Output = Area;
            fn my_add(self, _rhs: Meter) -> Area { Area(1) }
        }
        fn run() {
            let x = NumBox::my_from(5u32);
            let y = NumBox::my_from("abc");
            let m1 = Meter(2).my_add(3u32);
            let a1 = Meter(2).my_add(Meter(4));
            let p_m: <Meter as MyAdd<u32>>::Output = Meter(9);
            let p_a: <Meter as MyAdd<Meter>>::Output = Area(9);
            let _ = (x, y, m1, a1, p_m, p_a);
        }
    "#;
    let (st, fres, ctx, res) = infer_pure_local(src, "run");
    let m1_ty = local_ty(&st, &fres, &res, "m1");
    let a1_ty = local_ty(&st, &fres, &res, "a1");
    let pm_ty = local_ty(&st, &fres, &res, "p_m");
    let pa_ty = local_ty(&st, &fres, &res, "p_a");
    assert_eq!(m1_ty, pm_ty, "`Meter.my_add(u32)` must match `<Meter as MyAdd<u32>>::Output` (`Meter`)");
    assert_eq!(a1_ty, pa_ty, "`Meter.my_add(Meter)` must match `<Meter as MyAdd<Meter>>::Output` (`Area`)");
    assert_ne!(m1_ty, a1_ty, "`Meter` and `Area` must resolve to distinct ADTs");
    assert_ne!(local_ty(&st, &fres, &res, "x"), InferTyId::UNKNOWN);
    assert_eq!(local_ty(&st, &fres, &res, "x"), local_ty(&st, &fres, &res, "y"));
    let _ = ctx;
}

#[test]
fn assoc_type_projection_concrete_param_where_and_supertrait() {
    let fx = TraitFixture::new();
    let src = r#"
        fn projections<I: Iterator<Item = u64>, E>(mut it: I, mut ex: E) -> Option<u64>
        where
            E: ExactIter<Item = u16>,
        {
            let concrete: <IntoIter<bool> as Iterator>::Item = true;
            let from_inline = it.next();
            let from_super = ex.next();
            let explicit_proj: Option<<I as Iterator>::Item> = from_inline;
            let _ = (concrete, from_super, explicit_proj);
            from_inline
        }
    "#;
    let (st, fres, _, ctx, res) = fx.infer(src, "projections");
    assert_eq!(local_ty(&st, &fres, &res, "concrete"), InferTyId::BOOL);
    for (vname, prim) in [("from_inline", PrimTy::U64), ("explicit_proj", PrimTy::U64), ("from_super", PrimTy::U16)] {
        let ty = local_ty(&st, &fres, &res, vname);
        let InferTyKind::Adt { def, args } = ctx.table.arena.kind(ty) else {
            panic!("expected Option Adt for `{vname}`, got {:?}", ctx.table.arena.kind(ty));
        };
        assert_eq!(def, fx.opt_def);
        assert_eq!(ctx.table.arena.kind(ctx.table.arena.slice(args)[0]), InferTyKind::Prim(prim));
    }
}

#[test]
fn blanket_impls_over_bounds_local_and_external_and_return_directed() {
    let fx = TraitFixture::new();
    let src = r#"
        trait Marker {}
        trait Alpha {
            fn pick(&self) -> u32 { 1 }
            fn only_marked(&self) -> u32 { 2 }
        }
        trait Beta {
            fn pick(&self) -> bool { true }
        }
        impl<T: Marker> Alpha for T {}
        struct Marked;
        impl Marker for Marked {}
        struct Unmarked;
        impl Beta for Unmarked {}

        trait MyFrom<T> {
            fn my_from(v: T) -> Self;
        }
        trait MyInto<U> {
            fn my_into(self) -> U;
        }
        impl<T, U: MyFrom<T>> MyInto<U> for T {
            fn my_into(self) -> U { U::my_from(self) }
        }
        struct Dest(u32);
        impl MyFrom<u32> for Dest {
            fn my_from(v: u32) -> Self { Dest(v) }
        }

        fn check_blankets() {
            let ok_local = Marked.only_marked();
            let bad_local = Unmarked.only_marked();
            let disamb_good = Marked.pick();
            let disamb_bad = Unmarked.pick();
            let ok_ext = GoodExt.ext_val();
            let bad_ext = BadExt.ext_val();
            let conv: Dest = 7u32.my_into();
            let _ = (ok_local, bad_local, disamb_good, disamb_bad, ok_ext, bad_ext, conv);
        }
    "#;
    let (st, fres, _, ctx, res) = fx.infer(src, "check_blankets");
    let _ = (fx.good_ext_def, fx.bad_ext_def, &ctx);
    assert_eq!(local_ty(&st, &fres, &res, "ok_local"), InferTyId::U32);
    assert_eq!(local_ty(&st, &fres, &res, "bad_local"), InferTyId::UNKNOWN, "unmarked receiver must NOT match `impl<T: Marker> Alpha for T`");
    assert_eq!(local_ty(&st, &fres, &res, "disamb_good"), InferTyId::U32);
    assert_eq!(local_ty(&st, &fres, &res, "disamb_bad"), InferTyId::BOOL, "unmarked receiver must skip rejected blanket `Alpha` and select `Beta`");
    assert_eq!(local_ty(&st, &fres, &res, "ok_ext"), InferTyId::U64);
    assert_eq!(local_ty(&st, &fres, &res, "bad_ext"), InferTyId::UNKNOWN, "external blanket bound must reject `BadExt`");
    assert_ne!(local_ty(&st, &fres, &res, "conv"), InferTyId::UNKNOWN);
}
