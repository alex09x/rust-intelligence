/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

#[test]
fn test_map_def_kind() {
    assert_eq!(map_def_kind(hir::def::DefKind::Mod), RDefKind::Mod);
    assert_eq!(map_def_kind(hir::def::DefKind::Struct), RDefKind::Struct);
    assert_eq!(map_def_kind(hir::def::DefKind::Union), RDefKind::Union);
    assert_eq!(map_def_kind(hir::def::DefKind::Enum), RDefKind::Enum);
    assert_eq!(map_def_kind(hir::def::DefKind::Variant), RDefKind::Variant);
    assert_eq!(map_def_kind(hir::def::DefKind::Trait), RDefKind::Trait);
    assert_eq!(map_def_kind(hir::def::DefKind::TyAlias), RDefKind::TyAlias);
    assert_eq!(
        map_def_kind(hir::def::DefKind::ForeignTy),
        RDefKind::ForeignTy
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::TraitAlias),
        RDefKind::TraitAlias
    );
    assert_eq!(map_def_kind(hir::def::DefKind::AssocTy), RDefKind::AssocTy);
    assert_eq!(map_def_kind(hir::def::DefKind::TyParam), RDefKind::TyParam);
    assert_eq!(map_def_kind(hir::def::DefKind::Fn), RDefKind::Fn);
    assert_eq!(
        map_def_kind(hir::def::DefKind::Const {
            is_type_const: false
        }),
        RDefKind::Const
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::ConstParam),
        RDefKind::ConstParam
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::Static {
            mutability: hir::Mutability::Not,
            safety: hir::Safety::Safe,
            nested: false
        }),
        RDefKind::Static
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::Ctor(
            hir::def::CtorOf::Struct,
            hir::def::CtorKind::Fn
        )),
        RDefKind::StructCtor
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::Ctor(
            hir::def::CtorOf::Variant,
            hir::def::CtorKind::Fn
        )),
        RDefKind::VariantCtor
    );
    assert_eq!(map_def_kind(hir::def::DefKind::AssocFn), RDefKind::AssocFn);
    assert_eq!(
        map_def_kind(hir::def::DefKind::AssocConst {
            is_type_const: false
        }),
        RDefKind::AssocConst
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::Macro(
            rustc_span::hygiene::MacroKind::Bang.into()
        )),
        RDefKind::Macro
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::ExternCrate),
        RDefKind::ExternCrate
    );
    assert_eq!(map_def_kind(hir::def::DefKind::Use), RDefKind::Use);
    assert_eq!(
        map_def_kind(hir::def::DefKind::ForeignMod),
        RDefKind::ForeignMod
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::AnonConst),
        RDefKind::AnonConst
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::OpaqueTy),
        RDefKind::OpaqueTy
    );
    assert_eq!(map_def_kind(hir::def::DefKind::Field), RDefKind::Field);
    assert_eq!(
        map_def_kind(hir::def::DefKind::LifetimeParam),
        RDefKind::LifetimeParam
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::GlobalAsm),
        RDefKind::GlobalAsm
    );
    assert_eq!(
        map_def_kind(hir::def::DefKind::Impl { of_trait: false }),
        RDefKind::Impl
    );
    assert_eq!(map_def_kind(hir::def::DefKind::Closure), RDefKind::Closure);
    assert_eq!(
        map_def_kind(hir::def::DefKind::SyntheticCoroutineBody),
        RDefKind::Other
    );
}

#[test]
fn test_extract_target_disambiguator() {
    assert_eq!(
        extract_target_disambiguator(&["-Cextra-filename=123".to_string()]),
        Some("123".to_string())
    );
    assert_eq!(
        extract_target_disambiguator(&["-C".to_string(), "extra-filename=456".to_string()]),
        Some("456".to_string())
    );
    assert_eq!(
        extract_target_disambiguator(&["--crate-type=lib".to_string()]),
        Some("_lib".to_string())
    );
    assert_eq!(
        extract_target_disambiguator(&["--crate-type".to_string(), "bin".to_string()]),
        Some("_bin".to_string())
    );
    assert_eq!(
        extract_target_disambiguator(&["-Cextra-filename=".to_string()]),
        None
    );
    assert_eq!(
        extract_target_disambiguator(&["-C".to_string(), "extra-filename=".to_string()]),
        None
    );
    assert_eq!(
        extract_target_disambiguator(&["--crate-type=".to_string()]),
        None
    );
    assert_eq!(
        extract_target_disambiguator(&["--crate-type".to_string(), "".to_string()]),
        Some("_".to_string())
    );
    assert_eq!(extract_target_disambiguator(&["rustc".to_string()]), None);
}

#[test]
fn test_hash_helpers() {
    assert_eq!(hash_directory_contents(Path::new("/nonexistent_dir")), None);
    assert_eq!(hash_file_contents(Path::new("/nonexistent_file")), None);

    let temp_dir = std::env::temp_dir().join(format!("ridx_hash_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let f1 = temp_dir.join("a.txt");
    let _ = fs::write(&f1, b"hello");
    assert!(hash_file_contents(&f1).is_some());
    assert!(hash_directory_contents(&temp_dir).is_some());
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_compute_dependency_unit_key() {
    let key1 = compute_dependency_unit_key(
        "my_crate",
        "0.1.0",
        Some("abc"),
        Some(Path::new("/path")),
        &[
            "--target".to_string(),
            "x86_64-unknown-linux-gnu".to_string(),
        ],
    );
    let key2 = compute_dependency_unit_key(
        "my_crate",
        "0.2.0",
        Some("abc"),
        Some(Path::new("/path")),
        &[
            "--target".to_string(),
            "x86_64-unknown-linux-gnu".to_string(),
        ],
    );
    assert_eq!(key1.len(), 16);
    assert_eq!(key2.len(), 16);
    assert_ne!(key1, key2);

    let temp_dir = std::env::temp_dir().join(format!("ridx_key_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let ext_file = temp_dir.join("dep.rmeta");
    fs::write(&ext_file, b"rmeta_bytes").unwrap();

    std::env::set_var("OUT_DIR", &temp_dir);
    let key3 = compute_dependency_unit_key(
        "my_crate",
        "0.1.0",
        None,
        None,
        &[
            "--target=aarch64-apple-darwin".to_string(),
            "--cfg".to_string(),
            "feat_a".to_string(),
            "--cfg=feat_b".to_string(),
            "--extern".to_string(),
            format!("dep={}", ext_file.display()),
            "--extern=plain_dep".to_string(),
            "--extern=missing_dep=/nonexistent/path.rmeta".to_string(),
        ],
    );
    assert_eq!(key3.len(), 16);
    std::env::remove_var("OUT_DIR");
    let _ = fs::remove_dir_all(&temp_dir);
}

static DRIVER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_run_driver_queries() {
    let _ = run_driver(vec!["ridx-driver".into(), "--version".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "--help".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "-vV".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "-V".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "-h".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "--print".into(), "cfg".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "--print=cfg".into()]);
    let _ = run_driver(vec![
        "ridx-driver".into(),
        "foo-driver".into(),
        "rustc".into(),
        "--version".into(),
    ]);
    let _ = run_driver(vec![
        "ridx-driver".into(),
        "--crate-name".into(),
        "build_script_build".into(),
    ]);
    let _ = run_driver(vec![
        "ridx-driver".into(),
        "rustc".into(),
        "--crate-name".into(),
        "build_script_main".into(),
    ]);
}

#[test]
fn test_driver_index_compilation() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    std::env::remove_var("CARGO_MANIFEST_DIR");
    std::env::remove_var("CARGO_PKG_VERSION");
    std::env::remove_var("CARGO_HOME");

    let temp_dir =
        std::env::temp_dir().join(format!("ridx_driver_compile_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("lib.rs");
    let code = r#"
pub mod sub {
    pub fn sub_fn() -> i32 { 42 }
}
pub struct Point { pub x: i32, pub y: i32 }
pub struct TuplePt(pub i32, pub i32);
pub enum Status { Active, Inactive(u32), Detailed { code: u8 } }
pub union RawVal { pub f: f32, pub u: u32 }
pub trait Greeter {
    type Output;
    const GREETING: &'static str = "hi";
    fn greet(&self) -> String;
    fn default_greeting(&self) -> &'static str { Self::GREETING }
}
impl Greeter for Point {
    type Output = String;
    fn greet(&self) -> String { format!("{}:{}", self.x, self.y) }
}
pub const C: i32 = 123;
pub static S: &str = "abc";
pub type Pt = Point;
macro_rules! my_add {
    ($a:expr, $b:expr) => { $a + $b };
}
pub fn compute<T: Greeter>(p: &Point, g: &T) -> i32 {
    let mut val = my_add!(p.x, p.y) + C;
    let tp = TuplePt(1, 2);
    val += tp.0 + tp.1;
    let cl = |a: i32| a * 2;
    val += cl(sub::sub_fn());
    let _ = g.greet();
    let _ = g.default_greeting();
    let st = Status::Detailed { code: 7 };
    match st {
        Status::Active => 1,
        Status::Inactive(n) => n as i32,
        Status::Detailed { code } => code as i32,
    };
    val
}
"#;
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let rmeta_file = temp_dir.join("libtest_sample.rmeta");

    let sysroot = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "test_sample".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    std::env::set_var("RIDX_OUT_DIR", &out_dir);
    let _ = run_driver(args);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_is_immutable_package() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    assert!(is_immutable_package(Path::new(
        "/home/user/.cargo/registry/src/github.com-1ecc6299db9ec823/foo-0.1.0"
    )));
    assert!(is_immutable_package(Path::new(
        "/home/user/.cargo/git/checkouts/bar-12345/abc"
    )));
    assert!(!is_immutable_package(Path::new(
        "/home/user/project/my_crate"
    )));

    std::env::set_var("CARGO_HOME", "/custom/cargo");
    assert!(is_immutable_package(Path::new(
        "/custom/cargo/registry/foo"
    )));
    assert!(is_immutable_package(Path::new("/custom/cargo/git/foo")));
    assert!(!is_immutable_package(Path::new("/custom/cargo/other/foo")));
    std::env::remove_var("CARGO_HOME");
}

#[test]
fn test_driver_immutable_cache_hit_and_extern() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_cache_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let reg_dir = temp_dir.join(".cargo/registry/src/dep_pkg");
    fs::create_dir_all(&reg_dir).unwrap();
    let src_file = reg_dir.join("lib.rs");
    let code = "pub fn dep_calc(x: i32) -> i32 { x + 42 }\n";
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let deps_dir = out_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mock_dep_json = serde_json::json!({
        "name": "mock_dep",
        "unit_key": "mock_unit_1234",
        "generation": "mock_gen_5678"
    });
    fs::write(
        deps_dir.join("mock_dep.json"),
        serde_json::to_string(&mock_dep_json).unwrap(),
    )
    .unwrap();

    let core_dep_json = serde_json::json!({
        "name": "core",
        "unit_key": "core_unit_1234",
        "generation": "core_gen_5678"
    });
    fs::write(
        deps_dir.join("core.json"),
        serde_json::to_string(&core_dep_json).unwrap(),
    )
    .unwrap();

    let alias_dep_json = serde_json::json!({
        "name": "alias_dep",
        "unit_key": "alias_unit_9999",
        "generation": "alias_gen_9999"
    });
    fs::write(
        deps_dir.join("alias_dep.json"),
        serde_json::to_string(&alias_dep_json).unwrap(),
    )
    .unwrap();

    let rmeta_file = temp_dir.join("libdep_pkg.rmeta");
    let sysroot = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let mock_src = temp_dir.join("mock_dep.rs");
    fs::write(&mock_src, "pub fn mock() {}\n").unwrap();
    let mock_rmeta = temp_dir.join("libmock_dep.rmeta");
    let mock_args = vec![
        "rustc".to_string(),
        mock_src.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "mock_dep".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        mock_rmeta.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str.clone(),
    ];
    std::env::set_var("RIDX_OUT_DIR", &out_dir);
    std::env::set_var("CARGO_MANIFEST_DIR", &reg_dir);
    std::env::set_var("CARGO_PKG_VERSION", "0.1.0");
    let _ = run_driver(mock_args);

    let core_json = serde_json::json!({
        "name": "core",
        "unit_key": "core_unit_test_123",
        "generation": "core_gen_test"
    });
    let _ = fs::write(
        deps_dir.join("core.json"),
        serde_json::to_string(&core_json).unwrap(),
    );

    let code =
        "extern crate aliased_mock as mock_dep; pub fn dep_calc(x: i32) -> i32 { mock_dep::mock(); x + 42 }\n";
    fs::write(&src_file, code).unwrap();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dep_pkg".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
        "--extern".to_string(),
        format!("aliased_mock={}", mock_rmeta.display()),
        "--extern".to_string(),
        "core".to_string(),
    ];

    // Run 1: Cache miss
    let res_d1 = run_driver(args.clone());
    assert!(res_d1.is_ok());

    let cache_dir = ridx_format::resolve_cache_dir();
    let disambiguator = extract_target_disambiguator(&args);
    let unit_key = compute_dependency_unit_key(
        "dep_pkg",
        "0.1.0",
        disambiguator.as_deref(),
        Some(&reg_dir),
        &args,
    );
    let obj_path = ridx_format::cache_object_path(&cache_dir, &unit_key);
    assert!(obj_path.is_file());

    // Run 2: Cache hit (lines 773-785)
    let _ = run_driver(args.clone());

    // Run 3: Cache hit with invalid reader (lines 787-791)
    if obj_path.is_file() {
        fs::write(&obj_path, b"NOT_A_VALID_RIDX_INDEX").unwrap();
        let _ = run_driver(args.clone());
    }

    // Run 4: Dependency atomic rename failure (lines 864-869)
    let _ = fs::remove_file(&obj_path);
    let conflict_dep_dir = obj_path.join("nested");
    if fs::create_dir_all(&conflict_dep_dir).is_ok() {
        let _ = run_driver(args.clone());
        let _ = fs::remove_dir_all(&obj_path);
    }

    // Run 5: Temporary file create error for dependency (lines 904-909)
    let tmp_file_path = obj_path.with_extension(format!("tmp.{}", std::process::id()));
    if fs::create_dir_all(tmp_file_path.join("nested")).is_ok() {
        let _ = run_driver(args);
        let _ = fs::remove_dir_all(&tmp_file_path);
    }

    std::env::remove_var("CARGO_MANIFEST_DIR");
    std::env::remove_var("CARGO_PKG_VERSION");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_driver_workspace_crate() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_ws_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let ws_dir = temp_dir.join("ws_pkg");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = r#"
pub struct Point { pub x: i32, pub y: i32 }
pub fn add_point(p: Point) -> i32 { p.x + p.y }
"#;
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let deps_dir = out_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let core_dep_json = serde_json::json!({
        "name": "core_renamed",
        "unit_key": "core_unit_ws_1234",
        "generation": "core_gen_ws_5678"
    });
    fs::write(
        deps_dir.join("core.json"),
        serde_json::to_string(&core_dep_json).unwrap(),
    )
    .unwrap();

    let rmeta_file = temp_dir.join("libws_pkg.rmeta");
    let sysroot = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_pkg".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-Cextra-filename=custom123".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
        "--extern".to_string(),
        "core".to_string(),
    ];

    std::env::set_var("RIDX_OUT_DIR", &out_dir);
    std::env::set_var("CARGO_MANIFEST_DIR", &ws_dir);
    std::env::set_var("CARGO_PKG_VERSION", "0.2.0");

    let res = run_driver(args.clone());
    assert!(res.is_ok());

    let out_ridx = out_dir.join("ws_pkg_custom123.ridx");
    assert!(out_ridx.is_file(), "expected {:?} to exist", out_ridx);
    let out_dep = deps_dir.join("ws_pkg_custom123.json");
    assert!(out_dep.is_file(), "expected {:?} to exist", out_dep);

    // Regression test for def_ident_span:
    // "pub struct Point" -> col 12 (1-based), not 1
    // "pub fn add_point" -> col 8 (1-based), not 1
    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();
    let defs = reader.definitions();
    let point_def = defs
        .iter()
        .find(|d| reader.get_str(d.name_offset) == "Point")
        .expect("Point def");
    assert_eq!(
        point_def.span.start_col(),
        12,
        "Point definition must point to identifier name, not pub keyword"
    );
    let fn_def = defs
        .iter()
        .find(|d| reader.get_str(d.name_offset) == "add_point")
        .expect("add_point def");
    assert_eq!(
        fn_def.span.start_col(),
        8,
        "add_point definition must point to identifier name, not pub fn keyword"
    );

    // Test out_dir creation failure (lines 916-917)
    let not_a_dir = temp_dir.join("file_not_dir");
    fs::write(&not_a_dir, b"file").unwrap();
    std::env::set_var("RIDX_OUT_DIR", &not_a_dir);
    let _ = run_driver(args.clone());
    std::env::set_var("RIDX_OUT_DIR", &out_dir);

    // Test workspace crate atomic rename failure (lines 958-963)
    let _ = fs::remove_file(&out_ridx);
    let conflict_dir = out_ridx.join("nested");
    if fs::create_dir_all(&conflict_dir).is_ok() {
        let _ = run_driver(args.clone());
        let _ = fs::remove_dir_all(&out_ridx);
    }

    // Test workspace crate temp file creation failure (lines 996-1001)
    let tmp_file_path = out_dir.join(format!("ws_pkg_custom123.ridx.tmp.{}", std::process::id()));
    if fs::create_dir_all(tmp_file_path.join("nested")).is_ok() {
        let _ = run_driver(args.clone());
        let _ = fs::remove_dir_all(&tmp_file_path);
    }

    // Also test driver wrapper stripping and query flags
    let query_args = vec![
        "ridx-driver".to_string(),
        "rustc".to_string(),
        "-vV".to_string(),
    ];
    assert!(run_driver(query_args).is_ok());

    // Also test help and print query flags
    assert!(run_driver(vec!["ridx-driver".to_string(), "--help".to_string()]).is_ok());
    assert!(run_driver(vec![
        "ridx-driver".to_string(),
        "--print=sysroot".to_string()
    ])
    .is_ok());

    // Also test build script skipping
    let mut bs_args = args.clone();
    if let Some(pos) = bs_args.iter().position(|a| a == "ws_pkg") {
        bs_args[pos] = "build_script_build".to_string();
    }
    assert!(run_driver(bs_args).is_ok());

    // Disambiguator extraction edge cases (lines 325-348)
    assert_eq!(
        extract_target_disambiguator(&["-C".to_string(), "extra-filename=bar".to_string()]),
        Some("bar".to_string())
    );
    assert_eq!(
        extract_target_disambiguator(&["-C".to_string(), "extra-filename=".to_string()]),
        None
    );
    assert_eq!(
        extract_target_disambiguator(&["-Cextra-filename=".to_string()]),
        None
    );
    assert_eq!(
        extract_target_disambiguator(&["--crate-type=rlib".to_string()]),
        Some("_rlib".to_string())
    );
    assert_eq!(
        extract_target_disambiguator(&["--crate-type".to_string(), "dylib".to_string()]),
        Some("_dylib".to_string())
    );

    std::env::remove_var("CARGO_MANIFEST_DIR");
    std::env::remove_var("CARGO_PKG_VERSION");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_hash_directory_source_with_subdirs() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_hash_dir_{}", std::process::id()));
    let sub = temp_dir.join("sub");
    let _ = fs::create_dir_all(&sub);
    let nested_file = sub.join("nested.rs");
    fs::write(&nested_file, b"pub fn nested() {}").unwrap();
    let h = hash_directory_contents(&temp_dir);
    assert!(h.is_some());
    assert!(hash_directory_contents(&nested_file).is_none());
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_driver_main_and_disambiguator_extra() {
    // Disambiguator -C without extra-filename (line 335)
    assert_eq!(
        extract_target_disambiguator(&["-C".to_string(), "opt-level=3".to_string()]),
        None
    );

    // Driver main entrypoint (lines 1071-1074)
    main();
}

#[test]
fn test_driver_cas_rebuild_on_driver_version_change() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_drv_cas_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let cache_dir = temp_dir.join("cache");
    let ws_dir = temp_dir.join("dep_crate");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    fs::write(&src_file, b"pub fn helper() -> i32 { 42 }").unwrap();

    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let deps_dir = out_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let rmeta_file = temp_dir.join("libdep_crate.rmeta");
    let sysroot = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "dep_crate".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    std::env::set_var("RIDX_OUT_DIR", &out_dir);
    std::env::set_var("RIDX_CACHE_DIR", &cache_dir);
    // Point manifest_dir to a path containing .cargo/registry so is_immutable = true
    let reg_dir = temp_dir
        .join(".cargo")
        .join("registry")
        .join("src")
        .join("dep_crate-1.0.0");
    fs::create_dir_all(&reg_dir).unwrap();
    std::env::set_var("CARGO_MANIFEST_DIR", &reg_dir);
    std::env::set_var("CARGO_PKG_VERSION", "1.0.0");

    // 1. Build initial shard with driver hash v1
    std::env::set_var("RIDX_DRIVER_HASH", "0xaaaa1111");
    let res1 = run_driver(args.clone());
    assert!(res1.is_ok());

    let disam = extract_target_disambiguator(&args);
    let unit1 = compute_dependency_unit_key(
        "dep_crate",
        "1.0.0",
        disam.as_deref(),
        Some(&reg_dir),
        &args,
    );
    let shard1_path = ridx_format::cache_object_path(&cache_dir, &unit1);
    assert!(
        shard1_path.is_file(),
        "expected shard 1 at {:?}",
        shard1_path
    );
    let reader1 = ridx_format::IndexReader::open(&shard1_path).unwrap();
    assert_eq!(reader1.unit_key(), unit1);

    // 2. Change driver hash to v2 -> must NOT hit shard 1, must build shard 2
    std::env::set_var("RIDX_DRIVER_HASH", "0xbbbb2222");
    let unit2 = compute_dependency_unit_key(
        "dep_crate",
        "1.0.0",
        disam.as_deref(),
        Some(&reg_dir),
        &args,
    );
    assert_ne!(
        unit1, unit2,
        "unit keys must differ between driver versions"
    );

    let shard2_path = ridx_format::cache_object_path(&cache_dir, &unit2);
    assert!(
        !shard2_path.exists(),
        "shard 2 must not exist before rebuild"
    );

    let res2 = run_driver(args.clone());
    assert!(res2.is_ok());
    assert!(
        shard2_path.is_file(),
        "expected shard 2 to be rebuilt at {:?}",
        shard2_path
    );
    let reader2 = ridx_format::IndexReader::open(&shard2_path).unwrap();
    assert_eq!(reader2.unit_key(), unit2);

    // 3. Stale file mismatch check: write corrupted/mismatched data into shard2_path
    // When driver runs again, it must detect the mismatch, remove the stale file, and rebuild
    fs::write(&shard2_path, b"corrupted header bytes").unwrap();
    let res3 = run_driver(args.clone());
    assert!(res3.is_ok());
    assert!(
        shard2_path.is_file(),
        "expected shard 2 to be recovered and rebuilt"
    );
    let reader3 = ridx_format::IndexReader::open(&shard2_path).unwrap();
    assert_eq!(reader3.unit_key(), unit2);

    std::env::remove_var("RIDX_DRIVER_HASH");
    std::env::remove_var("RIDX_CACHE_DIR");
    std::env::remove_var("CARGO_MANIFEST_DIR");
    std::env::remove_var("CARGO_PKG_VERSION");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_driver_generic_trait_bound_owner() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_gen_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let ws_dir = temp_dir.join("ws_gen");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = r#"
pub trait Visitor {
    fn visit_bool(&mut self, v: bool);
}
pub fn test_generic<V: Visitor>(mut visitor: V) {
    visitor.visit_bool(true);
}
"#;
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let deps_dir = out_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let rmeta_file = temp_dir.join("libws_gen.rmeta");
    let sysroot = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_gen".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    std::env::set_var("RIDX_OUT_DIR", &out_dir);
    std::env::set_var("CARGO_MANIFEST_DIR", &ws_dir);
    std::env::set_var("CARGO_PKG_VERSION", "0.1.0");

    let res = run_driver(args);
    assert!(res.is_ok());

    let out_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "ridx"))
        .expect("expected .ridx index to exist in out_dir");

    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();
    let types = reader.types();
    let visitor_types: Vec<_> = types
        .iter()
        .filter(|t| reader.get_str(t.type_offset) == "V")
        .collect();
    assert!(!visitor_types.is_empty(), "expected at least one type entry for V");
    let has_owner = visitor_types
        .iter()
        .any(|t| reader.get_str(t.owner_path_offset) == "Visitor");
    assert!(
        has_owner,
        "generic parameter V must resolve owner to Visitor, got {:?}",
        visitor_types
            .iter()
            .map(|t| reader.get_str(t.owner_path_offset))
            .collect::<Vec<_>>()
    );

    std::env::remove_var("RIDX_OUT_DIR");
    std::env::remove_var("CARGO_MANIFEST_DIR");
    std::env::remove_var("CARGO_PKG_VERSION");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_driver_generic_supertrait_and_where_clause() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_gen2_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let ws_dir = temp_dir.join("ws_gen2");
    fs::create_dir_all(&ws_dir).unwrap();
    let src_file = ws_dir.join("lib.rs");
    let code = r#"
pub mod de {
    pub trait Visitor {
        fn visit_str(&mut self, s: &str);
    }
}
pub fn test_where<V>(mut v: V) where V: de::Visitor {
    v.visit_str("hello");
}

pub trait BaseTrait {
    fn base_method(&self);
}
pub trait SubTrait: BaseTrait {
    fn sub_method(&self);
}
pub fn test_sub<S: SubTrait>(s: S) {
    s.sub_method();
}
"#;
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let deps_dir = out_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let rmeta_file = temp_dir.join("libws_gen2.rmeta");
    let sysroot = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "ws_gen2".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    std::env::set_var("RIDX_OUT_DIR", &out_dir);
    std::env::set_var("CARGO_MANIFEST_DIR", &ws_dir);
    std::env::set_var("CARGO_PKG_VERSION", "0.1.0");

    let res = run_driver(args);
    assert!(res.is_ok());

    let out_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "ridx"))
        .expect("expected .ridx index to exist in out_dir");

    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();

    // 1. Where clause: V bounded by de::Visitor
    let types = reader.types();
    let v_types: Vec<_> = types
        .iter()
        .filter(|t| reader.get_str(t.type_offset) == "V")
        .collect();
    assert!(!v_types.is_empty(), "expected type entry for V");
    assert!(
        v_types.iter().any(|t| reader.get_str(t.owner_path_offset) == "de::Visitor"),
        "where-clause generic V must resolve owner to de::Visitor, got {:?}",
        v_types
            .iter()
            .map(|t| reader.get_str(t.owner_path_offset))
            .collect::<Vec<_>>()
    );

    // 2. Generic bound S: SubTrait
    let s_types: Vec<_> = types
        .iter()
        .filter(|t| reader.get_str(t.type_offset) == "S")
        .collect();
    assert!(!s_types.is_empty(), "expected type entry for S");
    assert!(
        s_types.iter().any(|t| reader.get_str(t.owner_path_offset) == "SubTrait"),
        "generic parameter S must resolve owner to SubTrait"
    );

    // 3. Supertrait deref chain: SubTrait -> BaseTrait
    let target = reader.find_deref_target("SubTrait");
    assert_eq!(
        target,
        Some("BaseTrait"),
        "SubTrait must declare deref target to BaseTrait"
    );

    std::env::remove_var("RIDX_OUT_DIR");
    std::env::remove_var("CARGO_MANIFEST_DIR");
    std::env::remove_var("CARGO_PKG_VERSION");
    let _ = fs::remove_dir_all(&temp_dir);
}
