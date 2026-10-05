/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP intelligence handlers: hover, context-aware completion, signature help, and inlay hints.

use super::protocol::{lsp_completion_kind, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn handle_hover(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
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

    // 1. Check overlay definition and inferred signature
    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        let hover_text = if let Some(sig) = def.signature {
            format!("```rust\n{}\n```", sig)
        } else {
            format!("```rust\n{} {}\n```", def.kind.as_str(), def.name)
        };
        return json!({
            "contents": {
                "kind": "markdown",
                "value": hover_text
            }
        });
    }

    // 2. Check workspace compiled hover and type-at
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(hover) = ws.hover_at(&file, line_1based, col_1based) {
                let mut parts = Vec::new();
                if let Some(ref d) = hover.definition {
                    if let Some(sig) = d.signature {
                        parts.push(sig.to_string());
                    } else {
                        parts.push(format!("{} {}", d.kind.as_str(), d.name));
                    }
                }
                if let Some(t) = hover.inferred_type {
                    if !parts.iter().any(|p| p.contains(t)) {
                        parts.push(format!("Type: {}", t));
                    }
                }
                if !parts.is_empty() {
                    let hover_text = format!("```rust\n{}\n```", parts.join("\n"));
                    return json!({
                        "contents": {
                            "kind": "markdown",
                            "value": hover_text
                        }
                    });
                }
            }
        }
    }

    Value::Null
}

pub fn handle_completion(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
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

    let mut items = Vec::new();
    let mut seen_names = HashSet::new();

    // Determine prefix under/before cursor
    let mut prefix = String::new();
    if let Some(overlay) = overlay_opt {
        let line_idx = line_0based as usize;
        if let Some(&line_start) = overlay.line_starts.get(line_idx) {
            let byte_pos = overlay.line_char_col_to_byte(line_1based, col_1based);
            let line_slice = &overlay.text[line_start..byte_pos.min(overlay.text.len())];
            for c in line_slice.chars().rev() {
                if c.is_alphanumeric() || c == '_' {
                    prefix.insert(0, c);
                } else {
                    break;
                }
            }
        }
    }

    // 1. Overlay symbols: ONLY in-scope symbols, excluding locals from sibling/unrelated scopes
    if let Some(overlay) = overlay_opt {
        let cursor_byte = overlay.line_char_col_to_byte(line_1based, col_1based);

        // Find enclosing scopes containing cursor
        let active_scope_ids: HashSet<usize> = {
            let mut set = HashSet::new();
            for s in &overlay.scopes {
                if cursor_byte >= s.start_byte && cursor_byte <= s.end_byte {
                    set.insert(s.id);
                    let mut cur = s.parent;
                    while let Some(pid) = cur {
                        set.insert(pid);
                        cur = overlay.scopes.get(pid).and_then(|p| p.parent);
                    }
                }
            }
            set
        };

        for s in &overlay.symbols {
            if !prefix.is_empty() && !s.name.starts_with(&prefix) {
                continue;
            }

            let is_in_scope = match s.kind {
                DefKind::Fn
                | DefKind::Struct
                | DefKind::Enum
                | DefKind::Trait
                | DefKind::TyAlias
                | DefKind::Const
                | DefKind::Mod => true,
                _ => active_scope_ids.contains(&s.scope_id) && s.decl_start_byte <= cursor_byte,
            };

            if is_in_scope && seen_names.insert(s.name.clone()) {
                items.push(json!({
                    "label": s.name,
                    "kind": lsp_completion_kind(s.kind),
                    "detail": s.signature
                }));
            }
        }
    }

    // 2. Workspace symbols
    let mut is_capped = false;
    if let Some(ws) = workspace {
        let ws_symbols = ws.search_symbols(&prefix);
        for s in ws_symbols {
            if seen_names.insert(s.name.to_string()) {
                items.push(json!({
                    "label": s.name,
                    "kind": lsp_completion_kind(s.kind),
                    "detail": s.signature.map(|sig| sig.to_string())
                }));
            }
            if items.len() >= 50 {
                is_capped = true;
                break;
            }
        }
    }

    json!({
        "isIncomplete": is_capped,
        "items": items
    })
}

pub fn handle_completion_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}

pub fn handle_signature_help(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
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

    // Extract text preceding cursor on the line (or across previous lines if multiline)
    let line_text = if let Some(overlay) = overlay_opt {
        let line_idx = line_0based as usize;
        let line_start = *overlay.line_starts.get(line_idx).unwrap_or(&0);
        let byte_pos = overlay.line_char_col_to_byte(line_1based, col_1based);
        overlay.text[line_start..byte_pos.min(overlay.text.len())].to_string()
    } else if let Ok(content) = std::fs::read_to_string(&file) {
        if let Some(line_str) = content.lines().nth(line_0based as usize) {
            let col_idx = col_1based.saturating_sub(1) as usize;
            line_str.chars().take(col_idx).collect()
        } else {
            return Value::Null;
        }
    } else {
        return Value::Null;
    };

    // Locate enclosing call expression: scan backwards tracking depth of nested parentheses/brackets
    let mut paren_depth = 0;
    let mut angle_depth = 0;
    let mut active_param = 0;
    let mut open_paren_pos = None;

    let chars: Vec<char> = line_text.chars().collect();
    for (i, &c) in chars.iter().enumerate().rev() {
        match c {
            ')' => paren_depth += 1,
            '(' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                } else {
                    open_paren_pos = Some(i);
                    break;
                }
            }
            '>' => angle_depth += 1,
            '<' => {
                if angle_depth > 0 {
                    angle_depth -= 1;
                }
            }
            ',' if paren_depth == 0 && angle_depth == 0 => {
                active_param += 1;
            }
            _ => {}
        }
    }

    let Some(call_start) = open_paren_pos else {
        return Value::Null;
    };

    // Extract callee name immediately preceding '('
    let mut name_end = call_start;
    while name_end > 0 && chars[name_end - 1].is_whitespace() {
        name_end -= 1;
    }
    let mut name_start = name_end;
    while name_start > 0
        && (chars[name_start - 1] == '_' || chars[name_start - 1].is_alphanumeric())
    {
        name_start -= 1;
    }

    if name_start == name_end {
        return Value::Null;
    }

    let callee_name: String = chars[name_start..name_end].iter().collect();

    // Resolve callee signature
    let signature = if let Some(overlay) = overlay_opt {
        overlay
            .symbols
            .iter()
            .find(|s| s.name == callee_name && s.signature.is_some())
            .and_then(|s| s.signature.clone())
    } else {
        None
    };

    let signature = signature.or_else(|| {
        if let Some(ws) = workspace {
            let syms = ws.search_symbols(&callee_name);
            for s in syms {
                if s.name == callee_name {
                    if let Some(sig) = s.signature {
                        return Some(sig.to_string());
                    }
                }
            }
        }
        None
    });

    let Some(sig) = signature else {
        return Value::Null;
    };

    // Safely parse parameters between outer parentheses without breaking nested types like Vec<A, B>
    let parameters: Vec<Value> = if let Some(start) = sig.find('(') {
        if let Some(end) = sig.rfind(')') {
            let inner = &sig[start + 1..end];
            let mut params = Vec::new();
            let mut cur_param = String::new();
            let mut depth = 0;
            for c in inner.chars() {
                match c {
                    '<' | '(' | '[' => {
                        depth += 1;
                        cur_param.push(c);
                    }
                    '>' | ')' | ']' => {
                        if depth > 0 {
                            depth -= 1;
                        }
                        cur_param.push(c);
                    }
                    ',' if depth == 0 => {
                        let trimmed = cur_param.trim();
                        if !trimmed.is_empty() {
                            params.push(json!({ "label": trimmed }));
                        }
                        cur_param.clear();
                    }
                    _ => cur_param.push(c),
                }
            }
            let trimmed = cur_param.trim();
            if !trimmed.is_empty() {
                params.push(json!({ "label": trimmed }));
            }
            params
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    json!({
        "signatures": [
            {
                "label": sig,
                "parameters": parameters
            }
        ],
        "activeSignature": 0,
        "activeParameter": active_param
    })
}

pub fn handle_inlay_hint(
    _msg: &Value,
    _workspace: Option<&WorkspaceIndex>,
    _overlay_mgr: &OverlayManager,
) -> Value {
    json!([])
}

pub fn handle_inline_value(_msg: &Value) -> Value {
    json!([])
}

pub fn handle_inline_completion(_msg: &Value) -> Value {
    json!({ "items": [] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::protocol::PositionConverter;
    use crate::overlay::OverlayManager;
    use ridx_format::{IndexWriter, RawDefinition, RawType, WorkspaceIndex};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    use std::sync::atomic::{AtomicUsize, Ordering};
    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn setup_test_intelligence() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
        let test_id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!(
            "ridx_test_intel_{}_{}",
            std::process::id(),
            test_id
        ));
        let _ = fs::create_dir_all(temp_dir.join("src"));
        let src_file = temp_dir.join("src/lib.rs");
        let code =
            "pub fn calculate(a: i32, b: i32) -> i32 { a + b }\npub struct User { pub id: u64 }\n";
        let _ = fs::write(&src_file, code);

        let mut writer = IndexWriter::new("test_intel", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "calculate".to_string(),
            start_line: 1,
            start_col: 8,
            end_line: 1,
            end_col: 17,
            path: "test_intel::calculate".to_string(),
            signature: Some("fn calculate(a: i32, b: i32) -> i32".to_string()),
        });
        writer.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "User".to_string(),
            start_line: 2,
            start_col: 12,
            end_line: 2,
            end_col: 16,
            path: "test_intel::User".to_string(),
            signature: None,
        });
        writer.add_definition(RawDefinition {
            def_id: 3,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "complex_fn".to_string(),
            start_line: 3,
            start_col: 8,
            end_line: 3,
            end_col: 18,
            path: "test_intel::complex_fn".to_string(),
            signature: Some("fn complex_fn(a: Vec<Option<i32>>, b: [u8; 4])".to_string()),
        });

        // Add 50 dummy symbols to test completion capping (lines 192-195)
        for i in 10..65 {
            writer.add_definition(RawDefinition {
                def_id: i,
                kind: ridx_format::DefKind::Fn,
                visibility: 0,
                is_macro_expansion: false,
                file: "src/lib.rs".to_string(),
                name: format!("cap_fn_{}", i),
                path: format!("test_intel::cap_fn_{}", i),
                signature: None,
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 5,
            });
        }

        writer.add_type(RawType {
            file: "src/lib.rs".to_string(),
            start_line: 1,
            start_col: 43,
            end_line: 1,
            end_col: 48,
            type_name: "i32".to_string(),
        });
        writer.add_type(RawType {
            file: src_file.to_str().unwrap().to_string(),
            start_line: 1,
            start_col: 43,
            end_line: 1,
            end_col: 48,
            type_name: "i32".to_string(),
        });

        let ridx_file = temp_dir.join("index.ridx");
        {
            let mut f = fs::File::create(&ridx_file).unwrap();
            writer.write_to(&mut f).unwrap();
        }

        let mut ws = WorkspaceIndex::open(&ridx_file).unwrap();
        ws.set_workspace_root(Some(temp_dir.clone()));
        let overlay_mgr = OverlayManager::new();
        let conv = PositionConverter::new();

        (temp_dir, ws, overlay_mgr, conv)
    }

    #[test]
    fn test_hover_comprehensive() {
        let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
        let file_path = temp_dir.join("src/lib.rs");
        let uri = format!("file://{}", file_path.to_str().unwrap());

        // 1. Workspace hover hit
        let req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 10 }
            }
        });
        let h_res = handle_hover(&req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!h_res.is_null());
        assert!(h_res["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("calculate"));

        // 2. Overlay hover hit
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            1,
            "fn local_work() -> bool { true }\n".to_string(),
        );
        let req_ov = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 4 }
            }
        });
        let h_ov = handle_hover(&req_ov, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!h_ov.is_null());

        // Workspace hover on struct without signature
        let req_user = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 12 }
            }
        });
        let h_user = handle_hover(&req_user, Some(&ws), &OverlayManager::new(), &mut conv);
        assert!(!h_user.is_null());

        // Hover fallback to type_at only (lines 81-85)
        let req_ty = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 45 }
            }
        });
        let h_ty = handle_hover(&req_ty, Some(&ws), &OverlayManager::new(), &mut conv);
        assert!(!h_ty.is_null());
        assert!(h_ty["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Type: i32"));

        // 3. Hover miss
        let req_miss = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 99, "character": 0 }
            }
        });
        let h_miss = handle_hover(&req_miss, Some(&ws), &overlay_mgr, &mut conv);
        assert!(h_miss.is_null());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_completion_and_signature_help() {
        let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_intelligence();
        let file_path = temp_dir.join("src/lib.rs");
        let uri = format!("file://{}", file_path.to_str().unwrap());

        // 1. Completion with workspace and keywords
        let comp_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 3 }
            }
        });
        let comp_res = handle_completion(&comp_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!comp_res["items"].as_array().unwrap().is_empty());
        let resolve_res = handle_completion_resolve(&comp_req);
        assert!(!resolve_res.is_null());

        // Completion capped at 50
        let comp_all = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 0 }
            }
        });
        let res_capped = handle_completion(&comp_all, Some(&ws), &overlay_mgr, &mut conv);
        assert!(res_capped["isIncomplete"].as_bool().unwrap());

        // 2. Signature help in buffer
        let call_code = "fn test() {\n    calculate(10, 20);\n}\n";
        overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, call_code.to_string());
        let sig_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 19 }
            }
        });
        let sig_res = handle_signature_help(&sig_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!sig_res.is_null());
        assert_eq!(sig_res["activeParameter"], 1);

        // Signature help with nested parens, angle brackets, and brackets (lines 254-267)
        let nested_call = "fn test() {\n    calculate(nested(1), gen::<A, B>(), [1, 2]);\n}\n";
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            2,
            nested_call.to_string(),
        );
        let sig_nested = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 46 }
            }
        });
        let sig_nested_res = handle_signature_help(&sig_nested, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!sig_nested_res.is_null());

        // Overlay with function signature having nested types (lines 338-346)
        let overlay_code = "fn complex_fn(a: Vec<Option<i32>>, b: [u8; 4]) {}\nfn run() {\n    complex_fn(1, 2);\n}\n";
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            3,
            overlay_code.to_string(),
        );
        let sig_complex = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 2, "character": 17 }
            }
        });
        let sig_complex_res =
            handle_signature_help(&sig_complex, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!sig_complex_res.is_null());
        let params = sig_complex_res["signatures"][0]["parameters"]
            .as_array()
            .unwrap();
        assert_eq!(params.len(), 2);

        // Signature help from disk file
        let _ = fs::write(&file_path, call_code);
        let empty_mgr = OverlayManager::new();
        let sig_disk = handle_signature_help(&sig_req, Some(&ws), &empty_mgr, &mut conv);
        assert!(!sig_disk.is_null());

        // Signature help for unknown function (line 326)
        let unknown_call = "fn run() {\n    unknown_func(1);\n}\n";
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            4,
            unknown_call.to_string(),
        );
        let sig_unknown = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 17 }
            }
        });
        assert!(handle_signature_help(&sig_unknown, Some(&ws), &overlay_mgr, &mut conv).is_null());

        // 3. Inlay hints, inline value, inline completion
        let hint_res = handle_inlay_hint(&comp_req, Some(&ws), &overlay_mgr);
        assert_eq!(hint_res, json!([]));
        let val_res = handle_inline_value(&comp_req);
        assert_eq!(val_res, json!([]));
        let inline_comp = handle_inline_completion(&comp_req);
        assert_eq!(inline_comp, json!({ "items": [] }));

        // 4. Hover with overlay def lacking signature (line 43)
        let mut ov_no_sig = OverlayManager::new();
        ov_no_sig.open_document(
            &uri,
            file_path.to_str().unwrap(),
            1,
            "struct NoSigStruct;\n".to_string(),
        );
        let hover_nosig_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 8 }
            }
        });
        let h_nosig = handle_hover(&hover_nosig_req, None, &ov_no_sig, &mut conv);
        assert!(h_nosig["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("struct NoSigStruct"));

        // 5. Signature help edge cases:
        // - Whitespace before '(' (lines 285-286)
        // - More arguments than parameters (line 367)
        let ws_sym_call = "fn run() {\n    calculate   (1, 2, 3, 4, 5);\n}\n";
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            5,
            ws_sym_call.to_string(),
        );
        let sig_spaces = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 30 }
            }
        });
        // Run with overlay_mgr so it resolves signature via ws.search_symbols (lines 312-321)
        let sig_ws_res = handle_signature_help(&sig_spaces, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!sig_ws_res.is_null());

        // - Non-identifier before '(' (line 295)
        let non_ident_call = "fn run() {\n    (1 + 2);\n}\n";
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            6,
            non_ident_call.to_string(),
        );
        let sig_non_ident = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 6 }
            }
        });
        assert!(
            handle_signature_help(&sig_non_ident, Some(&ws), &overlay_mgr, &mut conv).is_null()
        );

        // - Disk fallback missing file (line 242) and line out of bounds (line 239)
        let missing_file_req = json!({
            "params": {
                "textDocument": { "uri": "file:///path/to/missing_file.rs" },
                "position": { "line": 0, "character": 0 }
            }
        });
        assert!(
            handle_signature_help(&missing_file_req, Some(&ws), &empty_mgr, &mut conv).is_null()
        );

        let out_of_bounds_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 9999, "character": 0 }
            }
        });
        assert!(
            handle_signature_help(&out_of_bounds_req, Some(&ws), &empty_mgr, &mut conv).is_null()
        );

        // - Callee symbol in workspace without signature (lines 316-320)
        let user_call = "fn run() {\n    User();\n}\n";
        overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 7, user_call.to_string());
        let sig_user = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 10 }
            }
        });
        assert!(handle_signature_help(&sig_user, Some(&ws), &overlay_mgr, &mut conv).is_null());

        // - Overlay signature without parens or with unclosed parens (lines 362, 365)
        let sig_edge_code =
            "fn no_paren;\nfn run() {\n    no_paren();\n    unclosed();\n}\nfn unclosed(a: i32";
        overlay_mgr.open_document(
            &uri,
            file_path.to_str().unwrap(),
            8,
            sig_edge_code.to_string(),
        );
        let sig_no_paren = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 2, "character": 13 }
            }
        });
        let res_no_paren = handle_signature_help(&sig_no_paren, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!res_no_paren.is_null());
        assert_eq!(
            res_no_paren["signatures"][0]["parameters"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        let sig_unclosed = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 3, "character": 13 }
            }
        });
        let res_unclosed = handle_signature_help(&sig_unclosed, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!res_unclosed.is_null());
        assert_eq!(
            res_unclosed["signatures"][0]["parameters"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
