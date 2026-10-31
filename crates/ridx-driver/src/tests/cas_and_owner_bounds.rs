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
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
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

    // Point manifest_dir to a path containing .cargo/registry so is_immutable = true
    let reg_dir = temp_dir.join(".cargo").join("registry").join("src").join("dep_crate-1.0.0");
    fs::create_dir_all(&reg_dir).unwrap();
    let mut env = DriverEnv::for_test(&out_dir);
    env.cache_dir = cache_dir.clone();
    env.manifest_dir = Some(reg_dir.clone());
    env.pkg_version = "1.0.0".to_string();

    // 1. Build initial shard with driver hash v1
    env.driver_hash = 0xaaaa_1111;
    let res1 = run_driver_with_env(args.clone(), env.clone());
    assert!(res1.is_ok());

    let disam = extract_target_disambiguator(&args);
    let unit1 = compute_dependency_unit_key_with_env("dep_crate", "1.0.0", disam.as_deref(), Some(&reg_dir), &args, None, env.driver_hash);
    let shard1_path = ridx_format::cache_object_path(&cache_dir, &unit1);
    assert!(shard1_path.is_file(), "expected shard 1 at {:?}", shard1_path);
    let reader1 = ridx_format::IndexReader::open(&shard1_path).unwrap();
    assert_eq!(reader1.unit_key(), unit1);

    // 2. Change driver hash to v2 -> must NOT hit shard 1, must build shard 2
    env.driver_hash = 0xbbbb_2222;
    let unit2 = compute_dependency_unit_key_with_env("dep_crate", "1.0.0", disam.as_deref(), Some(&reg_dir), &args, None, env.driver_hash);
    assert_ne!(unit1, unit2, "unit keys must differ between driver versions");

    let shard2_path = ridx_format::cache_object_path(&cache_dir, &unit2);
    assert!(!shard2_path.exists(), "shard 2 must not exist before rebuild");

    let res2 = run_driver_with_env(args.clone(), env.clone());
    assert!(res2.is_ok());
    assert!(shard2_path.is_file(), "expected shard 2 to be rebuilt at {:?}", shard2_path);
    let reader2 = ridx_format::IndexReader::open(&shard2_path).unwrap();
    assert_eq!(reader2.unit_key(), unit2);

    // 3. Stale file mismatch check: write corrupted/mismatched data into shard2_path
    // When driver runs again, it must detect the mismatch, remove the stale file, and rebuild
    fs::write(&shard2_path, b"corrupted header bytes").unwrap();
    let res3 = run_driver_with_env(args.clone(), env);
    assert!(res3.is_ok());
    assert!(shard2_path.is_file(), "expected shard 2 to be recovered and rebuilt");
    let reader3 = ridx_format::IndexReader::open(&shard2_path).unwrap();
    assert_eq!(reader3.unit_key(), unit2);

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
    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
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

    let mut env = DriverEnv::for_test(&out_dir);
    env.manifest_dir = Some(ws_dir.clone());
    env.pkg_version = "0.1.0".to_string();

    let res = run_driver_with_env(args, env);
    assert!(res.is_ok());

    let out_ridx = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "ridx"))
        .expect("expected .ridx index to exist in out_dir");

    let reader = ridx_format::IndexReader::open(&out_ridx).unwrap();
    let types = reader.types();
    let visitor_types: Vec<_> = types.iter().filter(|t| reader.get_str(t.type_offset) == "V").collect();
    assert!(!visitor_types.is_empty(), "expected at least one type entry for V");
    let has_owner = visitor_types.iter().any(|t| reader.get_str(t.owner_path_offset) == "Visitor");
    assert!(has_owner, "generic parameter V must resolve owner to Visitor, got {:?}", visitor_types.iter().map(|t| reader.get_str(t.owner_path_offset)).collect::<Vec<_>>());

    let _ = fs::remove_dir_all(&temp_dir);
}
