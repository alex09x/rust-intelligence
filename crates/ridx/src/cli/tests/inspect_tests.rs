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
use crate::cli::args::Commands;
use crate::cli::dispatch::run_cli;
use crate::cli::resolve::*;
use std::path::PathBuf;

#[test]
fn test_cli_inspect_and_queries() {
    let _lock = lock_cli_test();
    let (test_dir, dir_str) = create_indexed_workspace();
    let path = PathBuf::from(&dir_str);

    // Inspect (text and json)
    run_cli(Commands::Inspect {
        index_path: Some(path.clone()),
        json: false,
    });
    run_cli(Commands::Inspect {
        index_path: Some(path.clone()),
        json: true,
    });

    // Def by symbol (text and json)
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        json: false,
    });
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        json: true,
    });

    // Def by location (text and json)
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("src/main.rs:20:5".to_string()),
        json: false,
    });
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("src/main.rs:20:5".to_string()),
        json: true,
    });

    // Def not found
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("missing::symbol".to_string()),
        json: false,
    });
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("missing::symbol".to_string()),
        json: true,
    });

    // Refs by symbol (text and json)
    run_cli(Commands::Refs {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        json: false,
    });
    run_cli(Commands::Refs {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        json: true,
    });

    // Refs by location (text and json)
    run_cli(Commands::Refs {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        json: false,
    });
    run_cli(Commands::Refs {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        json: true,
    });

    // TypeAt (text and json)
    run_cli(Commands::TypeAt {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        json: false,
    });
    run_cli(Commands::TypeAt {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        json: true,
    });
    run_cli(Commands::TypeAt {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:99:99".to_string()),
        json: false,
    });

    // Hover (text and json)
    run_cli(Commands::Hover {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        json: false,
    });
    run_cli(Commands::Hover {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        json: true,
    });
    run_cli(Commands::Hover {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:15:1".to_string()),
        json: false,
    });
    run_cli(Commands::Hover {
        arg1: dir_str.clone(),
        arg2: Some("invalid_loc_format".to_string()),
        json: false,
    });

    // Symbols (text and json)
    run_cli(Commands::Symbols {
        arg1: dir_str.clone(),
        arg2: Some("test_fn".to_string()),
        json: false,
    });
    run_cli(Commands::Symbols {
        arg1: dir_str.clone(),
        arg2: Some("test_fn".to_string()),
        json: true,
    });
    run_cli(Commands::Symbols {
        arg1: dir_str.clone(),
        arg2: Some("NO_SIG".to_string()),
        json: false,
    });

    // Compare (text and json)
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        symbol: None,
        scenarios: None,
        ridx_cmd: None,
        ra_cmd: "rust-analyzer".to_string(),
        capability: "def".to_string(),
        json: false,
        scip_file: None,
    });
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        symbol: None,
        scenarios: None,
        ridx_cmd: None,
        ra_cmd: "rust-analyzer".to_string(),
        capability: "def".to_string(),
        json: true,
        scip_file: None,
    });
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: None,
        symbol: Some("test_crate::test_fn".to_string()),
        scenarios: None,
        ridx_cmd: None,
        ra_cmd: "rust-analyzer".to_string(),
        capability: "def".to_string(),
        json: false,
        scip_file: None,
    });
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: None,
        symbol: Some("test_crate::test_fn".to_string()),
        scenarios: None,
        ridx_cmd: None,
        ra_cmd: "rust-analyzer".to_string(),
        capability: "def".to_string(),
        json: true,
        scip_file: None,
    });

    // Def by path (text and json)
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        json: false,
    });
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("test_crate::test_fn".to_string()),
        json: true,
    });

    // Def by location (text and json)
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        json: false,
    });
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        json: true,
    });

    // Inspect with directory path (text and json)
    run_cli(Commands::Inspect {
        index_path: Some(test_dir.0.clone()),
        json: false,
    });
    run_cli(Commands::Inspect {
        index_path: Some(test_dir.0.clone()),
        json: true,
    });

    let resolved = resolve_workspace(Some(test_dir.0.join("test_crate.ridx")));
    assert_eq!(resolved.stats().total_defs, 2);
    let (resolved_ws, target) =
        resolve_index_and_target(dir_str.clone(), Some("my_target".to_string()));
    assert_eq!(target, "my_target");
    assert_eq!(resolved_ws.stats().total_defs, 2);

    run_cli(Commands::TypeAt {
        arg1: dir_str.clone(),
        arg2: Some("invalid_loc".to_string()),
        json: false,
    });
    run_cli(Commands::Lsp { index_path: None });
    let _ = std::panic::catch_unwind(|| {
        resolve_index_and_target(
            "/nonexistent/bogus.ridx".to_string(),
            Some("target".to_string()),
        );
    });
}
