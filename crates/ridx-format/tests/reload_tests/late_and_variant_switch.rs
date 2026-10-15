/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use crate::common::unique_temp_dir;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use std::fs::{self, File};
use std::io::BufWriter;

#[test]
fn test_late_metadata_publication_and_reload() {
    let test_dir = unique_temp_dir("ridx_late_meta");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir_all(&test_dir).unwrap();

    let gen = "000000000000beef";
    let mut writer = IndexWriter::new("app", "0.1.0");
    writer.set_unit_key("app_uk");
    writer.set_generation(gen);
    let mut hashes = std::collections::HashMap::new();
    hashes.insert("src/main.rs".to_string(), 0xbeefu64);
    writer.set_file_hashes(hashes);
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        name: "main".to_string(),
        path: "app::main".to_string(),
        signature: None,
        file: "src/main.rs".to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 2,
        end_col: 1,
    });
    let src_dir = test_dir.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("main.rs"), "fn main() {}\n").unwrap();

    let index_path = test_dir.join("app.ridx");
    writer
        .write_to(&mut BufWriter::new(File::create(&index_path).unwrap()))
        .unwrap();

    // Open index before sidecar metadata exists
    let mut ws = WorkspaceIndex::open(&test_dir).expect("should open index");
    assert_eq!(ws.generation_at(0), Some(gen));
    assert_eq!(ws.source_hash_for_file("src/main.rs"), Some(0xbeef));

    // Later: sidecar published
    let deps_dir = test_dir.join(".deps");
    fs::create_dir_all(&deps_dir).unwrap();
    let sidecar_info = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "app_uk",
        "generation": gen,
        "is_dependency": false,
        "dependencies": ["dep_unit_123"],
        "extern_units": {"dep": "dep_unit_123"},
        "file_hashes": {"src/main.rs": 0xbeefu64},
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&sidecar_info).unwrap(),
    )
    .unwrap();

    // reload_if_modified detects metadata change and reloads
    let reloaded = ws.reload_if_modified().expect("reload should succeed");
    assert!(
        reloaded,
        "reload_if_modified should detect newly published metadata"
    );

    // Modify ONLY sidecar JSON (update dependencies) without touching app.ridx
    std::thread::sleep(std::time::Duration::from_millis(20));
    let sidecar_updated = serde_json::json!({
        "name": "app",
        "version": "0.1.0",
        "unit_key": "app_uk",
        "generation": gen,
        "is_dependency": false,
        "dependencies": ["dep_unit_123", "dep_unit_456"],
        "extern_units": {"dep": "dep_unit_123", "dep2": "dep_unit_456"},
        "file_hashes": {"src/main.rs": 0xbeefu64},
    });
    fs::write(
        deps_dir.join("app.json"),
        serde_json::to_string_pretty(&sidecar_updated).unwrap(),
    )
    .unwrap();

    let reloaded_meta = ws.reload_if_modified().expect("reload should succeed");
    assert!(reloaded_meta);
    assert!(ws.is_file_reloaded("src/main.rs"));
    assert!(ws.is_file_reloaded(&test_dir.join("src/main.rs").to_string_lossy()));
    assert!(!ws.is_file_reloaded("nonexistent/path.rs"));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_reload_variant_switch_computed_gen_mismatch() {
    let test_dir = unique_temp_dir("ridx_var_switch_comp");
    let _ = fs::remove_dir_all(&test_dir);
    let var1_dir = test_dir.join("variants/var1");
    let var2_dir = test_dir.join("variants/var2");
    fs::create_dir_all(&var1_dir).unwrap();
    fs::create_dir_all(&var2_dir).unwrap();

    let mut w1 = IndexWriter::new("app", "0.1.0");
    w1.add_definition(RawDefinition {
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
        end_col: 10,
    });
    w1.write_to(&mut BufWriter::new(
        File::create(var1_dir.join("app.ridx")).unwrap(),
    ))
    .unwrap();

    // In var2: dep crate with generation "reader_gen"
    let mut w2 = IndexWriter::new("dep", "0.1.0");
    w2.set_unit_key("unitDep");
    w2.set_generation("reader_gen_1234");
    w2.write_to(&mut BufWriter::new(
        File::create(var2_dir.join("dep.ridx")).unwrap(),
    ))
    .unwrap();

    // Manifest initially points to var1
    let manifest1 = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var1",
        "variant_dir": "variants/var1",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev"
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest1).unwrap(),
    )
    .unwrap();

    let mut ws = WorkspaceIndex::open(&test_dir).expect("open workspace");
    assert_eq!(ws.active_variant_id(), Some("var1"));

    // Switch manifest to var2 with dependency shard having computed digest mismatch (lines 236-239 in reload.rs)
    std::thread::sleep(std::time::Duration::from_millis(50));
    let mut file_hashes = std::collections::HashMap::new();
    file_hashes.insert("src/dep.rs".to_string(), 9999u64);
    let manifest2 = serde_json::json!({
        "version": "2.0",
        "format_version": ridx_format::CURRENT_FORMAT_VERSION,
        "extractor_version": ridx_format::EXTRACTOR_VERSION,
        "active_variant_id": "var2",
        "variant_dir": "variants/var2",
        "workspace_root": test_dir.display().to_string(),
        "manifest_path": test_dir.join("Cargo.toml").display().to_string(),
        "profile": "dev",
        "dependency_shards": [{
            "name": "dep",
            "version": "0.1.0",
            "unit_key": "unitDep",
            "object_path": var2_dir.join("dep.ridx").display().to_string(),
            "source_root": test_dir.display().to_string(),
            "is_dependency": true,
            "dependencies": ["other"],
            "extern_units": {},
            "file_hashes": file_hashes
        }]
    });
    fs::write(
        test_dir.join("ridx_manifest.json"),
        serde_json::to_string_pretty(&manifest2).unwrap(),
    )
    .unwrap();

    let reloaded = ws.reload_if_modified().expect("reload variant switch");
    assert!(reloaded);
    assert_eq!(ws.active_variant_id(), Some("var2"));

    let _ = fs::remove_dir_all(&test_dir);
}
