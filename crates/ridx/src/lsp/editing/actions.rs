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

use crate::overlay::OverlayManager;
use serde_json::{json, Value};

pub fn handle_code_action(msg: &Value, _overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let range = &msg["params"]["range"];
    let context = &msg["params"]["context"];

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
        actions.push(json!({
            "title": "Organize imports",
            "kind": "source.organizeImports",
            "edit": {
                "changes": {
                    uri: []
                }
            }
        }));
    }

    let is_range_empty = range["start"] == range["end"];
    if !is_range_empty && matches_kind("refactor.extract") {
        actions.push(json!({
            "title": "Extract variable",
            "kind": "refactor.extract",
            "edit": {
                "changes": {
                    uri: []
                }
            }
        }));
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
