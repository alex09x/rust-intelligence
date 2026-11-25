/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

mod common;
#[path = "reload_tests/late_and_variant_switch.rs"]
mod late_and_variant_switch;

use common::unique_temp_dir;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;

#[test]
fn test_workspace_relative_path_not_rebased_to_member_twice() {
    let test_dir = unique_temp_dir("ridx_rebase_test");
    let _ = fs::remove_dir_all(&test_dir);
    let member_dir = test_dir.join("crates/app");
    fs::create_dir_all(member_dir.join("src")).unwrap();
    let src_file = member_dir.join("src/lib.rs");
    fs::write(&src_file, "pub fn hello() {}\n").unwrap();

    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "hello".to_string(),
        path: "app::hello".to_string(),
        signature: Some("fn()".to_string()),
        file: "crates/app/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 17,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer.write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap())).unwrap();

    let app_meta = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": app_ridx.display().to_string(),
        "source_root": member_dir.display().to_string(),
        "is_dependency": false
    });
    fs::write(deps_dir.join("app.json"), serde_json::to_string_pretty(&app_meta).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&var_dir).expect("failed to open workspace");
    let symbols = ws.search_symbols("hello");
    assert_eq!(symbols.len(), 1);
    let resolved = symbols[0].resolved_path();

    // Must NOT be double rebased to crates/app/crates/app/src/lib.rs
    assert_eq!(resolved, src_file);
    assert!(resolved.exists(), "resolved path {:?} must exist", resolved);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_single_shard_reload_refreshes_metadata_and_hashes() {
    let test_dir = unique_temp_dir("ridx_single_reload_test");
    let _ = fs::remove_dir_all(&test_dir);
    let deps_dir = test_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();
    let src_dir = test_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn foo() {}\n").unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.set_unit_key("unitApp");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "app::foo".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    let ridx_file = test_dir.join("app.ridx");
    writer.write_to(&mut BufWriter::new(File::create(&ridx_file).unwrap())).unwrap();

    let meta = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": ridx_file.display().to_string(),
        "source_root": test_dir.display().to_string(),
        "file_hashes": {
            "src/lib.rs": 100u64
        }
    });
    fs::write(deps_dir.join("app.json"), serde_json::to_string_pretty(&meta).unwrap()).unwrap();

    let mut ws = WorkspaceIndex::open(&ridx_file).expect("failed to open single shard");
    assert_eq!(ws.source_hash_for_file("src/lib.rs"), Some(100));

    // Update metadata and touch .ridx
    std::thread::sleep(std::time::Duration::from_millis(50));
    let meta_updated = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "unitApp",
        "object_path": ridx_file.display().to_string(),
        "source_root": test_dir.display().to_string(),
        "file_hashes": {
            "src/lib.rs": 200u64
        }
    });
    fs::write(deps_dir.join("app.json"), serde_json::to_string_pretty(&meta_updated).unwrap()).unwrap();
    // Re-write ridx_file with later mtime
    let mut writer2 = IndexWriter::new("app", "0.1.0");
    writer2.set_unit_key("unitApp");
    writer2.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "app::foo".to_string(),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    // Published as the indexer does, beside the mapped shard and renamed over it.
    let tmp = ridx_file.with_extension("ridx.tmp");
    writer2.write_to(&mut BufWriter::new(File::create(&tmp).unwrap())).unwrap();
    fs::rename(&tmp, &ridx_file).unwrap();

    let reloaded = ws.reload_if_modified().expect("reload failed");
    assert!(reloaded, "expected reload_if_modified to return true");
    assert_eq!(ws.source_hash_for_file("src/lib.rs"), Some(200));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_null_local_source_root_resolves_via_workspace_root() {
    let test_dir = unique_temp_dir("ridx_ws_root_res_test");
    let _ = fs::remove_dir_all(&test_dir);
    let var_dir = test_dir.join("variants/var1");
    let deps_dir = var_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();

    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.set_unit_key("unitApp");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "foo".to_string(),
        path: "app::foo".to_string(),
        signature: None,
        file: "crates/app/src/lib.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 15,
    });
    let app_ridx = var_dir.join("app.ridx");
    writer.write_to(&mut BufWriter::new(File::create(&app_ridx).unwrap())).unwrap();

    let manifest = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev"
    });
    fs::write(test_dir.join("ridx_manifest.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();

    let ws = WorkspaceIndex::open(&test_dir).expect("failed to open workspace");
    let def = ws.find_definition_by_path("app::foo").expect("should find definition");

    let resolved = def.resolved_path();
    assert_eq!(resolved, test_dir.join("crates/app/src/lib.rs"));

    let _ = fs::remove_dir_all(&test_dir);
}
