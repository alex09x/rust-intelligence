/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::target_kind::{classify_target, TargetKind};

#[test]
fn test_classification_lib_in_examples_dir() {
    // A library crate living inside an examples directory (e.g. workspace member `examples/foo_lib`)
    // Must be classified as Lib, NOT Example.
    let compiler_args = vec!["rustc".to_string(), "examples/foo_lib/src/lib.rs".to_string(), "--crate-type".to_string(), "lib".to_string()];
    let kind = classify_target(Some("lib"), &compiler_args);
    assert_eq!(kind, TargetKind::Lib);
}

#[test]
fn test_classification_checkout_in_tests_dir() {
    // A checkout path containing `/tests/` in its directory prefix (e.g. `/tmp/x/tests/my_crate`)
    // Must be classified as Lib, NOT Test.
    let compiler_args = vec!["rustc".to_string(), "/tmp/x/tests/my_crate/src/lib.rs".to_string(), "--crate-type".to_string(), "lib".to_string()];
    let kind = classify_target(Some("lib"), &compiler_args);
    assert_eq!(kind, TargetKind::Lib);
}

#[test]
fn test_classification_test_flag_on_lib() {
    // Compiling a library unit with `--test` for unit testing.
    // Must be classified as Test.
    let compiler_args = vec!["rustc".to_string(), "src/lib.rs".to_string(), "--test".to_string(), "--crate-type".to_string(), "lib".to_string()];
    let kind = classify_target(Some("lib"), &compiler_args);
    assert_eq!(kind, TargetKind::Test);
}

#[test]
fn test_classification_bench_with_harness_false() {
    // A benchmark target configured with `harness = false`.
    // Cargo compiles it as `--crate-type bin` WITHOUT `--test`.
    // Authoritative cargo metadata `kind: bench` must classify it as Bench, NOT Bin.
    let compiler_args = vec!["rustc".to_string(), "benches/criterion_bench.rs".to_string(), "--crate-type".to_string(), "bin".to_string()];
    let kind = classify_target(Some("bench"), &compiler_args);
    assert_eq!(kind, TargetKind::Bench);
}

#[test]
fn test_classification_bin_target_with_examples_path() {
    // A binary target configured as `[[bin]] path = "examples/tool.rs"`.
    // Cargo metadata provides `kind: bin`. It must be classified as Bin, NOT Example.
    let compiler_args = vec!["rustc".to_string(), "examples/tool.rs".to_string(), "--crate-type".to_string(), "bin".to_string()];
    let kind = classify_target(Some("bin"), &compiler_args);
    assert_eq!(kind, TargetKind::Bin);
}

#[test]
fn test_target_kind_serialization_roundtrip() {
    let kinds = [
        (TargetKind::Lib, "\"lib\""),
        (TargetKind::Bin, "\"bin\""),
        (TargetKind::Test, "\"test\""),
        (TargetKind::Bench, "\"bench\""),
        (TargetKind::Example, "\"example\""),
        (TargetKind::CustomBuild, "\"custom-build\""),
    ];

    for (k, expected_json) in kinds {
        let serialized = serde_json::to_string(&k).unwrap();
        assert_eq!(serialized, expected_json);
        let deserialized: TargetKind = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, k);
    }
}
