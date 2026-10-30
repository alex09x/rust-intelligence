/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for textDocument/typeDefinition part 4:
//! local bindings, struct types, enum types, type aliases, and pattern variables.

use ridx::lsp::navigation::handle_type_definition;
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
fn test_type_definition_local_var_struct_type() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_1", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_1", "src/lib.rs", "pub struct «UserSession»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_1", "src/main.rs", "pub fn run() { let session = 42; }\n").unwrap();
    ws.add_type_at("pkg_t4_1", "src/main.rs", 1, 20, 1, 27, "UserSession").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let main_uri = ws.file_uri("pkg_t4_1", "src/main.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_t4_1", "src/lib.rs").unwrap();

    let req = make_pos_req(&main_uri, 0, 22);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], lib_uri);
    assert_range(&res["range"], 0, 11, 0, 22);
}

#[test]
fn test_type_definition_local_var_enum_type() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_2", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_2", "src/lib.rs", "pub enum «ConnectionState» { Open, Closed }\n", DefKind::Enum).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_2", "src/main.rs", "pub fn check() { let state = 1; }\n").unwrap();
    ws.add_type_at("pkg_t4_2", "src/main.rs", 1, 21, 1, 26, "ConnectionState").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let main_uri = ws.file_uri("pkg_t4_2", "src/main.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_t4_2", "src/lib.rs").unwrap();

    let req = make_pos_req(&main_uri, 0, 23);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], lib_uri);
    assert_range(&res["range"], 0, 9, 0, 24);
}

#[test]
fn test_type_definition_type_alias_resolution() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_3", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_3", "src/lib.rs", "pub type «UserId» = u64;\n", DefKind::TyAlias).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_3", "src/handler.rs", "pub fn auth() { let uid = 10; }\n").unwrap();
    ws.add_type_at("pkg_t4_3", "src/handler.rs", 1, 20, 1, 23, "UserId").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let handler_uri = ws.file_uri("pkg_t4_3", "src/handler.rs").unwrap();
    let lib_uri = ws.file_uri("pkg_t4_3", "src/lib.rs").unwrap();

    let req = make_pos_req(&handler_uri, 0, 21);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], lib_uri);
    assert_range(&res["range"], 0, 9, 0, 15);
}

#[test]
fn test_type_definition_fn_return_type_binding() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_4", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_4", "src/types.rs", "pub struct «Payload»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_4", "src/lib.rs", "pub fn fetch() { let result = 0; }\n").unwrap();
    ws.add_type_at("pkg_t4_4", "src/lib.rs", 1, 21, 1, 27, "Payload").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t4_4", "src/lib.rs").unwrap();
    let types_uri = ws.file_uri("pkg_t4_4", "src/types.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 23);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], types_uri);
    assert_range(&res["range"], 0, 11, 0, 18);
}

#[test]
fn test_type_definition_struct_tuple_field() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_5", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_5", "src/geom.rs", "pub struct «Coord»(pub i32, pub i32);\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_5", "src/lib.rs", "pub fn draw() { let pos = 0; }\n").unwrap();
    ws.add_type_at("pkg_t4_5", "src/lib.rs", 1, 20, 1, 23, "Coord").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t4_5", "src/lib.rs").unwrap();
    let geom_uri = ws.file_uri("pkg_t4_5", "src/geom.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 21);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], geom_uri);
    assert_range(&res["range"], 0, 11, 0, 16);
}

#[test]
fn test_type_definition_struct_named_field() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_6", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_6", "src/models.rs", "pub struct «Address» { pub city: String }\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_6", "src/lib.rs", "pub fn mail() { let addr = 0; }\n").unwrap();
    ws.add_type_at("pkg_t4_6", "src/lib.rs", 1, 20, 1, 24, "Address").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t4_6", "src/lib.rs").unwrap();
    let models_uri = ws.file_uri("pkg_t4_6", "src/models.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 22);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], models_uri);
    assert_range(&res["range"], 0, 11, 0, 18);
}

#[test]
fn test_type_definition_const_type_target() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_7", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_7", "src/cfg.rs", "pub struct «ConfigLimits»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_7", "src/lib.rs", "pub const LIMITS: u32 = 100;\n").unwrap();
    ws.add_type_at("pkg_t4_7", "src/lib.rs", 1, 10, 1, 16, "ConfigLimits").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t4_7", "src/lib.rs").unwrap();
    let cfg_uri = ws.file_uri("pkg_t4_7", "src/cfg.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 12);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], cfg_uri);
    assert_range(&res["range"], 0, 11, 0, 23);
}

#[test]
fn test_type_definition_static_item_type() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_8", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_8", "src/reg.rs", "pub struct «GlobalRegistry»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_8", "src/lib.rs", "pub static REGISTRY: u32 = 1;\n").unwrap();
    ws.add_type_at("pkg_t4_8", "src/lib.rs", 1, 11, 1, 19, "GlobalRegistry").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t4_8", "src/lib.rs").unwrap();
    let reg_uri = ws.file_uri("pkg_t4_8", "src/reg.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 15);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], reg_uri);
    assert_range(&res["range"], 0, 11, 0, 25);
}

#[test]
fn test_type_definition_match_arm_pattern_variable() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_9", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_9", "src/evt.rs", "pub struct «EventData»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_9", "src/lib.rs", "pub fn on_event() { let data = 1; }\n").unwrap();
    ws.add_type_at("pkg_t4_9", "src/lib.rs", 1, 24, 1, 28, "EventData").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t4_9", "src/lib.rs").unwrap();
    let evt_uri = ws.file_uri("pkg_t4_9", "src/evt.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 25);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], evt_uri);
    assert_range(&res["range"], 0, 11, 0, 20);
}

#[test]
fn test_type_definition_closure_parameter_type() {
    let mut ws = SyntheticWorkspaceFixture::new().unwrap();
    ws.add_crate("pkg_t4_10", "0.1.0").unwrap();
    let (_, ids) = ws.add_source_file_with_defs("pkg_t4_10", "src/ctx.rs", "pub struct «RequestContext»;\n", DefKind::Struct).unwrap();
    assert_eq!(ids.len(), 1);

    ws.add_source_file("pkg_t4_10", "src/lib.rs", "pub fn handle() { let ctx = 0; }\n").unwrap();
    ws.add_type_at("pkg_t4_10", "src/lib.rs", 1, 22, 1, 25, "RequestContext").unwrap();

    ws.build_all_indices().unwrap();
    let index = WorkspaceIndex::open(ws.root_path()).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let lib_uri = ws.file_uri("pkg_t4_10", "src/lib.rs").unwrap();
    let ctx_uri = ws.file_uri("pkg_t4_10", "src/ctx.rs").unwrap();

    let req = make_pos_req(&lib_uri, 0, 23);
    let res = handle_type_definition(&req, Some(&index), &overlay_mgr, &mut conv);
    assert_eq!(res["uri"], ctx_uri);
    assert_range(&res["range"], 0, 11, 0, 25);
}
