/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hermetic integration tests for Domain 9: textDocument/typeDefinition and declaration contract.

use ridx::lsp::navigation::{handle_declaration, handle_definition, handle_type_definition};
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

// ---------------------------------------------------------------------------
// textDocument/declaration — contract parity with definition
// ---------------------------------------------------------------------------

#[test]
fn test_l1_declaration_definition_parity_contract() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_decl_contract", "0.1.0").unwrap();
    ws.add_source_file_with_defs(
        "pkg_decl_contract",
        "src/lib.rs",
        "pub trait Runner {\n    fn «execute»(&self);\n}\npub struct App;\nimpl Runner for App {\n    fn «execute»(&self) {}\n}\n",
        DefKind::Fn,
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_decl_contract").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_decl_contract", "src/lib.rs").unwrap();
    let file_path = ws.file_path("pkg_decl_contract", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    // Clean overlay buffer
    let code = "pub trait Runner {\n    fn execute(&self);\n}\npub struct App;\nimpl Runner for App {\n    fn execute(&self) {}\n}\n";
    overlay_mgr.open_document(&uri, file_str, 1, code.to_string());

    // Positions:
    // 1. Trait method declaration site (line 1, col 8)
    // 2. Impl method definition site (line 5, col 8)
    // 3. Whitespace (line 0, col 0)
    // 4. Out of bounds (line 99, col 0)
    // 5. Unregistered URI
    let positions = [
        make_pos_req(&uri, 1, 8),
        make_pos_req(&uri, 5, 8),
        make_pos_req(&uri, 0, 0),
        make_pos_req(&uri, 99, 0),
        make_pos_req("file:///workspace/nonexistent.rs", 0, 0),
    ];

    for req in &positions {
        let decl_res = handle_declaration(req, Some(&index), &overlay_mgr, &mut conv);
        let def_res = handle_definition(req, Some(&index), &overlay_mgr, &mut conv);
        assert_eq!(decl_res, def_res, "Declaration must match definition verbatim");
    }

    // Dirty overlay buffer parity
    let dirty_code = "// Shift line\npub trait Runner {\n    fn execute(&self);\n}\n";
    overlay_mgr.update(file_str, 2, dirty_code.to_string());
    let req_dirty = make_pos_req(&uri, 2, 8);
    let decl_dirty = handle_declaration(&req_dirty, Some(&index), &overlay_mgr, &mut conv);
    let def_dirty = handle_definition(&req_dirty, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(decl_dirty, def_dirty, "Declaration dirty overlay must match definition");
}

// ---------------------------------------------------------------------------
// textDocument/typeDefinition — comprehensive edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_l1_type_definition_local_variable_with_overlay() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_local_typedef", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_local_typedef",
            "src/lib.rs",
            "pub struct «Widget»;\n",
            DefKind::Struct,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    let full_code = "pub struct Widget;\npub fn run() {\n    let w = Widget;\n}\n";
    ws.add_source_file("pkg_local_typedef", "src/lib.rs", full_code)
        .unwrap();
    // Line 3, cols 9..10 is `w`, type is "Widget"
    ws.add_type_at("pkg_local_typedef", "src/lib.rs", 3, 9, 3, 10, "Widget")
        .unwrap();

    let shard = ws.build_crate_index("pkg_local_typedef").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let uri = ws.file_uri("pkg_local_typedef", "src/lib.rs").unwrap();
    let file_path = ws.file_path("pkg_local_typedef", "src/lib.rs").unwrap();
    let file_str = file_path.to_str().unwrap();

    let mut overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    // Open clean overlay document
    overlay_mgr.open_document(&uri, file_str, 1, full_code.to_string());

    // Cursor on `w` on line 2 (0-based), col 8
    let req = make_pos_req(&uri, 2, 8);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    assert_eq!(res["range"]["end"]["line"], 0);
    assert_eq!(res["range"]["end"]["character"], 17);
}

#[test]
fn test_l1_type_definition_fn_parameter_cross_file() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_param_typedef", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_param_typedef",
            "src/config.rs",
            "pub struct «Config»;\n",
            DefKind::Struct,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file(
        "pkg_param_typedef",
        "src/lib.rs",
        "pub fn run(cfg: Config) {}\n",
    )
    .unwrap();
    // Line 1, cols 12..15 is `cfg`, type is "Config"
    ws.add_type_at("pkg_param_typedef", "src/lib.rs", 1, 12, 1, 15, "Config")
        .unwrap();

    let shard = ws.build_crate_index("pkg_param_typedef").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let lib_uri = ws.file_uri("pkg_param_typedef", "src/lib.rs").unwrap();
    let config_uri = ws.file_uri("pkg_param_typedef", "src/config.rs").unwrap();

    // Cursor on `cfg` on line 0, col 12
    let req = make_pos_req(&lib_uri, 0, 12);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], config_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    assert_eq!(res["range"]["end"]["character"], 17);
}

#[test]
fn test_l1_type_definition_struct_field_target() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_field", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_field",
            "src/models.rs",
            "pub struct «Payload»;\n",
            DefKind::Struct,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file(
        "pkg_field",
        "src/lib.rs",
        "pub struct Container {\n    pub data: Payload,\n}\n",
    )
    .unwrap();
    // Line 2, cols 9..13 is field `data`, type is "Payload"
    ws.add_type_at("pkg_field", "src/lib.rs", 2, 9, 2, 13, "Payload")
        .unwrap();

    let shard = ws.build_crate_index("pkg_field").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let lib_uri = ws.file_uri("pkg_field", "src/lib.rs").unwrap();
    let models_uri = ws.file_uri("pkg_field", "src/models.rs").unwrap();

    let req = make_pos_req(&lib_uri, 1, 9);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], models_uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    assert_eq!(res["range"]["end"]["character"], 18);
}

#[test]
fn test_l1_type_definition_field_access_expression() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_field_expr", "0.1.0").unwrap();
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_field_expr",
            "src/lib.rs",
            "pub struct «InnerCore»;\n",
            DefKind::Struct,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    let code = "pub struct InnerCore;\npub struct Wrapper {\n    pub inner: InnerCore,\n}\nimpl Wrapper {\n    pub fn get(&self) { self.inner; }\n}\n";
    ws.add_source_file("pkg_field_expr", "src/lib.rs", code)
        .unwrap();
    // Line 6, 1-based cols 30..35 is `inner` in `self.inner`, type is "InnerCore"
    ws.add_type_at("pkg_field_expr", "src/lib.rs", 6, 30, 6, 35, "InnerCore")
        .unwrap();

    let shard = ws.build_crate_index("pkg_field_expr").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_field_expr", "src/lib.rs").unwrap();

    // Cursor on `inner` in `self.inner` (line 5, col 30)
    let req = make_pos_req(&uri, 5, 30);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 11);
    assert_eq!(res["range"]["end"]["character"], 20);
}

#[test]
fn test_l1_type_definition_utf16_with_emoji_prefix() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_utf16_type", "0.1.0").unwrap();
    // 2 crabs in comment: each takes 2 UTF-16 code units (surrogate pair)
    // "/* 🦀🦀 */ pub struct " = 3 + 4 + 4 + 11 = 22 UTF-16 code units
    let (_, ids) = ws
        .add_source_file_with_defs(
            "pkg_utf16_type",
            "src/lib.rs",
            "/* 🦀🦀 */ pub struct «SpecialToken»;\n",
            DefKind::Struct,
        )
        .unwrap();
    assert_eq!(ids.len(), 1);

    let full_code = "/* 🦀🦀 */ pub struct SpecialToken;\npub fn run() { let token = SpecialToken; }\n";
    ws.add_source_file("pkg_utf16_type", "src/lib.rs", full_code)
        .unwrap();
    // Line 2, cols 20..25 is `token`, type is "SpecialToken"
    ws.add_type_at("pkg_utf16_type", "src/lib.rs", 2, 20, 2, 25, "SpecialToken")
        .unwrap();

    let shard = ws.build_crate_index("pkg_utf16_type").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_utf16_type", "src/lib.rs").unwrap();

    let req = make_pos_req(&uri, 1, 21);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res["uri"], uri);
    assert_eq!(res["range"]["start"]["line"], 0);
    assert_eq!(res["range"]["start"]["character"], 22);
    assert_eq!(res["range"]["end"]["character"], 34);
}

#[test]
fn test_l1_type_definition_missing_from_type_table_returns_null() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_missing_type", "0.1.0").unwrap();
    ws.add_source_file(
        "pkg_missing_type",
        "src/lib.rs",
        "pub fn compute() { let untyped = 123; }\n",
    )
    .unwrap();

    let shard = ws.build_crate_index("pkg_missing_type").unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = ws.file_uri("pkg_missing_type", "src/lib.rs").unwrap();

    // Query on valid code with clean buffer, but without entry in type table
    let req = make_pos_req(&uri, 0, 25);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);

    assert_eq!(res, Value::Null);
}
