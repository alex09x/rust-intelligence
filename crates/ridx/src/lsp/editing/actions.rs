/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Code actions, refactoring suggestions, and workspace command execution.

use super::ident::{byte_offset_to_utf16_col, utf16_col_to_byte_offset};
use crate::lsp::protocol::uri_to_path;
use crate::overlay::{DocumentOverlay, OverlayManager};
use serde_json::{json, Value};

pub fn handle_code_action(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let range = &msg["params"]["range"];
    let context = &msg["params"]["context"];

    let file = uri_to_path(uri);
    let doc_text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else {
        std::fs::read_to_string(&file).unwrap_or_default()
    };

    let only_kinds: Option<Vec<&str>> = context["only"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

    let matches_kind = |kind: &str| -> bool {
        match &only_kinds {
            None => true,
            Some(list) => list.iter().any(|k| kind.starts_with(*k) || *k == kind),
        }
    };

    let mut actions = Vec::new();

    if matches_kind("source.organizeImports") {
        if let Some(action) = super::organize::organize_imports_action(&doc_text, uri) {
            actions.push(action);
        }
    }

    let is_range_empty = range["start"] == range["end"];
    if !is_range_empty && matches_kind("refactor.extract") {
        let sl = range["start"]["line"].as_u64().unwrap_or(0) as usize;
        let sc = range["start"]["character"].as_u64().unwrap_or(0) as usize;
        let el = range["end"]["line"].as_u64().unwrap_or(0) as usize;
        let ec = range["end"]["character"].as_u64().unwrap_or(0) as usize;

        let lines: Vec<&str> = doc_text.lines().collect();
        if sl < lines.len() && el < lines.len() {
            // Check enclosing brace depth up to sl, sc
            let mut brace_depth: i32 = 0;
            let mut in_str = false;
            let mut in_comment = false;
            for (l_idx, line_str) in lines.iter().enumerate().take(sl + 1) {
                let limit = if l_idx == sl {
                    sc.min(line_str.len())
                } else {
                    line_str.len()
                };
                let bytes = line_str.as_bytes();
                let mut j = 0;
                while j < limit {
                    if in_comment {
                        if j + 1 < limit && bytes[j] == b'*' && bytes[j + 1] == b'/' {
                            in_comment = false;
                            j += 2;
                            continue;
                        }
                    } else if in_str {
                        if bytes[j] == b'\\' && j + 1 < limit {
                            j += 2;
                            continue;
                        }
                        if bytes[j] == b'"' {
                            in_str = false;
                        }
                    } else {
                        if j + 1 < limit && bytes[j] == b'/' && bytes[j + 1] == b'/' {
                            break;
                        }
                        if j + 1 < limit && bytes[j] == b'/' && bytes[j + 1] == b'*' {
                            in_comment = true;
                            j += 2;
                            continue;
                        }
                        if bytes[j] == b'"' {
                            in_str = true;
                        } else if bytes[j] == b'{' {
                            brace_depth += 1;
                        } else if bytes[j] == b'}' {
                            brace_depth = brace_depth.saturating_sub(1);
                        }
                    }
                    j += 1;
                }
            }

            // Only extract if inside a function/block
            if brace_depth > 0 {
                let (expr, start_byte) = if sl == el {
                    let sb = utf16_col_to_byte_offset(lines[sl], sc);
                    let eb = utf16_col_to_byte_offset(lines[sl], ec);
                    if sb < eb && eb <= lines[sl].len() {
                        (lines[sl][sb..eb].to_string(), sb)
                    } else {
                        (String::new(), 0)
                    }
                } else {
                    let mut s = String::new();
                    let sb = utf16_col_to_byte_offset(lines[sl], sc);
                    let eb = utf16_col_to_byte_offset(lines[el], ec);
                    for (l, line_str) in lines.iter().enumerate().take(el + 1).skip(sl) {
                        if l == sl {
                            if sb < line_str.len() {
                                s.push_str(&line_str[sb..]);
                            }
                        } else if l == el {
                            s.push('\n');
                            if eb <= line_str.len() {
                                s.push_str(&line_str[..eb]);
                            }
                        } else {
                            s.push('\n');
                            s.push_str(line_str);
                        }
                    }
                    (s, sb)
                };

                if !expr.trim().is_empty() {
                    let line_before = &lines[sl][..start_byte];
                    let last_semi = line_before.rfind(';');
                    let last_close = line_before.rfind('}');
                    let last_open = line_before.rfind('{');
                    let last_delim = [last_semi, last_close, last_open]
                        .into_iter()
                        .flatten()
                        .max();

                    let mut var_name = "extracted".to_string();
                    let mut counter = 1;
                    while doc_text.contains(&var_name) {
                        var_name = format!("extracted_{counter}");
                        counter += 1;
                    }

                    let (insert_line, insert_col, new_text) = if let Some(delim_idx) = last_delim {
                        let mut after_delim = delim_idx + 1;
                        while after_delim < lines[sl].len()
                            && lines[sl].as_bytes()[after_delim].is_ascii_whitespace()
                        {
                            after_delim += 1;
                        }
                        let col = byte_offset_to_utf16_col(lines[sl], after_delim) as u32;
                        (
                            sl as u32,
                            col,
                            format!("let {} = {}; ", var_name, expr.trim()),
                        )
                    } else {
                        let indent: String = lines[sl]
                            .chars()
                            .take_while(|c| c.is_whitespace())
                            .collect();
                        (
                            sl as u32,
                            0u32,
                            format!("{indent}let {} = {};\n", var_name, expr.trim()),
                        )
                    };

                    let insert_edit = json!({
                        "range": {
                            "start": { "line": insert_line, "character": insert_col },
                            "end": { "line": insert_line, "character": insert_col }
                        },
                        "newText": new_text
                    });
                    let replace_edit = json!({
                        "range": range,
                        "newText": var_name
                    });

                    actions.push(json!({
                        "title": "Extract variable",
                        "kind": "refactor.extract",
                        "edit": {
                            "changes": {
                                uri: [insert_edit, replace_edit]
                            }
                        }
                    }));
                }
            }
        }
    }

    if matches_kind("quickfix") {
        if let Some(diagnostics) = context["diagnostics"].as_array() {
            for diag in diagnostics {
                let code = diag["code"].as_str().unwrap_or("");
                let message = diag["message"].as_str().unwrap_or("");
                let diag_range = &diag["range"];

                if code == "unused_variable" || message.contains("unused variable") {
                    if let Some(var_name) = message.split('`').nth(1) {
                        actions.push(json!({
                            "title": format!("Prefix with underscore: `_{}`", var_name),
                            "kind": "quickfix",
                            "diagnostics": [diag],
                            "isPreferred": true,
                            "edit": {
                                "changes": {
                                    uri: [
                                        {
                                            "range": diag_range,
                                            "newText": format!("_{}", var_name)
                                        }
                                    ]
                                }
                            }
                        }));
                    }
                }
            }
        }
    }

    json!(actions)
}

pub fn handle_code_action_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}

pub fn handle_execute_command(msg: &Value, overlay_mgr: &mut OverlayManager) -> Value {
    let cmd = msg["params"]["command"].as_str().unwrap_or("");
    match cmd {
        "ridx.resolveWorkspace" => {
            let root = msg["params"]["rootUri"]
                .as_str()
                .or_else(|| msg["params"]["rootPath"].as_str())
                .unwrap_or("");
            json!({
                "resolved": true,
                "workspaceRoot": root,
                "status": "ready"
            })
        }
        "ridx.showReferences" => {
            let uri = msg["params"]["uri"].as_str().unwrap_or("");
            let pos = &msg["params"]["position"];
            let locs = msg["params"]["locations"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            json!({
                "success": true,
                "uri": uri,
                "position": pos,
                "locations": locs,
                "count": locs.len()
            })
        }
        "ridx.applyEdits" => {
            let mut applied_count = 0;
            if let Some(args) = msg["params"]["arguments"].as_array() {
                for arg in args {
                    let edit =
                        if arg.get("changes").is_some() || arg.get("documentChanges").is_some() {
                            arg
                        } else if let Some(e) = arg.get("edit") {
                            e
                        } else {
                            arg
                        };

                    if let Some(doc_changes) =
                        edit.get("documentChanges").and_then(|dc| dc.as_array())
                    {
                        for dc in doc_changes {
                            let uri = dc["textDocument"]["uri"].as_str().unwrap_or("");
                            let doc_version =
                                dc["textDocument"]["version"].as_i64().map(|v| v as i32);
                            let file_path = uri_to_path(uri);
                            if let Some(edits) = dc["edits"].as_array() {
                                if apply_workspace_text_edits(
                                    overlay_mgr,
                                    &file_path,
                                    uri,
                                    doc_version,
                                    edits,
                                ) {
                                    applied_count += edits.len();
                                }
                            }
                        }
                    }

                    if let Some(changes) = edit.get("changes").and_then(|c| c.as_object()) {
                        for (uri, edits) in changes {
                            let file_path = uri_to_path(uri);
                            if let Some(edits_arr) = edits.as_array() {
                                if apply_workspace_text_edits(
                                    overlay_mgr,
                                    &file_path,
                                    uri,
                                    None,
                                    edits_arr,
                                ) {
                                    applied_count += edits_arr.len();
                                }
                            }
                        }
                    }
                }
            }
            json!({ "applied": applied_count > 0, "count": applied_count })
        }
        _ => Value::Null,
    }
}

fn apply_workspace_text_edits(
    overlay_mgr: &mut OverlayManager,
    file_path: &str,
    uri: &str,
    version: Option<i32>,
    edits: &[Value],
) -> bool {
    if edits.is_empty() {
        return false;
    }
    if let Some(overlay) = overlay_mgr.get_mut(file_path) {
        let v = match version {
            Some(ver) if ver > 0 => ver,
            _ => overlay.version + 1,
        };
        overlay.apply_content_changes(v, edits);
        true
    } else if let Ok(content) = std::fs::read_to_string(file_path) {
        let mut overlay = DocumentOverlay::new_with_provenance(
            uri,
            file_path,
            version.unwrap_or(1),
            content,
            true,
        );
        overlay.apply_content_changes(version.unwrap_or(1), edits);
        let modified = overlay.text.clone();
        if std::fs::write(file_path, &modified).is_ok() {
            overlay_mgr.open_with_provenance(uri, file_path, version.unwrap_or(1), modified, true);
            true
        } else {
            false
        }
    } else {
        false
    }
}
