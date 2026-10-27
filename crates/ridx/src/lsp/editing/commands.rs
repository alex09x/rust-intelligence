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

pub use super::atomic_write::commit_closed_writes_atomic;
use super::ident::apply_workspace_text_edits_to_content;
use crate::lsp::error::{LspError, LspResult};
use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn handle_execute_command(msg: &Value, overlay_mgr: &mut OverlayManager) -> LspResult<Value> {
    let cmd = msg["params"]["command"].as_str().unwrap_or("");
    match cmd {
        "ridx.resolveWorkspace" => {
            let root = msg["params"]["rootUri"]
                .as_str()
                .or_else(|| msg["params"]["rootPath"].as_str())
                .unwrap_or("");
            Ok(json!({ "resolved": true, "workspaceRoot": root, "status": "ready" }))
        }
        "ridx.showReferences" => {
            let args = msg["params"]["arguments"].as_array();
            let uri = args
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .or_else(|| msg["params"]["uri"].as_str())
                .unwrap_or("");
            let pos = args
                .and_then(|a| a.get(1))
                .unwrap_or(&msg["params"]["position"]);
            let locs = args
                .and_then(|a| a.get(2))
                .and_then(|v| v.as_array())
                .or_else(|| msg["params"]["locations"].as_array())
                .cloned()
                .unwrap_or_default();
            Ok(
                json!({ "success": true, "uri": uri, "position": pos, "count": locs.len(), "locations": locs }),
            )
        }
        "ridx.applyEdits" => Ok(execute_apply_edits(msg, overlay_mgr)),
        _ => Err(LspError::invalid_params(format!(
            "Unknown command: '{cmd}'"
        ))),
    }
}

struct WorkingDoc {
    file_path: String,
    uri: String,
    is_overlay: bool,
    client_version: Option<i32>,
    version: i32,
    initial_text: String,
    current_text: String,
    edit_count: usize,
    arg_count: usize,
    all_edits: Vec<Value>,
}

fn full_document_range(text: &str) -> Value {
    let mut line = 0;
    let mut last_col = 0;
    for c in text.chars() {
        if c == '\n' {
            line += 1;
            last_col = 0;
        } else {
            last_col += c.len_utf16();
        }
    }
    json!({
        "start": { "line": 0, "character": 0 },
        "end": { "line": line, "character": last_col }
    })
}

#[derive(Debug, Clone)]
pub struct ApplyPlan {
    pub total_count: usize,
    pub combined_edit: Value,
    pub closed_writes: Vec<(String, String)>,
    pub open_updates: Vec<(String, i32, String)>,
}

pub fn plan_apply_edits(args: &[Value], overlay_mgr: &OverlayManager) -> Result<ApplyPlan, Value> {
    if args.is_empty() {
        return Err(json!({ "applied": false, "count": 0, "failure": "no_arguments" }));
    }

    let mut working_docs: HashMap<String, WorkingDoc> = HashMap::new();

    let get_or_load_doc =
        |docs: &mut HashMap<String, WorkingDoc>, file_path: &str, uri: &str| -> Option<()> {
            if !docs.contains_key(file_path) {
                let (is_overlay, version, text) = if let Some(overlay) = overlay_mgr.get(file_path)
                {
                    (true, overlay.version, overlay.text.clone())
                } else {
                    (false, 0, std::fs::read_to_string(file_path).ok()?)
                };
                let client_version = if is_overlay { Some(version) } else { None };
                docs.insert(
                    file_path.to_string(),
                    WorkingDoc {
                        file_path: file_path.to_string(),
                        uri: uri.to_string(),
                        is_overlay,
                        client_version,
                        version,
                        initial_text: text.clone(),
                        current_text: text,
                        edit_count: 0,
                        arg_count: 0,
                        all_edits: Vec::new(),
                    },
                );
            }
            Some(())
        };

    let mut uses_document_changes = false;
    for arg in args {
        let edit = if arg.get("changes").is_some() || arg.get("documentChanges").is_some() {
            arg
        } else if let Some(e) = arg.get("edit") {
            e
        } else {
            arg
        };

        let mut has_doc_changes = false;
        if let Some(doc_changes) = edit.get("documentChanges").and_then(|dc| dc.as_array()) {
            if !doc_changes.is_empty() {
                has_doc_changes = true;
                uses_document_changes = true;
                for dc in doc_changes {
                    let uri = dc["textDocument"]["uri"].as_str().unwrap_or("");
                    let doc_version = dc["textDocument"]["version"].as_i64().map(|v| v as i32);
                    let file_path = uri_to_path(uri);
                    let edits = match dc["edits"].as_array() {
                        Some(e) if !e.is_empty() => e,
                        _ => continue,
                    };

                    if get_or_load_doc(&mut working_docs, &file_path, uri).is_none() {
                        return Err(
                            json!({ "applied": false, "count": 0, "failure": "file_not_found" }),
                        );
                    }
                    let doc = working_docs.get_mut(&file_path).unwrap();
                    if let Some(expected_ver) = doc_version {
                        if doc.client_version.is_none() {
                            doc.client_version = Some(expected_ver);
                        }
                        if expected_ver != doc.version {
                            return Err(
                                json!({ "applied": false, "count": 0, "failure": "version_mismatch" }),
                            );
                        }
                    }
                    let Some(new_text) =
                        apply_workspace_text_edits_to_content(&doc.current_text, edits)
                    else {
                        return Err(
                            json!({ "applied": false, "count": 0, "failure": "validation_failed" }),
                        );
                    };
                    doc.current_text = new_text;
                    doc.version += 1;
                    doc.edit_count += edits.len();
                    doc.arg_count += 1;
                    doc.all_edits.extend(edits.iter().cloned());
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

                    if get_or_load_doc(&mut working_docs, &file_path, uri).is_none() {
                        return Err(
                            json!({ "applied": false, "count": 0, "failure": "file_not_found" }),
                        );
                    }
                    let doc = working_docs.get_mut(&file_path).unwrap();
                    let Some(new_text) =
                        apply_workspace_text_edits_to_content(&doc.current_text, edits_arr)
                    else {
                        return Err(
                            json!({ "applied": false, "count": 0, "failure": "validation_failed" }),
                        );
                    };
                    doc.current_text = new_text;
                    doc.version += 1;
                    doc.edit_count += edits_arr.len();
                    doc.arg_count += 1;
                    doc.all_edits.extend(edits_arr.iter().cloned());
                }
            }
        }
    }

    if working_docs.is_empty() {
        return Err(json!({ "applied": false, "count": 0 }));
    }

    let mut total_count = 0;
    let mut combined_changes = serde_json::Map::new();
    let mut doc_changes = Vec::new();
    let mut closed_writes = Vec::new();
    let mut open_updates = Vec::new();

    for doc in working_docs.into_values() {
        total_count += doc.edit_count;
        let edits = if doc.arg_count > 1 {
            vec![json!({
                "range": full_document_range(&doc.initial_text),
                "newText": doc.current_text
            })]
        } else {
            doc.all_edits
        };

        if uses_document_changes {
            doc_changes.push(json!({
                "textDocument": {
                    "uri": doc.uri.clone(),
                    "version": doc.client_version
                },
                "edits": edits
            }));
        } else {
            combined_changes.insert(doc.uri.clone(), Value::Array(edits));
        }

        if doc.is_overlay {
            open_updates.push((doc.file_path, doc.version, doc.current_text));
        } else {
            closed_writes.push((doc.file_path, doc.current_text));
        }
    }

    let combined_edit = if uses_document_changes {
        json!({ "documentChanges": doc_changes })
    } else {
        json!({ "changes": combined_changes })
    };

    Ok(ApplyPlan {
        total_count,
        combined_edit,
        closed_writes,
        open_updates,
    })
}

fn execute_apply_edits(msg: &Value, overlay_mgr: &mut OverlayManager) -> Value {
    let args = match msg["params"]["arguments"].as_array() {
        Some(a) if !a.is_empty() => a.as_slice(),
        _ => return json!({ "applied": false, "count": 0, "failure": "no_arguments" }),
    };

    let plan = match plan_apply_edits(args, overlay_mgr) {
        Ok(p) => p,
        Err(err) => return err,
    };

    if !commit_closed_writes_atomic(&plan.closed_writes) {
        return json!({ "applied": false, "count": 0, "failure": "disk_write_failed" });
    }

    for (fp, text) in &plan.closed_writes {
        let uri = format!("file://{fp}");
        overlay_mgr.open_with_provenance(&uri, fp, 0, text.clone(), false);
    }

    for (fp, ver, text) in plan.open_updates {
        if let Some(overlay) = overlay_mgr.get_mut(&fp) {
            overlay.update(ver, text);
        }
    }

    json!({
        "applied": true,
        "count": plan.total_count,
        "edit": plan.combined_edit,
    })
}

pub fn try_dispatch_apply_edits_command<W: std::io::Write>(
    msg: &Value,
    overlay_mgr: &mut OverlayManager,
    lifecycle: &crate::lsp::lifecycle::LifecycleManager,
    writer: &mut W,
    id: &mut Option<Value>,
) -> std::io::Result<Option<bool>> {
    if msg["params"]["command"].as_str() != Some("ridx.applyEdits") {
        return Ok(None);
    }
    let args = msg["params"]["arguments"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    match plan_apply_edits(args, overlay_mgr) {
        Ok(plan) => {
            let apply_req_id = lifecycle.get_next_apply_id();
            crate::lsp::protocol::send_response(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "id": apply_req_id,
                    "method": "workspace/applyEdit",
                    "params": { "label": "Apply refactoring edits", "edit": plan.combined_edit }
                }),
            )?;
            lifecycle.register_pending_apply(
                apply_req_id,
                id.take(),
                plan.total_count,
                plan.combined_edit,
            );
            Ok(Some(true))
        }
        Err(err) => {
            if let Some(ref req_id) = id {
                crate::lsp::protocol::send_response(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": req_id, "result": err }),
                )?;
            }
            Ok(Some(true))
        }
    }
}
