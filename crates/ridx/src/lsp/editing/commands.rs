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
use std::collections::HashMap;

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

struct WorkingDoc {
    file_path: String,
    uri: String,
    is_overlay: bool,
    version: i32,
    current_text: String,
    edit_count: usize,
    all_edits: Vec<Value>,
}

fn execute_apply_edits(msg: &Value, overlay_mgr: &mut OverlayManager) -> Value {
    let args = match msg["params"]["arguments"].as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return json!({ "applied": false, "count": 0, "failure": "no_arguments" }),
    };

    let mut working_docs: HashMap<String, WorkingDoc> = HashMap::new();

    let get_or_load_doc = |docs: &mut HashMap<String, WorkingDoc>,
                           file_path: &str,
                           uri: &str,
                           overlay_mgr: &OverlayManager|
     -> Option<()> {
        if !docs.contains_key(file_path) {
            if let Some(overlay) = overlay_mgr.get(file_path) {
                docs.insert(
                    file_path.to_string(),
                    WorkingDoc {
                        file_path: file_path.to_string(),
                        uri: uri.to_string(),
                        is_overlay: true,
                        version: overlay.version,
                        current_text: overlay.text.clone(),
                        edit_count: 0,
                        all_edits: Vec::new(),
                    },
                );
            } else {
                let content = std::fs::read_to_string(file_path).ok()?;
                docs.insert(
                    file_path.to_string(),
                    WorkingDoc {
                        file_path: file_path.to_string(),
                        uri: uri.to_string(),
                        is_overlay: false,
                        version: 0,
                        current_text: content,
                        edit_count: 0,
                        all_edits: Vec::new(),
                    },
                );
            }
        }
        Some(())
    };

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
                for dc in doc_changes {
                    let uri = dc["textDocument"]["uri"].as_str().unwrap_or("");
                    let doc_version = dc["textDocument"]["version"].as_i64().map(|v| v as i32);
                    let file_path = uri_to_path(uri);
                    let edits = match dc["edits"].as_array() {
                        Some(e) if !e.is_empty() => e,
                        _ => continue,
                    };

                    if get_or_load_doc(&mut working_docs, &file_path, uri, overlay_mgr).is_none() {
                        return json!({ "applied": false, "count": 0, "failure": "file_not_found" });
                    }
                    let doc = working_docs.get_mut(&file_path).unwrap();
                    if let Some(expected_ver) = doc_version {
                        if expected_ver != doc.version {
                            return json!({ "applied": false, "count": 0, "failure": "version_mismatch" });
                        }
                    }
                    let Some(new_text) =
                        apply_workspace_text_edits_to_content(&doc.current_text, edits)
                    else {
                        return json!({ "applied": false, "count": 0, "failure": "validation_failed" });
                    };
                    doc.current_text = new_text;
                    doc.version += 1;
                    doc.edit_count += edits.len();
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

                    if get_or_load_doc(&mut working_docs, &file_path, uri, overlay_mgr).is_none() {
                        return json!({ "applied": false, "count": 0, "failure": "file_not_found" });
                    }
                    let doc = working_docs.get_mut(&file_path).unwrap();
                    let Some(new_text) =
                        apply_workspace_text_edits_to_content(&doc.current_text, edits_arr)
                    else {
                        return json!({ "applied": false, "count": 0, "failure": "validation_failed" });
                    };
                    doc.current_text = new_text;
                    doc.version += 1;
                    doc.edit_count += edits_arr.len();
                    doc.all_edits.extend(edits_arr.iter().cloned());
                }
            }
        }
    }

    if working_docs.is_empty() {
        return json!({ "applied": false, "count": 0 });
    }

    // Phase 2: Commit.
    // 2a. Check and write closed files to disk. If any write fails, rollback and report failure.
    let mut written_closed_files: Vec<(String, String)> = Vec::new();
    let mut write_failed = false;
    for doc in working_docs.values() {
        if !doc.is_overlay {
            let orig = std::fs::read_to_string(&doc.file_path).unwrap_or_default();
            if std::fs::write(&doc.file_path, &doc.current_text).is_err() {
                write_failed = true;
                break;
            }
            written_closed_files.push((doc.file_path.clone(), orig));
        }
    }

    if write_failed {
        // Rollback any closed files written so far
        for (fp, orig_text) in written_closed_files {
            let _ = std::fs::write(fp, orig_text);
        }
        return json!({
            "applied": false,
            "count": 0,
            "failure": "disk_write_failed"
        });
    }

    // 2b. Commit in-memory state:
    // For open overlays: update memory only! DO NOT write to disk (prevents saving dirty draft).
    // For closed files: register in overlay manager with compiled baseline = false.
    let mut total_count = 0;
    let mut combined_changes = serde_json::Map::new();

    for doc in working_docs.into_values() {
        total_count += doc.edit_count;
        combined_changes.insert(doc.uri.clone(), Value::Array(doc.all_edits));

        if doc.is_overlay {
            if let Some(overlay) = overlay_mgr.get_mut(&doc.file_path) {
                overlay.update(doc.version, doc.current_text);
            }
        } else {
            overlay_mgr.open_with_provenance(
                &doc.uri,
                &doc.file_path,
                doc.version,
                doc.current_text,
                false,
            );
        }
    }

    let combined_edit = json!({ "changes": combined_changes });

    json!({
        "applied": true,
        "count": total_count,
        "edit": combined_edit,
    })
}
