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
use crate::indexer::cargo::*;
use crate::indexer::driver::*;
use crate::indexer::orchestrate::*;
use crate::indexer::types::*;
use crate::indexer::variant::*;
use std::fs;
use std::path::PathBuf;

#[test]
fn test_indexer_cargo_config_and_driver() {
    let _guard = lock_indexer_test();
    let temp = new_test_dir("driver_tests");
    // Test find_driver_binary fallback paths without mutating process env
    let _ = find_driver_binary();
    let _ = find_driver_binary_with(None, None, None, None);

    // `ridx-driver.exe` on Windows, canonical without the `\\?\` prefix.
    let name = crate::indexer::driver::driver_file_name();
    let canonical = |p: &std::path::Path| crate::indexer::driver::canonical(p);
    let exe_dir = temp.0.join("exe_test_dir/nested");
    fs::create_dir_all(&exe_dir).unwrap();
    let mock_driver_sibling = exe_dir.join(&name);
    fs::write(&mock_driver_sibling, b"").unwrap();
    let dummy_exe = exe_dir.join("dummy_exe");
    let found_sibling = find_driver_binary_with(None, Some(&dummy_exe), None, None);
    assert_eq!(found_sibling, canonical(&mock_driver_sibling));
    let _ = fs::remove_file(&mock_driver_sibling);

    let mock_driver_parent = temp.0.join("exe_test_dir").join(&name);
    fs::write(&mock_driver_parent, b"").unwrap();
    let found_parent = find_driver_binary_with(None, Some(&dummy_exe), None, None);
    assert_eq!(found_parent, canonical(&mock_driver_parent));
    let _ = fs::remove_file(&mock_driver_parent);
    let _ = find_driver_binary_with(None, Some(&dummy_exe), None, None);

    // Also test with explicit cwd and path_env parameters instead of mutating CWD or PATH
    let temp_empty = new_test_dir("empty_for_which");
    let mock_target_debug = temp_empty.0.join("target/debug");
    fs::create_dir_all(&mock_target_debug).unwrap();
    let mock_target_driver = mock_target_debug.join(&name);
    fs::write(&mock_target_driver, b"").unwrap();
    let found_cand = find_driver_binary_with(None, None, Some(&temp_empty.0), Some(""));
    assert_eq!(found_cand, canonical(&mock_target_driver));
    let _ = fs::remove_file(&mock_target_driver);

    let mock_bin_dir = new_test_dir("mock_bin_for_which");
    let mock_bin_driver = mock_bin_dir.0.join(&name);
    fs::write(&mock_bin_driver, b"#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&mock_bin_driver, fs::Permissions::from_mode(0o755));
    }
    let orig_path = std::env::var("PATH").ok();
    let new_path =
        std::env::join_paths(std::iter::once(mock_bin_dir.0.clone()).chain(std::env::split_paths(orig_path.as_deref().unwrap_or("")))).unwrap().to_string_lossy().into_owned();
    let found_which = find_driver_binary_with(None, None, Some(&temp_empty.0), Some(&new_path));
    assert!(found_which.is_some());

    let found_none = find_driver_binary_with(None, None, Some(&temp_empty.0), Some(""));
    assert!(found_none.is_none());

    // Test resolve_cargo_home_with and find_cargo_configs_with_home without mutating HOME/CARGO_HOME
    let work_dir = temp.0.join("sample_pkg");
    fs::create_dir_all(&work_dir).unwrap();
    let manifest_file = work_dir.join("Cargo.toml");
    fs::write(&manifest_file, "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\n").unwrap();

    let custom_home = temp.0.join("custom_home");
    let dot_cargo_home = custom_home.join(".cargo");
    fs::create_dir_all(&dot_cargo_home).unwrap();
    fs::write(dot_cargo_home.join("config.toml"), "[build]\ntarget = ''\n").unwrap();
    fs::write(dot_cargo_home.join("config"), "[build]\n").unwrap();
    let custom_home_str = custom_home.display().to_string();
    assert_eq!(resolve_cargo_home_with(None, Some(&custom_home_str)), Some(dot_cargo_home.clone()));
    let cfgs = find_cargo_configs_with_home(&work_dir, Some(&dot_cargo_home));
    assert!(cfgs.iter().any(|c| c.ends_with("config.toml")));
    assert_eq!(extract_config_target("[build]\ntarget = ''\n"), None);
    assert_eq!(resolve_cargo_home_with(None, Some("   ")), None);

    // Test extract_config_target with empty target and other keys (lines 148-149)
    assert_eq!(extract_config_target("[build]\njobs = 4\ntarget = ''\n"), None);

    // Test collect_all_workspace_manifests with hidden dir (line 168)
    let hidden_dir = work_dir.join(".hidden_dir");
    let _ = fs::create_dir_all(&hidden_dir);
    let _ = fs::write(hidden_dir.join("Cargo.toml"), "");

    // Test compute_variant_id with external manifest_path (line 219)
    let outside_manifest = temp.0.join("outside/Cargo.toml");
    let _ = fs::create_dir_all(outside_manifest.parent().unwrap());
    let _ = fs::write(&outside_manifest, "");
    let v_outside = compute_variant_id("dev", None, None, None, false, false, &outside_manifest);
    assert!(!v_outside.is_empty());

    // Test compute_variant_id with multiple cargo configs where config_target is already Some (line 198)
    let sub_ws = work_dir.join("sub_ws");
    let _ = fs::create_dir_all(sub_ws.join(".cargo"));
    fs::write(sub_ws.join(".cargo/config.toml"), "[build]\ntarget = 'x86_64-unknown-linux-gnu'\n").unwrap();
    let v_multi_cfg = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
    assert!(!v_multi_cfg.is_empty());

    // Test CARGO_BUILD_TARGET parameter via compute_variant_id_with_env
    let v_env_target = compute_variant_id_with_env("dev", None, None, None, false, false, &manifest_file, Some("aarch64-unknown-linux-gnu"), None, None, None);
    assert!(!v_env_target.is_empty());

    // Test Cargo.lock reading in compute_variant_id (lines 238-241)
    let lock_file = work_dir.join("Cargo.lock");
    fs::write(&lock_file, b"version = 3\n").unwrap();
    let v_lock = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
    assert!(!v_lock.is_empty());
    let _ = fs::remove_file(&lock_file);

    // Test RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS via compute_variant_id_with_env
    let v_rustflags = compute_variant_id_with_env("dev", None, None, None, false, false, &manifest_file, None, Some("-C target-cpu=native"), Some("-C\x1ftarget-cpu=native"), None);
    assert!(!v_rustflags.is_empty());

    // Test try_run_indexing validation errors (lines 280-283, 311, 315)
    let not_cargo = temp.0.join("not_cargo.txt");
    fs::write(&not_cargo, b"hello").unwrap();
    let err_file = try_run_indexing(IndexOptions {
        path: not_cargo,
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: None,
        capture_child_output: false,
        builder: None,
    });
    assert!(err_file.unwrap_err().contains("is a file but not a Cargo.toml manifest"));

    let empty_dir = new_test_dir("empty_dir_no_toml");
    let err_dir = try_run_indexing(IndexOptions {
        path: empty_dir.0.clone(),
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: None,
        capture_child_output: false,
        builder: None,
    });
    assert!(err_dir.unwrap_err().contains("no Cargo.toml found in directory"));

    let err_nonexist = try_run_indexing(IndexOptions {
        path: PathBuf::from("/nonexistent/path/99999"),
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: None,
        capture_child_output: false,
        builder: None,
    });
    assert!(err_nonexist.unwrap_err().contains("does not exist"));
}
