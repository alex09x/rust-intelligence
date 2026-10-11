/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::common::*;
use crate::cli::resolve::*;
use std::fs;

#[test]
fn test_parse_location_cases() {
    assert_eq!(
        parse_location("src/lib.rs:10:5"),
        Some(("src/lib.rs", 10, 5))
    );
    assert_eq!(parse_location("src/lib.rs"), None);
    assert_eq!(parse_location("src/lib.rs:not_num:5"), None);
    assert_eq!(parse_location("src/lib.rs:10:not_num"), None);
    assert_eq!(parse_location(""), None);
}

#[test]
fn test_find_enclosing_workspace_cases() {
    let test_dir = new_test_dir("ridx_ws_find");
    let cargo = test_dir.0.join("Cargo.toml");
    fs::write(&cargo, "[workspace]\n").unwrap();

    let sub_dir = test_dir.0.join("crates/sub");
    fs::create_dir_all(&sub_dir).unwrap();

    assert_eq!(find_enclosing_workspace(&sub_dir), Some(test_dir.0.clone()));
    assert_eq!(find_enclosing_workspace(&cargo), Some(test_dir.0.clone()));

    let empty_dir = new_test_dir("ridx_no_ws");
    assert_eq!(find_enclosing_workspace(&empty_dir.0), None);
}

#[test]
fn test_find_manifest_for_path_variants() {
    let test_dir = new_test_dir("ridx_manifest_find");
    let manifest_file = test_dir.0.join("ridx_manifest.json");

    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.0.display().to_string(),
    });
    fs::write(
        &manifest_file,
        serde_json::to_string(&manifest_data).unwrap(),
    )
    .unwrap();

    let found = find_manifest_for_path(&test_dir.0);
    assert!(found.is_some());
    let (dir, man) = found.unwrap();
    assert_eq!(dir, test_dir.0);
    assert_eq!(man.active_variant_id, "var1");

    // Target an inactive variant path
    let inactive_path = test_dir.0.join("variants/var2/test.ridx");
    let found_inactive = find_manifest_for_path(&inactive_path);
    assert!(found_inactive.is_none());

    // Target a directory with variant_manifest.json directly
    let v_dir = test_dir.0.join("v_dir");
    fs::create_dir_all(&v_dir).unwrap();
    let v_manifest_file = v_dir.join("variant_manifest.json");
    fs::write(
        &v_manifest_file,
        serde_json::to_string(&manifest_data).unwrap(),
    )
    .unwrap();
    let found_v = find_manifest_for_path(&v_dir);
    assert!(found_v.is_some());

    // Path with grandparents where candidate_dirs pushes grandparents (lines 200-207)
    let deep_file = test_dir.0.join("deep/nested/sub/file.ridx");
    let _ = find_manifest_for_path(&deep_file);

    // Invalid JSON variant_manifest.json (line 219)
    let inv_v_dir = test_dir.0.join("inv_v_dir");
    fs::create_dir_all(&inv_v_dir).unwrap();
    fs::write(inv_v_dir.join("variant_manifest.json"), b"invalid json").unwrap();
    let _ = find_manifest_for_path(&inv_v_dir);

    // Invalid JSON ridx_manifest.json (line 248)
    let inv_r_dir = test_dir.0.join("inv_r_dir");
    fs::create_dir_all(&inv_r_dir).unwrap();
    fs::write(inv_r_dir.join("ridx_manifest.json"), b"invalid json").unwrap();
    let _ = find_manifest_for_path(&inv_r_dir);
}

#[test]
fn test_try_resolve_index_and_target_from_file_location() {
    let (test_dir, _) = create_indexed_workspace();
    let cargo = test_dir.0.join("Cargo.toml");
    fs::write(&cargo, "[workspace]\n").unwrap();

    let target_ridx = test_dir.0.join("target/ridx");
    fs::create_dir_all(&target_ridx).unwrap();
    fs::rename(
        test_dir.0.join("test_crate.ridx"),
        target_ridx.join("test_crate.ridx"),
    )
    .unwrap();

    let src_file = test_dir.0.join("src/lib.rs");
    fs::create_dir_all(src_file.parent().unwrap()).unwrap();
    fs::write(&src_file, "fn foo() {}\n").unwrap();

    let loc = format!("{}:10:5", src_file.display());
    let res = try_resolve_index_and_target(loc.clone(), None);
    assert!(res.is_ok());
    let (ws, target) = res.unwrap();
    assert_eq!(target, loc);
    assert_eq!(ws.crate_count(), 1);
}
