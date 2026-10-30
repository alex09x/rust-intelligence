/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Unit tests for LSP textDocument/typeDefinition.

use super::setup_test_navigation;
use crate::lsp::navigation::handle_type_definition;
use crate::lsp::protocol::PositionConverter;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawType, WorkspaceIndex};
use serde_json::json;
use std::fs;

#[test]
fn test_typedef_from_struct_type() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    // Cursor on `Bar` on line 2 (0-based 1), char 12
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());
    assert_eq!(res["range"]["start"]["line"], 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_dirty_overlay_safely_returns_null() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.clone());
    // Insert single comment line at the top
    overlay_mgr.update(file_path.to_str().unwrap(), 2, format!("// header\n{}", code));

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 12 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_dirty_overlay_unmapped_returns_null() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, "a\nb\n".to_string());
    overlay_mgr.update(file_path.to_str().unwrap(), 2, "xyz\n".to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 4, "character": 0 }
        }
    });
    assert!(handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_dirty_overlay_modified_imports_returns_null() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.clone());
    overlay_mgr.update(file_path.to_str().unwrap(), 2, format!("use std::sync::Arc;\n{}", code));

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 12 }
        }
    });
    assert!(handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_missing_workspace_returns_null() {
    let (temp_dir, _, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    assert!(handle_type_definition(&req, None, &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_unknown_position_returns_null() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_clean_overlay_returns_def() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());
    let code = fs::read_to_string(&file_path).unwrap();

    overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code);

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 12 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert!(!res.is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_nonexistent_uri_returns_null() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let req = json!({
        "params": {
            "textDocument": { "uri": "file:///path/does/not/exist.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert!(handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv).is_null());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_out_of_bounds_character_clamped() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_navigation();
    let file_path = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file_path.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 9999 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    // Large character offset does not panic and returns null or closest type
    let _ = res;

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_typedef_prefers_exact_name_over_substring() {
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_typedef_exact_{}", id));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let code = "pub struct AppConfig;\npub struct Config;\npub fn run() { let cfg = Config; }\n";
    let _ = fs::write(&src_file, code);

    let src_path_str = src_file.to_str().unwrap().to_string();
    let mut writer = IndexWriter::new("test_exact", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 1,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "AppConfig".to_string(),
        start_line: 1,
        start_col: 12,
        end_line: 1,
        end_col: 21,
        path: "test_exact::AppConfig".to_string(),
        signature: None,
    });
    writer.add_definition(RawDefinition {
        def_id: 2,
        kind: DefKind::Struct,
        visibility: 0,
        is_macro_expansion: false,
        file: src_path_str.clone(),
        name: "Config".to_string(),
        start_line: 2,
        start_col: 12,
        end_line: 2,
        end_col: 18,
        path: "test_exact::Config".to_string(),
        signature: None,
    });
    writer.add_type(RawType { file: src_path_str.clone(), start_line: 3, start_col: 20, end_line: 3, end_col: 23, type_name: "Config".to_string() });

    let index_path = temp_dir.join("test.ridx");
    let mut f = fs::File::create(&index_path).unwrap();
    writer.write_to(&mut f).unwrap();
    let ws = WorkspaceIndex::open(&index_path).unwrap();
    let overlay_mgr = OverlayManager::new();
    let mut conv = PositionConverter::new();
    let uri = format!("file://{}", src_path_str);

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 2, "character": 20 }
        }
    });
    let res = handle_type_definition(&req, Some(&ws), &overlay_mgr, &mut conv);
    assert_eq!(res["range"]["start"]["line"], 1);
    assert_eq!(res["range"]["start"]["character"], 11);
    assert_eq!(res["range"]["end"]["character"], 17);

    let _ = fs::remove_dir_all(&temp_dir);
}
