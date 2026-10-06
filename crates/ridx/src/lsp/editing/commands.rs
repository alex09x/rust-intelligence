/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP workspace command execution and atomic multi-document WorkspaceEdit application.

use super::ident::apply_workspace_text_edits_to_content;
use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

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
        "ridx.applyEdits" => execute_apply_edits(msg, overlay_mgr),
        _ => Value::Null,
    }
}

struct PlannedDocEdit {
    file_path: String,
    uri: String,
    is_overlay: bool,
    new_version: i32,
    new_text: String,
    edit_count: usize,
}

fn execute_apply_edits(msg: &Value, overlay_mgr: &mut OverlayManager) -> Value {
    let args = match msg["params"]["arguments"].as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return json!({ "applied": false, "count": 0, "failure": "no_arguments" }),
    };

    let mut planned: Vec<PlannedDocEdit> = Vec::new();
    let mut combined_edits_val: Option<Value> = None;

    for arg in args {
        let edit = if arg.get("changes").is_some() || arg.get("documentChanges").is_some() {
            arg
        } else if let Some(e) = arg.get("edit") {
            e
        } else {
            arg
        };

        if combined_edits_val.is_none() {
            combined_edits_val = Some(edit.clone());
        }

        let mut has_doc_changes = false;
        if let Some(doc_changes) = edit.get("documentChanges").and_then(|dc| dc.as_array()) {
            if !doc_changes.is_empty() {
                has_doc_changes = true;
                for dc in doc_changes {
                    let uri = dc["textDocument"]["uri"].as_str().unwrap_or("");
                    let doc_version = dc["textDocument"]["version"].as_i64().map(|v| v as i32);
                    let file_path = uri_to_path(uri);
                    let edits = match dc["edits"].as_array() {
                        Some(e) if !e.is_empty() => e,
                        _ => continue,
                    };

                    match validate_doc_edit(overlay_mgr, &file_path, uri, doc_version, edits) {
                        Some(plan) => planned.push(plan),
                        None => {
                            return json!({
                                "applied": false,
                                "count": 0,
                                "failure": "validation_failed"
                            });
                        }
                    }
                }
            }
        }

        if !has_doc_changes {
            if let Some(changes) = edit.get("changes").and_then(|c| c.as_object()) {
                for (uri, edits) in changes {
                    let file_path = uri_to_path(uri);
                    let edits_arr = match edits.as_array() {
                        Some(e) if !e.is_empty() => e,
                        _ => continue,
                    };

                    match validate_doc_edit(overlay_mgr, &file_path, uri, None, edits_arr) {
                        Some(plan) => planned.push(plan),
                        None => {
                            return json!({
                                "applied": false,
                                "count": 0,
                                "failure": "validation_failed"
                            });
                        }
                    }
                }
            }
        }
    }

    if planned.is_empty() {
        return json!({ "applied": false, "count": 0 });
    }

    let mut total_count = 0;
    for plan in planned {
        total_count += plan.edit_count;
        if plan.is_overlay {
            if let Some(overlay) = overlay_mgr.get_mut(&plan.file_path) {
                overlay.update(plan.new_version, plan.new_text.clone());
            }
            let _ = std::fs::write(&plan.file_path, &plan.new_text);
        } else if std::fs::write(&plan.file_path, &plan.new_text).is_ok() {
            overlay_mgr.open_with_provenance(
                &plan.uri,
                &plan.file_path,
                plan.new_version,
                plan.new_text,
                false,
            );
        }
    }

    let mut response = json!({
        "applied": true,
        "count": total_count,
    });
    if let Some(edit_val) = combined_edits_val {
        response["edit"] = edit_val;
    }
    response
}

fn validate_doc_edit(
    overlay_mgr: &OverlayManager,
    file_path: &str,
    uri: &str,
    version: Option<i32>,
    edits: &[Value],
) -> Option<PlannedDocEdit> {
    if let Some(overlay) = overlay_mgr.get(file_path) {
        if let Some(expected_ver) = version {
            if expected_ver != overlay.version {
                return None;
            }
        }
        let new_text = apply_workspace_text_edits_to_content(&overlay.text, edits)?;
        Some(PlannedDocEdit {
            file_path: file_path.to_string(),
            uri: uri.to_string(),
            is_overlay: true,
            new_version: overlay.version + 1,
            new_text,
            edit_count: edits.len(),
        })
    } else if let Ok(content) = std::fs::read_to_string(file_path) {
        let new_text = apply_workspace_text_edits_to_content(&content, edits)?;
        Some(PlannedDocEdit {
            file_path: file_path.to_string(),
            uri: uri.to_string(),
            is_overlay: false,
            new_version: version.unwrap_or(1),
            new_text,
            edit_count: edits.len(),
        })
    } else {
        None
    }
}
