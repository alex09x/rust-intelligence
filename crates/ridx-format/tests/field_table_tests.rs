/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Roundtrip and binary-search tests for `.ridx` v5 `FieldEntry` and `DefTyEntry` tables.

use ridx_format::{
    CtorShape, FnAbiKind, HeaderV2, IndexReader, IndexWriter, IntTyWidth, ItemVis, RawDefTy, RawField, RawFnSig, RawGenericArg, RawTy, RawTyDef, UintTyWidth,
    DEF_TY_FLAG_MUT_STATIC, DEF_TY_FLAG_NON_EXHAUSTIVE, FIELD_FLAG_TUPLE, FIELD_FLAG_UNSAFE, FN_FLAG_CONST, MAGIC,
};

#[repr(align(8))]
struct AlignedBuf([u8; 65536]);

fn copy_aligned(bytes: &[u8]) -> Box<AlignedBuf> {
    assert!(bytes.len() <= 65536);
    let mut b = Box::new(AlignedBuf([0u8; 65536]));
    b.0[..bytes.len()].copy_from_slice(bytes);
    b
}

#[test]
fn test_fields_and_def_tys_roundtrip_and_binary_search() {
    let mut w = IndexWriter::new("g3_demo", "0.1.0");

    let ty_i32 = w.intern_ty(RawTy::Int(IntTyWidth::I32));
    let ty_u64 = w.intern_ty(RawTy::Uint(UintTyWidth::U64));
    let ty_param_t = w.intern_ty(RawTy::Param { index: 0, name: "T" });

    let point_def = w.intern_ty_def(RawTyDef::Local { path: "g3_demo::Point", local_def_idx: 10 });
    let point_ty = w.intern_ty(RawTy::Adt { def: point_def, args: &[] });

    let wrap_def = w.intern_ty_def(RawTyDef::Local { path: "g3_demo::Wrapper", local_def_idx: 20 });
    let wrap_args = [RawGenericArg::Type(ty_param_t)];
    let wrap_ty = w.intern_ty(RawTy::Adt { def: wrap_def, args: &wrap_args });

    let status_def = w.intern_ty_def(RawTyDef::Local { path: "g3_demo::Status", local_def_idx: 30 });
    let status_ty = w.intern_ty(RawTy::Adt { def: status_def, args: &[] });

    // Add out-of-order fields across owners 32, 10, 20 to verify deterministic sorting
    w.add_field(RawField { owner_def_index: 32, def_index: Some(320), name: "code", ty: ty_u64, field_idx: 0, vis: ItemVis::Public, flags: 0 });
    w.add_field(RawField { owner_def_index: 10, def_index: Some(102), name: "y", ty: ty_i32, field_idx: 1, vis: ItemVis::Private, flags: 0 });
    w.add_field(RawField { owner_def_index: 10, def_index: Some(101), name: "x", ty: ty_i32, field_idx: 0, vis: ItemVis::Public, flags: 0 });
    w.add_field(RawField { owner_def_index: 20, def_index: Some(201), name: "0", ty: ty_param_t, field_idx: 0, vis: ItemVis::Crate, flags: FIELD_FLAG_TUPLE });
    w.add_field(RawField { owner_def_index: 20, def_index: None, name: "1", ty: ty_u64, field_idx: 1, vis: ItemVis::Restricted, flags: FIELD_FLAG_TUPLE | FIELD_FLAG_UNSAFE });

    // Add out-of-order DefTyEntry rows: Static (50), TyAlias (40), Variants (31, 32), Tuple Struct (20), Record Struct (10), Enum (30)
    w.add_def_ty(RawDefTy { def_index: 50, ty: ty_u64, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags: DEF_TY_FLAG_MUT_STATIC });
    w.add_def_ty(RawDefTy { def_index: 40, ty: point_ty, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags: 0 });
    w.add_def_ty(RawDefTy { def_index: 32, ty: status_ty, parent_def_index: Some(30), variant_idx: Some(1), ctor_shape: CtorShape::Record, flags: 0 });
    w.add_def_ty(RawDefTy { def_index: 31, ty: status_ty, parent_def_index: Some(30), variant_idx: Some(0), ctor_shape: CtorShape::Unit, flags: 0 });
    w.add_def_ty(RawDefTy { def_index: 30, ty: status_ty, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags: DEF_TY_FLAG_NON_EXHAUSTIVE });
    w.add_def_ty(RawDefTy { def_index: 20, ty: wrap_ty, parent_def_index: None, variant_idx: Some(0), ctor_shape: CtorShape::Tuple, flags: 0 });
    w.add_def_ty(RawDefTy { def_index: 10, ty: point_ty, parent_def_index: None, variant_idx: Some(0), ctor_shape: CtorShape::Record, flags: 0 });

    // Tuple struct ctor FnSigEntry
    w.add_fn_sig(RawFnSig { def_index: 20, inputs: &[ty_param_t, ty_u64], output: wrap_ty, abi: FnAbiKind::Rust, flags: FN_FLAG_CONST });

    let mut bytes = Vec::new();
    w.write_to(&mut std::io::Cursor::new(&mut bytes)).expect("write v5 shard");
    let aligned = copy_aligned(&bytes);
    let reader = IndexReader::from_buffer(&aligned.0[..bytes.len()]).expect("read v5 shard");
    let arena = reader.ty_arena();

    assert_eq!(reader.fields().len(), 5);
    assert_eq!(reader.def_tys().len(), 7);

    // Point (owner 10) fields in exact declaration order [x, y]
    let point_fields: Vec<_> = reader.fields_for_owner(10).collect();
    assert_eq!(point_fields.len(), 2);
    assert_eq!(point_fields[0].name, "x");
    assert_eq!(point_fields[0].vis, ItemVis::Public);
    assert!(!point_fields[0].is_tuple_field());
    assert_eq!(point_fields[1].name, "y");
    assert_eq!(point_fields[1].vis, ItemVis::Private);

    // Wrapper (owner 20) tuple fields `.0` and `.1`
    let wrap_f0 = reader.find_field(20, "0").expect("Wrapper.0");
    assert!(wrap_f0.is_tuple_field());
    assert_eq!(wrap_f0.vis, ItemVis::Crate);
    assert_eq!(arena.display_ty(wrap_f0.ty), "T");
    let wrap_f1 = reader.find_field(20, "1").expect("Wrapper.1");
    assert!(wrap_f1.is_tuple_field());
    assert!(wrap_f1.is_unsafe());
    assert_eq!(wrap_f1.def_index, None);

    // DefTy lookups
    let alias_dt = reader.def_ty_for_def(40).expect("TyAlias DefTy");
    assert_eq!(alias_dt.ctor_shape(), CtorShape::None);
    assert_eq!(arena.display_ty(alias_dt.ty()), "Point");

    let static_dt = reader.def_ty_for_def(50).expect("Static DefTy");
    assert!(static_dt.is_mut_static());

    let enum_dt = reader.def_ty_for_def(30).expect("Status Enum DefTy");
    assert!(enum_dt.is_non_exhaustive());

    let var_unit = reader.def_ty_for_def(31).expect("Status::Idle");
    assert_eq!(var_unit.parent_def_index(), Some(30));
    assert_eq!(var_unit.variant_idx(), Some(0));
    assert_eq!(var_unit.ctor_shape(), CtorShape::Unit);

    let var_rec = reader.def_ty_for_def(32).expect("Status::Detailed");
    assert_eq!(var_rec.ctor_shape(), CtorShape::Record);
    assert_eq!(reader.find_field(32, "code").unwrap().ty, ty_u64);

    let wrap_dt = reader.def_ty_for_def(20).expect("Wrapper Tuple Struct");
    assert_eq!(wrap_dt.ctor_shape(), CtorShape::Tuple);
    let wrap_ctor_sig = reader.fn_sig_for_def(20).expect("Wrapper tuple ctor FnSig");
    assert_eq!(wrap_ctor_sig.inputs().len(), 2);
    assert!(wrap_ctor_sig.is_const());
}

#[test]
fn test_v2_header_has_empty_fields_and_def_tys() {
    let v2_hdr = HeaderV2 {
        magic: MAGIC,
        version: 2,
        crate_name_offset: 0,
        crate_version_offset: 0,
        string_pool_offset: 128,
        string_pool_len: 1,
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
    let aligned = copy_aligned(&raw_v2);
    let r = IndexReader::from_buffer(&aligned.0[..raw_v2.len()]).unwrap();
    assert!(r.fields().is_empty());
    assert!(r.def_tys().is_empty());
    assert!(r.def_ty_for_def(0).is_none());
}
