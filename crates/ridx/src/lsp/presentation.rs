/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP presentation handlers: selection range, document links, semantic tokens, folding, and code lens.

use super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_selection_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let positions = msg["params"]["positions"].as_array();

    let Some(overlay) = overlay_mgr.get(&file) else {
        return json!([]);
    };

    let mut results = Vec::new();
    if let Some(pos_list) = positions {
        for pos in pos_list {
            let line_0based = pos["line"].as_u64().unwrap_or(0) as u32;
            let utf16_col_0based = pos["character"].as_u64().unwrap_or(0) as u32;
            let byte_pos = overlay.position_to_byte_offset(line_0based, utf16_col_0based);

            // Collect all scopes containing byte_pos
            let mut matching_scopes: Vec<_> = overlay
                .scopes
                .iter()
                .filter(|s| byte_pos >= s.start_byte && byte_pos <= s.end_byte)
                .collect();

            // Sort from outermost (largest byte span) to innermost (smallest byte span)
            matching_scopes
                .sort_by_key(|s| std::cmp::Reverse(s.end_byte.saturating_sub(s.start_byte)));

            // Build inner-to-outer parent chain: each outer node becomes the parent of the next inner node
            let mut node: Option<Value> = None;
            for scope in matching_scopes {
                let (sl, sc) = overlay.byte_to_char_line_col(scope.start_byte);
                let (el, ec) = overlay.byte_to_char_line_col(scope.end_byte);
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                let range = json!({
                    "start": { "line": sl.saturating_sub(1), "character": start_char },
                    "end": { "line": el.saturating_sub(1), "character": end_char }
                });
                node = Some(json!({
                    "range": range,
                    "parent": node
                }));
            }

            if let Some(n) = node {
                results.push(n);
            }
        }
    }

    json!(results)
}

pub fn handle_document_link(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut links = Vec::new();

    let text_opt = if let Some(overlay) = overlay_mgr.get(&file) {
        Some(overlay.text.clone())
    } else {
        std::fs::read_to_string(&file).ok()
    };

    if let Some(text) = text_opt {
        for (line_idx, line) in text.lines().enumerate() {
            if let Some(http_pos) = line.find("https://") {
                let end_pos = line[http_pos..]
                    .find(|c: char| {
                        c.is_whitespace() || c == '>' || c == ')' || c == ']' || c == '\"'
                    })
                    .map(|p| http_pos + p)
                    .unwrap_or(line.len());
                let target_url = &line[http_pos..end_pos];

                // Calculate exact UTF-16 character columns directly from byte slices
                let sc = line[..http_pos].encode_utf16().count() as u32;
                let ec = line[..end_pos].encode_utf16().count() as u32;

                links.push(json!({
                    "range": {
                        "start": { "line": line_idx as u32, "character": sc },
                        "end": { "line": line_idx as u32, "character": ec }
                    },
                    "target": target_url
                }));
            }
        }
    }

    json!(links)
}

pub fn handle_document_link_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}

pub fn handle_semantic_tokens_full(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut data = Vec::new();

    if let Some(overlay) = overlay_mgr.get(&file) {
        let mut prev_line = 0;
        let mut prev_col = 0;

        for s in &overlay.symbols {
            let line_0based = s.start_line.saturating_sub(1);
            let col_0based = overlay.char_col_to_utf16_col(s.start_line, s.start_col);
            let length = overlay
                .char_col_to_utf16_col(s.end_line, s.end_col)
                .saturating_sub(col_0based);

            let delta_line = line_0based.saturating_sub(prev_line);
            let delta_start = if delta_line == 0 {
                col_0based.saturating_sub(prev_col)
            } else {
                col_0based
            };

            let token_type = match s.kind {
                ridx_format::DefKind::Fn | ridx_format::DefKind::AssocFn => 11, // function
                ridx_format::DefKind::Struct => 5,                              // struct
                ridx_format::DefKind::Enum => 3,                                // enum
                ridx_format::DefKind::Trait => 4,                               // interface
                ridx_format::DefKind::Mod => 0,                                 // namespace
                _ => 8,                                                         // variable
            };

            data.extend_from_slice(&[delta_line, delta_start, length, token_type, 1]); // modifier 1 = declaration
            prev_line = line_0based;
            prev_col = col_0based;
        }
    }

    json!({ "data": data })
}

pub fn handle_folding_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut ranges = Vec::new();

    if let Some(overlay) = overlay_mgr.get(&file) {
        for scope in &overlay.scopes {
            if scope.id == 0 {
                continue;
            }
            let (start_line, _) = overlay.byte_to_char_line_col(scope.start_byte);
            let (end_line, _) = overlay.byte_to_char_line_col(scope.end_byte);
            if end_line > start_line {
                ranges.push(json!({
                    "startLine": start_line.saturating_sub(1),
                    "endLine": end_line.saturating_sub(1),
                    "kind": "region"
                }));
            }
        }
        for s in &overlay.symbols {
            if s.end_line > s.start_line {
                let sl = s.start_line.saturating_sub(1);
                let el = s.end_line.saturating_sub(1);
                if !ranges
                    .iter()
                    .any(|r| r["startLine"] == sl && r["endLine"] == el)
                {
                    ranges.push(json!({
                        "startLine": sl,
                        "endLine": el,
                        "kind": "region"
                    }));
                }
            }
        }
    }

    json!(ranges)
}

pub fn handle_document_color(_msg: &Value) -> Value {
    json!([])
}

pub fn handle_color_presentation(_msg: &Value) -> Value {
    json!([])
}

pub fn handle_code_lens(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut lenses = Vec::new();

    if let Some(overlay) = overlay_mgr.get(&file) {
        for s in &overlay.symbols {
            if matches!(
                s.kind,
                ridx_format::DefKind::Fn
                    | ridx_format::DefKind::Struct
                    | ridx_format::DefKind::Enum
            ) {
                let range = json!({
                    "start": { "line": s.start_line.saturating_sub(1), "character": s.start_col.saturating_sub(1) },
                    "end": { "line": s.end_line.saturating_sub(1), "character": s.end_col.saturating_sub(1) }
                });
                lenses.push(json!({
                    "range": range,
                    "command": {
                        "title": "References: 0".to_string(),
                        "command": "ridx.showReferences"
                    }
                }));
            }
        }
    }

    json!(lenses)
}

pub fn handle_code_lens_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}

pub fn handle_diagnostic(_msg: &Value, _overlay_mgr: &OverlayManager) -> Value {
    json!({
        "kind": "full",
        "items": []
    })
}

pub fn handle_moniker(
    msg: &Value,
    _workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
    let utf16_col_0based = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
    let line_1based = line_0based + 1;

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    if let Some(overlay) = overlay_opt {
        if let Some(def) = overlay.find_definition_at(line_1based, col_1based) {
            return json!([
                {
                    "scheme": "rust",
                    "identifier": format!("{}::{}", def.file, def.name),
                    "unique": "project"
                }
            ]);
        }
    }

    json!([])
}

pub fn handle_text_document_content(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    if let Some(overlay) = overlay_mgr.get(&file) {
        json!({ "text": overlay.text })
    } else if let Ok(content) = std::fs::read_to_string(&file) {
        json!({ "text": content })
    } else {
        json!({ "text": "" })
    }
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

    fn setup_test_presentation() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
        let temp_dir = std::env::temp_dir().join(format!("ridx_test_pres_{}", std::process::id()));
        let _ = fs::create_dir_all(temp_dir.join("src"));
        let src_file = temp_dir.join("src/lib.rs");
        let code = "// привет https://example.com/test\npub fn hello() {\n    let a = 1;\n    let b = 2;\n}\n";
        let _ = fs::write(&src_file, code);

        let mut writer = IndexWriter::new("test_pres", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: ridx_format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "hello".to_string(),
            start_line: 2,
            start_col: 8,
            end_line: 2,
            end_col: 13,
            path: "test_pres::hello".to_string(),
            signature: Some("fn hello()".to_string()),
        });

        let ridx_file = temp_dir.join("index.ridx");
        let mut f = fs::File::create(&ridx_file).unwrap();
        writer.write_to(&mut f).unwrap();

        let ws = WorkspaceIndex::open(&ridx_file).unwrap();
        let overlay_mgr = OverlayManager::new();
        let conv = PositionConverter::new();

        (temp_dir, ws, overlay_mgr, conv)
    }

    #[test]
    fn test_presentation_handlers_comprehensive() {
        let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_presentation();
        let file_path = temp_dir.join("src/lib.rs");
        let uri = format!("file://{}", file_path.to_str().unwrap());

        let code = "// привет https://example.com/test\npub fn hello() {\n    let a = 1;\n    let b = 2;\n}\nmod my_mod {\n    struct MyStruct; enum MyEnum { A }\n    trait MyTrait {}\n    const MY_CONST: i32 = 1;\n}\n";
        overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());
        if let Some(doc) = overlay_mgr.get_mut(file_path.to_str().unwrap()) {
            doc.symbols.push(crate::overlay::OverlaySymbol {
                name: "multiline_symbol".to_string(),
                kind: ridx_format::DefKind::Struct,
                signature: None,
                start_line: 12,
                start_col: 1,
                end_line: 15,
                end_col: 1,
                scope_id: 0,
                decl_start_byte: 0,
                decl_end_byte: 10,
                scope_start_byte: 0,
            });
        }

        // 1. Selection range (inner to outer)
        let sel_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "positions": [{ "line": 2, "character": 8 }]
            }
        });
        let sel_res = handle_selection_range(&sel_req, &overlay_mgr);
        assert!(!sel_res.as_array().unwrap().is_empty());

        // 2. Document link
        let link_req = json!({
            "params": {
                "textDocument": { "uri": &uri }
            }
        });
        let link_res = handle_document_link(&link_req, &overlay_mgr);
        assert!(!link_res.as_array().unwrap().is_empty());
        let resolve_res = handle_document_link_resolve(&link_req);
        assert!(!resolve_res.is_null());

        // 3. Semantic tokens
        let sem_res = handle_semantic_tokens_full(&link_req, &overlay_mgr);
        assert!(sem_res["data"].as_array().is_some());

        // 4. Folding range
        let fold_res = handle_folding_range(&link_req, &overlay_mgr);
        assert!(fold_res.as_array().is_some());

        // 5. Code lens
        let lens_res = handle_code_lens(&link_req, &overlay_mgr);
        assert!(lens_res.as_array().is_some());
        let lens_resolve = handle_code_lens_resolve(&link_req);
        assert!(!lens_resolve.is_null());

        // 6. Moniker
        let mon_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 8 }
            }
        });
        let mon_res = handle_moniker(&mon_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(mon_res.as_array().is_some());

        // 7. Text document content
        let content_req = json!({
            "params": { "uri": &uri }
        });
        let content_res = handle_text_document_content(&content_req, &overlay_mgr);
        assert!(content_res["text"].as_str().unwrap().contains("hello"));

        // Colors and diagnostics
        assert_eq!(handle_document_color(&link_req), json!([]));
        assert_eq!(handle_color_presentation(&link_req), json!([]));
        assert_eq!(handle_diagnostic(&link_req, &overlay_mgr)["kind"], "full");

        // Missing overlay / disk fallback tests
        let empty_mgr = OverlayManager::new();
        let missing_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "positions": [{ "line": 0, "character": 0 }]
            }
        });
        assert_eq!(handle_selection_range(&missing_req, &empty_mgr), json!([]));
        assert!(!handle_document_link(&link_req, &empty_mgr)
            .as_array()
            .unwrap()
            .is_empty());
        assert!(
            handle_text_document_content(&content_req, &empty_mgr)["text"]
                .as_str()
                .unwrap()
                .contains("hello")
        );

        let non_existent_req = json!({ "params": { "uri": "file:///non/existent/file.rs" } });
        assert_eq!(
            handle_text_document_content(&non_existent_req, &empty_mgr)["text"],
            ""
        );

        let mon_non_existent = json!({
            "params": {
                "textDocument": { "uri": "file:///non/existent/file.rs" },
                "position": { "line": 0, "character": 0 }
            }
        });
        assert_eq!(
            handle_moniker(&mon_non_existent, Some(&ws), &empty_mgr, &mut conv),
            json!([])
        );

        // Test presentation handlers with nonexistent file / None workspace
        assert_eq!(
            handle_document_link(&non_existent_req, &empty_mgr),
            json!([])
        );
        assert_eq!(handle_document_color(&non_existent_req), json!([]));
        assert_eq!(handle_color_presentation(&non_existent_req), json!([]));
        assert_eq!(
            handle_folding_range(&non_existent_req, &empty_mgr),
            json!([])
        );
        assert_eq!(
            handle_selection_range(&non_existent_req, &empty_mgr),
            json!([])
        );
        assert_eq!(handle_code_lens(&non_existent_req, &empty_mgr), json!([]));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
