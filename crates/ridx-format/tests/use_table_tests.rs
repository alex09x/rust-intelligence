/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Roundtrip and binary-search tests for `.ridx` v5 `UseEntry` tables and `DefEntry::item_vis` (1.2-G7).

use std::io::Cursor;

use ridx_format::{
    DefKind, IndexReader, IndexWriter, ItemVis, RawDefinition, RawTyDef, RawUse, UseOwner, UsePrimTy, USE_FLAG_EXTERN_CRATE, USE_FLAG_GLOB, USE_FLAG_PRIM_TY, USE_FLAG_REEXPORT,
    USE_FLAG_UNDERSCORE,
};

#[test]
fn use_table_merges_multi_ns_and_preserves_globs_and_underscores() {
    let mut w = IndexWriter::new("demo", "0.1.0");
    let mod_idx = w.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Mod,
        visibility: ItemVis::Crate as u8,
        is_macro_expansion: false,
        name: "sub".to_string(),
        path: "demo::sub".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 10,
        end_col: 2,
    });

    let pair_struct = w.intern_ty_def(RawTyDef::External { path: "dep::Pair", unit_key: Some("dep-1") });
    let pair_ctor = w.intern_ty_def(RawTyDef::External { path: "dep::Pair", unit_key: Some("dep-1") });
    let derive_mac = w.intern_ty_def(RawTyDef::External { path: "dep_derive::Pair", unit_key: Some("dep_derive-1") });
    let glob_a = w.intern_ty_def(RawTyDef::External { path: "dep::a", unit_key: Some("dep-1") });
    let glob_b = w.intern_ty_def(RawTyDef::External { path: "dep::b", unit_key: Some("dep-1") });
    let trait_a = w.intern_ty_def(RawTyDef::External { path: "dep::TraitA", unit_key: Some("dep-1") });
    let trait_b = w.intern_ty_def(RawTyDef::External { path: "dep::TraitB", unit_key: Some("dep-1") });
    let ext_crate = w.intern_ty_def(RawTyDef::External { path: "alloc", unit_key: Some("alloc-1") });

    // Multi-namespace re-export in CrateRoot: TypeNS + ValueNS + MacroNS across separate rows
    w.add_use(RawUse {
        owner: UseOwner::CrateRoot,
        def_index: Some(42),
        name: "Pair",
        type_def: Some(pair_struct),
        value_def: None,
        macro_def: None,
        vis: ItemVis::Public,
        flags: USE_FLAG_REEXPORT,
        prim_ty: UsePrimTy::None,
    });
    w.add_use(RawUse {
        owner: UseOwner::CrateRoot,
        def_index: None,
        name: "Pair",
        type_def: None,
        value_def: Some(pair_ctor),
        macro_def: Some(derive_mac),
        vis: ItemVis::Crate,
        flags: USE_FLAG_REEXPORT,
        prim_ty: UsePrimTy::None,
    });
    // Primitive re-export + extern crate in CrateRoot
    w.add_use(RawUse {
        owner: UseOwner::CrateRoot,
        def_index: None,
        name: "u32",
        type_def: None,
        value_def: None,
        macro_def: None,
        vis: ItemVis::Public,
        flags: USE_FLAG_REEXPORT,
        prim_ty: UsePrimTy::U32,
    });
    w.add_use(RawUse {
        owner: UseOwner::CrateRoot,
        def_index: None,
        name: "my_alloc",
        type_def: Some(ext_crate),
        value_def: None,
        macro_def: None,
        vis: ItemVis::Private,
        flags: USE_FLAG_EXTERN_CRATE,
        prim_ty: UsePrimTy::None,
    });

    // Two distinct globs + two distinct `use Trait as _;` in submodule `sub`
    for g in [glob_b, glob_a, glob_a] {
        w.add_use(RawUse {
            owner: UseOwner::Def(mod_idx),
            def_index: None,
            name: "*",
            type_def: Some(g),
            value_def: None,
            macro_def: None,
            vis: ItemVis::Private,
            flags: USE_FLAG_GLOB,
            prim_ty: UsePrimTy::None,
        });
    }
    for tr in [trait_b, trait_a, trait_b] {
        w.add_use(RawUse {
            owner: UseOwner::Def(mod_idx),
            def_index: None,
            name: "_",
            type_def: Some(tr),
            value_def: None,
            macro_def: None,
            vis: ItemVis::Private,
            flags: USE_FLAG_UNDERSCORE,
            prim_ty: UsePrimTy::None,
        });
    }

    let mut buf = Cursor::new(Vec::new());
    w.write_to(&mut buf).expect("write failed");
    let reader = IndexReader::from_buffer(buf.into_inner()).expect("read failed");

    assert_eq!(reader.definitions()[0].item_vis(), ItemVis::Crate);

    // CrateRoot: Pair (merged), my_alloc, u32 = 3 rows
    let root_uses: Vec<_> = reader.uses_for_crate_root().collect();
    assert_eq!(root_uses.len(), 3);

    let pair = reader.use_by_name(UseOwner::CrateRoot, "Pair").expect("Pair use");
    assert_eq!(pair.def_index, Some(42));
    assert_eq!(pair.vis, ItemVis::Public);
    assert!(pair.is_reexport());
    assert_eq!(pair.type_def, Some(pair_struct));
    assert_eq!(pair.value_def, Some(pair_ctor));
    assert_eq!(pair.macro_def, Some(derive_mac));

    let prim = reader.use_by_name(UseOwner::CrateRoot, "u32").expect("u32 prim use");
    assert!(prim.has_prim_ty());
    assert_eq!(prim.flags & USE_FLAG_PRIM_TY, USE_FLAG_PRIM_TY);
    assert_eq!(prim.prim_ty, UsePrimTy::U32);
    assert_eq!(prim.prim_ty.as_str(), Some("u32"));

    let ext = reader.use_by_name(UseOwner::CrateRoot, "my_alloc").expect("my_alloc extern crate");
    assert!(ext.is_extern_crate());
    assert_eq!(ext.type_def, Some(ext_crate));

    // Submodule: 2 distinct globs + 2 distinct `_` trait imports (duplicates collapsed)
    assert!(reader.use_by_name(UseOwner::Def(mod_idx), "_").is_none());
    assert!(reader.use_by_name(UseOwner::Def(mod_idx), "*").is_none());
    let globs: Vec<_> = reader.glob_uses_for_owner(UseOwner::Def(mod_idx)).collect();
    assert_eq!(globs.len(), 2);
    assert!(globs.iter().all(|g| g.is_glob() && g.name == "*"));
    let underscores: Vec<_> = reader.underscore_uses_for_owner(UseOwner::Def(mod_idx)).collect();
    assert_eq!(underscores.len(), 2);
    assert!(underscores.iter().all(|u| u.is_underscore() && u.name == "_"));
}
