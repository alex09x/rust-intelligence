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
use ridx_format::IndexReader;

#[test]
fn test_driver_records_active_cfgs_features_target_and_lock_hash_key() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_cfg_feat_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    let ws_dir = temp_dir.join("ws");
    let src_dir = ws_dir.join("src");
    let out_dir = temp_dir.join("out");
    let cache_dir = temp_dir.join("cache");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&out_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();

    fs::write(ws_dir.join("Cargo.lock"), "# lock v1\nversion = 3\n").unwrap();
    let lib_rs = src_dir.join("lib.rs");
    fs::write(
        &lib_rs,
        r#"
#[cfg(feature = "alpha")]
pub fn enabled_alpha() -> u32 { 1 }

#[cfg(feature = "disabled_gamma")]
pub fn never_compiled() -> u32 { 2 }

#[cfg(custom_build_flag)]
pub fn from_build_script_cfg() -> bool { true }
"#,
    )
    .unwrap();

    let sysroot_out = std::process::Command::new("rustc").arg("--print").arg("sysroot").output().expect("rustc sysroot");
    let sysroot_str = String::from_utf8(sysroot_out.stdout).unwrap().trim().to_string();

    let args_v1 = vec![
        "ridx-driver".to_string(),
        "rustc".to_string(),
        lib_rs.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "cfg_probe_ws".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--edition=2021".to_string(),
        "--cfg".to_string(),
        "feature=\"alpha\"".to_string(),
        "--cfg=feature=\"beta\"".to_string(),
        "--cfg".to_string(),
        "custom_build_flag".to_string(),
        "--emit=metadata".to_string(),
        "--out-dir".to_string(),
        temp_dir.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    let mut env = DriverEnv::for_test(out_dir.clone());
    env.cache_dir = cache_dir;
    env.manifest_dir = Some(ws_dir.clone());
    run_driver_with_env(args_v1.clone(), env.clone()).expect("driver run v1 failed");

    let ridx_path = out_dir.join("cfg_probe_ws_lib.ridx");
    let reader = IndexReader::open(&ridx_path).expect("open cfg_probe_ws_lib.ridx");
    assert!(!reader.target_triple().is_empty());
    let feats: Vec<&str> = reader.enabled_features().collect();
    assert_eq!(feats, vec!["alpha", "beta"]);
    assert!(reader.has_cfg_key_value("feature", "alpha"));
    assert!(reader.has_cfg_key_value("feature", "beta"));
    assert!(!reader.has_cfg_key_value("feature", "disabled_gamma"));
    assert!(reader.has_cfg_flag("custom_build_flag"));
    assert!(reader.cfg_values("target_pointer_width").len() >= 1);
    let def_names: Vec<&str> = reader.definitions().iter().map(|d| reader.get_str(d.name_offset())).collect();
    assert!(def_names.contains(&"enabled_alpha"), "expected enabled_alpha in {def_names:?}");
    assert!(!def_names.contains(&"never_compiled"), "disabled cfg item must not be indexed: {def_names:?}");
    assert!(def_names.contains(&"from_build_script_cfg"), "expected from_build_script_cfg in {def_names:?}");

    let uk_lock1 = reader.unit_key().to_string();
    drop(reader);

    // Changing Cargo.lock changes the workspace crate's unit_key
    fs::write(ws_dir.join("Cargo.lock"), "# lock v2 changed\nversion = 3\n").unwrap();
    let uk_lock2 = compute_dependency_unit_key("cfg_probe_ws", "0.1.0", Some("_lib"), Some(&ws_dir), &args_v1);
    assert_ne!(uk_lock1, uk_lock2, "Cargo.lock change must update workspace unit_key");

    // Changing enabled features or target changes unit_key
    let mut args_feat_changed = args_v1.clone();
    args_feat_changed.push("--cfg=feature=\"delta\"".to_string());
    let uk_feat = compute_dependency_unit_key("cfg_probe_ws", "0.1.0", Some("_lib"), Some(&ws_dir), &args_feat_changed);
    assert_ne!(uk_lock2, uk_feat, "feature change must update unit_key");

    let mut args_target_changed = args_v1;
    args_target_changed.push("--target=wasm32-unknown-unknown".to_string());
    let uk_target = compute_dependency_unit_key("cfg_probe_ws", "0.1.0", Some("_lib"), Some(&ws_dir), &args_target_changed);
    assert_ne!(uk_lock2, uk_target, "target change must update unit_key");

    let _ = fs::remove_dir_all(&temp_dir);
}
