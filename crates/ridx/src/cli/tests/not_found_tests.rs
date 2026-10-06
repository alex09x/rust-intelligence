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
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn test_cli_not_found_branches_and_helpers() {
    let _lock = lock_cli_test();
    let (test_dir, _) = create_indexed_workspace();
    let dir_str = test_dir.0.display().to_string();

    // 1. Def not found (text & json)
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("nonexistent_symbol".to_string()),
        json: false,
    });
    run_cli(Commands::Def {
        arg1: dir_str.clone(),
        arg2: Some("nonexistent_symbol".to_string()),
        json: true,
    });

    // 2. Refs not found (text & json)
    run_cli(Commands::Refs {
        arg1: dir_str.clone(),
        arg2: Some("nonexistent_symbol".to_string()),
        json: false,
    });
    run_cli(Commands::Refs {
        arg1: dir_str.clone(),
        arg2: Some("nonexistent_symbol".to_string()),
        json: true,
    });

    // 3. Hover not found
    run_cli(Commands::Hover {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:999:999".to_string()),
        json: false,
    });
    run_cli(Commands::Hover {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:999:999".to_string()),
        json: true,
    });

    // 4. Symbols not found
    run_cli(Commands::Symbols {
        arg1: dir_str.clone(),
        arg2: Some("nonexistent_symbol_query".to_string()),
        json: false,
    });
    run_cli(Commands::Symbols {
        arg1: dir_str.clone(),
        arg2: Some("nonexistent_symbol_query".to_string()),
        json: true,
    });

    // 5. Compare not found
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: Some("nonexistent_symbol".to_string()),
        symbol: None,
        json: false,
    });
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: Some("nonexistent_symbol".to_string()),
        symbol: None,
        json: true,
    });
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        symbol: None,
        json: true,
    });
    run_cli(Commands::Compare {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:10:1".to_string()),
        symbol: None,
        json: false,
    });

    // 6. Inspect with direct path
    run_cli(Commands::Inspect {
        index_path: Some(test_dir.0.join("test_crate.ridx")),
        json: false,
    });
    run_cli(Commands::Inspect {
        index_path: Some(test_dir.0.join("test_crate.ridx")),
        json: true,
    });

    // 7. TypeAt not found & found
    run_cli(Commands::TypeAt {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:999:999".to_string()),
        json: false,
    });
    run_cli(Commands::TypeAt {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:999:999".to_string()),
        json: true,
    });
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

    // 8. Hover found
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

    // 9. Refs location not found (text & json)
    run_cli(Commands::Refs {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:999:999".to_string()),
        json: false,
    });
    run_cli(Commands::Refs {
        arg1: dir_str.clone(),
        arg2: Some("src/lib.rs:999:999".to_string()),
        json: true,
    });

    // 10. Location parser & workspace helpers
    assert_eq!(
        parse_location("src/lib.rs:10:5"),
        Some(("src/lib.rs", 10, 5))
    );
    assert_eq!(parse_location("invalid"), None);
    assert_eq!(parse_location("src:not_a_num:5"), None);
    assert_eq!(parse_location("src/lib.rs:10:not_a_num"), None);

    assert!(find_enclosing_workspace(Path::new("/")).is_none());
    assert!(find_manifest_for_path(Path::new("/nonexistent/path/for/test")).is_none());

    // Manifest with nonexistent workspace_root outside cargo workspace (lines 282-285)
    let outside_dir = new_test_dir("outside_no_cargo");
    let vm_outside = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "v1",
        "variant_dir": "variants/v1",
        "workspace_root": "/nonexistent/ws_root/12345",
        "shards": []
    });
    fs::write(
        outside_dir.0.join("variant_manifest.json"),
        serde_json::to_string(&vm_outside).unwrap(),
    )
    .unwrap();
    let res_no_cargo_ws = try_resolve_workspace(Some(outside_dir.0.join("shard.ridx")));
    assert!(res_no_cargo_ws.is_err());
    assert!(res_no_cargo_ws
        .unwrap_err()
        .contains("Cannot find Cargo workspace for index"));

    // 11. try_resolve_workspace error cases and auto-discover
    let inactive_variant = PathBuf::from("/tmp/test/variants/old_variant/shard.ridx");
    let res_inactive = try_resolve_workspace(Some(inactive_variant));
    assert!(res_inactive
        .unwrap_err()
        .contains("Cannot automatically reindex inactive variant shard"));

    let no_ws = PathBuf::from("/tmp/nonexistent_without_cargo_toml/shard.ridx");
    let res_no_ws = try_resolve_workspace(Some(no_ws));
    assert!(res_no_ws.is_err());

    std::env::set_var("RIDX_INDEX_DIR", &test_dir.0);
    let res_auto = try_resolve_workspace(None);
    assert!(res_auto.is_ok());
    std::env::remove_var("RIDX_INDEX_DIR");
}
