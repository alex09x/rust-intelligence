/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP editing handlers: rename, prepare rename, code formatting, code actions, and commands.

use super::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

#[inline]
pub fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

#[inline]
pub fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

pub fn is_valid_rust_ident(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !is_ident_start(first) {
        return false;
    }
    for c in chars {
        if !is_ident_continue(c) {
            return false;
        }
    }
    !is_rust_keyword(s)
}

pub fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
    )
}

pub fn handle_prepare_rename(
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

    // The returned range MUST identify the identifier string at the requested cursor position
    // in the REQUESTING document (not in the resolved target document).
    let (ident, start_char_col, end_char_col) = if let Some(overlay) = overlay_opt {
        if let Some((name, start_b, end_b)) = overlay.get_ident_and_span_at(line_1based, col_1based)
        {
            let line_idx = line_0based as usize;
            let line_start = *overlay.line_starts.get(line_idx).unwrap_or(&0);
            let line_text = &overlay.text[line_start..line_start + end_b.max(start_b)];
            let sc = line_text[..start_b].chars().count() as u32 + 1;
            let ec = line_text[..end_b].chars().count() as u32 + 1;
            (name, sc, ec)
        } else {
            return Value::Null;
        }
    } else if let Ok(content) = std::fs::read_to_string(&file) {
        if let Some(line_str) = content.lines().nth(line_0based as usize) {
            let col_idx = col_1based.saturating_sub(1) as usize;
            let char_indices: Vec<(usize, char)> = line_str.char_indices().collect();
            if col_idx < char_indices.len() {
                let mut start_i = col_idx;
                while start_i > 0 && is_ident_continue(char_indices[start_i - 1].1) {
                    start_i -= 1;
                }
                let mut end_i = col_idx;
                while end_i < char_indices.len() && is_ident_continue(char_indices[end_i].1) {
                    end_i += 1;
                }
                let name: String = char_indices[start_i..end_i]
                    .iter()
                    .map(|(_, c)| *c)
                    .collect();
                (name, start_i as u32 + 1, end_i as u32 + 1)
            } else {
                return Value::Null;
            }
        } else {
            return Value::Null;
        }
    } else {
        return Value::Null;
    };

    if !is_valid_rust_ident(&ident) {
        return Value::Null;
    }

    // Verify that the identifier actually corresponds to a symbol definition
    let is_resolvable = if overlay_opt
        .map(|o| o.find_definition_at(line_1based, col_1based).is_some())
        .unwrap_or(false)
    {
        true
    } else if let Some(ws) = workspace {
        ws.find_definition_at(&file, line_1based, col_1based)
            .is_some()
            || ws
                .find_references_at(&file, line_1based, col_1based)
                .is_some()
    } else {
        false
    };

    if !is_resolvable {
        return Value::Null;
    }

    let start_utf16 = if let Some(ov) = overlay_opt {
        ov.char_col_to_utf16_col(line_1based, start_char_col)
    } else {
        conv.char_col_to_utf16_col(&file, line_1based, start_char_col)
    };
    let end_utf16 = if let Some(ov) = overlay_opt {
        ov.char_col_to_utf16_col(line_1based, end_char_col)
    } else {
        conv.char_col_to_utf16_col(&file, line_1based, end_char_col)
    };

    json!({
        "range": {
            "start": { "line": line_0based, "character": start_utf16 },
            "end": { "line": line_0based, "character": end_utf16 }
        },
        "placeholder": ident
    })
}

pub fn handle_rename(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let new_name = msg["params"]["newName"].as_str().unwrap_or("");
    if !is_valid_rust_ident(new_name) {
        return json!({ "changes": {} });
    }

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

    let mut changes_map: HashMap<String, Vec<Value>> = HashMap::new();

    // 1. Check if renaming an overlay local variable / parameter
    if let Some(overlay) = overlay_opt {
        let local_refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !local_refs.is_empty() {
            let mut edits = Vec::new();
            for (sl, sc, el, ec) in local_refs {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                let edit = json!({
                    "range": {
                        "start": { "line": sl.saturating_sub(1), "character": start_char },
                        "end": { "line": el.saturating_sub(1), "character": end_char }
                    },
                    "newText": new_name
                });
                if !edits.iter().any(|e: &Value| e["range"] == edit["range"]) {
                    edits.push(edit);
                }
            }
            changes_map.insert(uri.to_string(), edits);
            return json!({ "changes": changes_map });
        }
    }

    // 2. Renaming a compiled symbol across the workspace
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                let target_name = loc_refs
                    .target_symbol
                    .rsplit("::")
                    .next()
                    .unwrap_or(loc_refs.target_symbol);

                // Add declaration edit with exact identifier span (same line, not entire item body!)
                if !loc_refs.target_file.is_empty() && loc_refs.target_file != "external" {
                    let decl_path = loc_refs.target_resolved_path();
                    let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                    let decl_uri = path_to_uri(decl_str);

                    let decl_ov = overlay_mgr.get(decl_str);
                    let (mapped_line, mapped_col) = if let Some(dov) = decl_ov {
                        if dov.is_dirty() {
                            dov.map_compiled_to_live(loc_refs.target_line, loc_refs.target_col)
                                .unwrap_or((loc_refs.target_line, loc_refs.target_col))
                        } else {
                            (loc_refs.target_line, loc_refs.target_col)
                        }
                    } else {
                        (loc_refs.target_line, loc_refs.target_col)
                    };

                    let name_len = target_name.chars().count() as u32;
                    let end_col = mapped_col + name_len;

                    let sc = if let Some(dov) = decl_ov {
                        dov.char_col_to_utf16_col(mapped_line, mapped_col)
                    } else {
                        conv.char_col_to_utf16_col(decl_str, mapped_line, mapped_col)
                    };
                    let ec = if let Some(dov) = decl_ov {
                        dov.char_col_to_utf16_col(mapped_line, end_col)
                    } else {
                        conv.char_col_to_utf16_col(decl_str, mapped_line, end_col)
                    };

                    let sl = mapped_line.saturating_sub(1);
                    let edits = changes_map.entry(decl_uri).or_default();
                    let edit = json!({
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": sl, "character": ec }
                        },
                        "newText": new_name
                    });
                    if !edits.iter().any(|e| e["range"] == edit["range"]) {
                        edits.push(edit);
                    }
                }

                // Add references edits with live buffer remapping for dirty target buffers
                for r in loc_refs.references {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);

                    let ref_ov = overlay_mgr.get(r_str);
                    let (mapped_line, mapped_col) = if let Some(rov) = ref_ov {
                        if rov.is_dirty() {
                            rov.map_compiled_to_live(r.span.start_line(), r.span.start_col())
                                .unwrap_or((r.span.start_line(), r.span.start_col()))
                        } else {
                            (r.span.start_line(), r.span.start_col())
                        }
                    } else {
                        (r.span.start_line(), r.span.start_col())
                    };

                    let name_len = target_name.chars().count() as u32;
                    let end_col = mapped_col + name_len;

                    let sc = if let Some(rov) = ref_ov {
                        rov.char_col_to_utf16_col(mapped_line, mapped_col)
                    } else {
                        conv.char_col_to_utf16_col(r_str, mapped_line, mapped_col)
                    };
                    let ec = if let Some(rov) = ref_ov {
                        rov.char_col_to_utf16_col(mapped_line, end_col)
                    } else {
                        conv.char_col_to_utf16_col(r_str, mapped_line, end_col)
                    };

                    let sl = mapped_line.saturating_sub(1);
                    let edits = changes_map.entry(r_uri).or_default();
                    let edit = json!({
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": sl, "character": ec }
                        },
                        "newText": new_name
                    });
                    if !edits.iter().any(|e| e["range"] == edit["range"]) {
                        edits.push(edit);
                    }
                }
            }
        }
    }

    json!({ "changes": changes_map })
}

fn find_crate_edition(file_path: &str) -> String {
    let mut cur = Path::new(file_path).parent();
    while let Some(dir) = cur {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.is_file() {
            if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("edition") {
                        if let Some((_, v)) = trimmed.split_once('=') {
                            let edition = v.trim().trim_matches('"').trim_matches('\'').trim();
                            if !edition.is_empty() {
                                return edition.to_string();
                            }
                        }
                    }
                }
            }
        }
        cur = dir.parent();
    }
    "2021".to_string()
}

pub fn handle_formatting(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let doc_text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return json!([]);
    };

    let edition = find_crate_edition(&file);

    let mut cmd = std::process::Command::new("rustfmt");
    cmd.arg("--edition").arg(&edition);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return json!([]),
    };

    if let Some(ref mut stdin) = child.stdin {
        let _ = stdin.write_all(doc_text.as_bytes());
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return json!([]),
    };

    if !output.status.success() {
        // Return empty edits on format error so document remains untouched
        return json!([]);
    }

    let formatted_text = String::from_utf8_lossy(&output.stdout).to_string();
    if formatted_text == doc_text || formatted_text.is_empty() {
        return json!([]);
    }

    let line_count = doc_text.lines().count().max(1) as u32;
    let last_line_len = doc_text
        .lines()
        .last()
        .map(|l| l.chars().count())
        .unwrap_or(0) as u32;

    json!([
        {
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": line_count, "character": last_line_len }
            },
            "newText": formatted_text
        }
    ])
}

pub fn handle_code_action(msg: &Value, _overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let range = &msg["params"]["range"];

    let mut actions = Vec::new();

    actions.push(json!({
        "title": "Organize imports",
        "kind": "source.organizeImports",
        "edit": {
            "changes": {
                uri: []
            }
        }
    }));

    actions.push(json!({
        "title": "Extract variable",
        "kind": "refactor.extract",
        "edit": {
            "changes": {
                uri: []
            }
        }
    }));

    actions.push(json!({
        "title": "Quick fix",
        "kind": "quickfix",
        "diagnostics": msg["params"]["context"]["diagnostics"].as_array().cloned().unwrap_or_default(),
        "edit": {
            "changes": {
                uri: [
                    {
                        "range": range,
                        "newText": ""
                    }
                ]
            }
        }
    }));

    json!(actions)
}

pub fn handle_code_action_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}

pub fn handle_execute_command(_msg: &Value) -> Value {
    Value::Null
}

pub fn handle_linked_editing_range(
    msg: &Value,
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
        let refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !refs.is_empty() {
            let mut ranges = Vec::new();
            for (sl, sc, el, ec) in refs {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                ranges.push(json!({
                    "start": { "line": sl.saturating_sub(1), "character": start_char },
                    "end": { "line": el.saturating_sub(1), "character": end_char }
                }));
            }
            return json!({ "ranges": ranges });
        }
    }

    Value::Null
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

    fn setup_test_editing() -> (PathBuf, WorkspaceIndex, OverlayManager, PositionConverter) {
        let temp_dir = std::env::temp_dir().join(format!("ridx_test_edit_{}", std::process::id()));
        let _ = fs::create_dir_all(temp_dir.join("src"));
        let src_file = temp_dir.join("src/lib.rs");
        let code = "pub fn add_values(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n";
        let _ = fs::write(&src_file, code);

        let src_file_str = src_file.to_str().unwrap().to_string();
        let mut writer = IndexWriter::new("test_edit", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: ridx_format::DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: src_file_str.clone(),
            name: "add_values".to_string(),
            start_line: 1,
            start_col: 8,
            end_line: 1,
            end_col: 18,
            path: "test_edit::add_values".to_string(),
            signature: Some("fn add_values(x: i32, y: i32) -> i32".to_string()),
        });

        writer.add_reference(ridx_format::RawReference {
            file: src_file_str.clone(),
            start_line: 1,
            start_col: 8,
            end_line: 1,
            end_col: 18,
            target_def_id: Some(1),
            target_path: "test_edit::add_values".to_string(),
            target_unit: None,
            target_file: Some(src_file_str.clone()),
            target_line: 1,
            target_col: 8,
            kind: ridx_format::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        let caller_file = temp_dir.join("src/caller.rs");
        let caller_code = "pub fn call() { test_edit::add_values(1, 2); }\n";
        let _ = fs::write(&caller_file, caller_code);
        let caller_str = caller_file.to_str().unwrap().to_string();

        writer.add_reference(ridx_format::RawReference {
            file: caller_str,
            start_line: 1,
            start_col: 17,
            end_line: 1,
            end_col: 27,
            target_def_id: Some(1),
            target_path: "test_edit::add_values".to_string(),
            target_unit: None,
            target_file: Some(src_file_str),
            target_line: 1,
            target_col: 8,
            kind: ridx_format::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
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
    fn test_ident_and_keyword_validation() {
        assert!(is_valid_rust_ident("my_var"));
        assert!(is_valid_rust_ident("_hidden"));
        assert!(!is_valid_rust_ident(""));
        assert!(!is_valid_rust_ident("123num"));
        assert!(!is_valid_rust_ident("let"));
        assert!(!is_valid_rust_ident("fn"));
        assert!(!is_valid_rust_ident("async"));
        assert!(is_rust_keyword("match"));
        assert!(!is_rust_keyword("my_func"));
    }

    #[test]
    fn test_editing_handlers_comprehensive() {
        let (temp_dir, ws, mut overlay_mgr, mut conv) = setup_test_editing();
        let file_path = temp_dir.join("src/lib.rs");
        let uri = format!("file://{}", file_path.to_str().unwrap());

        let code = "pub fn add_values(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n";
        overlay_mgr.open_document(&uri, file_path.to_str().unwrap(), 1, code.to_string());

        // 1. Prepare rename
        let prep_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 9 }
            }
        });
        let prep_res = handle_prepare_rename(&prep_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(!prep_res.is_null());

        let empty_mgr = OverlayManager::new();
        let prep_disk_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 10 }
            }
        });
        let prep_disk = handle_prepare_rename(&prep_disk_req, Some(&ws), &empty_mgr, &mut conv);
        assert!(!prep_disk.is_null());

        let prep_none = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 0 }
            }
        });
        assert_eq!(
            handle_prepare_rename(&prep_none, Some(&ws), &overlay_mgr, &mut conv),
            Value::Null
        );

        let prep_unres = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 4 }
            }
        });
        assert_eq!(
            handle_prepare_rename(&prep_unres, Some(&ws), &overlay_mgr, &mut conv),
            Value::Null
        );

        // 2. Rename (local variable in overlay)
        let rename_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 9 },
                "newName": "total"
            }
        });
        let rename_res = handle_rename(&rename_req, Some(&ws), &overlay_mgr, &mut conv);
        assert!(rename_res["changes"].get(&uri).is_some());

        // Rename compiled symbol across workspace
        let rename_compiled = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 8 },
                "newName": "sum_values"
            }
        });
        let res_compiled = handle_rename(&rename_compiled, Some(&ws), &overlay_mgr, &mut conv);
        assert!(res_compiled["changes"].get(&uri).is_some());

        let res_compiled_disk = handle_rename(&rename_compiled, Some(&ws), &empty_mgr, &mut conv);
        assert!(res_compiled_disk["changes"].get(&uri).is_some());

        // Invalid rename name -> empty changes
        let invalid_rename = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 9 },
                "newName": "123invalid"
            }
        });
        let err_res = handle_rename(&invalid_rename, Some(&ws), &overlay_mgr, &mut conv);
        assert!(err_res["changes"].as_object().unwrap().is_empty());

        // 3. Formatting
        let fmt_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "options": { "tabSize": 4, "insertSpaces": true }
            }
        });
        let fmt_res = handle_formatting(&fmt_req, &overlay_mgr);
        assert!(fmt_res.as_array().is_some());
        // Formatting unformatted document (hits lines 421-438)
        let unformatted_file = temp_dir.join("unformatted.rs");
        let unformatted_uri = path_to_uri(unformatted_file.to_str().unwrap());
        overlay_mgr.open_with_provenance(
            &unformatted_uri,
            unformatted_file.to_str().unwrap(),
            1,
            "fn  unformatted  (  x : i32 ) -> i32 {   x   }\n".to_string(),
            true,
        );
        let unformatted_req = json!({
            "params": {
                "textDocument": { "uri": &unformatted_uri }
            }
        });
        let unformatted_res = handle_formatting(&unformatted_req, &overlay_mgr);
        assert!(!unformatted_res.as_array().unwrap().is_empty());

        // Prepare rename error paths (lines 143, 146, 149, 173)
        let oob_line_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 999, "character": 0 }
            }
        });
        assert_eq!(
            handle_prepare_rename(&oob_line_req, None, &overlay_mgr, &mut conv),
            Value::Null
        );

        let oob_col_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 999 }
            }
        });
        assert_eq!(
            handle_prepare_rename(&oob_col_req, None, &overlay_mgr, &mut conv),
            Value::Null
        );

        let non_exist_prep = json!({
            "params": {
                "textDocument": { "uri": "file:///nonexistent/file.rs" },
                "position": { "line": 0, "character": 0 }
            }
        });
        assert_eq!(
            handle_prepare_rename(&non_exist_prep, None, &empty_mgr, &mut conv),
            Value::Null
        );

        // Rename with invalid ident (lines 243, 279, 301)
        let bad_rename = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 3 },
                "newName": "123_invalid"
            }
        });
        assert_eq!(
            handle_rename(&bad_rename, None, &overlay_mgr, &mut conv),
            json!({ "changes": {} })
        );

        // 4. Code action
        let action_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 1, "character": 0 }
                },
                "context": { "diagnostics": [] }
            }
        });
        let action_res = handle_code_action(&action_req, &overlay_mgr);
        assert!(action_res.is_array());
        let resolve_res = handle_code_action_resolve(&action_req);
        assert!(!resolve_res.is_null());

        let action_diag = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
                "context": { "diagnostics": [{ "message": "err", "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } } }] }
            }
        });
        let act_res = handle_code_action(&action_diag, &overlay_mgr);
        assert_eq!(act_res.as_array().unwrap().len(), 3);

        // 5. Execute command
        assert_eq!(handle_execute_command(&action_req), Value::Null);

        // 6. Linked editing range (including empty references fallback at line 525)
        let prep_req = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 1, "character": 9 }
            }
        });
        let linked_res = handle_linked_editing_range(&prep_req, &overlay_mgr, &mut conv);
        assert!(!linked_res.is_null());
        let prep_empty_ref = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 0, "character": 0 }
            }
        });
        assert_eq!(
            handle_linked_editing_range(&prep_empty_ref, &overlay_mgr, &mut conv),
            Value::Null
        );
        assert_eq!(
            handle_linked_editing_range(&prep_req, &empty_mgr, &mut conv),
            Value::Null
        );

        // 7. Edition helper
        let cargo_toml = temp_dir.join("Cargo.toml");
        fs::write(&cargo_toml, "[package]\nedition = \"2024\"\n").unwrap();
        assert_eq!(find_crate_edition(file_path.to_str().unwrap()), "2024");
        fs::write(&cargo_toml, "[package]\nedition = '2018'\n").unwrap();
        assert_eq!(find_crate_edition(file_path.to_str().unwrap()), "2018");
        fs::write(&cargo_toml, "[package]\nedition = \"\"\n").unwrap();
        assert_eq!(find_crate_edition(file_path.to_str().unwrap()), "2021");

        // 8. Prepare rename with missing symbol and reference fallback
        let prep_missing = json!({
            "params": {
                "textDocument": { "uri": &uri },
                "position": { "line": 999, "character": 0 }
            }
        });
        assert_eq!(
            handle_prepare_rename(&prep_missing, Some(&ws), &empty_mgr, &mut conv),
            Value::Null
        );

        // 9. Formatting from disk file with rustfmt and nonexistent file fallback
        let unformatted_file = temp_dir.join("src/unformatted.rs");
        fs::write(&unformatted_file, "fn unformatted(   ) {    1 + 1 ;   }\n").unwrap();
        let unformatted_uri = format!("file://{}", unformatted_file.display());
        let fmt_disk_req = json!({
            "params": {
                "textDocument": { "uri": &unformatted_uri }
            }
        });
        let fmt_disk_res = handle_formatting(&fmt_disk_req, &empty_mgr);
        assert!(fmt_disk_res.is_array());

        let fmt_nonexistent = json!({
            "params": {
                "textDocument": { "uri": "file:///nonexistent/disk/file.rs" }
            }
        });
        assert_eq!(handle_formatting(&fmt_nonexistent, &empty_mgr), json!([]));

        // 10. Rename with dirty overlay on declaration and reference (lines 265-267, 279, 284, 311-313, 325, 330)
        let mut rename_mgr = OverlayManager::new();
        let src_file_str = temp_dir.join("src/lib.rs").to_str().unwrap().to_string();
        let caller_file_str = temp_dir.join("src/caller.rs").to_str().unwrap().to_string();
        let src_uri = format!("file://{}", src_file_str);
        let caller_uri = format!("file://{}", caller_file_str);

        // src/lib.rs has a dirty overlay
        let code_base =
            "pub fn add_values(x: i32, y: i32) -> i32 {\n    let sum = x + y;\n    sum\n}\n";
        rename_mgr.open_document(&src_uri, &src_file_str, 1, code_base.to_string());
        rename_mgr.update(&src_file_str, 2, format!("// added line\n{}", code_base));

        // src/caller.rs has a dirty overlay
        let caller_code = "pub fn call() { test_edit::add_values(1, 2); }\n";
        rename_mgr.open_document(&caller_uri, &caller_file_str, 1, caller_code.to_string());
        rename_mgr.update(
            &caller_file_str,
            2,
            format!("// caller header\n{}", caller_code),
        );

        // Initiate rename from src/lib.rs without overlay (in empty_mgr), or from disk file
        let rename_cross_req = json!({
            "params": {
                "textDocument": { "uri": &src_uri },
                "position": { "line": 0, "character": 8 },
                "newName": "renamed_values"
            }
        });
        // With dirty overlays on target buffers:
        // Use an OverlayManager where src/lib.rs is clean (not dirty), but caller is dirty:
        let mut mixed_mgr = OverlayManager::new();
        mixed_mgr.open_document(&src_uri, &src_file_str, 1, code_base.to_string());
        mixed_mgr.open_document(&caller_uri, &caller_file_str, 1, caller_code.to_string());
        mixed_mgr.update(
            &caller_file_str,
            2,
            format!("// caller header\n{}", caller_code),
        );
        mixed_mgr.open_document("src/lib.rs", "src/lib.rs", 1, code_base.to_string());
        mixed_mgr.open_document("src/caller.rs", "src/caller.rs", 1, caller_code.to_string());
        mixed_mgr.update(
            "src/caller.rs",
            2,
            format!("// caller header\n{}", caller_code),
        );
        let rename_dirty_res = handle_rename(&rename_cross_req, Some(&ws), &mixed_mgr, &mut conv);
        assert!(rename_dirty_res.get("changes").is_some());

        // 11. Rename with non-dirty overlay on declaration and reference (lines 269, 279, 284, 315, 325, 330)
        let mut clean_rename_mgr = OverlayManager::new();
        clean_rename_mgr.open_document(&src_uri, &src_file_str, 1, code_base.to_string());
        clean_rename_mgr.open_document(&caller_uri, &caller_file_str, 1, caller_code.to_string());
        clean_rename_mgr.open_document("src/lib.rs", "src/lib.rs", 1, code_base.to_string());
        clean_rename_mgr.open_document(
            "src/caller.rs",
            "src/caller.rs",
            1,
            caller_code.to_string(),
        );
        let rename_clean_res =
            handle_rename(&rename_cross_req, Some(&ws), &clean_rename_mgr, &mut conv);
        assert!(rename_clean_res.get("changes").is_some());

        // 12. Prepare rename on whitespace (line 143)
        let prep_space_req = json!({
            "params": {
                "textDocument": { "uri": &src_uri },
                "position": { "line": 0, "character": 3 } // space between pub and fn
            }
        });
        assert!(handle_prepare_rename(&prep_space_req, Some(&ws), &empty_mgr, &mut conv).is_null());

        // 13. Prepare rename on a reference location (lines 165-167)
        let prep_ref_req = json!({
            "params": {
                "textDocument": { "uri": &caller_uri },
                "position": { "line": 0, "character": 20 } // on add_values inside caller (start_col: 17, end_col: 27)
            }
        });
        assert!(!handle_prepare_rename(&prep_ref_req, Some(&ws), &empty_mgr, &mut conv).is_null());

        // 14. Rename initiated from clean caller where declaration in lib.rs has dirty overlay (lines 265-267, 279, 284, 311-313, 325, 330)
        let mut decl_dirty_mgr = OverlayManager::new();
        decl_dirty_mgr.open_document(&caller_uri, &caller_file_str, 1, caller_code.to_string());
        decl_dirty_mgr.open_document(&src_uri, &src_file_str, 1, code_base.to_string());
        decl_dirty_mgr.update(
            &src_file_str,
            2,
            format!("// extra decl line\n{}", code_base),
        );
        decl_dirty_mgr.open_document("src/caller.rs", "src/caller.rs", 1, caller_code.to_string());
        decl_dirty_mgr.open_document("src/lib.rs", "src/lib.rs", 1, code_base.to_string());
        decl_dirty_mgr.update(
            "src/lib.rs",
            2,
            format!("// extra decl line\n{}", code_base),
        );
        let rename_from_clean_caller = json!({
            "params": {
                "textDocument": { "uri": &caller_uri },
                "position": { "line": 0, "character": 20 },
                "newName": "new_fn_name"
            }
        });
        let rename_decl_dirty_res = handle_rename(
            &rename_from_clean_caller,
            Some(&ws),
            &decl_dirty_mgr,
            &mut conv,
        );
        assert!(rename_decl_dirty_res.get("changes").is_some());

        // 15. find_crate_edition fallbacks (lines 369, 372)
        let no_edition_cargo = temp_dir.join("no_ed/Cargo.toml");
        let _ = fs::create_dir_all(no_edition_cargo.parent().unwrap());
        fs::write(
            &no_edition_cargo,
            b"[package]\nname = 'no_ed'\nedition\n[lib]\n",
        )
        .unwrap();
        assert_eq!(
            find_crate_edition(no_edition_cargo.to_str().unwrap()),
            "2021"
        );

        // 16. Prepare rename with None workspace (lines 169, 173)
        let prep_none = handle_prepare_rename(&prep_disk_req, None, &empty_mgr, &mut conv);
        assert!(prep_none.is_null());

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
