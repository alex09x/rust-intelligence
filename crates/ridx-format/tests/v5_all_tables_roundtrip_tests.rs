/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unified Phase 1 (`v5`, Step 1.5) roundtrip and section-integrity suite exercising
//! all 15 typed tables simultaneously inside a single `.ridx` shard and across a
//! multi-shard memory-mapped `WorkspaceIndex`.

use std::fs::{self, File};
use std::io::BufWriter;

use ridx_format::audit::audit_workspace_sizing;
use ridx_format::{
    CtorShape, DefKind, FnAbiKind, GenericOwner, GenericParamKind, Header, IndexReader, IndexWriter, IntTyWidth, ItemVis, LangItemKind, MacroDelim, MacroEdition, RawAssocTy,
    RawDefTy, RawDefinition, RawField, RawFnSig, RawGenericArg, RawGenericParam, RawImpl, RawLangItem, RawMacroDef, RawNote, RawReference, RawTy, RawTyBound, RawTyDef, RawType,
    RawUse, RawWherePred, SimplifiedSelfKind, TyMutability, UintTyWidth, UseOwner, UsePrimTy, WorkspaceIndex, DEF_TY_FLAG_MUT_STATIC, FIELD_FLAG_TUPLE, FN_FLAG_ASYNC,
    FN_FLAG_CONST, GENERIC_PARAM_FLAG_HAS_DEFAULT, GENERIC_PARAM_FLAG_SIZED, IMPL_FLAG_TRAIT, LIFETIME_ELIDED, LIFETIME_STATIC, MACRO_FLAG_BUILTIN, MACRO_FLAG_EXPORTED,
    MACRO_FLAG_LOCAL_INNER, MACRO_FLAG_MACRO_RULES, NOTE_CALL_ARGS, USE_FLAG_GLOB, USE_FLAG_PRELUDE, USE_FLAG_PRIM_TY, USE_FLAG_REEXPORT, USE_FLAG_UNDERSCORE, WHERE_FLAG_CONST,
};

fn mk_def(id: u32, kind: DefKind, vis: ItemVis, path: &str, line: u32) -> RawDefinition {
    RawDefinition {
        def_id: id,
        kind,
        visibility: vis as u8,
        is_macro_expansion: false,
        name: path.rsplit("::").next().unwrap().to_string(),
        path: path.to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: line,
        start_col: 1,
        end_line: line + 1,
        end_col: 2,
    }
}

#[test]
fn v5_unified_all_15_typed_tables_mmap_and_workspace_roundtrip() {
    let temp = tempfile::TempDir::new().unwrap();
    let vdir = temp.path().join("variants/dev-unified");
    let sys_dir = temp.path().join("sysroot/cas_v5");
    fs::create_dir_all(&vdir).unwrap();
    fs::create_dir_all(&sys_dir).unwrap();

    // 1. External sysroot shard `core` (lang items + builtin macro + trait)
    let mut dep_w = IndexWriter::new("core", "1.98.1");
    dep_w.set_unit_key("core-unit-01");
    dep_w.set_target_triple("x86_64-unknown-linux-gnu");
    dep_w.add_cfg_flag("unix");
    let d_iter = dep_w.add_definition(mk_def(1, DefKind::Trait, ItemVis::Public, "core::iter::Iterator", 10));
    let d_fmt_mac = dep_w.add_definition(mk_def(2, DefKind::Macro, ItemVis::Public, "core::format_args", 20));
    let td_iter_local = dep_w.intern_ty_def(RawTyDef::Local { path: "core::iter::Iterator", local_def_idx: d_iter });
    dep_w.add_lang_item(RawLangItem { kind: LangItemKind::Iterator, ty_def: td_iter_local, def_index: Some(d_iter) });
    dep_w.add_macro_def(RawMacroDef {
        def_index: d_fmt_mac,
        body: "($fmt:expr) => {{ /* compiler built-in */ }};",
        flags: MACRO_FLAG_MACRO_RULES | MACRO_FLAG_EXPORTED | MACRO_FLAG_BUILTIN,
        delim: MacroDelim::Brace,
        edition: MacroEdition::Edition2021,
    });
    let dep_ridx = sys_dir.join("core.ridx");
    dep_w.write_to(&mut BufWriter::new(File::create(&dep_ridx).unwrap())).unwrap();

    // 2. Workspace shard `app_ws`: populates ALL 15 v5 sections + v4 legacy sections simultaneously
    let mut w = IndexWriter::new("app_ws", "0.2.0");
    w.set_unit_key("app_ws-unit-99");
    w.set_generation("gen-v5-all");
    w.set_target_triple("x86_64-unknown-linux-gnu");
    w.add_file_hash("src/lib.rs", 0xcafe_babe_1234_5678);

    let d_mod = w.add_definition(mk_def(1, DefKind::Mod, ItemVis::Crate, "app_ws::inner", 1));
    let d_struct = w.add_definition(mk_def(2, DefKind::Struct, ItemVis::Public, "app_ws::Packet", 5));
    let d_field0 = w.add_definition(mk_def(3, DefKind::Field, ItemVis::Public, "app_ws::Packet::0", 6));
    let d_fn = w.add_definition(mk_def(4, DefKind::Fn, ItemVis::Public, "app_ws::process", 10));
    let d_impl = w.add_definition(mk_def(5, DefKind::Impl, ItemVis::Private, "app_ws::<impl Iterator for Packet>", 15));
    let d_assoc = w.add_definition(mk_def(6, DefKind::AssocTy, ItemVis::Public, "app_ws::<impl Iterator for Packet>::Item", 16));
    let d_static = w.add_definition(mk_def(7, DefKind::Static, ItemVis::Restricted, "app_ws::COUNTER", 22));
    let d_mac = w.add_definition(mk_def(8, DefKind::Macro, ItemVis::Public, "app_ws::make_pkt", 28));

    // Legacy v2-v4 sections alongside v5 tables (refs, types, derefs, notes)
    w.add_reference(RawReference {
        file: "src/lib.rs".into(),
        start_line: 11,
        start_col: 5,
        end_line: 11,
        end_col: 11,
        target_def_id: Some(2),
        target_path: "app_ws::Packet".into(),
        target_file: Some("src/lib.rs".into()),
        target_unit: Some("app_ws-unit-99".into()),
        target_line: 5,
        target_col: 1,
        kind: DefKind::Struct,
        is_write: false,
        is_macro_expansion: false,
    });
    w.add_type(RawType { file: "src/lib.rs".into(), start_line: 11, start_col: 5, end_line: 11, end_col: 20, type_name: "Packet<u32>".into() });
    w.add_deref("app_ws::Packet", "u32");
    w.add_note(RawNote { file: "src/lib.rs".into(), start_line: 11, start_col: 5, end_line: 11, end_col: 20, kind: NOTE_CALL_ARGS, text: "pkt".into() });

    // (1-4) TyArena: ty_defs, ty_entries, ty_bounds, ty_args
    let td_pkt = w.intern_ty_def(RawTyDef::Local { path: "app_ws::Packet", local_def_idx: d_struct });
    let td_iter = w.intern_ty_def(RawTyDef::External { path: "core::iter::Iterator", unit_key: Some("core-unit-01") });
    let td_prelude = w.intern_ty_def(RawTyDef::External { path: "core::prelude::rust_2021", unit_key: Some("core-unit-01") });

    let t_u32 = w.intern_ty(RawTy::Uint(UintTyWidth::U32));
    let t_i64 = w.intern_ty(RawTy::Int(IntTyWidth::I64));
    let t_bool = w.intern_ty(RawTy::Bool);
    let t_param = w.intern_ty(RawTy::Param { index: 0, name: "T" });
    let t_pkt_t = w.intern_ty(RawTy::Adt { def: td_pkt, args: &[RawGenericArg::Type(t_param), RawGenericArg::ConstValue(4)] });
    let t_pkt_u32 = w.intern_ty(RawTy::Adt { def: td_pkt, args: &[RawGenericArg::Type(t_u32), RawGenericArg::ConstValue(4)] });
    let t_ref_pkt = w.intern_ty(RawTy::Ref { mutability: TyMutability::Shared, inner: t_pkt_t, lifetime: LIFETIME_ELIDED });
    let dyn_bounds = [RawTyBound { trait_def: td_iter, args: &[], assocs: &[("Item", t_u32)], is_maybe: false }];
    let t_dyn_iter = w.intern_ty(RawTy::Dyn { bounds: &dyn_bounds, lifetime: LIFETIME_STATIC });
    let t_out_tuple = w.intern_ty(RawTy::Tuple(&[t_pkt_u32, t_dyn_iter]));

    // (5) fn_sigs
    w.add_fn_sig(RawFnSig { def_index: d_struct, inputs: &[t_param], output: t_pkt_t, abi: FnAbiKind::Rust, flags: FN_FLAG_CONST });
    w.add_fn_sig(RawFnSig { def_index: d_fn, inputs: &[t_ref_pkt, t_i64], output: t_out_tuple, abi: FnAbiKind::Rust, flags: FN_FLAG_ASYNC });

    // (6-7) impls & assoc_tys (2 impls in reverse SimplifiedSelfKind order to verify reindexing)
    let p_blanket = w.add_impl(RawImpl {
        def_index: None,
        self_ty: t_param,
        trait_def: Some(td_iter),
        trait_args: &[],
        assoc_tys: &[RawAssocTy { def_index: None, name: "Item", ty: t_bool }],
        assoc_defs: &[],
        flags: IMPL_FLAG_TRAIT,
    });
    let p_adt = w.add_impl(RawImpl {
        def_index: Some(d_impl),
        self_ty: t_pkt_t,
        trait_def: Some(td_iter),
        trait_args: &[],
        assoc_tys: &[RawAssocTy { def_index: Some(d_assoc), name: "Item", ty: t_param }],
        assoc_defs: &[d_assoc],
        flags: IMPL_FLAG_TRAIT,
    });

    // (8-9) fields & def_tys
    w.add_field(RawField { owner_def_index: d_struct, def_index: Some(d_field0), name: "0", ty: t_param, field_idx: 0, vis: ItemVis::Public, flags: FIELD_FLAG_TUPLE });
    w.add_def_ty(RawDefTy { def_index: d_struct, ty: t_pkt_t, parent_def_index: None, variant_idx: Some(0), ctor_shape: CtorShape::Tuple, flags: 0 });
    w.add_def_ty(RawDefTy { def_index: d_static, ty: t_u32, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags: DEF_TY_FLAG_MUT_STATIC });

    // (10-11) generic_params & where_preds
    w.add_def_generic_param(
        d_struct,
        RawGenericParam {
            def_index: None,
            name: "T",
            param_idx: 0,
            kind: GenericParamKind::Type,
            ty_or_default: Some(t_u32),
            flags: GENERIC_PARAM_FLAG_SIZED | GENERIC_PARAM_FLAG_HAS_DEFAULT,
        },
    );
    w.add_def_where_pred(d_fn, RawWherePred::TraitBound { lhs_ty: t_param, trait_def: td_iter, trait_args: &[], flags: WHERE_FLAG_CONST });
    w.add_def_where_pred(d_fn, RawWherePred::ProjectionEq { self_ty: t_param, trait_def: td_iter, trait_args: &[], assoc_name: "Item", rhs_ty: Some(t_u32), flags: 0 });
    w.add_def_where_pred(d_fn, RawWherePred::TypeOutlives { lhs_ty: t_param, region: 1, flags: 0 });
    w.add_def_where_pred(d_fn, RawWherePred::RegionOutlives { lhs_region: 1, rhs_region: 2, flags: 0 });
    w.add_impl_generic_param(
        p_adt,
        RawGenericParam { def_index: None, name: "T", param_idx: 0, kind: GenericParamKind::Type, ty_or_default: None, flags: GENERIC_PARAM_FLAG_SIZED },
    );
    w.add_impl_where_pred(p_blanket, RawWherePred::TraitBound { lhs_ty: t_param, trait_def: td_iter, trait_args: &[], flags: 0 });

    // (12) lang_items
    w.add_lang_item(RawLangItem { kind: LangItemKind::Sized, ty_def: td_pkt, def_index: Some(d_struct) });

    // (13) uses
    w.add_use(RawUse {
        owner: UseOwner::CrateRoot,
        def_index: None,
        name: "*",
        type_def: Some(td_prelude),
        value_def: None,
        macro_def: None,
        vis: ItemVis::Private,
        flags: USE_FLAG_GLOB | USE_FLAG_PRELUDE,
        prim_ty: UsePrimTy::None,
    });
    w.add_use(RawUse {
        owner: UseOwner::CrateRoot,
        def_index: None,
        name: "Packet",
        type_def: Some(td_pkt),
        value_def: Some(td_pkt),
        macro_def: None,
        vis: ItemVis::Public,
        flags: USE_FLAG_REEXPORT,
        prim_ty: UsePrimTy::None,
    });
    w.add_use(RawUse {
        owner: UseOwner::Def(d_mod),
        def_index: None,
        name: "_",
        type_def: Some(td_iter),
        value_def: None,
        macro_def: None,
        vis: ItemVis::Private,
        flags: USE_FLAG_UNDERSCORE,
        prim_ty: UsePrimTy::None,
    });
    w.add_use(RawUse {
        owner: UseOwner::Def(d_mod),
        def_index: None,
        name: "u32",
        type_def: None,
        value_def: None,
        macro_def: None,
        vis: ItemVis::Public,
        flags: USE_FLAG_PRIM_TY,
        prim_ty: UsePrimTy::U32,
    });

    // (14) cfgs
    w.add_cfg_flag("unix");
    w.add_cfg_key_value("feature", "live_ty");
    w.add_cfg_key_value("feature", "default");
    w.add_cfg_key_value("target_pointer_width", "64");

    // (15) macro_defs
    w.add_macro_def(RawMacroDef {
        def_index: d_mac,
        body: "($val:expr) => { $crate::Packet($val) };",
        flags: MACRO_FLAG_MACRO_RULES | MACRO_FLAG_EXPORTED | MACRO_FLAG_LOCAL_INNER,
        delim: MacroDelim::Paren,
        edition: MacroEdition::Edition2024,
    });

    let ws_ridx = vdir.join("app_ws.ridx");
    w.write_to(&mut BufWriter::new(File::create(&ws_ridx).unwrap())).unwrap();

    let r = IndexReader::open(&ws_ridx).unwrap();
    let h = r.header();
    assert_eq!(h.version, 5);
    assert_eq!(std::mem::size_of::<Header>(), 416);

    let v5_sections: &[(u64, usize)] = &[
        (h.ty_defs_offset, h.ty_defs_count as usize * 16),
        (h.ty_entries_offset, h.ty_entries_count as usize * 16),
        (h.ty_bounds_offset, h.ty_bounds_count as usize * 16),
        (h.ty_args_offset, h.ty_args_count as usize * 4),
        (h.fn_sigs_offset, h.fn_sigs_count as usize * 16),
        (h.impls_offset, h.impls_count as usize * 32),
        (h.assoc_tys_offset, h.assoc_tys_count as usize * 16),
        (h.fields_offset, h.fields_count as usize * 20),
        (h.def_tys_offset, h.def_tys_count as usize * 16),
        (h.generic_params_offset, h.generic_params_count as usize * 20),
        (h.where_preds_offset, h.where_preds_count as usize * 28),
        (h.lang_items_offset, h.lang_items_count as usize * 12),
        (h.uses_offset, h.uses_count as usize * 28),
        (h.cfgs_offset, h.cfgs_count as usize * 8),
        (h.macro_defs_offset, h.macro_defs_count as usize * 12),
    ];
    for (i, &(off, byte_len)) in v5_sections.iter().enumerate() {
        assert!(byte_len > 0, "v5 section {i} must be non-empty");
        assert_eq!(off % 8, 0, "v5 section {i} offset {off} must be 8-byte aligned");
        if let Some(&(next_off, _)) = v5_sections.get(i + 1) {
            assert!(off + byte_len as u64 <= next_off, "v5 section {i} overlaps {i}+1");
        }
    }

    assert_eq!(r.fn_sig_for_def(d_fn).unwrap().inputs().len(), 2);
    assert!(r.fn_sig_for_def(d_fn).unwrap().is_async());
    assert_eq!(r.impls_for_simplified(SimplifiedSelfKind::Adt).len(), 1);
    let adt_impl = r.impls_for_simplified(SimplifiedSelfKind::Adt).next().unwrap();
    assert_eq!(adt_impl.find_assoc_ty("Item"), Some(t_param));
    assert_eq!(r.generic_params_for_owner(GenericOwner::Impl(adt_impl.index())).count(), 1);
    assert_eq!(r.where_preds_for_owner(GenericOwner::Def(d_fn)).count(), 4);
    assert_eq!(r.find_field(d_struct, "0").unwrap().vis, ItemVis::Public);
    assert!(r.def_ty_for_def(d_static).unwrap().is_mut_static());
    assert_eq!(r.lang_item(LangItemKind::Sized).unwrap().def_index, Some(d_struct));
    assert!(r.prelude_use().is_some());
    let pkt_use = r.use_by_name(UseOwner::CrateRoot, "Packet").unwrap();
    assert!(pkt_use.type_def.is_some() && pkt_use.value_def.is_some());
    assert_eq!(r.underscore_uses_for_owner(UseOwner::Def(d_mod)).count(), 1);
    assert_eq!(r.use_by_name(UseOwner::Def(d_mod), "u32").unwrap().prim_ty, UsePrimTy::U32);
    assert_eq!(r.target_triple(), "x86_64-unknown-linux-gnu");
    assert!(r.has_cfg_flag("unix") && r.has_cfg_key_value("feature", "live_ty"));
    assert_eq!(r.enabled_features().collect::<Vec<_>>(), vec!["default", "live_ty"]);
    let mac = r.macro_def_for_def(d_mac).unwrap();
    assert!(mac.is_macro_rules() && mac.is_exported() && mac.has_local_inner_macros());
    assert_eq!(mac.delim, MacroDelim::Paren);
    assert_eq!(mac.edition, MacroEdition::Edition2024);
    assert_eq!(mac.body, "($val:expr) => { $crate::Packet($val) };");

    let mut ws = WorkspaceIndex::open(&vdir).unwrap();
    ws.attach_sysroot_shards_for_key_in(&temp.path().join("sysroot"), "cas_v5");
    assert_eq!(ws.readers().len(), 2);
    assert!(ws.lang_item(LangItemKind::Iterator).is_some());
    let sizing = audit_workspace_sizing(&ws);
    assert!(sizing.total_sections.item_tables_bytes > 500);
}
