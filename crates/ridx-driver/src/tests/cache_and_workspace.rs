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
