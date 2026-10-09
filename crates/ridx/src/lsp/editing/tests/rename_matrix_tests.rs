/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Matrix and cross-file workspace symbol rename tests with syn::parse_file oracle.
//!
//! Verifies renaming across compilation units for functions, structs, enums,
//! type aliases, consts, caller-site triggers, and dirty overlay line shift mappings.

use crate::lsp::editing::rename::handle_rename;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::DefKind;
use ridx_test_support::{apply_workspace_edit, CursorPos, FixtureIndexBuilder};
use serde_json::json;
use std::collections::HashMap;
use std::fs;

pub(crate) struct WorkspaceFixture {
    pub(crate) temp_dir: tempfile::TempDir,
    pub(crate) ws: ridx_format::WorkspaceIndex,
    pub(crate) files: HashMap<String, String>,
    pub(crate) cursor: CursorPos,
    pub(crate) cursor_uri: String,
    pub(crate) lib_uri: String,
    pub(crate) caller_uri: String,
}

impl WorkspaceFixture {
    pub(crate) fn new_two_file(
        crate_name: &str,
        def_kind: DefKind,
        decl_fixture: &str,
        caller_fixture: &str,
    ) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let lib_path = temp.path().join("lib.rs");
        let caller_path = temp.path().join("caller.rs");

        let lib_str = lib_path.to_str().unwrap().to_string();
        let caller_str = caller_path.to_str().unwrap().to_string();

        let mut builder = FixtureIndexBuilder::new(crate_name, "0.1.0");
        let (parsed_decl, def_ids) =
            builder.add_defs_and_self_refs_fixture(decl_fixture, &lib_str, crate_name, def_kind);
        fs::write(&lib_path, &parsed_decl.clean_text).unwrap();

        let parsed_caller =
            builder.add_refs_to_def_fixture(caller_fixture, &caller_str, def_ids[0]);
        fs::write(&caller_path, &parsed_caller.clean_text).unwrap();

        let idx_file = temp.path().join("index.ridx");
        builder.write_to_file(&idx_file);
        let ws = ridx_format::WorkspaceIndex::open(&idx_file).unwrap();

        let lib_uri = path_to_uri(&lib_str);
        let caller_uri = path_to_uri(&caller_str);

        let (cursor, cursor_uri) = if let Some(cur) = parsed_decl.cursor {
            (cur, lib_uri.clone())
        } else if let Some(cur) = parsed_caller.cursor {
            (cur, caller_uri.clone())
        } else {
            panic!("Fixture must contain at least one $0 cursor marker");
        };

        let mut files = HashMap::new();
        files.insert(lib_uri.clone(), parsed_decl.clean_text);
        files.insert(caller_uri.clone(), parsed_caller.clean_text);

        Self {
            temp_dir: temp,
            ws,
            files,
            cursor,
            cursor_uri,
            lib_uri,
            caller_uri,
        }
    }
}

#[test]
fn test_workspace_rename_fn_across_files() {
    let fix = WorkspaceFixture::new_two_file(
        "math",
        DefKind::Fn,
        "pub fn «$0calculate»(x: i32) -> i32 { x * 2 }\n",
        "pub fn invoke() -> i32 { math::«calculate»(21) }\n",
    );

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &fix.cursor_uri },
            "position": { "line": fix.cursor.line, "character": fix.cursor.col_utf16 },
            "newName": "compute_double"
        }
    });

    let res = handle_rename(&req, Some(&fix.ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(res["changes"].get(&fix.lib_uri).is_some());
    assert!(res["changes"].get(&fix.caller_uri).is_some());

    let updated = apply_workspace_edit(&fix.files, &res);
    assert_eq!(
        updated[&fix.lib_uri],
        "pub fn compute_double(x: i32) -> i32 { x * 2 }\n"
    );
    assert_eq!(
        updated[&fix.caller_uri],
        "pub fn invoke() -> i32 { math::compute_double(21) }\n"
    );

    syn::parse_file(&updated[&fix.lib_uri]).expect("lib.rs must be valid Rust");
    syn::parse_file(&updated[&fix.caller_uri]).expect("caller.rs must be valid Rust");
}

#[test]
fn test_workspace_rename_from_caller_site() {
    let fix = WorkspaceFixture::new_two_file(
        "pkg",
        DefKind::Fn,
        "pub fn «transform»(s: &str) -> usize { s.len() }\n",
        "pub fn run() -> usize { pkg::«$0transform»(\"hello\") }\n",
    );

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &fix.cursor_uri },
            "position": { "line": fix.cursor.line, "character": fix.cursor.col_utf16 },
            "newName": "measure"
        }
    });

    let res = handle_rename(&req, Some(&fix.ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(res["changes"].get(&fix.lib_uri).is_some());
    assert!(res["changes"].get(&fix.caller_uri).is_some());

    let updated = apply_workspace_edit(&fix.files, &res);
    assert_eq!(
        updated[&fix.lib_uri],
        "pub fn measure(s: &str) -> usize { s.len() }\n"
    );
    assert_eq!(
        updated[&fix.caller_uri],
        "pub fn run() -> usize { pkg::measure(\"hello\") }\n"
    );

    syn::parse_file(&updated[&fix.lib_uri]).expect("valid Rust");
    syn::parse_file(&updated[&fix.caller_uri]).expect("valid Rust");
}

#[test]
fn test_workspace_rename_struct() {
    let fix = WorkspaceFixture::new_two_file(
        "model",
        DefKind::Struct,
        "pub struct «$0Config» { pub port: u16 }\n",
        "pub fn setup(c: model::«Config») -> u16 { c.port }\n",
    );

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &fix.cursor_uri },
            "position": { "line": fix.cursor.line, "character": fix.cursor.col_utf16 },
            "newName": "ServerSettings"
        }
    });

    let res = handle_rename(&req, Some(&fix.ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    let updated = apply_workspace_edit(&fix.files, &res);

    assert_eq!(
        updated[&fix.lib_uri],
        "pub struct ServerSettings { pub port: u16 }\n"
    );
    assert_eq!(
        updated[&fix.caller_uri],
        "pub fn setup(c: model::ServerSettings) -> u16 { c.port }\n"
    );

    syn::parse_file(&updated[&fix.lib_uri]).expect("valid Rust");
    syn::parse_file(&updated[&fix.caller_uri]).expect("valid Rust");
}

#[test]
fn test_workspace_rename_enum() {
    let fix = WorkspaceFixture::new_two_file(
        "types",
        DefKind::Enum,
        "pub enum «$0Status» { Ready, Failed }\n",
        "pub fn check(s: types::«Status») -> bool { matches!(s, types::«Status»::Ready) }\n",
    );

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &fix.cursor_uri },
            "position": { "line": fix.cursor.line, "character": fix.cursor.col_utf16 },
            "newName": "ProcessState"
        }
    });

    let res = handle_rename(&req, Some(&fix.ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    let updated = apply_workspace_edit(&fix.files, &res);

    assert_eq!(
        updated[&fix.lib_uri],
        "pub enum ProcessState { Ready, Failed }\n"
    );
    assert_eq!(
        updated[&fix.caller_uri],
        "pub fn check(s: types::ProcessState) -> bool { matches!(s, types::ProcessState::Ready) }\n"
    );

    syn::parse_file(&updated[&fix.lib_uri]).expect("valid Rust");
    syn::parse_file(&updated[&fix.caller_uri]).expect("valid Rust");
}

#[test]
fn test_workspace_rename_type_alias() {
    let fix = WorkspaceFixture::new_two_file(
        "aliases",
        DefKind::TyAlias,
        "pub type «$0ByteCount» = usize;\n",
        "pub fn alloc(size: aliases::«ByteCount») {}\n",
    );

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &fix.cursor_uri },
            "position": { "line": fix.cursor.line, "character": fix.cursor.col_utf16 },
            "newName": "SizeInBytes"
        }
    });

    let res = handle_rename(&req, Some(&fix.ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    let updated = apply_workspace_edit(&fix.files, &res);

    assert_eq!(updated[&fix.lib_uri], "pub type SizeInBytes = usize;\n");
    assert_eq!(
        updated[&fix.caller_uri],
        "pub fn alloc(size: aliases::SizeInBytes) {}\n"
    );

    syn::parse_file(&updated[&fix.lib_uri]).expect("valid Rust");
    syn::parse_file(&updated[&fix.caller_uri]).expect("valid Rust");
}

#[test]
fn test_workspace_rename_const() {
    let fix = WorkspaceFixture::new_two_file(
        "constants",
        DefKind::Const,
        "pub const «$0MAX_RETRY»: u32 = 3;\n",
        "pub fn retry_limit() -> u32 { constants::«MAX_RETRY» }\n",
    );

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let req = json!({
        "params": {
            "textDocument": { "uri": &fix.cursor_uri },
            "position": { "line": fix.cursor.line, "character": fix.cursor.col_utf16 },
            "newName": "DEFAULT_ATTEMPTS"
        }
    });

    let res = handle_rename(&req, Some(&fix.ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    let updated = apply_workspace_edit(&fix.files, &res);

    assert_eq!(
        updated[&fix.lib_uri],
        "pub const DEFAULT_ATTEMPTS: u32 = 3;\n"
    );
    assert_eq!(
        updated[&fix.caller_uri],
        "pub fn retry_limit() -> u32 { constants::DEFAULT_ATTEMPTS }\n"
    );

    syn::parse_file(&updated[&fix.lib_uri]).expect("valid Rust");
    syn::parse_file(&updated[&fix.caller_uri]).expect("valid Rust");
}

#[test]
fn test_workspace_rename_with_dirty_overlay_line_shift() {
    let fix = WorkspaceFixture::new_two_file(
        "shift",
        DefKind::Fn,
        "pub fn «action»() {}\n",
        "pub fn invoke() { shift::«$0action»(); }\n",
    );

    let lib_path = fix.temp_dir.path().join("lib.rs");
    let mut overlay_mgr = OverlayManager::new();
    // Simulate dirty overlay where 2 comment lines are inserted at the top:
    let dirty_text = "// line 1\n// line 2\npub fn action() {}\n";
    overlay_mgr.open_document(
        &fix.lib_uri,
        lib_path.to_str().unwrap(),
        2,
        dirty_text.to_string(),
    );

    let mut conv = PositionConverter::new();
    let req = json!({
        "params": {
            "textDocument": { "uri": &fix.cursor_uri },
            "position": { "line": fix.cursor.line, "character": fix.cursor.col_utf16 },
            "newName": "new_action"
        }
    });

    let res = handle_rename(&req, Some(&fix.ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(res["changes"].get(&fix.lib_uri).is_some());
    assert!(res["changes"].get(&fix.caller_uri).is_some());

    let mut live_files = fix.files.clone();
    live_files.insert(fix.lib_uri.clone(), dirty_text.to_string());
    let updated = apply_workspace_edit(&live_files, &res);

    assert_eq!(
        updated[&fix.lib_uri],
        "// line 1\n// line 2\npub fn new_action() {}\n"
    );
    assert_eq!(
        updated[&fix.caller_uri],
        "pub fn invoke() { shift::new_action(); }\n"
    );

    syn::parse_file(&updated[&fix.lib_uri]).expect("valid Rust");
    syn::parse_file(&updated[&fix.caller_uri]).expect("valid Rust");
}
