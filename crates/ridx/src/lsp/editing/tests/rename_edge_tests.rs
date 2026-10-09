/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Edge case and robustness tests for symbol renaming:
//! mismatched index references, external symbols, raw identifiers, and broken syntax.

use crate::lsp::editing::rename::handle_rename;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::DefKind;
use ridx_test_support::{apply_workspace_edit, parse_fixture, FixtureIndexBuilder};
use serde_json::json;
use std::collections::HashMap;
use std::fs;

#[test]
fn test_workspace_rename_ignores_ref_with_mismatched_text() {
    let temp = tempfile::tempdir().unwrap();
    let lib_path = temp.path().join("lib.rs");
    let caller_path = temp.path().join("caller.rs");

    let lib_str = lib_path.to_str().unwrap().to_string();
    let caller_str = caller_path.to_str().unwrap().to_string();

    let mut builder = FixtureIndexBuilder::new("math", "0.1.0");
    let (parsed_decl, def_ids) = builder.add_defs_and_self_refs_fixture(
        "pub fn «$0calculate»(x: i32) -> i32 { x * 2 }\n",
        &lib_str,
        "math",
        DefKind::Fn,
    );
    fs::write(&lib_path, &parsed_decl.clean_text).unwrap();

    // Valid reference in caller
    let parsed_caller = builder.add_refs_to_def_fixture(
        "pub fn invoke() -> i32 { math::«calculate»(21) }\npub fn other() { different_word(); }\n",
        &caller_str,
        def_ids[0],
    );
    fs::write(&caller_path, &parsed_caller.clean_text).unwrap();

    // Inject a stale/mismatched reference pointing to "different_word" (line 2, col 18)
    builder.add_ref(
        "math::calculate",
        &caller_str,
        2,
        18,
        2,
        32,
        Some(def_ids[0]),
        Some(&lib_str),
        1,
        8,
        DefKind::Fn,
    );

    let idx_file = temp.path().join("index.ridx");
    builder.write_to_file(&idx_file);
    let ws = ridx_format::WorkspaceIndex::open(&idx_file).unwrap();

    let lib_uri = path_to_uri(&lib_str);
    let caller_uri = path_to_uri(&caller_str);

    let mut files = HashMap::new();
    files.insert(lib_uri.clone(), parsed_decl.clean_text);
    files.insert(caller_uri.clone(), parsed_caller.clean_text);

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let cur = parsed_decl.cursor.unwrap();
    let req = json!({
        "params": {
            "textDocument": { "uri": &lib_uri },
            "position": { "line": cur.line, "character": cur.col_utf16 },
            "newName": "compute_double"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(res["changes"].get(&lib_uri).is_some());
    assert!(res["changes"].get(&caller_uri).is_some());

    let updated = apply_workspace_edit(&files, &res);

    // lib.rs definition was renamed
    assert_eq!(
        updated[&lib_uri],
        "pub fn compute_double(x: i32) -> i32 { x * 2 }\n"
    );
    // In caller.rs, the true reference was renamed, but the mismatched ref was untouched!
    assert_eq!(
        updated[&caller_uri],
        "pub fn invoke() -> i32 { math::compute_double(21) }\npub fn other() { different_word(); }\n"
    );

    syn::parse_file(&updated[&lib_uri]).expect("lib.rs must be valid Rust");
    syn::parse_file(&updated[&caller_uri]).expect("caller.rs must be valid Rust");
}

#[test]
fn test_workspace_rename_rejects_external_symbol() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("app");
    let src_file = app_dir.join("src/lib.rs");
    fs::create_dir_all(src_file.parent().unwrap()).unwrap();
    let src_str = src_file.to_str().unwrap().to_string();
    let uri = path_to_uri(&src_str);

    // Realistic external dependency file outside workspace root (no magic substring)
    let ext_dep = temp.path().join("elsewhere/serde-1.0.0/src/lib.rs");
    fs::create_dir_all(ext_dep.parent().unwrap()).unwrap();
    fs::write(&ext_dep, "pub trait Serialize {}\n").unwrap();
    let ext_dep_str = ext_dep.to_str().unwrap();

    let mut builder = FixtureIndexBuilder::new("app", "0.1.0");
    let fixture = "use serde::«$0Serialize»;\npub struct Item;\n";
    let parsed = parse_fixture(fixture);
    fs::write(&src_file, &parsed.clean_text).unwrap();

    builder.add_refs_from_fixture(
        fixture,
        &src_str,
        "serde::Serialize",
        None,
        Some(ext_dep_str),
        1,
        1,
        DefKind::Trait,
    );
    let idx_file = app_dir.join("target/ridx/app.ridx");
    fs::create_dir_all(idx_file.parent().unwrap()).unwrap();
    builder.write_to_file(&idx_file);
    let mut ws = ridx_format::WorkspaceIndex::open(&idx_file).unwrap();
    ws.set_workspace_root(Some(app_dir));

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur.line, "character": cur.col_utf16 },
            "newName": "MySerialize"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(res.is_err(), "Must reject rename for external dependency");
    assert_eq!(res.unwrap_err().code, -32803);
}

#[test]
fn test_workspace_rename_in_local_registry_directory() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("app");
    let reg_file = app_dir.join("src/registry/mod.rs");
    fs::create_dir_all(reg_file.parent().unwrap()).unwrap();
    let reg_str = reg_file.to_str().unwrap().to_string();
    let reg_uri = path_to_uri(&reg_str);

    let main_file = app_dir.join("src/main.rs");
    let main_str = main_file.to_str().unwrap().to_string();
    let main_uri = path_to_uri(&main_str);

    let reg_fixture = "pub fn «$0fetch_package»() -> bool { true }\n";
    let reg_parsed = parse_fixture(reg_fixture);

    let main_fixture = "mod registry;\nfn run() { registry::«fetch_package»(); }\n";
    let main_parsed = parse_fixture(main_fixture);

    let mut builder = FixtureIndexBuilder::new("app", "0.1.0");
    let (reg_clean, ids) =
        builder.add_defs_and_self_refs(reg_fixture, &reg_str, "app::registry", DefKind::Fn);
    let main_clean = builder.add_refs_to_def(main_fixture, &main_str, ids[0]);
    fs::write(&reg_file, &reg_clean).unwrap();
    fs::write(&main_file, &main_clean).unwrap();

    let idx_file = app_dir.join("target/ridx/app.ridx");
    fs::create_dir_all(idx_file.parent().unwrap()).unwrap();
    builder.write_to_file(&idx_file);
    let mut ws = ridx_format::WorkspaceIndex::open(&idx_file).unwrap();
    ws.set_workspace_root(Some(app_dir));

    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();

    let cur = reg_parsed.cursor.unwrap();
    let req = json!({
        "params": {
            "textDocument": { "uri": &reg_uri },
            "position": { "line": cur.line, "character": cur.col_utf16 },
            "newName": "download_package"
        }
    });

    let res = handle_rename(&req, Some(&ws), &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(res["changes"].get(&reg_uri).is_some());
    assert!(res["changes"].get(&main_uri).is_some());

    let mut files = HashMap::new();
    files.insert(reg_uri.clone(), reg_parsed.clean_text);
    files.insert(main_uri.clone(), main_parsed.clean_text);

    let updated = apply_workspace_edit(&files, &res);
    assert_eq!(
        updated[&reg_uri],
        "pub fn download_package() -> bool { true }\n"
    );
    assert_eq!(
        updated[&main_uri],
        "mod registry;\nfn run() { registry::download_package(); }\n"
    );
    syn::parse_file(&updated[&reg_uri]).expect("valid Rust");
    syn::parse_file(&updated[&main_uri]).expect("valid Rust");
}

#[test]
fn test_rename_local_raw_identifier() {
    let fixture = "fn check() -> i32 {\n    let $0val = 42;\n    val + 1\n}\n";
    let parsed = parse_fixture(fixture);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, parsed.clean_text.clone());
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur.line, "character": cur.col_utf16 },
            "newName": "r#type"
        }
    });

    let res = handle_rename(&req, None, &overlay_mgr, &mut conv).expect("rename succeeds");
    let mut files = HashMap::new();
    files.insert(uri.clone(), parsed.clean_text);

    let updated = apply_workspace_edit(&files, &res);
    assert_eq!(
        updated[&uri],
        "fn check() -> i32 {\n    let r#type = 42;\n    r#type + 1\n}\n"
    );
    syn::parse_file(&updated[&uri]).expect("Raw identifier must produce valid Rust syntax");
}

#[test]
fn test_rename_local_with_broken_syntax() {
    let fixture = "fn broken() {\n    let $0x = 10;\n    let y = x + 1;\n    y +\n";
    let parsed = parse_fixture(fixture);
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("lib.rs");
    let file_str = file.to_str().unwrap();
    let uri = path_to_uri(file_str);

    let mut overlay_mgr = OverlayManager::new();
    overlay_mgr.open_document(&uri, file_str, 1, parsed.clean_text.clone());
    let mut conv = PositionConverter::new();

    let cur = parsed.cursor.unwrap();
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": cur.line, "character": cur.col_utf16 },
            "newName": "target_var"
        }
    });

    let res = handle_rename(&req, None, &overlay_mgr, &mut conv).expect("rename succeeds");
    assert!(res["changes"].get(&uri).is_some());

    let mut files = HashMap::new();
    files.insert(uri.clone(), parsed.clean_text);
    let updated = apply_workspace_edit(&files, &res);

    assert_eq!(
        updated[&uri],
        "fn broken() {\n    let target_var = 10;\n    let y = target_var + 1;\n    y +\n"
    );
}
