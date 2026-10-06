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

use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
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
        let lines: Vec<&str> = doc_text.lines().collect();
        struct UseEntry {
            attrs_and_comments: Vec<String>,
            use_line: String,
            sort_key: String,
        }
        let mut entries: Vec<UseEntry> = Vec::new();
        let mut current_attrs: Vec<String> = Vec::new();
        let mut block_start_line = None;
        let mut block_end_line = None;
        let mut in_import_block = false;

        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("#[") || trimmed.starts_with("//") {
                if block_start_line.is_none() {
                    let has_following_use = lines
                        .iter()
                        .skip(i + 1)
                        .take_while(|l| {
                            let t = l.trim();
                            t.starts_with("#[") || t.starts_with("//") || t.is_empty()
                        })
                        .count();
                    let next_line = lines.get(i + 1 + has_following_use).map(|l| l.trim());
                    if let Some(nl) = next_line {
                        if nl.starts_with("use ") || nl.starts_with("pub use ") {
                            block_start_line = Some(i);
                            in_import_block = true;
                        }
                    }
                }
                if in_import_block {
                    current_attrs.push(line.to_string());
                }
            } else if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
                if block_start_line.is_none() {
                    block_start_line = Some(i);
                }
                in_import_block = true;
                block_end_line = Some(i);
                let sort_key = trimmed.to_string();
                entries.push(UseEntry {
                    attrs_and_comments: std::mem::take(&mut current_attrs),
                    use_line: line.to_string(),
                    sort_key,
                });
            } else if in_import_block {
                if trimmed.is_empty() {
                    let next_non_empty = lines
                        .iter()
                        .skip(i + 1)
                        .find(|l| !l.trim().is_empty())
                        .map(|l| l.trim());
                    if let Some(nl) = next_non_empty {
                        if nl.starts_with("use ")
                            || nl.starts_with("pub use ")
                            || nl.starts_with("#[")
                        {
                            continue;
                        }
                    }
                }
                break;
            }
        }

        if let (Some(start_idx), Some(end_idx)) = (block_start_line, block_end_line) {
            if !entries.is_empty() {
                entries.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
                entries.dedup_by(|a, b| {
                    a.sort_key == b.sort_key && a.attrs_and_comments == b.attrs_and_comments
                });
                let mut new_text_lines = Vec::new();
                for e in &entries {
                    for a in &e.attrs_and_comments {
                        new_text_lines.push(a.clone());
                    }
                    new_text_lines.push(e.use_line.clone());
                }
                let new_text = new_text_lines.join("\n");
                let last_line_len = lines[end_idx].chars().count() as u32;

                actions.push(json!({
                    "title": "Organize imports",
                    "kind": "source.organizeImports",
                    "edit": {
                        "changes": {
                            uri: [
                                {
                                    "range": {
                                        "start": { "line": start_idx as u32, "character": 0 },
                                        "end": { "line": end_idx as u32, "character": last_line_len }
                                    },
                                    "newText": new_text
                                }
                            ]
                        }
                    }
                }));
            }
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
                let expr = if sl == el {
                    let line_chars: Vec<char> = lines[sl].chars().collect();
                    if sc <= line_chars.len() && ec <= line_chars.len() && sc < ec {
                        line_chars[sc..ec].iter().collect::<String>()
                    } else {
                        String::new()
                    }
                } else {
                    let mut s = String::new();
                    for (l, line_str) in lines.iter().enumerate().take(el + 1).skip(sl) {
                        let chars: Vec<char> = line_str.chars().collect();
                        if l == sl {
                            if sc < chars.len() {
                                s.push_str(&chars[sc..].iter().collect::<String>());
                            }
                        } else if l == el {
                            let end_c = ec.min(chars.len());
                            s.push('\n');
                            s.push_str(&chars[..end_c].iter().collect::<String>());
                        } else {
                            s.push('\n');
                            s.push_str(line_str);
                        }
                    }
                    s
                };

                if !expr.trim().is_empty() {
                    // Check if opening brace { exists on line `sl` before `sc`
                    let line_before = &lines[sl][..sc.min(lines[sl].len())];
                    let (insert_line, insert_col, new_text) =
                        if let Some(b_idx) = line_before.rfind('{') {
                            let mut after_brace = b_idx + 1;
                            if lines[sl].as_bytes().get(after_brace) == Some(&b' ') {
                                after_brace += 1;
                            }
                            (
                                sl as u32,
                                after_brace as u32,
                                format!("let extracted = {}; ", expr.trim()),
                            )
                        } else {
                            let indent: String = lines[sl]
                                .chars()
                                .take_while(|c| c.is_whitespace())
                                .collect();
                            (
                                sl as u32,
                                0u32,
                                format!("{indent}let extracted = {};\n", expr.trim()),
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
                        "newText": "extracted"
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
        "ridx.resolveWorkspace" => json!({ "resolved": true }),
        "ridx.showReferences" => json!({ "success": true }),
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
                    if let Some(changes) = edit.get("changes").and_then(|c| c.as_object()) {
                        for (uri, edits) in changes {
                            let file_path = uri_to_path(uri);
                            if let Some(edits_arr) = edits.as_array() {
                                if overlay_mgr.apply_content_changes(&file_path, 0, edits_arr) {
                                    applied_count += 1;
                                }
                            }
                        }
                    }
                }
            }
            json!({ "applied": true, "count": applied_count })
        }
        _ => Value::Null,
    }
}
