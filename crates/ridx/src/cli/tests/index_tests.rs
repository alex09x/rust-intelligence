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
use crate::indexer::find_driver_binary;
use std::fs;
use std::path::Path;

#[test]
fn test_parse_location_and_workspace_helpers() {
    let _lock = lock_cli_test();
    assert_eq!(parse_location("src/lib.rs:10:5"), Some(("src/lib.rs", 10, 5)));
    assert_eq!(parse_location("invalid"), None);
    assert_eq!(parse_location("src/lib.rs:bad:5"), None);
    assert_eq!(parse_location("src/lib.rs:10:bad"), None);

    let cur = std::env::current_dir().unwrap();
    let enc = find_enclosing_workspace(&cur);
    assert!(enc.is_some());

    let fake_path = Path::new("/nonexistent_directory/test");
    assert!(find_manifest_for_path(fake_path).is_none());
}

#[test]
fn test_cli_index_command() {
    let _lock = lock_cli_test();
    let (test_dir, _) = create_indexed_workspace();
    let cargo_toml = test_dir.0.join("Cargo.toml");
    fs::write(&cargo_toml, "[package]\nname = \"cli_sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let src_dir = test_dir.0.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn sample() {}\n").unwrap();

    if find_driver_binary().is_some() {
        run_cli(Commands::Index {
            path: test_dir.0.clone(),
            out_dir: None,
            package: None,
            release: false,
            features: None,
            no_default_features: false,
            all_features: false,
            target: None,
            builder: None,
        });

        // Test auto-reindexing on missing shard in valid workspace
        let missing_shard = test_dir.0.join("target/ridx/missing_shard.ridx");
        let ws = resolve_workspace(Some(missing_shard));
        assert!(ws.crate_count() > 0);

        // Test resolve_workspace for missing shard without variant manifest (lines 326-327, 353-355)
        let no_man_ws = new_test_dir("no_man_ws");
        let cargo_toml = no_man_ws.0.join("Cargo.toml");
        fs::write(&cargo_toml, "[package]\nname = \"no_man\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        let src_dir = no_man_ws.0.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("lib.rs"), "pub fn f() {}\n").unwrap();
        let missing_in_no_man = no_man_ws.0.join("target/ridx/no_man.ridx");
        let ws_no_man = resolve_workspace(Some(missing_in_no_man));
        assert!(ws_no_man.crate_count() > 0);

        // Test inactive variant path in find_manifest_for_path
        let inactive_path = test_dir.0.join("target/ridx/variants/inactive_var/shard.ridx");
        assert!(find_manifest_for_path(&inactive_path).is_none());
    }
}

#[test]
fn test_cli_single_arg_auto_discover() {
    let _lock = lock_cli_test();
    let _ra_guard = EnvGuard::set("RUST_ANALYZER_BIN", "/nonexistent/ra");
    let (test_dir, dir_str) = create_indexed_workspace();

    // Write variant manifest so Inspect prints active variant
    let manifest_data = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "test_var_1",
        "variant_dir": "variants/test_var_1",
        "workspace_root": dir_str,
        "dependency_shards": []
    });
    let _ = fs::write(test_dir.0.join("variant_manifest.json"), serde_json::to_string(&manifest_data).unwrap());

    let _idx_guard = EnvGuard::set("RIDX_INDEX_DIR", &dir_str);

    // 1. Inspect with None (triggers auto_discover and active variant print)
    run_cli(Commands::Inspect { index_path: None, json: false });
    run_cli(Commands::Inspect { index_path: None, json: true });

    // 2. Def with arg2 = None (target as arg1)
    run_cli(Commands::Def { arg1: "test_crate::test_fn".to_string(), arg2: None, json: false });
    run_cli(Commands::Def { arg1: "test_crate::test_fn".to_string(), arg2: None, json: true });
    run_cli(Commands::Def { arg1: "src/lib.rs:10:1".to_string(), arg2: None, json: false });
    run_cli(Commands::Def { arg1: "src/lib.rs:10:1".to_string(), arg2: None, json: true });

    // 3. Refs with arg2 = None (target as arg1)
    run_cli(Commands::Refs { arg1: "test_crate::test_fn".to_string(), arg2: None, json: false });
    run_cli(Commands::Refs { arg1: "test_crate::test_fn".to_string(), arg2: None, json: true });
    run_cli(Commands::Refs { arg1: "src/main.rs:20:5".to_string(), arg2: None, json: false });
    run_cli(Commands::Refs { arg1: "src/main.rs:20:5".to_string(), arg2: None, json: true });

    // 4. TypeAt with arg2 = None
    run_cli(Commands::TypeAt { arg1: "src/lib.rs:10:1".to_string(), arg2: None, json: false });
    run_cli(Commands::TypeAt { arg1: "src/lib.rs:10:1".to_string(), arg2: None, json: true });

    // 5. Hover with arg2 = None
    run_cli(Commands::Hover { arg1: "src/lib.rs:10:1".to_string(), arg2: None, json: false });
    run_cli(Commands::Hover { arg1: "src/lib.rs:10:1".to_string(), arg2: None, json: true });
    // Hover on definition without signature (line 912)
    run_cli(Commands::Hover { arg1: "src/lib.rs:15:1".to_string(), arg2: None, json: false });

    // 6. Symbols with arg2 = None
    run_cli(Commands::Symbols { arg1: "test_fn".to_string(), arg2: None, json: false });
    run_cli(Commands::Symbols { arg1: "test_fn".to_string(), arg2: None, json: true });

    // 7. Compare with arg2 = None
    run_cli(Commands::Compare {
        arg1: "test_crate::test_fn".to_string(),
        arg2: None,
        symbol: None,
        scenarios: None,
        ridx_cmd: None,
        ra_cmd: "rust-analyzer".to_string(),
        capability: "def".to_string(),
        json: false,
        scip_file: None,
    });
    run_cli(Commands::Compare {
        arg1: "test_crate::test_fn".to_string(),
        arg2: None,
        symbol: None,
        scenarios: None,
        ridx_cmd: None,
        ra_cmd: "rust-analyzer".to_string(),
        capability: "def".to_string(),
        json: true,
        scip_file: None,
    });
}

#[test]
fn test_cli_parser_and_resolve_errors() {
    let _lock = lock_cli_test();
    use crate::cli::args::Cli;
    use clap::Parser;

    // 1. Cli parsing (lines 30-33 in cli/mod.rs)
    let parsed = Cli::try_parse_from(["ridx", "inspect", "some/path"]);
    assert!(parsed.is_ok());

    // 1b. Drop-in rust-analyzer mode: no subcommand defaults to Commands::Lsp
    let no_sub = Cli::try_parse_from(["ridx"]).unwrap();
    assert_eq!(no_sub.into_command(), Commands::Lsp { index_path: None });

    let ra_flags = Cli::try_parse_from(["ridx", "--log-file", "/tmp/ridx-ra.log", "--no-log-buffering", "-vv", "--stdio", "--index-path", "target/ridx"]).unwrap();
    assert_eq!(ra_flags.verbose, 2);
    assert!(ra_flags.no_log_buffering);
    assert!(ra_flags.stdio);
    assert_eq!(ra_flags.into_command(), Commands::Lsp { index_path: Some(std::path::PathBuf::from("target/ridx")) });

    let schema_flag = Cli::try_parse_from(["ridx", "--print-config-schema"]).unwrap();
    assert!(schema_flag.print_config_schema);

    // 2. try_resolve_index_and_target with an index that does not exist. Never without one: that looks in the
    // current directory, this repository, and indexed all of it, and the e2e tests then completed from that index.
    assert!(try_resolve_index_and_target("/nonexistent/idx".to_string(), Some("target".to_string())).is_err());

    // 3. Invalid variant_manifest.json and ridx_manifest.json (lines 75, 104 in resolve.rs)
    let temp = std::env::temp_dir().join(format!("ridx_invalid_manifest_{}", std::process::id()));
    fs::create_dir_all(&temp).unwrap();
    fs::write(temp.join("variant_manifest.json"), "invalid json content").unwrap();
    fs::write(temp.join("ridx_manifest.json"), "invalid json content").unwrap();
    assert!(find_manifest_for_path(&temp.join("sub/file.rs")).is_none());
    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn test_cli_index_builder_flag() {
    use crate::cli::args::Cli;
    use crate::indexer::IndexBuilder;
    use clap::Parser;

    let cli = Cli::try_parse_from(["ridx", "index", "--builder", "mitts"]).unwrap();
    match cli.into_command() {
        Commands::Index { builder, .. } => assert_eq!(builder, Some(IndexBuilder::Mitts)),
        _ => panic!("expected Commands::Index"),
    }

    let cli = Cli::try_parse_from(["ridx", "index", "--builder", "cargo"]).unwrap();
    match cli.into_command() {
        Commands::Index { builder, .. } => assert_eq!(builder, Some(IndexBuilder::Cargo)),
        _ => panic!("expected Commands::Index"),
    }

    let cli = Cli::try_parse_from(["ridx", "index"]).unwrap();
    match cli.into_command() {
        Commands::Index { builder, .. } => assert_eq!(builder, None),
        _ => panic!("expected Commands::Index"),
    }

    let err = Cli::try_parse_from(["ridx", "index", "--builder", "unknown_builder"]);
    assert!(err.is_err());
}
