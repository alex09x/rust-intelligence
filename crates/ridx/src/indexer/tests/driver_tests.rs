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
    // Test find_driver_binary fallback paths (lines 56-80)
    let orig_driver = std::env::var("RIDX_DRIVER_BIN").ok();
    std::env::remove_var("RIDX_DRIVER_BIN");
    let _ = find_driver_binary();
    let _ = find_driver_binary_internal(None);

    let exe_dir = temp.0.join("exe_test_dir/nested");
    fs::create_dir_all(&exe_dir).unwrap();
    let mock_driver_sibling = exe_dir.join("ridx-driver");
    fs::write(&mock_driver_sibling, b"").unwrap();
    let dummy_exe = exe_dir.join("dummy_exe");
    let found_sibling = find_driver_binary_internal(Some(&dummy_exe));
    assert_eq!(found_sibling, mock_driver_sibling.canonicalize().ok());
    let _ = fs::remove_file(&mock_driver_sibling);

    let mock_driver_parent = temp.0.join("exe_test_dir/ridx-driver");
    fs::write(&mock_driver_parent, b"").unwrap();
    let found_parent = find_driver_binary_internal(Some(&dummy_exe));
    assert_eq!(found_parent, mock_driver_parent.canonicalize().ok());
    let _ = fs::remove_file(&mock_driver_parent);
    let _ = find_driver_binary_internal(Some(&dummy_exe));

    // Also test with empty directory as cwd to trigger which and None fallback (lines 73-84)
    let orig_cwd = std::env::current_dir().ok();
    let temp_empty = new_test_dir("empty_for_which");
    let mock_target_debug = temp_empty.0.join("target/debug");
    fs::create_dir_all(&mock_target_debug).unwrap();
    let mock_target_driver = mock_target_debug.join("ridx-driver");
    fs::write(&mock_target_driver, b"").unwrap();
    if std::env::set_current_dir(&temp_empty.0).is_ok() {
        let found_cand = find_driver_binary_internal(None);
        assert_eq!(found_cand, mock_target_driver.canonicalize().ok());
        let _ = fs::remove_file(&mock_target_driver);

        // Test which command resolution when driver is in PATH (lines 77-84)
        let mock_bin_dir = new_test_dir("mock_bin_for_which");
        let mock_bin_driver = mock_bin_dir.0.join("ridx-driver");
        fs::write(&mock_bin_driver, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&mock_bin_driver, fs::Permissions::from_mode(0o755));
        }
        let orig_path = std::env::var("PATH").ok();
        let new_path = format!(
            "{}:{}",
            mock_bin_dir.0.display(),
            orig_path.as_deref().unwrap_or("")
        );
        std::env::set_var("PATH", &new_path);
        let found_which = find_driver_binary_internal(None);
        assert!(found_which.is_some());
        if let Some(p) = orig_path.as_ref() {
            std::env::set_var("PATH", p);
        } else {
            std::env::remove_var("PATH");
        }

        // Test which command failure with empty PATH (line 84)
        std::env::set_var("PATH", "");
        let found_none = find_driver_binary_internal(None);
        assert!(found_none.is_none());
        if let Some(p) = orig_path {
            std::env::set_var("PATH", p);
        } else {
            std::env::remove_var("PATH");
        }

        let _ = find_driver_binary_internal(None);
        if let Some(c) = orig_cwd {
            let _ = std::env::set_current_dir(c);
        }
    }
    if let Some(d) = orig_driver {
        std::env::set_var("RIDX_DRIVER_BIN", d);
    }

    // Test resolve_cargo_home and find_cargo_configs (lines 85-96, 119-126)
    let orig_home = std::env::var("HOME").ok();
    let orig_cargo_home = std::env::var("CARGO_HOME").ok();

    let work_dir = temp.0.join("sample_pkg");
    fs::create_dir_all(&work_dir).unwrap();
    let manifest_file = work_dir.join("Cargo.toml");
    fs::write(
        &manifest_file,
        "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let custom_home = temp.0.join("custom_home");
    let dot_cargo_home = custom_home.join(".cargo");
    fs::create_dir_all(&dot_cargo_home).unwrap();
    fs::write(dot_cargo_home.join("config.toml"), "[build]\ntarget = ''\n").unwrap();
    fs::write(dot_cargo_home.join("config"), "[build]\n").unwrap();
    std::env::set_var("HOME", custom_home.display().to_string());
    std::env::remove_var("CARGO_HOME");
    assert_eq!(resolve_cargo_home(), Some(dot_cargo_home.clone()));
    let cfgs = find_cargo_configs(&work_dir);
    assert!(cfgs.iter().any(|c| c.ends_with("config.toml")));
    assert_eq!(extract_config_target("[build]\ntarget = ''\n"), None);

    // Test collect_all_workspace_manifests with .git and target subdirs (lines 164-168)
    let sub_ignored = work_dir.join(".git");
    fs::create_dir_all(&sub_ignored).unwrap();
    fs::write(sub_ignored.join("Cargo.toml"), "").unwrap();
    let mut all_mf = Vec::new();
    collect_all_workspace_manifests(&work_dir, &mut all_mf);
    assert!(!all_mf.iter().any(|m| m.starts_with(&sub_ignored)));

    // Test resolve_cargo_home with empty string (line 96)
    std::env::set_var("HOME", "   ");
    std::env::remove_var("CARGO_HOME");
    assert_eq!(resolve_cargo_home(), None);

    if let Some(h) = orig_home {
        std::env::set_var("HOME", h);
    } else {
        std::env::remove_var("HOME");
    }
    if let Some(ch) = orig_cargo_home {
        std::env::set_var("CARGO_HOME", ch);
    } else {
        std::env::remove_var("CARGO_HOME");
    }

    // Test extract_config_target with empty target and other keys (lines 148-149)
    assert_eq!(
        extract_config_target("[build]\njobs = 4\ntarget = ''\n"),
        None
    );

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
    fs::write(
        sub_ws.join(".cargo/config.toml"),
        "[build]\ntarget = 'x86_64-unknown-linux-gnu'\n",
    )
    .unwrap();
    let v_multi_cfg = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
    assert!(!v_multi_cfg.is_empty());

    // Test CARGO_BUILD_TARGET environment variable (lines 206-208)
    std::env::set_var("CARGO_BUILD_TARGET", "aarch64-unknown-linux-gnu");
    let v_env_target = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
    assert!(!v_env_target.is_empty());
    std::env::remove_var("CARGO_BUILD_TARGET");

    // Test Cargo.lock reading in compute_variant_id (lines 238-241)
    let lock_file = work_dir.join("Cargo.lock");
    fs::write(&lock_file, b"version = 3\n").unwrap();
    let v_lock = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
    assert!(!v_lock.is_empty());
    let _ = fs::remove_file(&lock_file);

    // Test RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS in compute_variant_id (lines 253-258)
    std::env::set_var("RUSTFLAGS", "-C target-cpu=native");
    std::env::set_var("CARGO_ENCODED_RUSTFLAGS", "-C\x1ftarget-cpu=native");
    let v_rustflags = compute_variant_id("dev", None, None, None, false, false, &manifest_file);
    assert!(!v_rustflags.is_empty());
    std::env::remove_var("RUSTFLAGS");
    std::env::remove_var("CARGO_ENCODED_RUSTFLAGS");

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
    });
    assert!(err_file
        .unwrap_err()
        .contains("is a file but not a Cargo.toml manifest"));

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
    });
    assert!(err_dir
        .unwrap_err()
        .contains("no Cargo.toml found in directory"));

    let err_nonexist = try_run_indexing(IndexOptions {
        path: PathBuf::from("/nonexistent/path/99999"),
        out_dir_opt: None,
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
    });
    assert!(err_nonexist.unwrap_err().contains("does not exist"));
}
