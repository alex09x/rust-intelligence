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
                    let mut delim_info: Option<(usize, usize)> = None;
                    let mut prefix_text = String::new();

                    for cur_l in (0..=sl).rev() {
                        let cur_line = lines[cur_l];
                        let cur_limit = if cur_l == sl {
                            start_byte
                        } else {
                            cur_line.len()
                        };
                        let slice = &cur_line[..cur_limit];

                        let last_semi = slice.rfind(';');
                        let last_close = slice.rfind('}');
                        let last_open = slice.rfind('{');
                        let local_delim = [last_semi, last_close, last_open]
                            .into_iter()
                            .flatten()
                            .max();

                        if let Some(d_idx) = local_delim {
                            delim_info = Some((cur_l, d_idx));
                            let seg = &slice[d_idx + 1..];
                            if prefix_text.is_empty() {
                                prefix_text = seg.to_string();
                            } else {
                                prefix_text = format!("{seg}\n{prefix_text}");
                            }
                            break;
                        } else if prefix_text.is_empty() {
                            prefix_text = slice.to_string();
                        } else {
                            prefix_text = format!("{slice}\n{prefix_text}");
                        }
                    }

                    let has_side_effect_or_control_flow = {
                        let p = &prefix_text;
                        p.contains("&&")
                            || p.contains("||")
                            || p.contains("=>")
                            || p.contains("while")
                            || p.contains("for")
                            || p.contains("if")
                            || p.contains("match")
                            || p.contains("loop")
                            || p.contains(',')
                            || p.contains(')')
                            || p.contains(']')
                            || p.contains('?')
                            || p.contains('|')
                    };

                    if !has_side_effect_or_control_flow {
                        let mut var_name = "extracted".to_string();
                        let mut counter = 1;
                        while doc_text.contains(&var_name) {
                            var_name = format!("extracted_{counter}");
                            counter += 1;
                        }

                        let (insert_line, insert_col, new_text) = if let Some((dl, d_idx)) =
                            delim_info
                        {
                            let mut after_delim = d_idx + 1;
                            while after_delim < lines[dl].len()
                                && lines[dl].as_bytes()[after_delim].is_ascii_whitespace()
                            {
                                after_delim += 1;
                            }
                            if after_delim < lines[dl].len() {
                                let col = byte_offset_to_utf16_col(lines[dl], after_delim) as u32;
                                (
                                    dl as u32,
                                    col,
                                    format!("let {} = {}; ", var_name, expr.trim()),
                                )
                            } else if dl + 1 < lines.len() {
                                let next_l = dl + 1;
                                let indent: String = lines[next_l]
                                    .chars()
                                    .take_while(|c| c.is_whitespace())
                                    .collect();
                                (
                                    next_l as u32,
                                    0u32,
                                    format!("{indent}let {} = {};\n", var_name, expr.trim()),
                                )
                            } else {
                                (
                                    dl as u32,
                                    lines[dl].len() as u32,
                                    format!("let {} = {}; ", var_name, expr.trim()),
                                )
                            }
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
