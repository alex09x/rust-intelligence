/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/definition and textDocument/declaration (Part 2):
//! cross-crate multi-shard resolution, target file dirty overlay mapping, deleted definition
//! fallback, UTF-16 non-BMP surrogate pairs, same-named cross-crate symbols, and modified imports.

use ridx::lsp::navigation::handle_definition;
use ridx::lsp::protocol::PositionConverter;
use ridx::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use ridx_test_support::fixture::SyntheticWorkspaceFixture;
use serde_json::{json, Value};

fn make_pos_req(uri: &str, line: u32, col: u32) -> Value {
    json!({
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": col }
        }
    })
}

fn assert_range(range: &Value, start_line: u32, start_col: u32, end_line: u32, end_col: u32) {
    assert_eq!(range["start"]["line"], start_line);
    assert_eq!(range["start"]["character"], start_col);
    assert_eq!(range["end"]["line"], end_line);
    assert_eq!(range["end"]["character"], end_col);
}

#[test]
fn test_l1_definition_cross_crate_two_shards() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("core_pkg", "0.1.0").unwrap();
    ws.add_crate("app_pkg", "0.1.0").unwrap();

    let (_, core_ids) = ws.add_source_file_with_defs("core_pkg", "src/lib.rs", "pub struct «EngineCore»;\n", DefKind::Struct).unwrap();

    ws.add_refs_to_foreign_def("app_pkg", "src/lib.rs", "use core_pkg::EngineCore;\npub fn run() { let _ = «EngineCore»; }\n", "core_pkg", core_ids[0]).unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    assert_eq!(index.readers().len(), 2);

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let app_uri = ws.file_uri("app_pkg", "src/lib.rs").unwrap();
    let core_uri = ws.file_uri("core_pkg", "src/lib.rs").unwrap();

    // Query on line 1, col 23 in app_pkg/src/lib.rs
    let req = make_pos_req(&app_uri, 1, 23);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], core_uri);
    assert_range(&res["range"], 0, 11, 0, 21);
}

#[test]
fn test_l1_definition_target_file_in_dirty_overlay_mapped_live_range() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_target_ov", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_target_ov", "src/models.rs", "pub struct «Worker»;\n", DefKind::Struct).unwrap();

    let caller_code = "mod models;\npub fn run() { let _ = models::«Worker»; }\n";
    ws.add_refs_to_def("pkg_target_ov", "src/lib.rs", caller_code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_target_ov").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let lib_uri = ws.file_uri("pkg_target_ov", "src/lib.rs").unwrap();
    let models_uri = ws.file_uri("pkg_target_ov", "src/models.rs").unwrap();
    let models_path = ws.file_path("pkg_target_ov", "src/models.rs").unwrap();
    let models_str = models_path.to_str().unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Clean overlay document for models.rs
    let initial_models = "pub struct Worker;\n";
    overlay_mgr.open_document(&models_uri, models_str, 1, initial_models.to_string());

    // Update models.rs with 2 prepended comment lines shifting Worker from line 0 to line 2
    let dirty_models = "// line1\n// line2\npub struct Worker;\n";
    overlay_mgr.update(models_str, 2, dirty_models.to_string());

    // Query on line 1, col 32 (models::Worker) in src/lib.rs
    let req = make_pos_req(&lib_uri, 1, 32);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], models_uri);
    // Definition at compiled line 0 is mapped via map_compiled_to_live to line 2 (0-based)
    assert_range(&res["range"], 2, 11, 2, 17);
}

#[test]
fn test_l1_definition_target_file_dirty_overlay_definition_deleted_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_del_ov", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_del_ov", "src/models.rs", "pub struct «DeletedItem»;\n", DefKind::Struct).unwrap();

    let caller_code = "mod models;\npub fn run() { let _ = models::«DeletedItem»; }\n";
    ws.add_refs_to_def("pkg_del_ov", "src/lib.rs", caller_code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_del_ov").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let lib_uri = ws.file_uri("pkg_del_ov", "src/lib.rs").unwrap();
    let models_uri = ws.file_uri("pkg_del_ov", "src/models.rs").unwrap();
    let models_path = ws.file_path("pkg_del_ov", "src/models.rs").unwrap();
    let models_str = models_path.to_str().unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Clean overlay document for models.rs
    let initial_models = "pub struct DeletedItem;\n";
    overlay_mgr.open_document(&models_uri, models_str, 1, initial_models.to_string());

    // Dirty overlay where the definition was completely deleted
    let dirty_models = "// definition removed\n";
    overlay_mgr.update(models_str, 2, dirty_models.to_string());

    // Query on line 1, col 32 in src/lib.rs
    let req = make_pos_req(&lib_uri, 1, 32);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    // is_def_valid_in_overlay returns false because DeletedItem is missing from models.rs overlay,
    // collapsing range to (0,0,0,0) and causing handle_definition to return Null
    assert_eq!(res, Value::Null);
}

#[test]
fn test_l1_definition_utf16_multibyte_prefix_with_emoji() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_utf16_def", "0.1.0").unwrap();

    // Line 0: "/* 🚀 */ pub struct «RocketShip»;\n"
    // "/* " (3) + "🚀" (1 char, 2 UTF-16 units) + " */ pub struct " (15) = 19 chars -> start_col = 20
    // UTF-16 code units before RocketShip: 3 + 2 + 15 = 20 UTF-16 units
    // RocketShip has 10 chars -> end_col = 30
    let def_code = "/* 🚀 */ pub struct «RocketShip»;\n";
    let (_, ids) = ws.add_source_file_with_defs("pkg_utf16_def", "src/lib.rs", def_code, DefKind::Struct).unwrap();

    // Line 1: "/* 🦀 */ pub fn launch() { let _ = «RocketShip»; }\n"
    // "/* " (3) + "🦀" (1 char, 2 UTF-16 units) + " */ pub fn launch() { let _ = " (30) = 34 chars -> start_col = 35
    // UTF-16 code units before RocketShip on line 1: 3 + 2 + 30 = 35 UTF-16 units (RocketShip is 35..45)
    let caller_code = "/* 🚀 */ pub struct RocketShip;\n/* 🦀 */ pub fn launch() { let _ = «RocketShip»; }\n";
    ws.add_refs_to_def("pkg_utf16_def", "src/lib.rs", caller_code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_utf16_def").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_utf16_def", "src/lib.rs").unwrap();

    // Query on line 1, character 38 (inside RocketShip, offset by 2 UTF-16 surrogate code units)
    let req = make_pos_req(&uri, 1, 38);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], uri);
    // Target definition on line 0 has 20 UTF-16 start character, 30 end character
    assert_range(&res["range"], 0, 20, 0, 30);
}

#[test]
fn test_l1_definition_same_named_symbol_in_two_crates_resolves_referenced_target() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    // Order matters: aaa_other is first shard alphabetically, zzz_target is second
    ws.add_crate("aaa_other", "0.1.0").unwrap();
    ws.add_crate("zzz_target", "0.1.0").unwrap();
    ws.add_crate("consumer_pkg", "0.1.0").unwrap();

    let (_, other_ids) = ws.add_source_file_with_defs("aaa_other", "src/lib.rs", "pub struct «Config»;\n", DefKind::Struct).unwrap();
    let (_, target_ids) = ws.add_source_file_with_defs("zzz_target", "src/lib.rs", "pub struct «Config»;\n", DefKind::Struct).unwrap();
    assert_eq!(other_ids.len(), 1);
    assert_eq!(target_ids.len(), 1);

    ws.add_refs_to_foreign_def("consumer_pkg", "src/lib.rs", "use zzz_target::Config;\npub fn run() { let _ = «Config»; }\n", "zzz_target", target_ids[0]).unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    assert_eq!(index.readers().len(), 3);

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let consumer_uri = ws.file_uri("consumer_pkg", "src/lib.rs").unwrap();
    let target_uri = ws.file_uri("zzz_target", "src/lib.rs").unwrap();

    let req = make_pos_req(&consumer_uri, 1, 23);
    let res = handle_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    // Must resolve to zzz_target, NOT aaa_other (even though aaa_other is the first reader)
    assert_eq!(res["uri"], target_uri);
    assert_range(&res["range"], 0, 11, 0, 17);
}

#[test]
fn test_l1_definition_modified_imports_skips_workspace_index() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_mod_imports", "0.1.0").unwrap();

    let (_, ids) = ws.add_source_file_with_defs("pkg_mod_imports", "src/models.rs", "pub struct «RemoteItem»;\n", DefKind::Struct).unwrap();

    let caller_code = "mod models;\npub fn run() { let _ = models::«RemoteItem»; }\n";
    ws.add_refs_to_def("pkg_mod_imports", "src/lib.rs", caller_code, ids[0]).unwrap();

    let shard = ws.build_crate_index("pkg_mod_imports").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let lib_uri = ws.file_uri("pkg_mod_imports", "src/lib.rs").unwrap();
    let models_uri = ws.file_uri("pkg_mod_imports", "src/models.rs").unwrap();
    let lib_path = ws.file_path("pkg_mod_imports", "src/lib.rs").unwrap();
    let lib_str = lib_path.to_str().unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Clean overlay document for src/lib.rs
    let initial_code = "mod models;\npub fn run() { let _ = models::RemoteItem; }\n";
    overlay_mgr.open_document(&lib_uri, lib_str, 1, initial_code.to_string());

    // 1. Control query on clean overlay: resolves to models.rs
    let req_clean = make_pos_req(&lib_uri, 1, 32);
    let res_clean = handle_definition(&req_clean, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res_clean["uri"], models_uri);
    assert_range(&res_clean["range"], 0, 11, 0, 21);

    // 2. An added import of another name (`Foo`) changes nothing for `RemoteItem`: the index still answers, mapped.
    let dirty_code = "use external_crate::Foo;\nmod models;\npub fn run() { let _ = models::RemoteItem; }\n";
    overlay_mgr.update(lib_str, 2, dirty_code.to_string());
    let res = handle_definition(&make_pos_req(&lib_uri, 2, 32), Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], models_uri, "an unrelated import keeps the index answer");

    // 3. An import of that very name may change what it resolves to: the index is skipped.
    let shadowing = "use external_crate::RemoteItem;\nmod models;\npub fn run() { let _ = models::RemoteItem; }\n";
    overlay_mgr.update(lib_str, 3, shadowing.to_string());
    let res = handle_definition(&make_pos_req(&lib_uri, 2, 32), Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res, Value::Null);
}
