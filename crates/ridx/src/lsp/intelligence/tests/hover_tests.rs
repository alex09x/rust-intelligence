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
use crate::lsp::intelligence::handle_hover;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use serde_json::json;
use std::fs;

#[test]
fn test_hover_missing_params() {
    let (_temp, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let req = json!({});
    let err = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn test_hover_missing_or_empty_uri() {
    let (_temp, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let req_missing = json!({
        "params": {
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(
        handle_hover(&req_missing, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );

    let req_empty = json!({
        "params": {
            "textDocument": { "uri": "" },
            "position": { "line": 0, "character": 0 }
        }
    });
    assert_eq!(
        handle_hover(&req_empty, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );
}

#[test]
fn test_hover_missing_position() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req_no_pos = json!({
        "params": {
            "textDocument": { "uri": &uri }
        }
    });
    assert_eq!(
        handle_hover(&req_no_pos, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );

    let req_no_line = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "character": 0 }
        }
    });
    assert_eq!(
        handle_hover(&req_no_line, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );

    let req_no_char = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0 }
        }
    });
    assert_eq!(
        handle_hover(&req_no_char, Some(&ws), &overlay_mgr, &mut conv)
            .unwrap_err()
            .code,
        INVALID_PARAMS
    );
}

#[test]
fn test_hover_null_workspace() {
    let (temp_dir, _ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 8 }
        }
    });
    let res = handle_hover(&req, None, &overlay_mgr, &mut conv).unwrap();
    assert!(res.is_null());
}

#[test]
fn test_hover_whitespace_and_comments() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req_ws = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 3 }
        }
    });
    let res = handle_hover(&req_ws, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(res.is_null());
}

#[test]
fn test_hover_clean_function_signature() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 10 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("fn calculate(a: i32, b: i32) -> i32"));
    assert!(val.starts_with("```rust\n"));
    let header_block = val.split("\n---\n").next().unwrap();
    assert!(header_block.ends_with("\n```"));
    assert!(val.contains("**Provenance:**"));
    assert!(val.contains("**Blast Radius:**"));
}

#[test]
fn test_hover_clean_struct_definition() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 13 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("struct User"));
}

#[test]
fn test_hover_clean_inferred_type() {
    let (temp_dir, ws, overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 32 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("Type: i32"));
}

#[test]
fn test_hover_dirty_overlay_mapped_position() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let clean = "pub fn calculate(a: i32, b: i32) -> i32 { a + b }\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, clean.to_string());
    overlay_mgr.update(
        file.to_str().unwrap(),
        2,
        format!("// prepended\n{}", clean),
    );

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 10 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("calculate"));
}

#[test]
fn test_hover_dirty_overlay_new_symbol() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    let code = "fn brand_new() {}\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());
    if let Some(ov) = overlay_mgr.get_mut(file.to_str().unwrap()) {
        ov.symbols.push(crate::overlay::OverlaySymbol {
            name: "brand_new".to_string(),
            kind: DefKind::Fn,
            signature: Some("fn brand_new()".to_string()),
            start_line: 0,
            start_col: 3,
            end_line: 0,
            end_col: 12,
            scope_id: 0,
            decl_start_byte: 0,
            decl_end_byte: 12,
            scope_start_byte: 0,
        });
    }

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 5 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("fn brand_new()"));
}

#[test]
fn test_hover_dirty_overlay_unmapped_position() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, "\n\n\n".to_string());
    overlay_mgr.update(
        file.to_str().unwrap(),
        2,
        "// inserted comment\n\n\n".to_string(),
    );

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 5 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(res.is_null());
}

#[test]
fn test_hover_generic_and_trait_bounds() {
    let temp_dir =
        std::env::temp_dir().join(format!("ridx_test_intel_generic_{}", std::process::id()));
    let _ = fs::create_dir_all(temp_dir.join("src"));
    let src_file = temp_dir.join("src/lib.rs");
    let src_str = src_file.to_str().unwrap().to_string();

    let mut writer = IndexWriter::new("generic_pkg", "0.1.0");
    writer.add_definition(RawDefinition {
        def_id: 10,
        kind: DefKind::Fn,
        visibility: 0,
        is_macro_expansion: false,
        file: src_str.clone(),
        name: "transform".to_string(),
        start_line: 1,
        start_col: 8,
        end_line: 1,
        end_col: 17,
        path: "generic_pkg::transform".to_string(),
        signature: Some(
            "fn transform<T: Clone + Send, U>(item: T) -> Result<U, String> where U: Default"
                .to_string(),
        ),
    });
    let ridx_file = temp_dir.join("index.ridx");
    let mut f = fs::File::create(&ridx_file).unwrap();
    writer.write_to(&mut f).unwrap();

    let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
    ws.set_workspace_root(Some(temp_dir.clone()));
    let overlay_mgr = crate::overlay::OverlayManager::new();
    let mut conv = crate::lsp::protocol::PositionConverter::new();

    let uri = format!("file://{}", src_str);
    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 0, "character": 10 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
    let val = res["contents"]["value"].as_str().unwrap();
    assert!(val.contains("transform<T: Clone + Send, U>"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_hover_utf16_multibyte_alignment() {
    let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
    let file = temp_dir.join("src/lib.rs");
    let uri = format!("file://{}", file.to_str().unwrap());

    // Unicode comment with multibyte UTF-8/UTF-16 characters before symbol
    let code = "// Multibyte: 🦀 Rust 🚀\npub fn calculate(a: i32, b: i32) -> i32 { a + b }\n";
    overlay_mgr.open_document(&uri, file.to_str().unwrap(), 1, code.to_string());

    let req = json!({
        "params": {
            "textDocument": { "uri": &uri },
            "position": { "line": 1, "character": 10 }
        }
    });
    let res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv).unwrap();
    assert!(!res.is_null());
}

#[test]
fn test_hover_variant_trait_assoc_and_macro_headers() {
    use crate::lsp::intelligence::hover::format_hover;
    use ridx_format::{ResolvedDefinition, WorkspaceHover};

    let (_temp, ws, _, _) = setup_test_intelligence();
    let mk = |kind, name, path, signature, inferred_type| WorkspaceHover {
        definition: Some(ResolvedDefinition {
            name,
            path,
            kind,
            signature,
            file: "src/lib.rs",
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 4,
            is_macro_expansion: false,
            is_reexport: false,
            source_root: None,
            unit_key: None,
            reader_idx: None,
        }),
        inferred_type,
    };

    let v_val = format_hover(
        &mk(
            DefKind::Variant,
            "Some",
            "hdr_pkg::MyEnum::Some",
            Some("Some(T)"),
            Some("MyEnum<i32>"),
        ),
        Some(&ws),
    );
    let v_str = v_val["contents"]["value"].as_str().unwrap();
    assert!(v_str.contains("hdr_pkg::MyEnum\n```\n\n```rust\nSome(T)"));
    assert!(!v_str.contains("Type:"));

    let m_val = format_hover(
        &mk(DefKind::Macro, "my_mac", "hdr_pkg::my_mac", None, None),
        Some(&ws),
    );
    let m_str = m_val["contents"]["value"].as_str().unwrap();
    assert!(m_str.contains("macro_rules! my_mac"));
}
