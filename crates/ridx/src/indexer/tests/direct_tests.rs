/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration regression tests for direct crate fast-path reindexing.

use super::*;
use crate::indexer::driver::*;
use crate::indexer::orchestrate::*;
use crate::indexer::types::*;
use ridx_format::WorkspaceIndex;
use std::fs;

#[test]
fn test_direct_reindex_cross_crate_line_shift() {
    let _lock = lock_indexer_test();
    if find_driver_binary().is_none() {
        return;
    }

    let test_dir = new_test_dir("ridx_direct_shift_test");

    // Root workspace Cargo.toml
    let root_toml = test_dir.0.join("Cargo.toml");
    fs::write(&root_toml, "[workspace]\nmembers = [\"crate_a\", \"crate_b\"]\nresolver = \"2\"\n").unwrap();

    // crate_a
    let crate_a_dir = test_dir.0.join("crate_a");
    let crate_a_src = crate_a_dir.join("src");
    fs::create_dir_all(&crate_a_src).unwrap();
    fs::write(crate_a_dir.join("Cargo.toml"), "[package]\nname = \"crate_a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    fs::write(crate_a_src.join("lib.rs"), "pub fn target_func() -> i32 {\n    42\n}\n").unwrap();

    // crate_b depends on crate_a
    let crate_b_dir = test_dir.0.join("crate_b");
    let crate_b_src = crate_b_dir.join("src");
    fs::create_dir_all(&crate_b_src).unwrap();
    fs::write(crate_b_dir.join("Cargo.toml"), "[package]\nname = \"crate_b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncrate_a = { path = \"../crate_a\" }\n")
        .unwrap();
    fs::write(crate_b_src.join("lib.rs"), "pub fn caller() -> i32 {\n    crate_a::target_func()\n}\n").unwrap();

    // Generate initial Cargo.lock so variant_id is stable from the start
    let _ = std::process::Command::new("cargo").arg("generate-lockfile").current_dir(&test_dir.0).status();

    // 1. Initial full index via Cargo orchestration
    let out0 = try_run_indexing(IndexOptions {
        path: test_dir.0.clone(),
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
    assert!(out0.is_ok(), "Initial index failed: {:?}", out0.err());
    let pv0 = out0.unwrap();

    let ws0 = WorkspaceIndex::open(&pv0.variant_out_dir).expect("open ws0");
    let def0 = ws0.find_definition_by_path("crate_a::target_func").expect("def0 found");
    assert_eq!(def0.start_line, 1, "target_func initially at line 1");

    // Check cross-crate callsite resolution in crate_b
    let caller_file = crate_b_src.join("lib.rs");
    let caller_file_str = caller_file.to_str().unwrap();
    let cross_def0 = ws0.find_definition_at(caller_file_str, 2, 14).expect("cross def0 found");
    assert_eq!(cross_def0.start_line, 1, "cross def initially at line 1");

    // 2. Shift lines in crate_a by exactly 10 empty lines
    let shifted_src = format!("{}pub fn target_func() -> i32 {{\n    42\n}}\n", "\n".repeat(10));
    fs::write(crate_a_src.join("lib.rs"), shifted_src).unwrap();

    // 3. Reindex: direct fast-path should execute for crate_a and downstream crate_b
    let out1 = try_run_indexing(IndexOptions {
        path: test_dir.0.clone(),
        out_dir_opt: Some(pv0.base_out_dir.clone()),
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
    assert!(out1.is_ok(), "Direct reindex failed: {:?}", out1.err());
    let pv1 = out1.unwrap();

    // Ensure variant_id was stable across direct reindex
    assert_eq!(pv0.variant_id, pv1.variant_id);

    let ws1 = WorkspaceIndex::open(&pv1.variant_out_dir).expect("open ws1");
    let def1 = ws1.find_definition_by_path("crate_a::target_func").expect("def1 found");
    assert_eq!(def1.start_line, 11, "target_func after shift at line 11");

    let cross_def1 = ws1.find_definition_at(caller_file_str, 2, 14).expect("cross def1 found");
    assert_eq!(cross_def1.start_line, 11, "cross def after shift at line 11");
}

#[test]
fn test_direct_fallback_on_proc_macro() {
    let _lock = lock_indexer_test();
    if find_driver_binary().is_none() {
        return;
    }

    let test_dir = new_test_dir("ridx_direct_macro_fallback_test");

    // Root workspace Cargo.toml
    let root_toml = test_dir.0.join("Cargo.toml");
    fs::write(&root_toml, "[workspace]\nmembers = [\"macro_crate\", \"app_crate\"]\nresolver = \"2\"\n").unwrap();

    // macro_crate
    let macro_dir = test_dir.0.join("macro_crate");
    let macro_src = macro_dir.join("src");
    fs::create_dir_all(&macro_src).unwrap();
    fs::write(macro_dir.join("Cargo.toml"), "[package]\nname = \"macro_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nproc-macro = true\n").unwrap();
    fs::write(macro_src.join("lib.rs"), "use proc_macro::TokenStream;\n\n#[proc_macro]\npub fn my_macro(input: TokenStream) -> TokenStream {\n    input\n}\n").unwrap();

    // app_crate
    let app_dir = test_dir.0.join("app_crate");
    let app_src = app_dir.join("src");
    fs::create_dir_all(&app_src).unwrap();
    fs::write(
        app_dir.join("Cargo.toml"),
        "[package]\nname = \"app_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nmacro_crate = { path = \"../macro_crate\" }\n",
    )
    .unwrap();
    fs::write(app_src.join("lib.rs"), "macro_crate::my_macro! {\n    pub fn app_func() -> i32 { 100 }\n}\n").unwrap();

    let _ = std::process::Command::new("cargo").arg("generate-lockfile").current_dir(&test_dir.0).status();

    // 1. Initial index via Cargo
    let out0 = try_run_indexing(IndexOptions {
        path: test_dir.0.clone(),
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
    assert!(out0.is_ok(), "Initial index failed: {:?}", out0.err());
    let pv0 = out0.unwrap();

    // 2. Modify proc-macro crate
    fs::write(macro_src.join("lib.rs"), "use proc_macro::TokenStream;\n\n// Added comment\n#[proc_macro]\npub fn my_macro(input: TokenStream) -> TokenStream {\n    input\n}\n")
        .unwrap();

    // 3. Reindex: direct indexer must detect proc-macro modification, fall back to Cargo, and succeed
    let out1 = try_run_indexing(IndexOptions {
        path: test_dir.0.clone(),
        out_dir_opt: Some(pv0.base_out_dir),
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
    assert!(out1.is_ok(), "Reindex with proc-macro fallback failed: {:?}", out1.err());
}
