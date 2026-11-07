/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Roundtrip, binary-search, `Impl` owner re-indexing, and backward-compatibility tests
//! for `GenericParamEntry`, `WherePredEntry`, and trait default `AssocTy` (`DEF_TY_FLAG_DEFAULT_ASSOC`).

use std::io::Cursor;

use ridx_format::{
    CtorShape, DefKind, GenericArgView, GenericOwner, GenericParamKind, HeaderV2, IndexReader, IndexWriter, RawDefTy, RawDefinition, RawGenericArg, RawGenericParam, RawImpl,
    RawTy, RawTyDef, RawWherePred, SimplifiedSelfKind, TyMutability, UintTyWidth, WherePredKind, DEF_TY_FLAG_DEFAULT_ASSOC, GENERIC_PARAM_FLAG_HAS_DEFAULT,
    GENERIC_PARAM_FLAG_IS_SELF, GENERIC_PARAM_FLAG_SIZED, GENERIC_PARAM_FLAG_SYNTHETIC, IMPL_FLAG_TRAIT, LIFETIME_ELIDED, WHERE_FLAG_CONST, WHERE_FLAG_HRTB, WHERE_FLAG_NEGATIVE,
};

#[repr(align(8))]
struct AlignedBuf([u8; 65536]);

fn copy_aligned(bytes: &[u8]) -> Box<AlignedBuf> {
    assert!(bytes.len() <= 65536);
    let mut b = Box::new(AlignedBuf([0u8; 65536]));
    b.0[..bytes.len()].copy_from_slice(bytes);
    b
}

fn mk_def(id: u32, kind: DefKind, path: &str, line: u32) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: 0,
        is_macro_expansion: false,
        name: path.rsplit("::").next().unwrap().into(),
        path: path.into(),
        signature: None,
        file: "src/lib.rs".into(),
        start_line: line,
        start_col: 1,
        end_line: line,
        end_col: 20,
    }
}

#[test]
fn test_generic_params_where_preds_and_impl_reindexing_roundtrip() {
    let mut w = IndexWriter::new("gen_test", "0.1.0");
    // Def 0: trait Mapper<In = u8> { type Out = bool; }
    // Def 1: AssocTy Out
    // Def 2: fn transform<'a, T, const N: usize>(...) where ...
    w.add_definition(mk_def(1, DefKind::Trait, "gen_test::Mapper", 1));
    w.add_definition(mk_def(2, DefKind::AssocTy, "gen_test::Mapper::Out", 2));
    w.add_definition(mk_def(3, DefKind::Fn, "gen_test::transform", 5));

    let mapper_td = w.intern_ty_def(RawTyDef::Local { path: "gen_test::Mapper", local_def_idx: 0 });
    let iter_td = w.intern_ty_def(RawTyDef::External { path: "core::iter::traits::iterator::Iterator", unit_key: Some("core") });
    let fn_td = w.intern_ty_def(RawTyDef::External { path: "core::ops::function::Fn", unit_key: Some("core") });

    let ty_u8 = w.intern_ty(RawTy::Uint(UintTyWidth::U8));
    let ty_bool = w.intern_ty(RawTy::Bool);
    let ty_usize = w.intern_ty(RawTy::Uint(UintTyWidth::Usize));
    let ty_self = w.intern_ty(RawTy::Param { index: 0, name: "Self" });
    let ty_t = w.intern_ty(RawTy::Param { index: 1, name: "T" });
    let ty_i = w.intern_ty(RawTy::Param { index: 0, name: "I" });
    let ty_str = w.intern_ty(RawTy::Str);
    let ty_ref_str = w.intern_ty(RawTy::Ref { mutability: TyMutability::Shared, inner: ty_str, lifetime: LIFETIME_ELIDED });

    // Trait Mapper (def 0): Self (idx 0, unsized) + In = u8 (idx 1, sized + default)
    w.add_def_generic_param(
        0,
        RawGenericParam { def_index: None, name: "Self", param_idx: 0, kind: GenericParamKind::Type, ty_or_default: None, flags: GENERIC_PARAM_FLAG_IS_SELF },
    );
    w.add_def_generic_param(
        0,
        RawGenericParam {
            def_index: None,
            name: "In",
            param_idx: 1,
            kind: GenericParamKind::Type,
            ty_or_default: Some(ty_u8),
            flags: GENERIC_PARAM_FLAG_HAS_DEFAULT | GENERIC_PARAM_FLAG_SIZED,
        },
    );
    // Default AssocTy on trait Mapper::Out = bool (def 1)
    w.add_def_ty(RawDefTy { def_index: 1, ty: ty_bool, parent_def_index: Some(0), variant_idx: None, ctor_shape: CtorShape::None, flags: DEF_TY_FLAG_DEFAULT_ASSOC });

    // Fn transform (def 2): added intentionally out of param_idx order to test sorting!
    w.add_def_generic_param(2, RawGenericParam { def_index: None, name: "N", param_idx: 2, kind: GenericParamKind::Const, ty_or_default: Some(ty_usize), flags: 0 });
    w.add_def_generic_param(2, RawGenericParam { def_index: None, name: "'a", param_idx: 0, kind: GenericParamKind::Lifetime, ty_or_default: None, flags: 0 });
    w.add_def_generic_param(2, RawGenericParam { def_index: None, name: "T", param_idx: 1, kind: GenericParamKind::Type, ty_or_default: None, flags: GENERIC_PARAM_FLAG_SIZED });
    w.add_def_generic_param(
        2,
        RawGenericParam {
            def_index: None,
            name: "impl Iterator",
            param_idx: 3,
            kind: GenericParamKind::Type,
            ty_or_default: None,
            flags: GENERIC_PARAM_FLAG_SYNTHETIC | GENERIC_PARAM_FLAG_SIZED,
        },
    );

    let extra_in = [RawGenericArg::Type(ty_ref_str)];
    // Add all 4 WherePredKind variants on def 2
    w.add_def_where_pred(2, RawWherePred::TraitBound { lhs_ty: ty_t, trait_def: iter_td, trait_args: &[], flags: WHERE_FLAG_CONST });
    w.add_def_where_pred(2, RawWherePred::ProjectionEq { self_ty: ty_t, trait_def: iter_td, trait_args: &[], assoc_name: "Item", rhs_ty: Some(ty_u8), flags: 0 });
    w.add_def_where_pred(2, RawWherePred::TraitBound { lhs_ty: ty_t, trait_def: fn_td, trait_args: &extra_in, flags: WHERE_FLAG_HRTB | WHERE_FLAG_NEGATIVE });
    w.add_def_where_pred(2, RawWherePred::TypeOutlives { lhs_ty: ty_t, region: 1, flags: 0 });
    w.add_def_where_pred(2, RawWherePred::RegionOutlives { lhs_region: 1, rhs_region: 2, flags: 0 });

    // Add TWO impls in reverse SimplifiedSelfKind order so sorted_impls swaps their indices!
    // Pending 0: Param (bucket 17) -> becomes final impl_index 1
    let p_blanket = w.add_impl(RawImpl {
        def_index: None,
        self_ty: ty_i,
        trait_def: Some(mapper_td),
        trait_args: &[RawGenericArg::Type(ty_u8)],
        assoc_tys: &[],
        assoc_defs: &[],
        flags: IMPL_FLAG_TRAIT,
    });
    w.add_impl_generic_param(
        p_blanket,
        RawGenericParam { def_index: None, name: "I", param_idx: 0, kind: GenericParamKind::Type, ty_or_default: None, flags: GENERIC_PARAM_FLAG_SIZED },
    );
    w.add_impl_where_pred(p_blanket, RawWherePred::TraitBound { lhs_ty: ty_i, trait_def: iter_td, trait_args: &[], flags: 0 });

    // Pending 1: Bool (bucket 1) -> becomes final impl_index 0!
    let p_bool = w.add_impl(RawImpl {
        def_index: None,
        self_ty: ty_bool,
        trait_def: Some(mapper_td),
        trait_args: &[RawGenericArg::Type(ty_u8)],
        assoc_tys: &[],
        assoc_defs: &[],
        flags: IMPL_FLAG_TRAIT,
    });
    w.add_impl_where_pred(p_bool, RawWherePred::TraitBound { lhs_ty: ty_self, trait_def: mapper_td, trait_args: &[RawGenericArg::Type(ty_u8)], flags: 0 });

    assert_eq!(w.generic_param_count(), 7);
    assert_eq!(w.where_pred_count(), 7);

    let mut buf = Vec::new();
    w.write_to(&mut Cursor::new(&mut buf)).unwrap();
    let aligned = copy_aligned(&buf);
    let reader = IndexReader::from_buffer(&aligned.0[..buf.len()]).unwrap();
    let arena = reader.ty_arena();

    // Check default AssocTy on def 1
    let assoc_dt = reader.def_ty_for_def(1).unwrap();
    assert!(assoc_dt.is_default_assoc_ty());
    assert_eq!(assoc_dt.parent_def_index(), Some(0));
    assert_eq!(arena.display_ty(assoc_dt.ty()), "bool");

    // Check Trait Mapper (def 0) generic params
    let gp0: Vec<_> = reader.generic_params_for_def(0).collect();
    assert_eq!(gp0.len(), 2);
    assert_eq!(gp0[0].owner, GenericOwner::Def(0));
    assert_eq!(gp0[0].name, "Self");
    assert!(gp0[0].is_self() && !gp0[0].is_sized());
    assert_eq!(gp0[1].name, "In");
    assert!(gp0[1].has_default() && gp0[1].is_sized());
    assert_eq!(gp0[1].default_ty(), Some(ty_u8));

    // Check Fn transform (def 2) generic params sorted by param_idx
    let gp2: Vec<_> = reader.generic_params_for_def(2).collect();
    assert_eq!(gp2.len(), 4);
    assert_eq!((gp2[0].param_idx, gp2[0].name, gp2[0].kind), (0, "'a", GenericParamKind::Lifetime));
    assert_eq!((gp2[1].param_idx, gp2[1].name, gp2[1].kind), (1, "T", GenericParamKind::Type));
    assert_eq!((gp2[2].param_idx, gp2[2].name, gp2[2].kind), (2, "N", GenericParamKind::Const));
    assert_eq!(gp2[2].const_ty(), Some(ty_usize));
    assert!(gp2[3].is_synthetic());

    // Check all 5 WherePreds on def 2
    let wp2: Vec<_> = reader.where_preds_for_def(2).collect();
    assert_eq!(wp2.len(), 5);
    assert_eq!(wp2[0].kind(), WherePredKind::TraitBound);
    assert!(wp2[0].is_const());
    assert_eq!(wp2[1].kind(), WherePredKind::ProjectionEq);
    assert_eq!(wp2[1].assoc_name(), Some("Item"));
    assert_eq!(wp2[1].rhs_ty(), Some(ty_u8));
    assert!(wp2[2].is_hrtb() && wp2[2].is_negative());
    let args2: Vec<_> = wp2[2].trait_args().collect();
    assert_eq!(args2, vec![GenericArgView::Type(ty_ref_str)]);
    assert_eq!((wp2[3].kind(), wp2[3].lhs_ty(), wp2[3].outlives_region()), (WherePredKind::TypeOutlives, ty_t, Some(1)));
    assert_eq!((wp2[4].kind(), wp2[4].lhs_region(), wp2[4].outlives_region()), (WherePredKind::RegionOutlives, 1, Some(2)));

    // Check swapped Impl indices: Bool is impl 0 (0 generic params, 1 pred), Param is impl 1 (1 generic param "I", 1 pred)!
    let all_impls: Vec<_> = reader.impls_iter().collect();
    assert_eq!(all_impls.len(), 2);
    assert_eq!(all_impls[0].simplified_self(), SimplifiedSelfKind::Bool);
    assert_eq!(all_impls[1].simplified_self(), SimplifiedSelfKind::Param);

    assert_eq!(reader.generic_params_for_impl(0).len(), 0);
    assert_eq!(reader.where_preds_for_impl(0).len(), 1);

    let impl1_gp: Vec<_> = reader.generic_params_for_impl(1).collect();
    assert_eq!(impl1_gp.len(), 1);
    assert_eq!(impl1_gp[0].owner, GenericOwner::Impl(1));
    assert_eq!(impl1_gp[0].name, "I");
    let impl1_wp: Vec<_> = reader.where_preds_for_impl(1).collect();
    assert_eq!(impl1_wp.len(), 1);
    assert_eq!(impl1_wp[0].owner(), GenericOwner::Impl(1));
    assert_eq!(impl1_wp[0].trait_def(), Some(iter_td));
}

#[test]
fn test_legacy_v2_header_has_empty_generic_and_where_tables() {
    let mut buf = vec![0u8; std::mem::size_of::<HeaderV2>() + 16];
    let pool_off = std::mem::size_of::<HeaderV2>() as u64;
    let h2 = HeaderV2 {
        magic: ridx_format::MAGIC,
        version: 2,
        crate_name_offset: 0,
        crate_version_offset: 0,
        string_pool_offset: pool_off,
        string_pool_len: 8,
        files_offset: pool_off + 8,
        files_count: 0,
        unit_key_offset: 0,
        defs_offset: pool_off + 8,
        defs_count: 0,
        generation_offset: 0,
        defs_by_hash_offset: pool_off + 8,
        defs_by_hash_count: 0,
        _pad2: 0,
        refs_offset: pool_off + 8,
        refs_count: 0,
        _pad3: 0,
        refs_by_target_offset: pool_off + 8,
        refs_by_target_count: 0,
        _pad4: 0,
        types_offset: pool_off + 8,
        types_count: 0,
        _pad5: 0,
    }
    .to_le();
    unsafe {
        std::ptr::copy_nonoverlapping(&h2 as *const HeaderV2 as *const u8, buf.as_mut_ptr(), std::mem::size_of::<HeaderV2>());
    }
    let aligned = copy_aligned(&buf);
    let reader = IndexReader::from_buffer(&aligned.0[..buf.len()]).unwrap();
    assert!(reader.generic_params().is_empty());
    assert!(reader.where_preds().is_empty());
    assert_eq!(reader.generic_params_for_def(0).len(), 0);
    assert_eq!(reader.where_preds_for_impl(0).len(), 0);
}
