/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Roundtrip, binary-search partitioning, and non-collapsing (`RefShared` vs `RefMut` vs `Adt` vs `Param`)
//! tests for `ImplEntry` and `AssocTyEntry` in `.ridx` v5.

use std::io::Cursor;

use ridx_format::{
    DefKind, HeaderV2, IndexReader, IndexWriter, IntTyWidth, RawAssocTy, RawDefinition, RawGenericArg, RawImpl, RawTy, RawTyDef, SimplifiedSelfKind, TyMutability, UintTyWidth,
    IMPL_FLAG_NEGATIVE, IMPL_FLAG_TRAIT, IMPL_FLAG_UNSAFE, LIFETIME_ELIDED, MAGIC,
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
fn test_impls_and_assoc_tys_roundtrip_and_partition_search() {
    let mut w = IndexWriter::new("impl_demo", "0.1.0");
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

    let d_box = w.add_definition(mk_def(1, DefKind::Struct, "impl_demo::Boxed"));
    let d_feed = w.add_definition(mk_def(2, DefKind::Trait, "impl_demo::Feed"));
    let d_tag = w.add_definition(mk_def(3, DefKind::Trait, "impl_demo::BlanketTag"));
    let d_inh_impl = w.add_definition(mk_def(4, DefKind::Impl, "impl_demo::<impl Boxed>"));
    let d_inh_fn = w.add_definition(mk_def(5, DefKind::AssocFn, "impl_demo::Boxed::unwrap_val"));
    let d_assoc_out = w.add_definition(mk_def(6, DefKind::AssocTy, "impl_demo::<impl Feed for &Boxed>::Output"));

    let box_def = w.intern_ty_def(RawTyDef::Local { path: "impl_demo::Boxed", local_def_idx: d_box });
    let feed_def = w.intern_ty_def(RawTyDef::Local { path: "impl_demo::Feed", local_def_idx: d_feed });
    let tag_def = w.intern_ty_def(RawTyDef::Local { path: "impl_demo::BlanketTag", local_def_idx: d_tag });
    let sync_def = w.intern_ty_def(RawTyDef::External { path: "core::marker::Sync", unit_key: Some("core") });

    let param_t = w.intern_ty(RawTy::Param { index: 0, name: "T" });
    let ty_u32 = w.intern_ty(RawTy::Uint(UintTyWidth::U32));
    let ty_i64 = w.intern_ty(RawTy::Int(IntTyWidth::I64));
    let ty_boxed_t = w.intern_ty(RawTy::Adt { def: box_def, args: &[RawGenericArg::Type(param_t)] });
    let ref_boxed = w.intern_ty(RawTy::Ref { mutability: TyMutability::Shared, inner: ty_boxed_t, lifetime: LIFETIME_ELIDED });
    let mut_boxed = w.intern_ty(RawTy::Ref { mutability: TyMutability::Mut, inner: ty_boxed_t, lifetime: LIFETIME_ELIDED });
    let tuple_ty = w.intern_ty(RawTy::Tuple(&[ty_u32, ty_i64]));

    // Insert deliberately out of SimplifiedSelfKind order:
    // 1. Blanket `impl<T> BlanketTag for T` (Param)
    w.add_impl(RawImpl { def_index: None, self_ty: param_t, trait_def: Some(tag_def), trait_args: &[], assoc_tys: &[], assoc_defs: &[], flags: IMPL_FLAG_TRAIT });
    // 2. `&mut Boxed<T>: Feed<u32, Output = i64>` (RefMut — must NOT collapse with RefShared or Adt!)
    w.add_impl(RawImpl {
        def_index: None,
        self_ty: mut_boxed,
        trait_def: Some(feed_def),
        trait_args: &[RawGenericArg::Type(ty_u32)],
        assoc_tys: &[RawAssocTy { def_index: None, name: "Output", ty: ty_i64 }],
        assoc_defs: &[],
        flags: IMPL_FLAG_TRAIT,
    });
    // 3. `&Boxed<T>: Feed<i64, Output = u32>` (RefShared)
    w.add_impl(RawImpl {
        def_index: None,
        self_ty: ref_boxed,
        trait_def: Some(feed_def),
        trait_args: &[RawGenericArg::Type(ty_i64)],
        assoc_tys: &[RawAssocTy { def_index: Some(d_assoc_out), name: "Output", ty: ty_u32 }],
        assoc_defs: &[],
        flags: IMPL_FLAG_TRAIT,
    });
    // 4. Negative `impl !Sync for (u32, i64)` (Tuple)
    w.add_impl(RawImpl {
        def_index: None,
        self_ty: tuple_ty,
        trait_def: Some(sync_def),
        trait_args: &[],
        assoc_tys: &[],
        assoc_defs: &[],
        flags: IMPL_FLAG_TRAIT | IMPL_FLAG_NEGATIVE,
    });
    // 5. Unsafe trait impl `unsafe impl Sync for Boxed<T>` (Adt, trait)
    w.add_impl(RawImpl {
        def_index: None,
        self_ty: ty_boxed_t,
        trait_def: Some(sync_def),
        trait_args: &[],
        assoc_tys: &[],
        assoc_defs: &[],
        flags: IMPL_FLAG_TRAIT | IMPL_FLAG_UNSAFE,
    });
    // 6. Inherent `impl<T> Boxed<T> { fn unwrap_val ... }` (Adt, inherent)
    w.add_impl(RawImpl { def_index: Some(d_inh_impl), self_ty: ty_boxed_t, trait_def: None, trait_args: &[], assoc_tys: &[], assoc_defs: &[d_inh_fn], flags: 0 });

    assert_eq!(w.impl_count(), 6);
    assert_eq!(w.assoc_ty_count(), 2);

    let mut bytes = Vec::new();
    w.write_to(&mut Cursor::new(&mut bytes)).unwrap();
    let aligned = copy_aligned(&bytes);
    let reader = IndexReader::from_buffer(&aligned.0[..bytes.len()]).unwrap();
    let arena = reader.ty_arena();

    assert_eq!(reader.impls().len(), 6);
    assert_eq!(reader.assoc_tys().len(), 2);

    // Partition search: Adt has 1 inherent + 1 trait impl (inherent sorted before trait)
    let adt_inh: Vec<_> = reader.inherent_impls_for_simplified(SimplifiedSelfKind::Adt).collect();
    assert_eq!(adt_inh.len(), 1);
    assert!(adt_inh[0].is_inherent());
    assert_eq!(adt_inh[0].def_index(), Some(d_inh_impl));
    assert_eq!(adt_inh[0].assoc_def_indices().collect::<Vec<_>>(), vec![d_inh_fn]);

    let adt_tr: Vec<_> = reader.trait_impls_for_simplified(SimplifiedSelfKind::Adt).collect();
    assert_eq!(adt_tr.len(), 1);
    assert!(adt_tr[0].is_trait() && adt_tr[0].is_unsafe());
    assert_eq!(adt_tr[0].trait_def(), Some(sync_def));

    // RefShared vs RefMut stay strictly distinct (no peel_refs)
    let ref_shared: Vec<_> = reader.trait_impls_for_simplified(SimplifiedSelfKind::RefShared).collect();
    assert_eq!(ref_shared.len(), 1);
    assert_eq!(arena.display_ty(ref_shared[0].self_ty()), "&Boxed<T>");
    assert_eq!(ref_shared[0].find_assoc_ty("Output"), Some(ty_u32));
    let assoc0 = ref_shared[0].assoc_tys().next().unwrap();
    assert_eq!(assoc0.impl_index, ref_shared[0].index());
    assert_eq!(assoc0.def_index, Some(d_assoc_out));

    let ref_mut: Vec<_> = reader.trait_impls_for_simplified(SimplifiedSelfKind::RefMut).collect();
    assert_eq!(ref_mut.len(), 1);
    assert_eq!(arena.display_ty(ref_mut[0].self_ty()), "&mut Boxed<T>");
    assert_eq!(ref_mut[0].find_assoc_ty("Output"), Some(ty_i64));

    // Blanket Param bucket
    let blanket: Vec<_> = reader.trait_impls_for_simplified(SimplifiedSelfKind::Param).collect();
    assert_eq!(blanket.len(), 1);
    assert_eq!(blanket[0].trait_def(), Some(tag_def));

    // Negative Tuple impl
    let tup: Vec<_> = reader.trait_impls_for_simplified(SimplifiedSelfKind::Tuple).collect();
    assert_eq!(tup.len(), 1);
    assert!(tup[0].is_negative());

    // Empty bucket returns 0
    assert_eq!(reader.impls_for_simplified(SimplifiedSelfKind::FnPtr).len(), 0);
}

#[test]
fn test_v2_header_has_empty_impls_and_assoc_tys() {
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
    assert!(r.impls().is_empty());
    assert!(r.assoc_tys().is_empty());
}
