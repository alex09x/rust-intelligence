/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::setup_test_intelligence;
use crate::lsp::error::INVALID_PARAMS;
use crate::lsp::intelligence::{handle_inlay_hint, handle_inlay_hint_resolve};
use ridx_format::{IndexWriter, RawType, WorkspaceIndex};
use serde_json::json;
use std::fs;

#[test]
fn test_inlay_hint_missing_params() {
    let (_temp, ws, overlay_mgr, _conv) = setup_test_intelligence();
    let req = json!({});
    let err = handle_inlay_hint(&req, Some(&ws), &overlay_mgr).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_inlay_hint_missing_or_empty_uri() {
    let (_temp, ws, overlay_mgr, _conv) = setup_test_intelligence();
    let req_missing = json!({
        "params": {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 10, "character": 0 }
            }
        }
    });
    assert_eq!(
        handle_inlay_hint(&req_missing, Some(&ws), &overlay_mgr)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );

    let req_empty = json!({
        "params": {
            "textDocument": { "uri": "" },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 10, "character": 0 }
            }
        }
    });
    assert_eq!(
        handle_inlay_hint(&req_empty, Some(&ws), &overlay_mgr)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );
}

#[test]
fn test_inlay_hint_missing_range_or_boundaries() {
    let (temp_dir, ws, overlay_mgr, _conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req_no_range = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    assert_eq!(
        handle_inlay_hint(&req_no_range, Some(&ws), &overlay_mgr)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );

    let req_no_start = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "end": { "line": 10 }
            }
        }
    });
    assert_eq!(
        handle_inlay_hint(&req_no_start, Some(&ws), &overlay_mgr)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );
}

#[test]
fn test_inlay_hint_let_without_type_produces_hint() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inlay_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/main.rs");
    let src_str = src_file.to_str().unwrap().to_string();

    let code = "fn main() {\n    let answer = 42;\n}\n";
    let _ = fs::write(&src_file, code);

    let mut writer = IndexWriter::new("test_inlay", "0.1.0");
    writer.add_type(RawType {
        file: src_str.clone(),
        start_line: 2,
        start_col: 9,
        end_line: 2,
        end_col: 15,
        type_name: "i32".to_string(),
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let uri = format!("file://{}", src_str);
    overlay_mgr.open_document(&uri, &src_str, 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 2, "character": 1 }
            }
        }
    });

    let res = handle_inlay_hint(&req, Some(&ws), &overlay_mgr).unwrap();
    let hints = res.as_array().unwrap();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0]["label"], ": i32");
    assert_eq!(hints[0]["kind"], 1);
    assert_eq!(hints[0]["paddingLeft"], true);
    assert_eq!(hints[0]["position"]["line"], 1);
    assert_eq!(hints[0]["position"]["character"], 14);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_inlay_hint_let_with_explicit_type_no_hint() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inlay_exp_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/main.rs");
    let src_str = src_file.to_str().unwrap().to_string();

    let code = "fn main() {\n    let answer: i32 = 42;\n}\n";
    let _ = fs::write(&src_file, code);

    let mut writer = IndexWriter::new("test_inlay_exp", "0.1.0");
    writer.add_type(RawType {
        file: src_str.clone(),
        start_line: 2,
        start_col: 9,
        end_line: 2,
        end_col: 15,
        type_name: "i32".to_string(),
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let uri = format!("file://{}", src_str);
    overlay_mgr.open_document(&uri, &src_str, 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 2, "character": 1 }
            }
        }
    });

    let res = handle_inlay_hint(&req, Some(&ws), &overlay_mgr).unwrap();
    let hints = res.as_array().unwrap();
    assert!(hints.is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_inlay_hint_let_mut_identifier_position() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inlay_mut_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/main.rs");
    let src_str = src_file.to_str().unwrap().to_string();

    let code = "fn main() {\n    let mut counter = 0;\n}\n";
    let _ = fs::write(&src_file, code);

    let mut writer = IndexWriter::new("test_inlay_mut", "0.1.0");
    writer.add_type(RawType {
        file: src_str.clone(),
        start_line: 2,
        start_col: 13,
        end_line: 2,
        end_col: 20,
        type_name: "u32".to_string(),
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let uri = format!("file://{}", src_str);
    overlay_mgr.open_document(&uri, &src_str, 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 2, "character": 1 }
            }
        }
    });

    let res = handle_inlay_hint(&req, Some(&ws), &overlay_mgr).unwrap();
    let hints = res.as_array().unwrap();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0]["label"], ": u32");
    assert_eq!(hints[0]["position"]["line"], 1);
    assert_eq!(hints[0]["position"]["character"], 19);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_inlay_hint_range_filtering() {
    let temp_dir = std::env::temp_dir().join(format!("ridx_test_inlay_rf_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/main.rs");
    let src_str = src_file.to_str().unwrap().to_string();

    let code = "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n";
    let _ = fs::write(&src_file, code);

    let mut writer = IndexWriter::new("test_inlay_rf", "0.1.0");
    writer.add_type(RawType {
        file: src_str.clone(),
        start_line: 2,
        start_col: 9,
        end_line: 2,
        end_col: 10,
        type_name: "i32".to_string(),
    });
    writer.add_type(RawType {
        file: src_str.clone(),
        start_line: 3,
        start_col: 9,
        end_line: 3,
        end_col: 10,
        type_name: "i32".to_string(),
    });
    writer.add_type(RawType {
        file: src_str.clone(),
        start_line: 4,
        start_col: 9,
        end_line: 4,
        end_col: 10,
        type_name: "i32".to_string(),
    });

    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let ws = WorkspaceIndex::open(&ridx_file).unwrap();
    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let uri = format!("file://{}", src_str);
    overlay_mgr.open_document(&uri, &src_str, 1, code.to_string());

    // Only query range line 2..2 (middle let statement)
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "range": {
                "start": { "line": 2, "character": 0 },
                "end": { "line": 2, "character": 20 }
            }
        }
    });

    let res = handle_inlay_hint(&req, Some(&ws), &overlay_mgr).unwrap();
    let hints = res.as_array().unwrap();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0]["position"]["line"], 2);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_inlay_hint_resolve_validation_and_enrichment() {
    let req_missing = json!({});
    assert_eq!(
        handle_inlay_hint_resolve(&req_missing).unwrap_err().code,
        INVALID_PARAMS
    );

    let req_no_label = json!({
        "params": {
            "position": { "line": 1, "character": 10 }
        }
    });
    assert_eq!(
        handle_inlay_hint_resolve(&req_no_label).unwrap_err().code,
        INVALID_PARAMS
    );

    let req_valid = json!({
        "params": {
            "position": { "line": 1, "character": 10 },
            "label": ": String",
            "kind": 1,
            "paddingLeft": true
        }
    });
    let res = handle_inlay_hint_resolve(&req_valid).unwrap();
    assert_eq!(res["label"], ": String");
    assert_eq!(res["kind"], 1);
    assert_eq!(res["paddingLeft"], true);
    assert_eq!(res["tooltip"], "Inferred type by ridx");
}

#[test]
fn test_inlay_hint_resolve_preserves_existing_tooltip() {
    let req = json!({
        "params": {
            "position": { "line": 0, "character": 5 },
            "label": ": bool",
            "tooltip": "Existing custom tooltip"
        }
    });
    let res = handle_inlay_hint_resolve(&req).unwrap();
    assert_eq!(res["tooltip"], "Existing custom tooltip");
}
