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
        let mut first_use = None;
        let mut last_use = None;
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
                if first_use.is_none() {
                    first_use = Some(i);
                }
                last_use = Some(i);
            } else if first_use.is_some() && !trimmed.is_empty() && !trimmed.starts_with("//") {
                break;
            }
        }

        if let (Some(start_idx), Some(end_idx)) = (first_use, last_use) {
            let mut use_lines: Vec<String> = lines[start_idx..=end_idx]
                .iter()
                .filter(|l| {
                    let t = l.trim();
                    t.starts_with("use ") || t.starts_with("pub use ")
                })
                .map(|s| s.to_string())
                .collect();
            use_lines.sort();
            use_lines.dedup();
            let new_text = use_lines.join("\n");
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

    let is_range_empty = range["start"] == range["end"];
    if !is_range_empty && matches_kind("refactor.extract") {
        let sl = range["start"]["line"].as_u64().unwrap_or(0) as usize;
        let sc = range["start"]["character"].as_u64().unwrap_or(0) as usize;
        let el = range["end"]["line"].as_u64().unwrap_or(0) as usize;
        let ec = range["end"]["character"].as_u64().unwrap_or(0) as usize;

        let lines: Vec<&str> = doc_text.lines().collect();
        if sl < lines.len() && el < lines.len() {
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
                let indent: String = lines[sl]
                    .chars()
                    .take_while(|c| c.is_whitespace())
                    .collect();
                let insert_edit = json!({
                    "range": {
                        "start": { "line": sl as u32, "character": 0 },
                        "end": { "line": sl as u32, "character": 0 }
                    },
                    "newText": format!("{indent}let extracted = {};\n", expr.trim())
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

pub fn handle_execute_command(msg: &Value) -> Value {
    let cmd = msg["params"]["command"].as_str().unwrap_or("");
    match cmd {
        "ridx.resolveWorkspace" => json!({ "resolved": true }),
        "ridx.showReferences" => json!({ "success": true }),
        "ridx.applyEdits" => json!({ "applied": true }),
        _ => Value::Null,
    }
}
