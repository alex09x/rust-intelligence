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

/// prod-code, issue #7: every dependent of an edited crate was compiled again, though only a body changed. An edit that
/// keeps what the dependents see (a body, lines above a definition) compiles the crate alone and moves the dependents'
/// references; the next compile of a dependent first compiles the one whose metadata names the crate's older build.
#[test]
fn test_dependents_of_a_body_edit_are_not_compiled() {
    let _lock = lock_indexer_test();
    if find_driver_binary().is_none() {
        return;
    }
    let test_dir = new_test_dir("ridx_direct_cutoff_test");
    let root = test_dir.0.clone();
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"crate_a\", \"crate_b\", \"crate_c\"]\nresolver = \"2\"\n").unwrap();
    let krate = |name: &str, deps: &str, lib: &str| {
        fs::create_dir_all(root.join(name).join("src")).unwrap();
        fs::write(root.join(name).join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}")).unwrap();
        fs::write(root.join(name).join("src/lib.rs"), lib).unwrap();
    };
    krate("crate_a", "", "pub fn target_func() -> i32 {\n    42\n}\n");
    krate("crate_b", "crate_a = { path = \"../crate_a\" }\n", "pub fn caller() -> i32 {\n    crate_a::target_func()\n}\n");
    krate("crate_c", "crate_a = { path = \"../crate_a\" }\ncrate_b = { path = \"../crate_b\" }\n", "pub fn top() -> i32 {\n    crate_b::caller() + crate_a::target_func()\n}\n");
    let _ = std::process::Command::new("cargo").arg("generate-lockfile").current_dir(&root).status();
    let a_lib = root.join("crate_a/src/lib.rs");
    let (b_lib, c_lib) = (root.join("crate_b/src/lib.rs").display().to_string(), root.join("crate_c/src/lib.rs").display().to_string());

    let index = |base: Option<std::path::PathBuf>| try_run_indexing(IndexOptions { path: root.clone(), out_dir_opt: base, ..Default::default() }).expect("index");
    let pv = index(None);
    // The sidecars of a crate's units, as written: a compiled unit's are rewritten.
    let sidecars = |name: &str| -> Vec<Vec<u8>> {
        let mut out: Vec<(std::path::PathBuf, Vec<u8>)> = fs::read_dir(pv.variant_out_dir.join(".deps"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .filter_map(|p| fs::read(&p).ok().map(|b| (p, b)))
            .filter(|(_, b)| serde_json::from_slice::<serde_json::Value>(b).is_ok_and(|v| v["name"] == name && v["is_dependency"] != true))
            .collect();
        out.sort();
        out.into_iter().map(|(_, b)| b).collect()
    };

    // The first edit after a Cargo index too: the dependents' sidecars record the text of crate_a they read.
    let (b0, c0) = (sidecars("crate_b"), sidecars("crate_c"));
    fs::write(&a_lib, "pub fn target_func() -> i32 {\n    41 + 1\n}\n").unwrap();
    index(Some(pv.base_out_dir.clone()));
    assert_eq!(sidecars("crate_b"), b0, "crate_b not compiled after a body edit");
    assert_eq!(sidecars("crate_c"), c0, "crate_c not compiled after a body edit");

    fs::write(&a_lib, "// one\n// two\npub fn target_func() -> i32 {\n    let x = 42;\n    x\n}\n").unwrap();
    index(Some(pv.base_out_dir.clone()));
    assert_eq!(sidecars("crate_b"), b0, "crate_b not compiled");
    assert_eq!(sidecars("crate_c"), c0, "crate_c not compiled");
    let ws = WorkspaceIndex::open(&pv.variant_out_dir).expect("open");
    assert_eq!(ws.find_definition_at(&b_lib, 2, 14).expect("definition from crate_b").start_line, 3, "to the definition's new line");
    let refs = ws.find_references_at(a_lib.to_str().unwrap(), 3, 8).expect("references of target_func");
    let mut referrers: Vec<(String, u32)> = refs.references.iter().map(|r| (r.crate_name.to_string(), r.span.start_line())).collect();
    referrers.sort();
    assert!(referrers.contains(&("crate_b".into(), 2)) && referrers.contains(&("crate_c".into(), 2)), "the moved references are found: {referrers:?}");
    drop(ws);

    // crate_c compiled against crate_b compiled anew: the crate_b it was compiled against named crate_a's older build.
    fs::write(&c_lib, "pub fn top() -> i32 {\n    crate_b::caller() * crate_a::target_func()\n}\n").unwrap();
    index(Some(pv.base_out_dir.clone()));
    assert_ne!(sidecars("crate_b"), b0, "crate_b compiled before crate_c");
    let ws = WorkspaceIndex::open(&pv.variant_out_dir).expect("open");
    assert_eq!(ws.find_definition_at(&c_lib, 2, 14).expect("crate_b::caller from crate_c").start_line, 1);
    assert_eq!(ws.find_definition_at(&c_lib, 2, 34).expect("crate_a::target_func from crate_c").start_line, 3);
    drop(ws);

    // An item added changes what the dependents see: they are compiled.
    let (b1, c1) = (sidecars("crate_b"), sidecars("crate_c"));
    fs::write(&a_lib, "// one\n// two\npub fn extra() {}\npub fn target_func() -> i32 {\n    42\n}\n").unwrap();
    index(Some(pv.base_out_dir.clone()));
    assert_ne!(sidecars("crate_b"), b1);
    assert_ne!(sidecars("crate_c"), c1);
    let ws = WorkspaceIndex::open(&pv.variant_out_dir).expect("open");
    assert_eq!(ws.find_definition_at(&b_lib, 2, 14).expect("definition from crate_b").start_line, 4);
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
