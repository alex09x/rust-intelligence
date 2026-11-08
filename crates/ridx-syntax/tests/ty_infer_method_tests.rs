/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for Phase 3 Checkbox `3.3` (`MethodCallExpr` + `.field` resolution):
//! inherent vs trait precedence, `xform_self_ty` (`&String::clone` vs `&&String::clone`),
//! multi-step `Deref`, raw-pointer non-autoderef, `[T; N]` unsizing vs by-value `IntoIterator`,
//! `Pin<&mut Self>`, turbofish `::<T>`, `Iterator::map` closure `Alias` normalization,
//! `T: SubTrait` supertraits, `dyn`/`impl Trait`, block-local `use Trait`, and `dot_refs`.

use ridx_syntax::name_res::{CrateDefMap, CrateId, Edition, FileNameRes, GlobalDefId, ImplTargetKey, ItemKind, MemItemDb, MemSourceDb, ModuleTree, OwnedDef, PrimTy, VisKind};
use ridx_syntax::syntax_kind::SyntaxKind as K;
use ridx_syntax::ty_infer::{BodyInferCtx, CtorShapeKind, FnInferResult, InferTyId, InferTyKind, LocalCstDefs, MemTyDefDb, TyLangItem};
use ridx_syntax::{parse, SyntaxTree};

struct MethodFixture {
    item_db: MemItemDb,
    ty_db: MemTyDefDb,
    krate: CrateId,
    opt_def: GlobalDefId,
    str_adt_def: GlobalDefId,
    vec_def: GlobalDefId,
    into_iter_adt: GlobalDefId,
    map_adt: GlobalDefId,
    str_clone_fn: GlobalDefId,
    ref_clone_fn: GlobalDefId,
    vec_len_fn: GlobalDefId,
    slice_first_fn: GlobalDefId,
    arr_into_iter_fn: GlobalDefId,
    ptr_is_null_fn: GlobalDefId,
    fut_poll_fn: GlobalDefId,
}

fn gdef(c: u32, i: u32) -> GlobalDefId {
    GlobalDefId::new(CrateId(c), i)
}

fn odef(id: GlobalDefId, name: &str, kind: ItemKind, ctor: bool) -> OwnedDef {
    OwnedDef { id, name: name.into(), kind, vis: VisKind::Public, has_value_ctor: ctor }
}

impl MethodFixture {
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
        let (box_def, str_adt_def, vec_def, pin_def) = (gdef(std_c.0, 13), gdef(std_c.0, 14), gdef(std_c.0, 15), gdef(std_c.0, 16));
        let (into_iter_adt, map_adt, cx_adt) = (gdef(std_c.0, 17), gdef(std_c.0, 18), gdef(std_c.0, 19));
        let (clone_tr, deref_tr, iter_tr, iter_item_assoc, into_iter_tr, fut_tr, fn_mut_tr) =
            (gdef(std_c.0, 20), gdef(std_c.0, 21), gdef(std_c.0, 22), gdef(std_c.0, 23), gdef(std_c.0, 24), gdef(std_c.0, 25), gdef(std_c.0, 26));

        for d in [
            odef(opt_def, "Option", ItemKind::Enum, false),
            odef(some_def, "Some", ItemKind::Variant, true),
            odef(none_def, "None", ItemKind::Variant, true),
            odef(box_def, "Box", ItemKind::Struct, false),
            odef(str_adt_def, "String", ItemKind::Struct, false),
            odef(vec_def, "Vec", ItemKind::Struct, false),
            odef(pin_def, "Pin", ItemKind::Struct, false),
            odef(cx_adt, "Context", ItemKind::Struct, false),
            odef(clone_tr, "Clone", ItemKind::Trait, false),
            odef(deref_tr, "Deref", ItemKind::Trait, false),
            odef(iter_tr, "Iterator", ItemKind::Trait, false),
            odef(into_iter_tr, "IntoIterator", ItemKind::Trait, false),
            odef(fut_tr, "Future", ItemKind::Trait, false),
            odef(fn_mut_tr, "FnMut", ItemKind::Trait, false),
        ] {
            item_db.add_module_def(prelude, d);
        }

        let mut ty_db = MemTyDefDb::new();
        ty_db.add_lang_item(TyLangItem::Option, opt_def);
        ty_db.add_lang_item(TyLangItem::OptionSome, some_def);
        ty_db.add_lang_item(TyLangItem::OptionNone, none_def);
        ty_db.add_lang_item(TyLangItem::OwnedBox, box_def);
        ty_db.add_lang_item(TyLangItem::String, str_adt_def);
        ty_db.add_lang_item(TyLangItem::Pin, pin_def);
        ty_db.add_lang_item(TyLangItem::Clone, clone_tr);
        ty_db.add_lang_item(TyLangItem::Deref, deref_tr);
        ty_db.add_lang_item(TyLangItem::Iterator, iter_tr);
        ty_db.add_lang_item(TyLangItem::IteratorItem, iter_item_assoc);
        ty_db.add_lang_item(TyLangItem::IntoIterator, into_iter_tr);
        ty_db.add_lang_item(TyLangItem::Future, fut_tr);
        ty_db.add_lang_item(TyLangItem::FnMut, fn_mut_tr);

        let (p0, p1, p2) = (ty_db.param(0), ty_db.param(1), ty_db.param(2));
        ty_db.add_adt(opt_def, 1, CtorShapeKind::None);
        ty_db.add_adt(box_def, 1, CtorShapeKind::None);
        let str_ty = ty_db.add_adt(str_adt_def, 0, CtorShapeKind::None);
        let vec_p0 = ty_db.add_adt(vec_def, 1, CtorShapeKind::None);
        let _ = ty_db.add_adt(pin_def, 1, CtorShapeKind::None);
        let into_iter_p0 = ty_db.add_adt(into_iter_adt, 1, CtorShapeKind::None);
        let _ = ty_db.add_adt(map_adt, 2, CtorShapeKind::None);
        let cx_ty = ty_db.add_adt(cx_adt, 0, CtorShapeKind::None);

        // Vec<T>: Deref<Target = [T]>, Vec<T>::len(&self) -> usize, [T]::first(&self) -> Option<&T>.
        let sl_p0 = ty_db.templates.mk_slice(p0);
        ty_db.add_assoc_rule(TyLangItem::DerefTarget, 1, vec_p0, &[], sl_p0);
        let vec_len_fn = gdef(std_c.0, 100);
        let ref_vec = ty_db.templates.mk_ref(false, vec_p0);
        ty_db.add_fn(vec_len_fn, 1, &[ref_vec], InferTyId::USIZE);
        ty_db.add_inherent_method(ImplTargetKey::Adt(vec_def), "len", vec_len_fn, 1, 0);

        let slice_len_fn = gdef(std_c.0, 101);
        let ref_sl = ty_db.templates.mk_ref(false, sl_p0);
        ty_db.add_fn(slice_len_fn, 1, &[ref_sl], InferTyId::USIZE);
        ty_db.add_inherent_method(ImplTargetKey::Slice, "len", slice_len_fn, 1, 0);

        let slice_first_fn = gdef(std_c.0, 102);
        let ref_p0 = ty_db.templates.mk_ref(false, p0);
        let opt_ref_p0 = ty_db.templates.mk_adt(opt_def, &[ref_p0]);
        ty_db.add_fn(slice_first_fn, 1, &[ref_sl], opt_ref_p0);
        ty_db.add_inherent_method(ImplTargetKey::Slice, "first", slice_first_fn, 1, 0);

        // Clone::clone(&self) -> Self; impl Clone for String + impl<T> Clone for &T.
        let clone_decl_fn = gdef(std_c.0, 110);
        ty_db.add_fn(clone_decl_fn, 1, &[ref_p0], p0);
        ty_db.add_trait_method(clone_tr, 0, "clone", clone_decl_fn, 0);
        let str_clone_fn = gdef(std_c.0, 111);
        let ref_str_adt = ty_db.templates.mk_ref(false, str_ty);
        ty_db.add_fn(str_clone_fn, 0, &[ref_str_adt], str_ty);
        ty_db.add_trait_impl(clone_tr, 0, str_ty, &[], &[("clone", str_clone_fn, 0)]);
        let ref_clone_fn = gdef(std_c.0, 112);
        let ref_ref_p0 = ty_db.templates.mk_ref(false, ref_p0);
        ty_db.add_fn(ref_clone_fn, 1, &[ref_ref_p0], ref_p0);
        ty_db.add_trait_impl(clone_tr, 1, ref_p0, &[], &[("clone", ref_clone_fn, 0)]);

        // str::is_ascii(&self) -> bool (reached via String -> Deref<Target = str>).
        let str_ascii_fn = gdef(std_c.0, 113);
        ty_db.add_fn(str_ascii_fn, 0, &[InferTyId::STR_REF], InferTyId::BOOL);
        ty_db.add_inherent_method(ImplTargetKey::Prim(PrimTy::Str), "is_ascii", str_ascii_fn, 0, 0);

        // IntoIterator::into_iter(self) -> IntoIter; impl<T> IntoIterator for Vec<T> + impl<T> IntoIterator for [T; N].
        let into_iter_decl = gdef(std_c.0, 120);
        ty_db.add_fn(into_iter_decl, 1, &[p0], p0);
        ty_db.add_trait_method(into_iter_tr, 0, "into_iter", into_iter_decl, 0);
        let vec_into_iter_fn = gdef(std_c.0, 121);
        ty_db.add_fn(vec_into_iter_fn, 1, &[vec_p0], into_iter_p0);
        ty_db.add_trait_impl(into_iter_tr, 1, vec_p0, &[], &[("into_iter", vec_into_iter_fn, 0)]);
        let arr_into_iter_fn = gdef(std_c.0, 122);
        let arr_p0 = ty_db.templates.mk_array(p0, 3);
        ty_db.add_fn(arr_into_iter_fn, 1, &[arr_p0], into_iter_p0);
        ty_db.add_trait_impl(into_iter_tr, 1, arr_p0, &[], &[("into_iter", arr_into_iter_fn, 0)]);

        // Iterator default method: fn map<B, F: FnMut(Self::Item) -> B>(self, f: F) -> Map<Self, F>.
        let iter_map_decl = gdef(std_c.0, 125);
        let self_item = ty_db.templates.mk_alias(iter_tr, iter_item_assoc, p0, &[]);
        let map_ret = ty_db.templates.mk_adt(map_adt, &[p0, p2]);
        ty_db.add_fn(iter_map_decl, 3, &[p0, p2], map_ret);
        ty_db.add_param_fn_bound(iter_map_decl, 2, TyLangItem::FnMut, &[self_item], p1);
        ty_db.add_trait_method(iter_tr, 0, "map", iter_map_decl, 2);
        ty_db.add_assoc_rule(TyLangItem::IteratorItem, 1, into_iter_p0, &[], p0);
        ty_db.add_trait_impl(iter_tr, 1, into_iter_p0, &[], &[]);

        // *const T::is_null(self) -> bool.
        let ptr_is_null_fn = gdef(std_c.0, 130);
        let const_ptr_p0 = ty_db.templates.mk_ptr(false, p0);
        ty_db.add_fn(ptr_is_null_fn, 1, &[const_ptr_p0], InferTyId::BOOL);
        ty_db.add_inherent_method(ImplTargetKey::Ptr { is_mut: false }, "is_null", ptr_is_null_fn, 1, 0);

        // Future::poll(self: Pin<&mut Self>, cx: &mut Context) -> Option<Self::Output>.
        let fut_poll_fn = gdef(std_c.0, 140);
        let mut_self = ty_db.templates.mk_ref(true, p0);
        let pin_mut_self = ty_db.templates.mk_adt(pin_def, &[mut_self]);
        let mut_cx = ty_db.templates.mk_ref(true, cx_ty);
        ty_db.add_fn(fut_poll_fn, 1, &[pin_mut_self, mut_cx], InferTyId::BOOL);
        ty_db.add_trait_method(fut_tr, 0, "poll", fut_poll_fn, 0);

        Self {
            item_db,
            ty_db,
            krate,
            opt_def,
            str_adt_def,
            vec_def,
            into_iter_adt,
            map_adt,
            str_clone_fn,
            ref_clone_fn,
            vec_len_fn,
            slice_first_fn,
            arr_into_iter_fn,
            ptr_is_null_fn,
            fut_poll_fn,
        }
    }

    fn infer_fn<'s>(&self, src: &'s str, fn_name: &str) -> (SyntaxTree<'s>, FileNameRes, BodyInferCtx, FnInferResult) {
        let mut sources = MemSourceDb::new();
        let file = sources.add_file("src/lib.rs", src);
        let mod_tree = ModuleTree::build(&self.item_db, &sources, self.krate, file);
        let def_map = CrateDefMap::build(&self.item_db, &sources, &mod_tree);
        let st = parse(src);
        let file_res = def_map.resolve_file_paths(&self.item_db, &sources, &mod_tree, file);
        let local_defs = LocalCstDefs::from_file(&st, Some(&def_map), file);
        let fn_node = st.descendants(st.root()).find(|&n| st.kind(n) == K::Fn && st.child(n, K::Name).is_some_and(|nn| st.text(nn) == fn_name)).expect("fn node");
        let mut ctx = BodyInferCtx::new();
        let res = ctx.infer_fn(&st, &file_res, &self.ty_db, &local_defs, fn_node);
        (st, file_res, ctx, res)
    }
}

fn local_ty_by_name(st: &SyntaxTree<'_>, file_res: &FileNameRes, res: &FnInferResult, name: &str) -> InferTyId {
    let lid = file_res.locals.iter().find(|l| l.name(st.src) == name).expect("local").id;
    res.local_ty(lid).expect("local ty")
}

#[test]
fn clone_on_ref_string_vs_double_ref_string_uses_xform_self_ty() {
    let fx = MethodFixture::new();
    let src = r#"
        fn check(s1: &String, s2: &&String) {
            let owned = s1.clone();
            let borrowed = s2.clone();
            let ascii = s1.is_ascii();
        }
    "#;
    let (st, fres, ctx, res) = fx.infer_fn(src, "check");
    let owned = local_ty_by_name(&st, &fres, &res, "owned");
    let borrowed = local_ty_by_name(&st, &fres, &res, "borrowed");
    let ascii = local_ty_by_name(&st, &fres, &res, "ascii");
    assert!(matches!(ctx.table.arena.kind(owned), InferTyKind::Adt { def, .. } if def == fx.str_adt_def));
    assert!(matches!(ctx.table.arena.kind(borrowed), InferTyKind::Ref { is_mut: false, inner }
        if matches!(ctx.table.arena.kind(inner), InferTyKind::Adt { def, .. } if def == fx.str_adt_def)));
    assert_eq!(ascii, InferTyId::BOOL);
    assert!(res.dot_refs.iter().any(|d| d.def == fx.str_clone_fn));
    assert!(res.dot_refs.iter().any(|d| d.def == fx.ref_clone_fn));
}

#[test]
fn autoderef_step_order_vec_len_vs_slice_first_and_array_into_iter() {
    let fx = MethodFixture::new();
    let src = r#"
        fn check(v: &mut Vec<u16>) {
            let n = v.len();
            let f = v.first();
            let arr = [1u32, 2, 3];
            let it = arr.into_iter();
            let af = arr.first();
        }
    "#;
    let (st, fres, ctx, res) = fx.infer_fn(src, "check");
    assert_eq!(local_ty_by_name(&st, &fres, &res, "n"), InferTyId::USIZE);
    // `v.len()` must pick `Vec::len` at step 1 (`Vec<u16>`), NOT `slice::len` at step 2 (`[u16]`)!
    assert!(res.dot_refs.iter().any(|d| d.def == fx.vec_len_fn));
    assert!(res.dot_refs.iter().any(|d| d.def == fx.slice_first_fn));
    assert!(res.dot_refs.iter().any(|d| d.def == fx.arr_into_iter_fn));

    let f_ty = local_ty_by_name(&st, &fres, &res, "f");
    assert!(matches!(ctx.table.arena.kind(f_ty), InferTyKind::Adt { def, args }
        if def == fx.opt_def && matches!(ctx.table.arena.kind(ctx.table.arena.slice(args)[0]), InferTyKind::Ref { inner: InferTyId::U16, .. })));

    let it_ty = local_ty_by_name(&st, &fres, &res, "it");
    assert!(matches!(ctx.table.arena.kind(it_ty), InferTyKind::Adt { def, args } if def == fx.into_iter_adt && ctx.table.arena.slice(args) == [InferTyId::U32]));
    let af_ty = local_ty_by_name(&st, &fres, &res, "af");
    assert!(matches!(ctx.table.arena.kind(af_ty), InferTyKind::Adt { def, .. } if def == fx.opt_def));
}

#[test]
fn iterator_default_map_method_normalizes_self_item_for_closure() {
    let fx = MethodFixture::new();
    let src = r#"
        fn check(v: Vec<i64>) {
            let mapped = v.into_iter().map(|elem| elem + 1);
        }
    "#;
    let (st, fres, ctx, res) = fx.infer_fn(src, "check");
    assert_eq!(local_ty_by_name(&st, &fres, &res, "elem"), InferTyId::I64);
    let mapped = local_ty_by_name(&st, &fres, &res, "mapped");
    assert!(matches!(ctx.table.arena.kind(mapped), InferTyKind::Adt { def, .. } if def == fx.map_adt));
}

#[test]
fn raw_pointer_methods_work_without_autodereferencing_pointee() {
    let fx = MethodFixture::new();
    let src = r#"
        struct Pointee { val: i32 }
        impl Pointee {
            fn get_val(&self) -> i32 { self.val }
        }
        fn check(p: *const Pointee) {
            let null_flag = p.is_null();
            let forbidden_deref = p.get_val();
            let forbidden_field = p.val;
        }
    "#;
    let (st, fres, _ctx, res) = fx.infer_fn(src, "check");
    assert_eq!(local_ty_by_name(&st, &fres, &res, "null_flag"), InferTyId::BOOL);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "forbidden_deref"), InferTyId::UNKNOWN);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "forbidden_field"), InferTyId::UNKNOWN);
    assert!(res.dot_refs.iter().any(|d| d.def == fx.ptr_is_null_fn));
}

#[test]
fn inherent_precedence_turbofish_local_deref_and_block_scoped_trait_use() {
    let fx = MethodFixture::new();
    let src = r#"
        mod ext {
            pub trait ExtNum {
                fn tag(&self) -> bool;
                fn only_ext(&self) -> u64;
            }
        }
        struct Holder<T> { inner: T }
        struct Smart<U>(Holder<U>);
        impl<U> Deref for Smart<U> {
            type Target = Holder<U>;
        }
        impl<T> Holder<T> {
            fn tag(&self) -> i32 { 7 }
            fn cast_pair<V>(&self, v: V) -> (i32, V) { (1, v) }
        }
        impl<T> ext::ExtNum for Holder<T> {
            fn tag(&self) -> bool { false }
            fn only_ext(&self) -> u64 { 99 }
        }
        fn check(sm: &Box<Smart<u8>>) {
            let f_val = sm.inner;
            let outside = sm.only_ext();
            let inside = {
                use ext::ExtNum;
                let inh_wins = sm.tag();
                let turbo = sm.cast_pair::<f32>(1.5);
                let tr_hit = sm.only_ext();
                (inh_wins, turbo, tr_hit)
            };
        }
    "#;
    let (st, fres, ctx, res) = fx.infer_fn(src, "check");
    assert_eq!(local_ty_by_name(&st, &fres, &res, "f_val"), InferTyId::U8);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "outside"), InferTyId::UNKNOWN);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "inh_wins"), InferTyId::I32);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "tr_hit"), InferTyId::U64);
    let turbo = local_ty_by_name(&st, &fres, &res, "turbo");
    assert!(matches!(ctx.table.arena.kind(turbo), InferTyKind::Tuple(r) if ctx.table.arena.slice(r) == [InferTyId::I32, InferTyId::F32]));
    // Both `.inner` (Field) and `.tag()` / `.cast_pair()` / `.only_ext()` (AssocFn) recorded in `dot_refs`.
    assert!(res.dot_refs.iter().any(|d| d.kind == ItemKind::Field));
    assert!(res.dot_refs.iter().filter(|d| d.kind == ItemKind::AssocFn).count() >= 3);
}

#[test]
fn param_supertrait_dyn_opaque_and_pin_mut_self_receivers() {
    let fx = MethodFixture::new();
    let src = r#"
        mod hidden {
            pub trait Base { fn base_id(&self) -> u16; }
            pub trait Sub: Base { fn sub_flag(&self) -> bool; }
        }
        fn make_opaque() -> impl hidden::Sub { loop {} }
        fn check<T: hidden::Sub, F: Future>(t: &T, d: &dyn hidden::Sub, f: Pin<&mut F>, cx: &mut Context) {
            let (a, b, c) = (t.sub_flag(), t.base_id(), d.base_id());
            let (op, polled) = (make_opaque().sub_flag(), f.poll(cx));
        }
    "#;
    let (st, fres, _ctx, res) = fx.infer_fn(src, "check");
    assert_eq!(local_ty_by_name(&st, &fres, &res, "a"), InferTyId::BOOL);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "b"), InferTyId::U16);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "c"), InferTyId::U16);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "op"), InferTyId::BOOL);
    assert_eq!(local_ty_by_name(&st, &fres, &res, "polled"), InferTyId::BOOL);
    assert!(res.dot_refs.iter().any(|d| d.def == fx.fut_poll_fn));
    let _ = fx.vec_def;
}

#[test]
fn recursive_assoc_ty_impl_on_generic_adt_terminates_cleanly() {
    let fx = MethodFixture::new();
    let src = r#"
        trait FromStreamPriv<T> { type InternalCollection; fn init() -> Self::InternalCollection; }
        struct Collect<T, U, C> { stream: T, collection: C, _u: Option<U> }
        impl<T, U, E> FromStreamPriv<Result<T, E>> for Result<U, E> where U: FromStreamPriv<T> {
            type InternalCollection = Result<U::InternalCollection, E>;
            fn init() -> Self::InternalCollection { loop {} }
        }
        fn make_collect<T, U: FromStreamPriv<T>>(stream: T) -> Collect<T, U, U::InternalCollection> {
            let collection = U::init();
            Collect { stream, collection, _u: None }
        }
    "#;
    let (st, fres, _ctx, res) = fx.infer_fn(src, "make_collect");
    assert_ne!(local_ty_by_name(&st, &fres, &res, "collection"), InferTyId::NEVER);
}
