/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit and regression tests for Phase 2.2 (`use` trees, globs, re-exports, preludes -> `GlobalDefId`).

use ridx_syntax::name_res::{CrateDefMap, CrateId, Edition, GlobalDefId, ItemKind, MemItemDb, MemSourceDb, ModuleId, ModuleTree, OwnedDef, OwnedUse, PathAnchor, PrimTy, VisKind};

fn build_sysroot_db(main_ed: Edition) -> (MemItemDb, CrateId) {
    let mut db = MemItemDb::new();
    let core_k = db.add_crate("core", Edition::Edition2021);
    let std_k = db.add_crate("std", Edition::Edition2021);
    let main_k = db.add_crate("app", main_ed);

    let core_root = GlobalDefId::new(core_k, 0);
    let core_option_mod = GlobalDefId::new(core_k, 1);
    let core_option_enum = GlobalDefId::new(core_k, 2);
    let core_some_var = GlobalDefId::new(core_k, 3);
    let core_none_var = GlobalDefId::new(core_k, 4);
    let core_convert_mod = GlobalDefId::new(core_k, 5);
    let core_try_from = GlobalDefId::new(core_k, 6);
    let core_future_mod = GlobalDefId::new(core_k, 7);
    let core_future_trait = GlobalDefId::new(core_k, 8);
    let core_fmt_mod = GlobalDefId::new(core_k, 9);
    let core_display_trait = GlobalDefId::new(core_k, 10);
    let core_prelude = GlobalDefId::new(core_k, 11);
    let core_pre_v1 = GlobalDefId::new(core_k, 12);
    let core_pre_2021 = GlobalDefId::new(core_k, 13);
    let core_pre_2024 = GlobalDefId::new(core_k, 14);

    db.set_crate_root_def(core_k, core_root);
    for (p, id, name, kind, ctor) in [
        (core_root, core_option_mod, "option", ItemKind::Mod, false),
        (core_option_mod, core_option_enum, "Option", ItemKind::Enum, false),
        (core_option_enum, core_some_var, "Some", ItemKind::Variant, true),
        (core_option_enum, core_none_var, "None", ItemKind::Variant, true),
        (core_root, core_convert_mod, "convert", ItemKind::Mod, false),
        (core_convert_mod, core_try_from, "TryFrom", ItemKind::Trait, false),
        (core_root, core_future_mod, "future", ItemKind::Mod, false),
        (core_future_mod, core_future_trait, "Future", ItemKind::Trait, false),
        (core_root, core_fmt_mod, "fmt", ItemKind::Mod, false),
        (core_fmt_mod, core_display_trait, "Display", ItemKind::Trait, false),
        (core_root, core_prelude, "prelude", ItemKind::Mod, false),
        (core_prelude, core_pre_v1, "v1", ItemKind::Mod, false),
        (core_prelude, core_pre_2021, "rust_2021", ItemKind::Mod, false),
        (core_prelude, core_pre_2024, "rust_2024", ItemKind::Mod, false),
    ] {
        db.add_module_def(p, OwnedDef { id, name: name.into(), kind, vis: VisKind::Public, has_value_ctor: ctor });
    }
    for (m, name, tdef, vdef) in [
        (core_pre_v1, "Option", core_option_enum, GlobalDefId::NONE),
        (core_pre_v1, "Some", core_some_var, core_some_var),
        (core_pre_v1, "None", core_none_var, core_none_var),
        (core_pre_2021, "TryFrom", core_try_from, GlobalDefId::NONE),
        (core_pre_2024, "Future", core_future_trait, GlobalDefId::NONE),
    ] {
        db.add_module_use(
            m,
            OwnedUse {
                name: name.into(),
                type_def: tdef,
                value_def: vdef,
                macro_def: GlobalDefId::NONE,
                vis: VisKind::Public,
                is_glob: false,
                is_reexport: true,
                is_prelude: false,
                is_underscore: false,
                prim_ty_code: 0,
            },
        );
    }
    // `rust_2021` glob-reexports `v1`, `rust_2024` glob-reexports `rust_2021`:
    for (m, target) in [(core_pre_2021, core_pre_v1), (core_pre_2024, core_pre_2021)] {
        db.add_module_use(
            m,
            OwnedUse {
                name: String::new(),
                type_def: target,
                value_def: GlobalDefId::NONE,
                macro_def: GlobalDefId::NONE,
                vis: VisKind::Public,
                is_glob: true,
                is_reexport: true,
                is_prelude: false,
                is_underscore: false,
                prim_ty_code: 0,
            },
        );
    }

    // `std` re-exports `core::fmt`, `core::option`, plus adds `Vec` in `std::prelude::rust_2021`.
    let std_root = GlobalDefId::new(std_k, 0);
    let std_vec_mod = GlobalDefId::new(std_k, 1);
    let std_vec_struct = GlobalDefId::new(std_k, 2);
    let std_prelude = GlobalDefId::new(std_k, 3);
    let std_pre_2021 = GlobalDefId::new(std_k, 4);
    db.set_crate_root_def(std_k, std_root);
    for (p, id, name, kind) in [
        (std_root, std_vec_mod, "vec", ItemKind::Mod),
        (std_vec_mod, std_vec_struct, "Vec", ItemKind::Struct),
        (std_root, std_prelude, "prelude", ItemKind::Mod),
        (std_prelude, std_pre_2021, "rust_2021", ItemKind::Mod),
    ] {
        db.add_module_def(p, OwnedDef { id, name: name.into(), kind, vis: VisKind::Public, has_value_ctor: false });
    }
    // `pub use core::fmt;` in `std` root:
    db.add_module_use(
        std_root,
        OwnedUse {
            name: "fmt".into(),
            type_def: core_fmt_mod,
            value_def: GlobalDefId::NONE,
            macro_def: GlobalDefId::NONE,
            vis: VisKind::Public,
            is_glob: false,
            is_reexport: true,
            is_prelude: false,
            is_underscore: false,
            prim_ty_code: 0,
        },
    );
    db.add_module_use(
        std_pre_2021,
        OwnedUse {
            name: String::new(),
            type_def: core_pre_2021,
            value_def: GlobalDefId::NONE,
            macro_def: GlobalDefId::NONE,
            vis: VisKind::Public,
            is_glob: true,
            is_reexport: true,
            is_prelude: false,
            is_underscore: false,
            prim_ty_code: 0,
        },
    );
    db.add_module_use(
        std_pre_2021,
        OwnedUse {
            name: "Vec".into(),
            type_def: std_vec_struct,
            value_def: GlobalDefId::NONE,
            macro_def: GlobalDefId::NONE,
            vis: VisKind::Public,
            is_glob: false,
            is_reexport: true,
            is_prelude: false,
            is_underscore: false,
            prim_ty_code: 0,
        },
    );

    db.add_extern_crate(main_k, "core", core_k);
    db.add_extern_crate(main_k, "std", std_k);
    (db, main_k)
}

#[test]
fn resolves_nested_use_trees_cfg_branches_aliases_underscore_and_index_def_ids() {
    let (mut db, app) = build_sysroot_db(Edition::Edition2021);
    db.add_cfg_kv(app, "feature", "extra");

    // Populate indexed `GlobalDefId`s for `app` items to verify exact `DefId` matching!
    let app_root_def = GlobalDefId::new(app, 0);
    let mod_m_def = GlobalDefId::new(app, 1);
    let unit_s_def = GlobalDefId::new(app, 2);
    let rec_s_def = GlobalDefId::new(app, 3);
    let rec_fn_def = GlobalDefId::new(app, 4);
    let extra_fn_def = GlobalDefId::new(app, 5);
    db.set_crate_root_def(app, app_root_def);
    db.add_module_def(app_root_def, OwnedDef { id: mod_m_def, name: "m".into(), kind: ItemKind::Mod, vis: VisKind::Public, has_value_ctor: false });
    for (id, name, kind, ctor) in [
        (unit_s_def, "UnitS", ItemKind::Struct, true),
        (rec_s_def, "Dual", ItemKind::Struct, false),
        (rec_fn_def, "Dual", ItemKind::Fn, false),
        (extra_fn_def, "extra_fn", ItemKind::Fn, false),
    ] {
        db.add_module_def(mod_m_def, OwnedDef { id, name: name.into(), kind, vis: VisKind::Public, has_value_ctor: ctor });
    }

    let mut src = MemSourceDb::new();
    let root = src.add_file(
        "src/lib.rs",
        r#"
        pub mod m {
            pub struct UnitS;
            pub struct Dual { pub x: i32 }
            pub fn Dual() {}
            pub fn extra_fn() {}
        }
        pub use crate::m::{
            self as m_alias,
            UnitS as MyUnit,
            Dual,
            #[cfg(feature = "extra")]
            extra_fn,
            #[cfg(feature = "disabled")]
            missing_symbol,
        };
        use std::fmt::{self, Display as _};
        use ::core::primitive::u32 as PrimU32;
        "#,
    );

    let tree = ModuleTree::build(&db, &src, app, root);
    let def_map = CrateDefMap::build(&db, &src, &tree);
    assert!(def_map.diagnostics.is_empty(), "diags: {:?}", def_map.diagnostics);

    let m_alias = def_map.resolve_name_in_module(&tree, ModuleId::ROOT, "m_alias");
    assert_eq!(m_alias.types.unwrap().def, mod_m_def);

    let my_unit = def_map.resolve_name_in_module(&tree, ModuleId::ROOT, "MyUnit");
    assert_eq!(my_unit.types.unwrap().def, unit_s_def);
    assert_eq!(my_unit.values.unwrap().def, unit_s_def);

    // Multi-namespace import: `Dual` resolves to `rec_s_def` in TYPE and `rec_fn_def` in VALUE!
    let dual = def_map.resolve_name_in_module(&tree, ModuleId::ROOT, "Dual");
    assert_eq!(dual.types.unwrap().def, rec_s_def);
    assert_eq!(dual.values.unwrap().def, rec_fn_def);

    let extra = def_map.resolve_name_in_module(&tree, ModuleId::ROOT, "extra_fn");
    assert_eq!(extra.values.unwrap().def, extra_fn_def);

    // Re-exported `core::fmt` through `std::fmt` + anonymous `Display as _` trait import!
    let fmt_b = def_map.resolve_name_in_module(&tree, ModuleId::ROOT, "fmt");
    assert_eq!(fmt_b.types.unwrap().def, GlobalDefId::new(CrateId(0), 9));
    let traits = def_map.traits_in_scope(&tree, ModuleId::ROOT);
    assert!(traits.iter().any(|t| t.def == GlobalDefId::new(CrateId(0), 10)));

    let prim = def_map.resolve_name_in_module(&tree, ModuleId::ROOT, "PrimU32");
    assert_eq!(prim.types.unwrap().prim_ty, PrimTy::U32 as u8);
}

#[test]
fn resolves_cyclic_globs_reexport_chains_shadowing_ambiguity_and_enums() {
    let (db, app) = build_sysroot_db(Edition::Edition2021);
    let mut src = MemSourceDb::new();
    let root = src.add_file(
        "src/lib.rs",
        r#"
        pub mod a {
            pub use super::b::*;
            pub struct OnlyA;
            pub struct Clash;
            pub struct Shadowed;
        }
        pub mod b {
            pub use super::a::*;
            pub struct OnlyB;
            pub struct Clash;
            pub struct Shadowed;
        }
        pub mod consumer {
            use crate::a::*;
            use crate::b::*;
            pub struct Shadowed;
        }
        pub enum Color {
            Red,
            Rgb(u8, u8, u8),
            Named { label: &'static str },
        }
        pub mod enum_user {
            use crate::Color::*;
            use Option::Some as MySome;
        }
        "#,
    );

    let tree = ModuleTree::build(&db, &src, app, root);
    let def_map = CrateDefMap::build(&db, &src, &tree);
    assert!(def_map.diagnostics.is_empty(), "diags: {:?}", def_map.diagnostics);

    // Cyclic globs converge: `a` sees `OnlyB` and `b` sees `OnlyA`!
    let m_a = tree.find_by_path("crate::a").unwrap();
    let m_b = tree.find_by_path("crate::b").unwrap();
    assert!(def_map.resolve_name_in_module(&tree, m_a, "OnlyB").types.is_some());
    assert!(def_map.resolve_name_in_module(&tree, m_b, "OnlyA").types.is_some());

    // In `consumer`, `Clash` came from two distinct globs -> ambiguous (filtered out),
    // whereas `Shadowed` has an explicit local definition that wins over both globs!
    let m_cons = tree.find_by_path("crate::consumer").unwrap();
    assert!(def_map.resolve_name_in_module(&tree, m_cons, "Clash").types.is_none());
    let shad = def_map.resolve_name_in_module(&tree, m_cons, "Shadowed").types.unwrap();
    assert_eq!(def_map.item(shad.local).module, m_cons);

    // Enum glob + `use Option::Some as MySome;` via prelude!
    let m_eu = tree.find_by_path("crate::enum_user").unwrap();
    let red = def_map.resolve_name_in_module(&tree, m_eu, "Red");
    assert!(red.types.is_some() && red.values.is_some());
    let named_v = def_map.resolve_name_in_module(&tree, m_eu, "Named");
    assert!(named_v.types.is_some() && named_v.values.is_none());
    let my_some = def_map.resolve_name_in_module(&tree, m_eu, "MySome");
    assert_eq!(my_some.values.unwrap().def, GlobalDefId::new(CrateId(0), 3));
}

#[test]
fn respects_visibility_no_std_editions_and_no_implicit_prelude_inheritance() {
    let (db_std, app_std) = build_sysroot_db(Edition::Edition2021);
    let mut src_std = MemSourceDb::new();
    let r_std = src_std.add_file(
        "src/lib.rs",
        r#"
        mod secret {
            struct Hidden;
            pub(super) struct ParentOnly;
            pub(in crate::allowed) struct SubtreeOnly;
        }
        pub mod allowed {
            pub mod child {
                use crate::secret::SubtreeOnly;
            }
        }
        pub mod outsider {
            use crate::secret::Hidden;
            use crate::secret::SubtreeOnly;
        }
        #[no_implicit_prelude]
        pub mod bare_outer {
            pub mod bare_inner {}
        }
        "#,
    );
    let t_std = ModuleTree::build(&db_std, &src_std, app_std, r_std);
    let dm_std = CrateDefMap::build(&db_std, &src_std, &t_std);
    // `outsider`'s 2 illegal private/restricted imports must fail while `allowed::child` succeeds!
    assert_eq!(dm_std.diagnostics.len(), 2);
    let m_child = t_std.find_by_path("crate::allowed::child").unwrap();
    assert!(dm_std.resolve_name_in_module(&t_std, m_child, "SubtreeOnly").types.is_some());

    // Root has `std::prelude::rust_2021` (`Vec` + `TryFrom`, NOT `Future`):
    assert!(dm_std.resolve_name_in_module(&t_std, ModuleId::ROOT, "Vec").types.is_some());
    assert!(dm_std.resolve_name_in_module(&t_std, ModuleId::ROOT, "TryFrom").types.is_some());
    assert!(dm_std.resolve_name_in_module(&t_std, ModuleId::ROOT, "Future").types.is_none());

    // `bare_inner` inherits `#[no_implicit_prelude]` from `bare_outer`:
    let m_bi = t_std.find_by_path("crate::bare_outer::bare_inner").unwrap();
    assert!(dm_std.resolve_name_in_module(&t_std, m_bi, "Vec").types.is_none());
    assert!(dm_std.resolve_name_in_module(&t_std, m_bi, "Option").types.is_none());
    assert_eq!(dm_std.resolve_name_in_module(&t_std, m_bi, "usize").types.unwrap().prim_ty, PrimTy::Usize as u8);

    // `#![no_std]` + Edition 2024 picks `core::prelude::rust_2024` (`Option` + `Future`, NO `Vec`),
    // plus `macro_rules!`-generated inline modules containing `pub use`:
    let (db_24, app_24) = build_sysroot_db(Edition::Edition2024);
    let mut src_24 = MemSourceDb::new();
    let r_24 = src_24.add_file(
        "src/lib.rs",
        "#![no_std]\nextern crate self as me;\npub struct RootS;\nmacro_rules! cfg_gate { ($($i:item)*) => { $($i)* }; }\ncfg_gate! { pub mod s { pub use me::RootS; } }\n",
    );
    let t_24 = ModuleTree::build(&db_24, &src_24, app_24, r_24);
    let dm_24 = CrateDefMap::build(&db_24, &src_24, &t_24);
    assert!(dm_24.diagnostics.is_empty(), "diags: {:?}", dm_24.diagnostics);
    assert!(dm_24.resolve_name_in_module(&t_24, ModuleId::ROOT, "Vec").types.is_none());
    assert!(dm_24.resolve_name_in_module(&t_24, ModuleId::ROOT, "Future").types.is_some());
    let m_s = t_24.find_by_path("crate::s").unwrap();
    assert!(dm_24.resolve_name_in_module(&t_24, m_s, "RootS").types.is_some());
    assert!(dm_24.resolve_path(&db_24, &t_24, m_s, PathAnchor::ExternRoot, &["me", "RootS"]).types.is_some());
    assert!(dm_24.resolve_path(&db_24, &t_24, m_s, PathAnchor::Super(1), &["RootS"]).types.is_some());
}
