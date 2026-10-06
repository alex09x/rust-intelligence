/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP symbol handlers: document symbols, multi-root workspace symbols, and symbol resolution.

use super::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_document_symbol(
    msg: &Value,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut results = Vec::new();

    if let Some(symbols) = overlay_mgr.document_symbols(&file) {
        for s in symbols {
            let start_char = conv.char_col_to_utf16_col(&file, s.start_line, s.start_col);
            let end_char = conv.char_col_to_utf16_col(&file, s.end_line, s.end_col);
            let range = json!({
                "start": { "line": s.start_line.saturating_sub(1), "character": start_char },
                "end": { "line": s.end_line.saturating_sub(1), "character": end_char }
            });
            results.push(json!({
                "name": s.name,
                "kind": lsp_symbol_kind(s.kind),
                "range": range,
                "selectionRange": range,
                "detail": s.signature
            }));
        }
    }

    json!(results)
}

pub fn handle_workspace_symbol<'a>(
    msg: &Value,
    workspaces: impl Iterator<Item = &'a WorkspaceIndex>,
    conv: &mut PositionConverter,
) -> Value {
    let query = msg["params"]["query"].as_str().unwrap_or("");
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for ws in workspaces {
        let symbols = ws.search_symbols(query);
        for s in symbols {
            let resolved_path = s.resolved_path();
            let path_str = resolved_path.to_str().unwrap_or(s.file);
            let dedupe_key = (
                s.name.to_string(),
                path_str.to_string(),
                s.span.start_line(),
            );
            if !seen.insert(dedupe_key) {
                continue;
            }

            let uri = path_to_uri(path_str);
            let start_char =
                conv.char_col_to_utf16_col(path_str, s.span.start_line(), s.span.start_col());
            let end_char =
                conv.char_col_to_utf16_col(path_str, s.span.end_line(), s.span.end_col());
            results.push(json!({
                "name": s.name,
                "kind": lsp_symbol_kind(s.kind),
                "location": {
                    "uri": uri,
                    "range": {
                        "start": { "line": s.span.start_line().saturating_sub(1), "character": start_char },
                        "end": { "line": s.span.end_line().saturating_sub(1), "character": end_char }
                    }
                },
                "containerName": s.path.rsplit("::").nth(1).unwrap_or("")
            }));

            if results.len() >= 100 {
                break;
            }
        }
        if results.len() >= 100 {
            break;
        }
    }

    json!(results)
}

pub fn handle_workspace_symbol_resolve<'a>(
    msg: &Value,
    workspaces: impl Iterator<Item = &'a WorkspaceIndex>,
    conv: &mut PositionConverter,
) -> Value {
    let mut item = msg.get("params").cloned().unwrap_or(Value::Null);
    if item.is_null() {
        return Value::Null;
    }

    // If location is already present, ensure containerName / detail are resolved
    if item.get("location").is_none() {
        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
        for ws in workspaces {
            let symbols = ws.search_symbols(name);
            if let Some(s) = symbols.into_iter().find(|sym| sym.name == name) {
                let resolved_path = s.resolved_path();
                let path_str = resolved_path.to_str().unwrap_or(s.file);
                let uri = path_to_uri(path_str);
                let start_char =
                    conv.char_col_to_utf16_col(path_str, s.span.start_line(), s.span.start_col());
                let end_char =
                    conv.char_col_to_utf16_col(path_str, s.span.end_line(), s.span.end_col());
                item["location"] = json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": s.span.start_line().saturating_sub(1), "character": start_char },
                        "end": { "line": s.span.end_line().saturating_sub(1), "character": end_char }
                    }
                });
                item["containerName"] = json!(s.path.rsplit("::").nth(1).unwrap_or(""));
                break;
            }
        }
    }

    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::protocol::PositionConverter;
    use crate::overlay::OverlayManager;
    use ridx_format::{IndexWriter, RawDefinition, WorkspaceIndex};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn setup_test_symbols() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
        let temp_dir =
            std::env::temp_dir().join(format!("ridx_test_symbols_{}", std::process::id()));
        let _ = fs::create_dir_all(temp_dir.join("src"));
        let src_file = temp_dir.join("src/lib.rs");
        let code = "pub fn my_symbol() {}\npub struct MyStruct;\n";
        let _ = fs::write(&src_file, code);

        let mut writer = IndexWriter::new("test_syms", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: ridx_format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "my_symbol".to_string(),
            start_line: 1,
            start_col: 8,
            end_line: 1,
            end_col: 17,
            path: "test_syms::my_symbol".to_string(),
            signature: Some("fn my_symbol()".to_string()),
        });
        writer.add_definition(RawDefinition {
            def_id: 2,
            kind: ridx_format::DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "MyStruct".to_string(),
            start_line: 2,
            start_col: 12,
            end_line: 2,
            end_col: 20,
            path: "test_syms::MyStruct".to_string(),
            signature: None,
        });

        for i in 10..125 {
            writer.add_definition(RawDefinition {
                def_id: i,
                kind: ridx_format::DefKind::Fn,
                visibility: 0,
                is_macro_expansion: false,
                file: "src/lib.rs".to_string(),
                name: format!("sym_{i}"),
                start_line: i,
                start_col: 1,
                end_line: i,
                end_col: 5,
                path: format!("test_syms::sym_{i}"),
                signature: None,
            });
        }

        let ridx_file = temp_dir.join("index.ridx");
        let mut f = fs::File::create(&ridx_file).unwrap();
        writer.write_to(&mut f).unwrap();

        let ws = WorkspaceIndex::open(&ridx_file).unwrap();
        let overlay_mgr = OverlayManager::new();
        let conv = PositionConverter::new();

        (temp_dir, ws, overlay_mgr, conv)
    }

    #[test]
    fn test_symbols_comprehensive() {
        let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_symbols();
        let file_path = temp_dir.join("src/lib.rs");
        let uri = format!("file://{}", file_path.to_str().unwrap());

        // 1. Document symbols (overlay)
        let code = "pub fn my_symbol() {}\npub struct MyStruct;\n";
        overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());
        let doc_sym_req = json!({
            "params": {
                "textDocument": { "uri": &uri }
            }
        });
        let doc_sym_res = handle_document_symbol(&doc_sym_req, &overlay_mgr, &mut conv);
        assert!(!doc_sym_res.as_array().unwrap().is_empty());

        // Document symbols for unopened file
        let unopened_req = json!({
            "params": {
                "textDocument": { "uri": "file:///nonexistent.rs" }
            }
        });
        let un_res = handle_document_symbol(&unopened_req, &overlay_mgr, &mut conv);
        assert_eq!(un_res, json!([]));

        // 2. Workspace symbols
        let ws_sym_req = json!({
            "params": {
                "query": "my"
            }
        });
        let ws_sym_res = handle_workspace_symbol(&ws_sym_req, std::iter::once(&ws), &mut conv);
        assert!(!ws_sym_res.as_array().unwrap().is_empty());

        let ws_all_req = json!({
            "params": {
                "query": "sym"
            }
        });
        let ws_all_res = handle_workspace_symbol(&ws_all_req, std::iter::once(&ws), &mut conv);
        assert_eq!(ws_all_res.as_array().unwrap().len(), 100);

        // 3. Workspace symbol resolve
        let resolve_req = json!({
            "params": {
                "name": "my_symbol"
            }
        });
        let resolve_res =
            handle_workspace_symbol_resolve(&resolve_req, std::iter::once(&ws), &mut conv);
        assert!(resolve_res["location"]["uri"].as_str().is_some());

        let null_resolve =
            handle_workspace_symbol_resolve(&json!({}), std::iter::once(&ws), &mut conv);
        assert_eq!(null_resolve, Value::Null);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
