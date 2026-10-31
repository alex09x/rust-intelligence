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
fn test_indexer_helpers_and_error_paths() {
    let _lock = lock_indexer_test();
    // 1. try_run_indexing errors
    let non_existent = PathBuf::from("/nonexistent_path_for_indexer");
    assert!(try_run_indexing(IndexOptions {
        path: non_existent,
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
    })
    .is_err());

    let temp = new_test_dir("ridx_err_tests");
    let not_cargo = temp.0.join("not_cargo.txt");
    fs::write(&not_cargo, "hello").unwrap();
    assert!(try_run_indexing(IndexOptions {
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
    })
    .is_err());

    let no_manifest_dir = temp.0.join("empty_dir");
    fs::create_dir_all(&no_manifest_dir).unwrap();
    assert!(try_run_indexing(IndexOptions {
        path: no_manifest_dir,
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
    })
    .is_err());

    // 2. extract_config_target
    let config_toml = "[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n";
    assert_eq!(extract_config_target(config_toml), Some("x86_64-unknown-linux-gnu".to_string()));
    assert_eq!(extract_config_target("[other]\ntarget = \"abc\""), None);
    assert_eq!(extract_config_target("# [build]\n# target = 1"), None);
    assert_eq!(extract_config_target("[build]\ntarget = \"\""), None);
    assert_eq!(extract_config_target("[build]\ntarget = ''"), None);

    // 3. find_cargo_configs & collect_all_workspace_manifests
    let ws_dir = temp.0.join("ws");
    let cargo_dir = ws_dir.join(".cargo");
    fs::create_dir_all(&cargo_dir).unwrap();
    fs::write(cargo_dir.join("config.toml"), config_toml).unwrap();
    let configs = find_cargo_configs(&ws_dir);
    assert!(!configs.is_empty());

    let member_dir = ws_dir.join("crates/member");
    fs::create_dir_all(&member_dir).unwrap();
    fs::write(member_dir.join("Cargo.toml"), "[package]\nname = \"member\"\nversion = \"0.1.0\"\n").unwrap();
    let mut manifests = Vec::new();
    collect_all_workspace_manifests(&ws_dir, &mut manifests);
    assert!(!manifests.is_empty());

    let mut empty_manifests = Vec::new();
    collect_all_workspace_manifests(&PathBuf::from("/nonexistent_dir_for_test"), &mut empty_manifests);
    assert!(empty_manifests.is_empty());

    // 4. resolve_cargo_home_with with CARGO_HOME and HOME variants
    assert_eq!(resolve_cargo_home_with(Some("/custom/cargo/home"), Some("/custom/home")), Some(PathBuf::from("/custom/cargo/home")));
    assert_eq!(resolve_cargo_home_with(None, Some("/custom/home")), Some(PathBuf::from("/custom/home/.cargo")));
    assert_eq!(resolve_cargo_home_with(Some(""), Some("")), None);
    assert_eq!(resolve_cargo_home_with(Some("   "), Some("   ")), None);

    // 5. find_driver_binary_with with RIDX_DRIVER_BIN override and fallbacks
    let dummy_driver = temp.0.join("mock_driver");
    fs::write(&dummy_driver, b"#!/bin/sh\nexec \"$@\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dummy_driver, fs::Permissions::from_mode(0o755));
    }
    let found = find_driver_binary_with(dummy_driver.to_str(), None, None, None);
    assert_eq!(found, dummy_driver.canonicalize().ok());
    let _ = find_driver_binary();

    // 6. Direct Cargo.toml path and relative out_dir_opt
    let direct_manifest = temp.0.join("direct_crate/Cargo.toml");
    fs::create_dir_all(direct_manifest.parent().unwrap()).unwrap();
    fs::write(&direct_manifest, "[package]\nname = \"direct_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"lib.rs\"\n").unwrap();
    fs::write(temp.0.join("direct_crate/lib.rs"), "pub fn direct_fn() {}\n").unwrap();

    let cur_d = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let rel_name = format!("rel_out_dir_for_test_{}", std::process::id());
    let expected_rel_dir = cur_d.join(&rel_name);
    let _ = try_run_indexing(IndexOptions {
        path: direct_manifest.clone(),
        out_dir_opt: Some(PathBuf::from(&rel_name)),
        package: None,
        release: true,
        features: None,
        no_default_features: false,
        all_features: false,
        target: Some("x86_64-unknown-linux-gnu".to_string()),
        cache_dir: None,
        driver_bin: Some(dummy_driver.clone()),
        capture_child_output: false,
    });
    let _ = fs::remove_dir_all(&expected_rel_dir);
    let _ = fs::remove_dir_all(&rel_name);

    // 7. Output directory creation failure (lines 327-330)
    let bad_out_file = temp.0.join("bad_out_file");
    fs::write(&bad_out_file, "file").unwrap();
    let err_out = try_run_indexing(IndexOptions {
        path: direct_manifest.clone(),
        out_dir_opt: Some(bad_out_file.join("sub")),
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: None,
        capture_child_output: false,
    });
    assert!(err_out.is_err());
    assert!(err_out.unwrap_err().contains("failed to create output directory"));

    // 8. Driver binary not found error
    let err_driver = try_run_indexing(IndexOptions {
        path: direct_manifest.clone(),
        out_dir_opt: Some(temp.0.join("out_test")),
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: Some(PathBuf::from("/nonexistent/mock/driver/path")),
        capture_child_output: false,
    });
    assert!(err_driver.is_err());

    // 9. compute_variant_id_with_env with RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS
    let v_flags = compute_variant_id_with_env("dev", None, None, None, false, false, &direct_manifest, None, Some("-C opt-level=3"), Some("-C\x1fopt-level=3"), None);
    assert!(!v_flags.is_empty());

    // 10. compute_variant_id_with_env with RUSTUP_TOOLCHAIN
    let v_toolchain = compute_variant_id_with_env("dev", None, None, None, false, false, &direct_manifest, None, None, None, Some("test-toolchain"));
    assert!(!v_toolchain.is_empty());

    // 11. try_run_indexing cargo/driver failure when driver exits non-zero
    let failing_driver = temp.0.join("failing_driver");
    fs::write(&failing_driver, b"#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&failing_driver, fs::Permissions::from_mode(0o755));
    }
    let err_cargo = try_run_indexing(IndexOptions {
        path: direct_manifest.clone(),
        out_dir_opt: Some(temp.0.join("out_test_cargo")),
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: Some(failing_driver),
        capture_child_output: false,
    });
    assert!(err_cargo.is_err());

    // 12. WorkspaceIndex::open failure after cargo check exits 0
    let _ = fs::remove_file(direct_manifest.parent().unwrap().join("Cargo.lock"));
    let err_open = try_run_indexing(IndexOptions {
        path: direct_manifest,
        out_dir_opt: Some(temp.0.join("out_test_empty")),
        package: None,
        release: false,
        features: None,
        no_default_features: false,
        all_features: false,
        target: None,
        cache_dir: None,
        driver_bin: Some(dummy_driver),
        capture_child_output: false,
    });
    assert!(err_open.is_err());
    assert!(err_open.unwrap_err().contains("failed to open generated variant index"));
}
