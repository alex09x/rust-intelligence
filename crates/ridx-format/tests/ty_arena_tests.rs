/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Roundtrip, structural deduplication, and backward-compatibility tests for `TyArena` (`.ridx` v5).

use std::io::Cursor;

use ridx_format::{
    ConstArg, FloatTyWidth, FnAbiKind, GenericArgView, Header, HeaderV2, IndexReader, IndexWriter, IntTyWidth, RawGenericArg, RawTy, RawTyBound, RawTyDef, TyDefId, TyId,
    TyKindView, TyMutability, UintTyWidth, HEADER_V3_SIZE, HEADER_V4_SIZE, LIFETIME_ELIDED, LIFETIME_STATIC, MAGIC,
};

#[repr(C, align(8))]
struct AlignedBuf([u8; 4096]);

fn write_and_open(writer: &mut IndexWriter) -> Vec<u8> {
    let mut cur = Cursor::new(Vec::new());
    writer.write_to(&mut cur).expect("write_to failed");
    cur.into_inner()
}

fn copy_aligned(src: &[u8]) -> Box<AlignedBuf> {
    assert!(src.len() <= 4096);
    let mut b = Box::new(AlignedBuf([0u8; 4096]));
    b.0[..src.len()].copy_from_slice(src);
    b
}

#[test]
fn test_ty_arena_all_kinds_roundtrip() {
    let mut w = IndexWriter::new("demo", "0.1.0");

    let vec_def = w.intern_ty_def(RawTyDef::External { path: "alloc::vec::Vec", unit_key: Some("alloc-sysroot") });
    let map_def = w.intern_ty_def(RawTyDef::External { path: "std::collections::HashMap", unit_key: Some("std-sysroot") });
    let widget_def = w.intern_ty_def(RawTyDef::Local { path: "demo::Widget", local_def_idx: 7 });
    let fn_item_def = w.intern_ty_def(RawTyDef::Local { path: "demo::make_widget", local_def_idx: 8 });
    let iter_trait = w.intern_ty_def(RawTyDef::External { path: "core::iter::Iterator", unit_key: None });
    let index_trait = w.intern_ty_def(RawTyDef::External { path: "core::ops::Index", unit_key: None });
    let send_trait = w.intern_ty_def(RawTyDef::External { path: "core::marker::Send", unit_key: None });
    let sized_trait = w.intern_ty_def(RawTyDef::External { path: "core::marker::Sized", unit_key: None });
    let fn_trait = w.intern_ty_def(RawTyDef::External { path: "core::ops::Fn", unit_key: None });

    let t_unit = w.intern_ty(RawTy::Unit);
    let t_empty_tuple = w.intern_ty(RawTy::Tuple(&[]));
    assert_eq!(t_unit, t_empty_tuple, "empty tuple canonicalizes to Unit");

    let t_bool = w.intern_ty(RawTy::Bool);
    let t_char = w.intern_ty(RawTy::Char);
    let t_str = w.intern_ty(RawTy::Str);
    let t_never = w.intern_ty(RawTy::Never);
    let t_unknown = w.intern_ty(RawTy::Unknown);

    let ints = [IntTyWidth::Isize, IntTyWidth::I8, IntTyWidth::I16, IntTyWidth::I32, IntTyWidth::I64, IntTyWidth::I128].map(|iw| (iw, w.intern_ty(RawTy::Int(iw))));
    let uints = [UintTyWidth::Usize, UintTyWidth::U8, UintTyWidth::U16, UintTyWidth::U32, UintTyWidth::U64, UintTyWidth::U128].map(|uw| (uw, w.intern_ty(RawTy::Uint(uw))));
    let floats = [FloatTyWidth::F16, FloatTyWidth::F32, FloatTyWidth::F64, FloatTyWidth::F128].map(|fw| (fw, w.intern_ty(RawTy::Float(fw))));

    let t_u8 = uints[1].1;
    let t_usize = uints[0].1;
    let t_i32 = ints[3].1;

    let t_param_t = w.intern_ty(RawTy::Param { index: 0, name: "T" });
    let t_param_anon = w.intern_ty(RawTy::Param { index: 1, name: "" });

    let t_ref_str = w.intern_ty(RawTy::Ref { mutability: TyMutability::Shared, inner: t_str, lifetime: LIFETIME_STATIC });
    let t_mut_u8 = w.intern_ty(RawTy::Ref { mutability: TyMutability::Mut, inner: t_u8, lifetime: 1 });
    let t_const_ptr = w.intern_ty(RawTy::Ptr { mutability: TyMutability::Shared, inner: t_i32 });
    let t_mut_ptr = w.intern_ty(RawTy::Ptr { mutability: TyMutability::Mut, inner: t_u8 });
    let t_slice = w.intern_ty(RawTy::Slice(t_u8));
    let t_arr_val = w.intern_ty(RawTy::ArrayVal { elem: t_u8, len: 32 });
    let t_arr_param = w.intern_ty(RawTy::ArrayConstParam { elem: t_i32, param_idx: 2 });
    let t_arr_expr = w.intern_ty(RawTy::ArrayConstExpr { elem: t_bool, expr: "N + 1" });

    let t_tuple1 = w.intern_ty(RawTy::Tuple(&[t_i32]));
    let t_tuple3 = w.intern_ty(RawTy::Tuple(&[t_ref_str, t_usize, t_bool]));

    let t_vec_param = w.intern_ty(RawTy::Adt { def: vec_def, args: &[RawGenericArg::Type(t_param_t)] });
    let t_widget = w.intern_ty(RawTy::Adt { def: widget_def, args: &[RawGenericArg::Lifetime(1), RawGenericArg::Type(t_vec_param), RawGenericArg::ConstValue(16)] });
    let t_map = w.intern_ty(RawTy::Adt { def: map_def, args: &[RawGenericArg::Type(t_ref_str), RawGenericArg::Type(t_widget)] });

    let t_fn_ptr = w.intern_ty(RawTy::FnPtr { inputs: &[t_const_ptr, t_usize], output: t_i32, abi: FnAbiKind::C, is_unsafe: true, is_variadic: true });
    let t_fn_def = w.intern_ty(RawTy::FnDef { def: fn_item_def, args: &[RawGenericArg::Type(t_u8)] });

    let t_assoc_iter = w.intern_ty(RawTy::Assoc { trait_def: iter_trait, name: "Item", self_ty: t_param_t, trait_args: &[] });
    let t_assoc_idx = w.intern_ty(RawTy::Assoc { trait_def: index_trait, name: "Output", self_ty: t_param_t, trait_args: &[RawGenericArg::Type(t_usize)] });
    let t_assoc_inherent = w.intern_ty(RawTy::Assoc { trait_def: TyDefId::NONE, name: "Assoc", self_ty: t_param_t, trait_args: &[] });

    let dyn_bounds = [
        RawTyBound { trait_def: iter_trait, args: &[], assocs: &[("Item", t_u8)], is_maybe: false },
        RawTyBound { trait_def: send_trait, args: &[], assocs: &[], is_maybe: false },
        RawTyBound { trait_def: sized_trait, args: &[], assocs: &[], is_maybe: true },
    ];
    let t_dyn = w.intern_ty(RawTy::Dyn { bounds: &dyn_bounds, lifetime: LIFETIME_STATIC });

    let fn_args_tuple = w.intern_ty(RawTy::Tuple(&[t_i32, t_ref_str]));
    let impl_bounds = [
        RawTyBound { trait_def: fn_trait, args: &[RawGenericArg::Type(fn_args_tuple)], assocs: &[("Output", t_bool)], is_maybe: false },
        RawTyBound { trait_def: send_trait, args: &[], assocs: &[], is_maybe: false },
    ];
    let t_impl = w.intern_ty(RawTy::ImplTrait { bounds: &impl_bounds });

    let raw_bytes = write_and_open(&mut w);
    let aligned = copy_aligned(&raw_bytes);
    let reader = IndexReader::from_buffer(&aligned.0[..raw_bytes.len()]).expect("open v5 reader");
    let arena = reader.ty_arena();
    assert!(!arena.is_empty());

    assert_eq!(arena.display_ty(t_unit), "()");
    assert_eq!(arena.display_ty(t_bool), "bool");
    assert_eq!(arena.display_ty(t_char), "char");
    assert_eq!(arena.display_ty(t_str), "str");
    assert_eq!(arena.display_ty(t_never), "!");
    assert_eq!(arena.display_ty(t_unknown), "{unknown}");

    for (iw, id) in ints {
        assert_eq!(arena.display_ty(id), iw.as_str());
    }
    for (uw, id) in uints {
        assert_eq!(arena.display_ty(id), uw.as_str());
    }
    for (fw, id) in floats {
        assert_eq!(arena.display_ty(id), fw.as_str());
    }

    assert_eq!(arena.display_ty(t_param_t), "T");
    assert_eq!(arena.display_ty(t_param_anon), "T#1");
    assert_eq!(arena.display_ty(t_ref_str), "&'static str");
    assert_eq!(arena.display_ty(t_mut_u8), "&'l0 mut u8");
    assert_eq!(arena.display_ty(t_const_ptr), "*const i32");
    assert_eq!(arena.display_ty(t_mut_ptr), "*mut u8");
    assert_eq!(arena.display_ty(t_slice), "[u8]");
    assert_eq!(arena.display_ty(t_arr_val), "[u8; 32]");
    assert_eq!(arena.display_ty(t_arr_param), "[i32; C#2]");
    assert_eq!(arena.display_ty(t_arr_expr), "[bool; N + 1]");
    assert_eq!(arena.display_ty(t_tuple1), "(i32,)");
    assert_eq!(arena.display_ty(t_tuple3), "(&'static str, usize, bool)");
    assert_eq!(arena.display_ty(t_map), "HashMap<&'static str, Widget<'l0, Vec<T>, 16>>");
    assert_eq!(arena.display_ty(t_fn_ptr), "unsafe extern \"C\" fn(*const i32, usize, ...) -> i32");
    assert_eq!(arena.display_ty(t_fn_def), "make_widget<u8>");
    assert_eq!(arena.display_ty(t_assoc_iter), "<T as Iterator>::Item");
    assert_eq!(arena.display_ty(t_assoc_idx), "<T as Index<usize>>::Output");
    assert_eq!(arena.display_ty(t_assoc_inherent), "T::Assoc");
    assert_eq!(arena.display_ty(t_dyn), "dyn Iterator<Item = u8> + Send + ?Sized + 'static");
    assert_eq!(arena.display_ty(t_impl), "impl Fn<(i32, &'static str), Output = bool> + Send");

    // Verify local vs external TyDefView provenance
    let w_view = arena.def(widget_def).unwrap();
    assert_eq!(w_view.path, "demo::Widget");
    assert_eq!(w_view.local_def_idx, Some(7));
    assert_eq!(w_view.extern_unit, None);

    let v_view = arena.def(vec_def).unwrap();
    assert_eq!(v_view.path, "alloc::vec::Vec");
    assert_eq!(v_view.local_def_idx, None);
    assert_eq!(v_view.extern_unit, Some("alloc-sysroot"));

    // Verify zero-alloc GenericArgIter & ConstArg decoding
    match arena.kind(t_arr_param) {
        TyKindView::Array { elem, len } => {
            assert_eq!(elem, t_i32);
            assert_eq!(len, ConstArg::Param(2));
        }
        other => panic!("unexpected kind: {other:?}"),
    }
    match arena.kind(t_widget) {
        TyKindView::Adt { def, mut args } => {
            assert_eq!(def, widget_def);
            assert_eq!(args.len(), 3);
            assert_eq!(args.next(), Some(GenericArgView::Lifetime(1)));
            assert_eq!(args.next(), Some(GenericArgView::Type(t_vec_param)));
            assert_eq!(args.next(), Some(GenericArgView::Const(ConstArg::Value(16))));
            assert_eq!(args.next(), None);
        }
        other => panic!("unexpected kind: {other:?}"),
    }

    // Out-of-bounds TyId safety
    assert!(arena.get(TyId::NONE).is_none());
    assert!(matches!(arena.kind(TyId(999_999)), TyKindView::Unknown));
    assert_eq!(arena.display_ty(TyId(999_999)), "{unknown}");
}

#[test]
fn test_ty_arena_structural_deduplication_and_arg_sharing() {
    let mut w = IndexWriter::new("dedup", "0.1.0");
    let vec_def = w.intern_ty_def(RawTyDef::External { path: "alloc::vec::Vec", unit_key: None });
    let vec_def_dup = w.intern_ty_def(RawTyDef::External { path: "alloc::vec::Vec", unit_key: None });
    assert_eq!(vec_def, vec_def_dup);

    let opt_def = w.intern_ty_def(RawTyDef::External { path: "core::option::Option", unit_key: None });
    let box_def = w.intern_ty_def(RawTyDef::External { path: "alloc::boxed::Box", unit_key: None });

    let t_u32 = w.intern_ty(RawTy::Uint(UintTyWidth::U32));
    let t_ref = w.intern_ty(RawTy::Ref { mutability: TyMutability::Shared, inner: t_u32, lifetime: LIFETIME_ELIDED });

    // Vec<&u32>, Option<&u32>, Box<&u32> all share the single 1-word arg slot `[Type(t_ref)]`!
    let t_vec1 = w.intern_ty(RawTy::Adt { def: vec_def, args: &[RawGenericArg::Type(t_ref)] });
    let args_after_first = w.ty_interner().args().len();
    assert_eq!(args_after_first, 1);

    let t_opt = w.intern_ty(RawTy::Adt { def: opt_def, args: &[RawGenericArg::Type(t_ref)] });
    let t_box = w.intern_ty(RawTy::Adt { def: box_def, args: &[RawGenericArg::Type(t_ref)] });
    assert_eq!(w.ty_interner().args().len(), 1, "1-arg generic instantiations of the same T share one u32 slot");
    assert_ne!(t_vec1, t_opt);
    assert_ne!(t_opt, t_box);

    let entries_before = w.ty_entry_count();
    let t_vec2 = w.intern_ty(RawTy::Adt { def: vec_def, args: &[RawGenericArg::Type(t_ref)] });
    assert_eq!(t_vec1, t_vec2, "re-interning identical nested type yields identical TyId");
    assert_eq!(w.ty_entry_count(), entries_before, "no duplicate TyEntry allocated");
}

#[test]
fn test_backward_compat_v2_v3_v4_headers_have_empty_ty_arena() {
    // v4 header (176 bytes + 8-byte string pool)
    let mut v4_hdr = Header {
        magic: MAGIC,
        version: 4,
        string_pool_offset: HEADER_V4_SIZE as u64,
        string_pool_len: 8,
        files_offset: (HEADER_V4_SIZE + 8) as u64,
        defs_offset: (HEADER_V4_SIZE + 8) as u64,
        defs_by_hash_offset: (HEADER_V4_SIZE + 8) as u64,
        refs_offset: (HEADER_V4_SIZE + 8) as u64,
        refs_by_target_offset: (HEADER_V4_SIZE + 8) as u64,
        types_offset: (HEADER_V4_SIZE + 8) as u64,
        defs_by_owner_offset: (HEADER_V4_SIZE + 8) as u64,
        derefs_offset: (HEADER_V4_SIZE + 8) as u64,
        notes_offset: (HEADER_V4_SIZE + 8) as u64,
        ..Header::default()
    }
    .to_le();
    let hdr_bytes: &[u8] = unsafe { std::slice::from_raw_parts((&v4_hdr as *const Header) as *const u8, HEADER_V4_SIZE) };
    let mut raw_v4 = Vec::from(hdr_bytes);
    raw_v4.extend_from_slice(&[0u8; 8]);
    let aligned_v4 = copy_aligned(&raw_v4);
    let r4 = IndexReader::from_buffer(&aligned_v4.0[..raw_v4.len()]).expect("open v4");
    assert!(r4.ty_arena().is_empty());

    // v3 header (160 bytes + 8-byte string pool)
    v4_hdr.version = 3u32.to_le();
    v4_hdr.string_pool_offset = (HEADER_V3_SIZE as u64).to_le();
    let off3 = ((HEADER_V3_SIZE + 8) as u64).to_le();
    v4_hdr.files_offset = off3;
    v4_hdr.defs_offset = off3;
    v4_hdr.defs_by_hash_offset = off3;
    v4_hdr.refs_offset = off3;
    v4_hdr.refs_by_target_offset = off3;
    v4_hdr.types_offset = off3;
    v4_hdr.defs_by_owner_offset = off3;
    v4_hdr.derefs_offset = off3;
    let hdr3_bytes: &[u8] = unsafe { std::slice::from_raw_parts((&v4_hdr as *const Header) as *const u8, HEADER_V3_SIZE) };
    let mut raw_v3 = Vec::from(hdr3_bytes);
    raw_v3.extend_from_slice(&[0u8; 8]);
    let aligned_v3 = copy_aligned(&raw_v3);
    let r3 = IndexReader::from_buffer(&aligned_v3.0[..raw_v3.len()]).expect("open v3");
    assert!(r3.ty_arena().is_empty());

    // v2 header (128 bytes + 8-byte string pool)
    let v2_hdr = HeaderV2 {
        magic: MAGIC,
        version: 2,
        crate_name_offset: 0,
        crate_version_offset: 0,
        string_pool_offset: 128,
        string_pool_len: 8,
        files_offset: 136,
        files_count: 0,
        unit_key_offset: 0,
        defs_offset: 136,
        defs_count: 0,
        generation_offset: 0,
        defs_by_hash_offset: 136,
        defs_by_hash_count: 0,
        _pad2: 0,
        refs_offset: 136,
        refs_count: 0,
        _pad3: 0,
        refs_by_target_offset: 136,
        refs_by_target_count: 0,
        _pad4: 0,
        types_offset: 136,
        types_count: 0,
        _pad5: 0,
    }
    .to_le();
    let hdr2_bytes: &[u8] = unsafe { std::slice::from_raw_parts((&v2_hdr as *const HeaderV2) as *const u8, 128) };
    let mut raw_v2 = Vec::from(hdr2_bytes);
    raw_v2.extend_from_slice(&[0u8; 8]);
    let aligned_v2 = copy_aligned(&raw_v2);
    let r2 = IndexReader::from_buffer(&aligned_v2.0[..raw_v2.len()]).expect("open v2");
    assert!(r2.ty_arena().is_empty());
    assert!(r2.fn_sigs().is_empty());
}

#[test]
fn test_fn_sigs_roundtrip_and_binary_search() {
    use ridx_format::{DefKind, RawDefinition, RawFnSig, FN_FLAG_ASYNC, FN_FLAG_CONST, FN_FLAG_HAS_SELF, FN_FLAG_UNSAFE, FN_FLAG_VARIADIC};

    let mut w = IndexWriter::new("fnsig_demo", "0.1.0");
    let mk_def = |id: u32, kind: DefKind, path: &str| RawDefinition {
        def_id: id,
        kind,
        visibility: 0,
        is_macro_expansion: false,
        name: path.rsplit("::").next().unwrap().to_string(),
        path: path.to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: id,
        start_col: 1,
        end_line: id,
        end_col: 10,
    };

    let d0 = w.add_definition(mk_def(10, DefKind::Struct, "fnsig_demo::Ctx"));
    let d1 = w.add_definition(mk_def(11, DefKind::AssocFn, "fnsig_demo::Ctx::fetch"));
    let d2 = w.add_definition(mk_def(12, DefKind::Fn, "fnsig_demo::c_printf"));
    let d3 = w.add_definition(mk_def(13, DefKind::Fn, "fnsig_demo::const_id"));
    assert_eq!((d0, d1, d2, d3), (0, 1, 2, 3));

    let ctx_def = w.intern_ty_def(RawTyDef::Local { path: "fnsig_demo::Ctx", local_def_idx: d0 });
    let ctx_ty = w.intern_ty(RawTy::Adt { def: ctx_def, args: &[] });
    let ref_ctx = w.intern_ty(RawTy::Ref { mutability: TyMutability::Shared, inner: ctx_ty, lifetime: LIFETIME_ELIDED });
    let ty_usize = w.intern_ty(RawTy::Uint(UintTyWidth::Usize));
    let ty_i32 = w.intern_ty(RawTy::Int(IntTyWidth::I32));
    let ty_u8 = w.intern_ty(RawTy::Uint(UintTyWidth::U8));
    let ptr_u8 = w.intern_ty(RawTy::Ptr { mutability: TyMutability::Shared, inner: ty_u8 });
    let param_t = w.intern_ty(RawTy::Param { index: 0, name: "T" });

    // Insert deliberately out of order by def_index (3, 1, 2) to verify sorting + binary search
    w.add_fn_sig(RawFnSig { def_index: d3, inputs: &[param_t], output: param_t, abi: FnAbiKind::Rust, flags: FN_FLAG_CONST });
    w.add_fn_sig(RawFnSig { def_index: d1, inputs: &[ref_ctx, ty_usize], output: ty_usize, abi: FnAbiKind::Rust, flags: FN_FLAG_ASYNC | FN_FLAG_HAS_SELF });
    w.add_fn_sig(RawFnSig { def_index: d2, inputs: &[ptr_u8], output: ty_i32, abi: FnAbiKind::C, flags: FN_FLAG_UNSAFE | FN_FLAG_VARIADIC });
    assert_eq!(w.fn_sig_count(), 3);

    let mut bytes = Vec::new();
    w.write_to(&mut Cursor::new(&mut bytes)).unwrap();
    let aligned = copy_aligned(&bytes);
    let reader = IndexReader::from_buffer(&aligned.0[..bytes.len()]).unwrap();
    let arena = reader.ty_arena();

    assert_eq!(reader.fn_sigs().len(), 3);
    assert!(reader.fn_sig_for_def(d0).is_none(), "struct has no FnSigEntry");

    let s1 = reader.fn_sig_for_def(d1).expect("fetch sig");
    assert!(s1.is_async() && s1.has_self() && !s1.is_unsafe() && !s1.is_const());
    assert_eq!(s1.abi(), FnAbiKind::Rust);
    let s1_in: Vec<String> = s1.inputs().map(|t| arena.display_ty(t)).collect();
    assert_eq!(s1_in, vec!["&Ctx", "usize"]);
    assert_eq!(arena.display_ty(s1.output()), "usize");

    let s2 = reader.fn_sig_for_def(d2).expect("c_printf sig");
    assert!(s2.is_unsafe() && s2.is_variadic() && !s2.is_async() && !s2.has_self());
    assert_eq!(s2.abi(), FnAbiKind::C);
    let s2_in: Vec<String> = s2.inputs().map(|t| arena.display_ty(t)).collect();
    assert_eq!(s2_in, vec!["*const u8"]);
    assert_eq!(arena.display_ty(s2.output()), "i32");

    let s3 = reader.fn_sig_for_def(d3).expect("const_id sig");
    assert!(s3.is_const() && !s3.has_self());
    assert_eq!(arena.display_ty(s3.output()), "T");
}
