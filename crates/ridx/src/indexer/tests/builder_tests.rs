/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for index builder resolution and check command construction.

use crate::indexer::builder::{build_check_command, build_check_command_with_no_deps, lib_and_bin_fallback_args, IndexBuilder};
use crate::indexer::types::IndexOptions;
use std::path::{Path, PathBuf};

#[test]
fn test_builder_resolution() {
    // Explicit flag takes highest precedence
    assert_eq!(IndexBuilder::resolve_with_env(Some(IndexBuilder::Cargo), Some("mitts")), IndexBuilder::Cargo);
    assert_eq!(IndexBuilder::resolve_with_env(Some(IndexBuilder::Mitts), Some("cargo")), IndexBuilder::Mitts);

    // String resolution helper
    assert_eq!(IndexBuilder::resolve_from_str(Some("cargo"), Some("mitts")), IndexBuilder::Cargo);
    assert_eq!(IndexBuilder::resolve_from_str(Some("mitts"), Some("cargo")), IndexBuilder::Mitts);
    assert_eq!(IndexBuilder::resolve_from_str(Some("CARGO"), None), IndexBuilder::Cargo);
    assert_eq!(IndexBuilder::resolve_from_str(Some("MITTS"), None), IndexBuilder::Mitts);

    // Fallback to environment variable when flag is None
    assert_eq!(IndexBuilder::resolve_with_env(None, Some("cargo")), IndexBuilder::Cargo);
    assert_eq!(IndexBuilder::resolve_with_env(None, Some("mitts")), IndexBuilder::Mitts);
    assert_eq!(IndexBuilder::resolve_with_env(None, Some("CARGO")), IndexBuilder::Cargo);
    assert_eq!(IndexBuilder::resolve_with_env(None, Some("Mitts")), IndexBuilder::Mitts);

    // Fallback to Cargo default when flag and env are None or invalid
    assert_eq!(IndexBuilder::resolve_with_env(None, None), IndexBuilder::Cargo);
    assert_eq!(IndexBuilder::resolve_with_env(None, Some("")), IndexBuilder::Cargo);
    assert_eq!(IndexBuilder::resolve_with_env(None, Some("invalid_backend")), IndexBuilder::Cargo);
}

#[test]
fn test_builder_display_and_parse() {
    assert_eq!(IndexBuilder::Cargo.as_str(), "cargo");
    assert_eq!(IndexBuilder::Mitts.as_str(), "mitts");
    assert_eq!(IndexBuilder::Cargo.to_string(), "cargo");
    assert_eq!(IndexBuilder::Mitts.to_string(), "mitts");

    assert_eq!("cargo".parse::<IndexBuilder>().unwrap(), IndexBuilder::Cargo);
    assert_eq!("mitts".parse::<IndexBuilder>().unwrap(), IndexBuilder::Mitts);
    assert_eq!("Cargo".parse::<IndexBuilder>().unwrap(), IndexBuilder::Cargo);
    assert_eq!("MITTS".parse::<IndexBuilder>().unwrap(), IndexBuilder::Mitts);
    assert!("unknown".parse::<IndexBuilder>().is_err());
}

#[test]
fn test_build_check_command_cargo_and_mitts_parity() {
    let opts = IndexOptions {
        path: PathBuf::from("/ws"),
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
    };

    let manifest = Path::new("/ws/Cargo.toml");
    let work = Path::new("/ws");
    let driver = Path::new("/bin/ridx-driver");
    let variant_out = Path::new("/ws/target/ridx/variants/dev");
    let cache = Path::new("/cache/ridx");
    let cargo_target = Path::new("/ws/target/ridx_cargo/dev");
    let target_kinds = Path::new("/ws/target/ridx/variants/dev/target_kinds.json");

    let (cargo_prog, cargo_args, cargo_envs) =
        build_check_command_with_no_deps(&opts, IndexBuilder::Cargo, manifest, work, driver, variant_out, cache, cargo_target, target_kinds, Some("/sysroot/lib"), false);

    let (mitts_prog, mitts_args, mitts_envs) =
        build_check_command_with_no_deps(&opts, IndexBuilder::Mitts, manifest, work, driver, variant_out, cache, cargo_target, target_kinds, Some("/sysroot/lib"), false);

    // Binary programs differ
    assert_eq!(cargo_prog, "cargo");
    assert_eq!(mitts_prog, "mitts");

    // Args are identical but for Cargo's `--keep-going` (indexing the crates that build when one fails).
    assert_eq!(mitts_args, vec!["check", "--manifest-path", "/ws/Cargo.toml", "--all-targets", "--workspace",]);
    assert_eq!(cargo_args, vec!["check", "--manifest-path", "/ws/Cargo.toml", "--all-targets", "--keep-going", "--workspace",]);

    // The fallback pass of a builder without `--keep-going` narrows the targets and keeps the rest.
    assert_eq!(lib_and_bin_fallback_args(&mitts_args), vec!["check", "--manifest-path", "/ws/Cargo.toml", "--lib", "--bins", "--workspace",]);

    // Envs are identical
    assert_eq!(cargo_envs, mitts_envs);
    assert_eq!(
        cargo_envs,
        vec![
            ("RIDX_OUT_DIR".to_string(), "/ws/target/ridx/variants/dev".to_string()),
            ("RIDX_CACHE_DIR".to_string(), "/cache/ridx".to_string()),
            ("RIDX_WORKSPACE_ROOT".to_string(), "/ws".to_string()),
            ("CARGO_TARGET_DIR".to_string(), "/ws/target/ridx_cargo/dev".to_string()),
            ("RUSTC_BOOTSTRAP".to_string(), "1".to_string()),
            ("RIDX_TARGET_KINDS_FILE".to_string(), "/ws/target/ridx/variants/dev/target_kinds.json".to_string()),
            ("RUSTC_WRAPPER".to_string(), "/bin/ridx-driver".to_string()),
            ("RUSTC_WORKSPACE_WRAPPER".to_string(), "".to_string()),
            ("LD_LIBRARY_PATH".to_string(), "/sysroot/lib".to_string()),
            ("DYLD_FALLBACK_LIBRARY_PATH".to_string(), "/sysroot/lib".to_string()),
        ]
    );
}

#[test]
fn test_build_check_command_options_flags() {
    let opts = IndexOptions {
        path: PathBuf::from("/ws"),
        out_dir_opt: None,
        package: Some("pkg_a".to_string()),
        release: true,
        features: Some("feat1,feat2".to_string()),
        no_default_features: true,
        all_features: true,
        target: Some("x86_64-unknown-linux-gnu".to_string()),
        cache_dir: None,
        driver_bin: None,
        capture_child_output: false,
        builder: Some(IndexBuilder::Mitts),
    };

    let (prog, args, envs) = build_check_command_with_no_deps(
        &opts,
        IndexBuilder::Mitts,
        Path::new("/ws/Cargo.toml"),
        Path::new("/ws"),
        Path::new("/bin/ridx-driver"),
        Path::new("/ws/target/ridx/variants/rel"),
        Path::new("/cache"),
        Path::new("/ws/target/ridx_cargo/rel"),
        Path::new("/ws/target/ridx/variants/rel/target_kinds.json"),
        None,
        true,
    );

    assert_eq!(prog, "mitts");
    assert_eq!(
        args,
        vec![
            "check",
            "--manifest-path",
            "/ws/Cargo.toml",
            "--all-targets",
            "-p",
            "pkg_a",
            "--release",
            "--features",
            "feat1,feat2",
            "--no-default-features",
            "--all-features",
            "--target",
            "x86_64-unknown-linux-gnu",
        ]
    );

    assert!(envs.contains(&("RUSTC_WORKSPACE_WRAPPER".to_string(), "/bin/ridx-driver".to_string())));
    assert!(envs.contains(&("RUSTC_WRAPPER".to_string(), "".to_string())));
}

#[test]
fn test_build_check_command_convenience_wrapper() {
    let opts = IndexOptions {
        path: PathBuf::from("/ws"),
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
    };

    let (prog, args, envs) = build_check_command(
        &opts,
        IndexBuilder::Cargo,
        Path::new("/ws/Cargo.toml"),
        Path::new("/ws"),
        Path::new("/bin/ridx-driver"),
        Path::new("/ws/target/ridx/variants/dev"),
        Path::new("/cache"),
        Path::new("/ws/target/ridx_cargo/dev"),
        Path::new("/ws/target/ridx/variants/dev/target_kinds.json"),
        None,
    );

    assert_eq!(prog, "cargo");
    assert_eq!(args[0], "check");
    assert!(envs.iter().any(|(k, _)| k == "RIDX_WORKSPACE_ROOT"));
}
