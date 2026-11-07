/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit and regression tests for Phase 2.1 (`ridx_syntax::name_res::mod_tree`).

use ridx_syntax::name_res::{CrateId, Edition, MemItemDb, MemSourceDb, ModDiag, ModuleId, ModuleTree, SourceDb, VisKind};
use ridx_syntax::parse;

fn make_db(features: &[&str], flags: &[&str]) -> MemItemDb {
    let mut db = MemItemDb::new();
    let k = db.add_crate("demo", Edition::Edition2021);
    assert_eq!(k, CrateId(0));
    for &f in flags {
        db.add_cfg_flag(k, f);
    }
    for &feat in features {
        db.add_cfg_kv(k, "feature", feat);
    }
    db
}

#[test]
fn resolves_foo_rs_and_mod_rs_and_raw_ident() {
    let db = make_db(&[], &[]);
    let mut src = MemSourceDb::new();
    let root = src.add_file("src/lib.rs", "pub mod alpha;\npub(crate) mod beta;\npub mod r#type;\n");
    let f_alpha = src.add_file("src/alpha.rs", "pub mod nested;\n");
    let f_nested = src.add_file("src/alpha/nested.rs", "pub fn ok() {}\n");
    let f_beta = src.add_file("src/beta/mod.rs", "mod leaf;\n");
    let f_leaf = src.add_file("src/beta/leaf.rs", "fn leaf() {}\n");
    let f_type = src.add_file("src/type.rs", "pub struct Kind;\n");

    let tree = ModuleTree::build(&db, &src, CrateId(0), root);
    assert!(tree.diagnostics.is_empty(), "diags: {:?}", tree.diagnostics);
    assert_eq!(tree.modules.len(), 6);

    let m_alpha = tree.find_by_path("crate::alpha").unwrap();
    assert_eq!(tree.module(m_alpha).file, f_alpha);
    assert_eq!(tree.module(m_alpha).vis, VisKind::Public);
    assert_eq!(tree.dir_path(m_alpha), "src/alpha");
    assert_eq!(tree.attr_base_dir(m_alpha), "src");

    let m_nested = tree.find_by_path("alpha::nested").unwrap();
    assert_eq!(tree.module(m_nested).file, f_nested);

    let m_beta = tree.find_by_path("crate::beta").unwrap();
    assert_eq!(tree.module(m_beta).file, f_beta);
    assert_eq!(tree.module(m_beta).vis, VisKind::Crate);
    assert_eq!(tree.attr_base_dir(m_beta), "src/beta");

    let m_leaf = tree.find_by_path("crate::beta::leaf").unwrap();
    assert_eq!(tree.module(m_leaf).file, f_leaf);
    assert_eq!(tree.module(m_leaf).vis, VisKind::Private);

    let m_type = tree.find_by_path("crate::r#type").unwrap();
    assert_eq!(tree.module(m_type).file, f_type);
    assert_eq!(tree.crate_path(m_type), "crate::type");
}

#[test]
fn matches_rustc_dir_ownership_for_explicit_path_attributes() {
    let db = make_db(&[], &["unix"]);
    let mut src = MemSourceDb::new();
    let root = src.add_file(
        "src/lib.rs",
        r#"
        #[cfg(unix)]
        #[path = "sys/unix.rs"]
        pub mod sys;
        pub mod non_mod;
        #[path = "os"]
        pub mod inline_os {
            pub mod linux;
        }
        "#,
    );
    let f_unix = src.add_file("src/sys/unix.rs", "pub mod net;\n");
    let f_net = src.add_file("src/sys/net.rs", "pub struct Socket;\n");
    let f_non_mod = src.add_file(
        "src/non_mod.rs",
        r#"
        #[path = "sibling_custom.rs"]
        pub mod direct_path;
        pub mod inner_inline {
            #[path = "deep_custom.rs"]
            pub mod deep;
        }
        "#,
    );
    let f_sibling = src.add_file("src/sibling_custom.rs", "pub fn a() {}\n");
    let f_deep = src.add_file("src/non_mod/inner_inline/deep_custom.rs", "pub fn b() {}\n");
    let f_linux = src.add_file("src/os/linux.rs", "pub fn c() {}\n");

    let tree = ModuleTree::build(&db, &src, CrateId(0), root);
    assert!(tree.diagnostics.is_empty(), "diags: {:?}", tree.diagnostics);

    assert_eq!(tree.module(tree.find_by_path("crate::sys").unwrap()).file, f_unix);
    assert_eq!(tree.module(tree.find_by_path("crate::sys::net").unwrap()).file, f_net);
    assert_eq!(tree.module(tree.find_by_path("crate::non_mod").unwrap()).file, f_non_mod);
    assert_eq!(tree.module(tree.find_by_path("crate::non_mod::direct_path").unwrap()).file, f_sibling);
    assert_eq!(tree.module(tree.find_by_path("crate::non_mod::inner_inline::deep").unwrap()).file, f_deep);
    assert_eq!(tree.module(tree.find_by_path("crate::inline_os::linux").unwrap()).file, f_linux);
}

#[test]
fn evaluates_cfg_cfg_attr_inner_cfg_and_prelude_flags() {
    let db = make_db(&["rt", "net"], &["unix"]);
    let mut src = MemSourceDb::new();
    let root = src.add_file(
        "src/lib.rs",
        r##"
        #![cfg_attr(not(feature = "std"), no_std)]
        #[cfg(all(unix, any(feature = "rt", feature = " bogus ")))]
        pub mod active;
        #[cfg(all(unix, feature = "missing"))]
        pub mod skipped;
        #[cfg_attr(unix, path = r#"alt_net.rs"#)]
        pub mod net;
        pub mod inner_disabled_file;
        pub mod inner_disabled_inline {
            #![cfg(false)]
            pub mod unreachable;
        }
        #[no_implicit_prelude]
        pub mod bare_inline {}
        "##,
    );
    let f_active = src.add_file("src/active.rs", "pub fn run() {}\n");
    let f_net = src.add_file("src/alt_net.rs", "#![no_implicit_prelude]\n");
    let _f_disabled = src.add_file("src/inner_disabled_file.rs", "#![cfg(windows)]\n");

    let tree = ModuleTree::build(&db, &src, CrateId(0), root);
    assert!(tree.diagnostics.is_empty(), "diags: {:?}", tree.diagnostics);
    assert!(tree.no_std);
    assert_eq!(tree.module(tree.find_by_path("crate::active").unwrap()).file, f_active);
    assert!(tree.find_by_path("crate::skipped").is_none());
    let m_net = tree.find_by_path("crate::net").unwrap();
    assert_eq!(tree.module(m_net).file, f_net);
    assert!(tree.module(m_net).has_no_implicit_prelude());
    assert!(tree.find_by_path("crate::inner_disabled_file").is_none());
    assert!(tree.find_by_path("crate::inner_disabled_inline").is_none());
    assert!(tree.module(tree.find_by_path("crate::bare_inline").unwrap()).has_no_implicit_prelude());
}

#[test]
fn expands_macro_use_cfg_wrappers_cfg_if_and_external_db_macros() {
    let mut db = make_db(&["fs", "rt"], &["unix"]);
    db.add_macro_rules(CrateId(0), "cfg::ext_gate", "{ ($($tt:tt)*) => { $($tt)* } }");

    let mut src = MemSourceDb::new();
    let root = src.add_file(
        "src/lib.rs",
        r#"
        #[macro_use]
        mod macros;
        #[macro_use(keep_me)]
        mod filtered {
            macro_rules! keep_me { ($i:item) => { $i }; }
            macro_rules! drop_me { ($i:item) => { $i }; }
        }

        cfg_fs! {
            pub mod fs;
        }
        cfg_if! {
            if #[cfg(windows)] {
                pub mod win_only;
            } else if #[cfg(unix)] {
                pub mod unix_only;
            } else {
                pub mod fallback;
            }
        }
        keep_me! { pub mod kept; }
        drop_me! { pub mod dropped; }
        cfg::ext_gate! { pub mod gated_ext; }
        "#,
    );
    let _f_macros = src.add_file(
        "src/macros.rs",
        r#"
        macro_rules! cfg_fs {
            ($($item:item)*) => {
                $( #[cfg(feature = "fs")] $item )*
            }
        }
        macro_rules! cfg_if {
            (if #[cfg($meta:meta)] { $($it:item)* } $( else if #[cfg($em:meta)] { $($eit:item)* } )* $( else { $($fit:item)* } )?) => {
                #[cfg($meta)]
                cfg_if! { @__items () ; $($it)* }
                $(
                    #[cfg(all(not($meta), $em))]
                    cfg_if! { @__items () ; $($eit)* }
                )*
            };
            (@__items ($($ _m:meta),*) ; $($it:item)*) => { $($it)* };
        }
        "#,
    );
    let f_fs = src.add_file("src/fs.rs", "");
    let f_unix = src.add_file("src/unix_only.rs", "");
    let f_kept = src.add_file("src/kept.rs", "");
    let f_ext = src.add_file("src/gated_ext.rs", "");

    let tree = ModuleTree::build(&db, &src, CrateId(0), root);
    assert!(tree.diagnostics.is_empty(), "diags: {:?}", tree.diagnostics);
    assert_eq!(tree.module(tree.find_by_path("crate::fs").unwrap()).file, f_fs);
    assert_eq!(tree.module(tree.find_by_path("crate::unix_only").unwrap()).file, f_unix);
    assert!(tree.find_by_path("crate::win_only").is_none());
    assert_eq!(tree.module(tree.find_by_path("crate::kept").unwrap()).file, f_kept);
    assert!(tree.find_by_path("crate::dropped").is_none());
    assert_eq!(tree.module(tree.find_by_path("crate::gated_ext").unwrap()).file, f_ext);
}

#[test]
fn reports_ambiguous_missing_cyclic_duplicate_and_offset_lookup() {
    let db = make_db(&[], &[]);
    let mut src = MemSourceDb::new();
    let root_text = concat!(
        "pub mod both;\n",
        "pub mod missing;\n",
        "#[path = \"lib.rs\"] pub mod cycle;\n",
        "pub mod inline_a {\n    pub mod inline_b {\n        pub fn here() {}\n    }\n}\n",
        "pub mod inline_a {}\n",
    );
    let root = src.add_file("src/lib.rs", root_text);
    let f_both_rs = src.add_file("src/both.rs", "");
    let f_both_mod = src.add_file("src/both/mod.rs", "");

    let parsed_root = parse(src.file_text(root));
    let parsed_both = parse(src.file_text(f_both_rs));
    let tree = ModuleTree::build_cached(&db, &src, CrateId(0), root, |fid| {
        if fid == root {
            Some(&parsed_root)
        } else if fid == f_both_rs {
            Some(&parsed_both)
        } else {
            None
        }
    });

    assert!(tree.diagnostics.contains(&ModDiag::AmbiguousModFile { parent: ModuleId::ROOT, name: "both".to_string(), chosen: f_both_rs, alternate: f_both_mod }));
    assert!(tree.diagnostics.contains(&ModDiag::UnresolvedModFile {
        parent: ModuleId::ROOT,
        name: "missing".to_string(),
        tried_primary: "src/missing.rs".to_string(),
        tried_mod_rs: Some("src/missing/mod.rs".to_string()),
    }));
    assert!(tree.diagnostics.contains(&ModDiag::CyclicModFile { parent: ModuleId::ROOT, name: "cycle".to_string(), file: root }));
    assert!(tree.diagnostics.contains(&ModDiag::DuplicateModDecl { parent: ModuleId::ROOT, name: "inline_a".to_string() }));

    assert!(tree.module(tree.find_by_path("crate::missing").unwrap()).is_unresolved());
    let here_off = root_text.find("here").unwrap() as u32;
    let m_at = tree.module_at_offset(root, here_off).unwrap();
    assert_eq!(tree.crate_path(m_at), "crate::inline_a::inline_b");
    assert_eq!(tree.module_at_offset(root, 2), Some(ModuleId::ROOT));
}

#[test]
fn survives_broken_overlay_syntax_without_panicking() {
    let db = make_db(&[], &[]);
    let mut src = MemSourceDb::new();
    let root = src.add_file("src/lib.rs", "#[cfg(all(unix, \n #[path = \"unterminated] mod ; mod ok { mod inner; ");
    let tree = ModuleTree::build(&db, &src, CrateId(0), root);
    assert_eq!(tree.module(ModuleId::ROOT).file, root);
}
