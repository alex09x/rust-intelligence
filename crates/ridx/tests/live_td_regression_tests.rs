/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Targeted regression tests for `b16` parity fixes:
//! 1. Lazy metadata loading in `unit_for_cursor` across `#[cfg(test)]` / `#[cfg(not(test))]` units.
//! 2. Preserving block-scoped `Struct`/`Enum`/`Union` spans in `register_single_file_spans` on clean
//!    `v5` shards so indexed inherent methods on function-local ADTs remain visible.

use std::fs::{self, File};
use std::path::PathBuf;

use ridx::live::LiveDb;
use ridx_format::format::{CtorShape, DefKind, FnAbiKind, TyMutability, UintTyWidth, FN_FLAG_HAS_SELF, LIFETIME_ELIDED};
use ridx_format::ty_arena::{RawDefTy, RawFnSig, RawImpl, RawTy, RawTyDef};
use ridx_format::workspace::{DependencyShardInfo, TargetKind, WorkspaceIndex};
use ridx_format::writer::{IndexWriter, RawDefinition};
use tempfile::TempDir;

fn write_shard(ridx_dir: &std::path::Path, stem: &str, mut writer: IndexWriter, info: &DependencyShardInfo) {
    let ridx_path = ridx_dir.join(format!("{stem}.ridx"));
    let mut file = File::create(&ridx_path).expect("create ridx shard");
    writer.write_to(&mut file).expect("write ridx shard");

    let deps_dir = ridx_dir.join(".deps");
    fs::create_dir_all(&deps_dir).expect("create .deps dir");
    let meta_path = deps_dir.join(format!("{stem}.json"));
    let json = serde_json::to_vec_pretty(info).expect("serialize DependencyShardInfo");
    fs::write(&meta_path, json).expect("write sidecar json");
}

fn def(def_id: u32, kind: DefKind, name: &str, path: &str, file: &str, (sl, sc, el, ec): (u32, u32, u32, u32)) -> RawDefinition {
    RawDefinition {
        def_id,
        kind,
        visibility: 1,
        is_macro_expansion: false,
        name: name.to_string(),
        path: path.to_string(),
        signature: None,
        file: file.to_string(),
        start_line: sl,
        start_col: sc,
        end_line: el,
        end_col: ec,
    }
}

#[test]
fn unit_for_cursor_cfg_test_switches_units_with_lazy_meta() {
    let tmp = TempDir::new().expect("tempdir");
    let ws_root = tmp.path();
    let src_dir = ws_root.join("src");
    let ridx_dir = ws_root.join("target/ridx");
    let cache_dir = ws_root.join("empty_cache");
    fs::create_dir_all(&src_dir).expect("create src");
    fs::create_dir_all(&ridx_dir).expect("create target/ridx");
    fs::create_dir_all(&cache_dir).expect("create empty cache");

    let lib_src = concat!(
        "#[cfg(not(test))]\n",
        "pub struct NotTest { pub a: u32 }\n",
        "#[cfg(not(test))]\n",
        "pub fn probe_not_test(v: NotTest) { let _ = v; }\n",
        "#[cfg(test)]\n",
        "pub struct OnlyTest { pub b: bool }\n",
        "#[cfg(test)]\n",
        "pub fn probe_test(w: OnlyTest) { let _ = w; }\n",
    );
    let lib_rs: PathBuf = src_dir.join("lib.rs");
    fs::write(&lib_rs, lib_src).expect("write lib.rs");

    let mut w_lib = IndexWriter::new("cfg_crate", "0.1.0");
    w_lib.set_unit_key("cfg_crate-lib");
    w_lib.get_or_insert_file("src/lib.rs");
    w_lib.add_definition(def(1, DefKind::Struct, "NotTest", "cfg_crate::NotTest", "src/lib.rs", (2, 1, 2, 34)));
    w_lib.add_definition(def(2, DefKind::Fn, "probe_not_test", "cfg_crate::probe_not_test", "src/lib.rs", (4, 1, 4, 51)));
    let info_lib = DependencyShardInfo {
        name: "cfg_crate".to_string(),
        unit_key: "cfg_crate-lib".to_string(),
        source_root: Some(ws_root.to_string_lossy().into_owned()),
        target_kind: Some(TargetKind::Lib),
        compiler_args: vec!["rustc".to_string()],
        ..Default::default()
    };
    write_shard(&ridx_dir, "cfg_crate", w_lib, &info_lib);

    let mut w_test = IndexWriter::new("cfg_crate", "0.1.0");
    w_test.set_unit_key("cfg_crate-test");
    w_test.get_or_insert_file("src/lib.rs");
    w_test.add_definition(def(1, DefKind::Struct, "OnlyTest", "cfg_crate::OnlyTest", "src/lib.rs", (6, 1, 6, 36)));
    w_test.add_definition(def(2, DefKind::Fn, "probe_test", "cfg_crate::probe_test", "src/lib.rs", (8, 1, 8, 48)));
    let info_test = DependencyShardInfo {
        name: "cfg_crate".to_string(),
        unit_key: "cfg_crate-test".to_string(),
        source_root: Some(ws_root.to_string_lossy().into_owned()),
        target_kind: Some(TargetKind::Lib),
        compiler_args: vec!["rustc".to_string(), "--test".to_string()],
        ..Default::default()
    };
    write_shard(&ridx_dir, "cfg_crate_test", w_test, &info_test);

    let mut ws = WorkspaceIndex::open_with_cache_dir(&ridx_dir, &cache_dir).expect("open ws");
    ws.set_workspace_root(Some(ws_root.to_path_buf()));
    let mut live = LiveDb::new();

    let not_test_off = lib_src.find("let _ = v;").expect("v use") as u32 + "let _ = ".len() as u32;
    let test_off = lib_src.find("let _ = w;").expect("w use") as u32 + "let _ = ".len() as u32;

    let ty_prod = live.type_at(&ws, &lib_rs, not_test_off, None).expect("NotTest type_at");
    assert_eq!(ty_prod.display, "NotTest", "prod cfg unit must resolve `v`");

    let ty_test = live.type_at(&ws, &lib_rs, test_off, None).expect("OnlyTest type_at");
    assert_eq!(ty_test.display, "OnlyTest", "test cfg unit must be activated via ensure_meta");

    // Verify switching back also works cleanly.
    let ty_prod_again = live.type_at(&ws, &lib_rs, not_test_off, None).expect("NotTest second lookup");
    assert_eq!(ty_prod_again.display, "NotTest");
}

#[test]
fn block_scoped_enum_in_v5_shard_retains_indexed_impl_methods() {
    let tmp = TempDir::new().expect("tempdir");
    let ws_root = tmp.path();
    let src_dir = ws_root.join("src");
    let ridx_dir = ws_root.join("target/ridx");
    let cache_dir = ws_root.join("empty_cache");
    fs::create_dir_all(&src_dir).expect("create src");
    fs::create_dir_all(&ridx_dir).expect("create target/ridx");
    fs::create_dir_all(&cache_dir).expect("create empty cache");

    let lib_src = concat!(
        "pub fn deserialize() {\n",         // line 1 (`deserialize`: 1,8..1,19)
        "    enum Field {\n",               // line 2 (`Field`: 2,10..2,15)
        "        Alpha,\n",                 // line 3
        "        Beta,\n",                  // line 4
        "    }\n",                          // line 5
        "    let f = Field::Alpha;\n",      // line 6
        "    let _ = f.indexed_check();\n", // line 7
        "}\n",                              // line 8
    );
    let lib_rs: PathBuf = src_dir.join("lib.rs");
    fs::write(&lib_rs, lib_src).expect("write lib.rs");

    let mut writer = IndexWriter::new("block_adt", "0.1.0");
    writer.set_unit_key("block_adt-lib");
    writer.get_or_insert_file("src/lib.rs");

    let fn_idx = writer.add_definition(def(1, DefKind::Fn, "deserialize", "block_adt::deserialize", "src/lib.rs", (1, 8, 1, 19)));
    let field_idx = writer.add_definition(def(2, DefKind::Enum, "Field", "block_adt::deserialize::Field", "src/lib.rs", (2, 10, 2, 15)));
    let alpha_idx = writer.add_definition(def(3, DefKind::Variant, "Alpha", "block_adt::deserialize::Field::Alpha", "src/lib.rs", (3, 9, 3, 14)));
    let beta_idx = writer.add_definition(def(4, DefKind::Variant, "Beta", "block_adt::deserialize::Field::Beta", "src/lib.rs", (4, 9, 4, 13)));
    let mut method_def = def(5, DefKind::AssocFn, "indexed_check", "block_adt::deserialize::Field::indexed_check", "src/lib.rs", (1, 1, 1, 1));
    method_def.signature = Some("pub fn indexed_check(&self) -> u32".to_string());
    let method_idx = writer.add_definition(method_def);

    let unit_ty = writer.intern_ty(RawTy::Unit);
    writer.add_fn_sig(RawFnSig { def_index: fn_idx, inputs: &[], output: unit_ty, abi: FnAbiKind::Rust, flags: 0 });

    let field_ty_def = writer.intern_ty_def(RawTyDef::Local { path: "block_adt::deserialize::Field", local_def_idx: field_idx });
    let field_ty = writer.intern_ty(RawTy::Adt { def: field_ty_def, args: &[] });
    writer.add_def_ty(RawDefTy { def_index: field_idx, ty: field_ty, parent_def_index: None, variant_idx: None, ctor_shape: CtorShape::None, flags: 0 });
    writer.add_def_ty(RawDefTy { def_index: alpha_idx, ty: field_ty, parent_def_index: Some(field_idx), variant_idx: Some(0), ctor_shape: CtorShape::Unit, flags: 0 });
    writer.add_def_ty(RawDefTy { def_index: beta_idx, ty: field_ty, parent_def_index: Some(field_idx), variant_idx: Some(1), ctor_shape: CtorShape::Unit, flags: 0 });
    let ref_field_ty = writer.intern_ty(RawTy::Ref { mutability: TyMutability::Shared, inner: field_ty, lifetime: LIFETIME_ELIDED });
    let u32_ty = writer.intern_ty(RawTy::Uint(UintTyWidth::U32));
    writer.add_fn_sig(RawFnSig { def_index: method_idx, inputs: &[ref_field_ty], output: u32_ty, abi: FnAbiKind::Rust, flags: FN_FLAG_HAS_SELF });
    writer.add_impl(RawImpl { def_index: None, self_ty: field_ty, trait_def: None, trait_args: &[], assoc_tys: &[], assoc_defs: &[method_idx], flags: 0 });

    let info = DependencyShardInfo {
        name: "block_adt".to_string(),
        unit_key: "block_adt-lib".to_string(),
        source_root: Some(ws_root.to_string_lossy().into_owned()),
        target_kind: Some(TargetKind::Lib),
        compiler_args: vec!["rustc".to_string()],
        ..Default::default()
    };
    write_shard(&ridx_dir, "block_adt", writer, &info);

    let mut ws = WorkspaceIndex::open_with_cache_dir(&ridx_dir, &cache_dir).expect("open ws");
    ws.set_workspace_root(Some(ws_root.to_path_buf()));
    let mut live = LiveDb::new();

    let f_dot_off = lib_src.find("f.indexed_check").expect("dot call") as u32 + 2;
    let f_use_off = lib_src.find("f.indexed_check").expect("f use") as u32;

    let ty = live.type_at(&ws, &lib_rs, f_use_off, None).expect("Field type_at");
    assert_eq!(ty.display, "Field");
    assert!(ty.defs.iter().any(|loc| loc.line == 2 && loc.col == 10), "expected Field location at line 2 col 10, got {:?}", ty.defs);

    let cands = live.dot_candidates(&ws, &lib_rs, f_dot_off, None);
    let labels: Vec<&str> = cands.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"indexed_check"), "block-scoped enum must map to indexed GlobalDefId so `.ridx` impl methods resolve; got {labels:?}");
}
