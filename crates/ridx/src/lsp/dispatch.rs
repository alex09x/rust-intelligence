/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP request dispatching, protocol routing, and response formatting.

use super::editing::*;
use super::hierarchy::*;
use super::intelligence::*;
use super::lifecycle::LifecycleManager;
use super::navigation::*;
use super::presentation::*;
use super::protocol::{send_response, uri_to_path, PositionConverter};
use super::symbols::*;
use super::sync_reloaded_index;
use super::workspace::*;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub fn dispatch_lsp_message<W: Write>(
    msg: Value,
    writer: &mut W,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut crate::overlay::OverlayManager,
    cancelled_requests: &Arc<Mutex<HashSet<Value>>>,
    index_path: Option<&PathBuf>,
) -> io::Result<bool> {
    let method = msg["method"].as_str().unwrap_or("");
    let id = msg.get("id").cloned();

    if let Some(should_continue) =
        check_lifecycle_and_cancel(method, &id, lifecycle, cancelled_requests, writer)?
    {
        return Ok(should_continue);
    }

    // 5. Single reload: automatically reload indices if modified and sync overlays
    for ws in multi_root.all_workspaces_mut() {
        sync_reloaded_index(ws, overlay_mgr);
    }

    let mut conv = PositionConverter::new();

    let result = match method {
        "initialize" => match lifecycle.handle_initialize(&msg) {
            Ok((capabilities, folders)) => {
                if !folders.is_empty() {
                    if index_path.is_none() {
                        multi_root.clear();
                    }
                    multi_root.open_from_folders(&folders);
                }
                if let Some(ref req_id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": capabilities
                    });
                    send_response(writer, &resp)?;
                }
                return Ok(true);
            }
            Err((code, message)) => {
                if let Some(ref req_id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": { "code": code, "message": message }
                    });
                    let _ = send_response(writer, &resp);
                }
                return Ok(true);
            }
        },

        "initialized" => return Ok(true),

        "shutdown" => {
            let _ = lifecycle.handle_shutdown();
            if let Some(ref req_id) = id {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": Value::Null
                });
                send_response(writer, &resp)?;
            }
            return Ok(true);
        }

        "textDocument/didOpen" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let text = msg["params"]["textDocument"]["text"].as_str().unwrap_or("");
            let version = msg["params"]["textDocument"]["version"]
                .as_i64()
                .unwrap_or(1) as i32;
            let file = uri_to_path(uri);
            let has_provenance = check_file_provenance(multi_root, &file, text);
            overlay_mgr.open_with_provenance(uri, &file, version, text.to_string(), has_provenance);
            return Ok(true);
        }

        "textDocument/didChange" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            let version = msg["params"]["textDocument"]["version"]
                .as_i64()
                .unwrap_or(1) as i32;
            if let Some(changes) = msg["params"]["contentChanges"].as_array() {
                if !overlay_mgr.apply_content_changes(&file, version, changes) {
                    let initial_text = std::fs::read_to_string(&file).unwrap_or_default();
                    let has_provenance = check_file_provenance(multi_root, &file, &initial_text);
                    overlay_mgr.open_with_provenance(
                        uri,
                        &file,
                        version,
                        initial_text,
                        has_provenance,
                    );
                    let _ = overlay_mgr.apply_content_changes(&file, version, changes);
                }
            }
            return Ok(true);
        }

        "textDocument/didClose" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            overlay_mgr.close_document(&file);
            return Ok(true);
        }

        "textDocument/didSave" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            overlay_mgr.save_document(&file);
            return Ok(true);
        }

        "workspace/didChangeWorkspaceFolders" => {
            multi_root.handle_did_change_workspace_folders(&msg);
            return Ok(true);
        }

        "workspace/didChangeWatchedFiles" => {
            multi_root.reload_all_if_modified();
            return Ok(true);
        }

        "workspace/didRenameFiles" => {
            if let Some(files) = msg["params"]["files"].as_array() {
                for op in files {
                    let old_uri = op["oldUri"].as_str().unwrap_or("");
                    let new_uri = op["newUri"].as_str().unwrap_or("");
                    overlay_mgr.rename(&uri_to_path(old_uri), new_uri, &uri_to_path(new_uri));
                }
            }
            sync_all_open_overlays(multi_root, overlay_mgr);
            return Ok(true);
        }

        "workspace/didDeleteFiles" => {
            if let Some(files) = msg["params"]["files"].as_array() {
                for op in files {
                    let uri = op["uri"].as_str().unwrap_or("");
                    overlay_mgr.close(&uri_to_path(uri));
                }
            }
            sync_all_open_overlays(multi_root, overlay_mgr);
            return Ok(true);
        }

        "textDocument/definition" => {
            let ws = doc_ws(&msg, multi_root);
            handle_definition(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/references" => {
            let ws = doc_ws(&msg, multi_root);
            handle_references(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/typeDefinition" => {
            let ws = doc_ws(&msg, multi_root);
            handle_type_definition(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/implementation" => {
            let ws = doc_ws(&msg, multi_root);
            handle_implementation(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/declaration" => {
            let ws = doc_ws(&msg, multi_root);
            handle_declaration(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/documentSymbol" => handle_document_symbol(&msg, overlay_mgr, &mut conv),
        "workspace/symbol" => handle_workspace_symbol(&msg, multi_root.all_workspaces(), &mut conv),
        "workspaceSymbol/resolve" | "workspace/symbol/resolve" => {
            handle_workspace_symbol_resolve(&msg, multi_root.all_workspaces(), &mut conv)
        }
        "textDocument/documentHighlight" => {
            let ws = doc_ws(&msg, multi_root);
            handle_document_highlight(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/hover" => {
            let ws = doc_ws(&msg, multi_root);
            handle_hover(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/completion" => {
            let ws = doc_ws(&msg, multi_root);
            handle_completion(&msg, ws, overlay_mgr, &mut conv)
        }
        "completionItem/resolve" => handle_completion_resolve(&msg),
        "textDocument/signatureHelp" => {
            let ws = doc_ws(&msg, multi_root);
            handle_signature_help(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/inlayHint" => {
            let ws = doc_ws(&msg, multi_root);
            handle_inlay_hint(&msg, ws, overlay_mgr)
        }
        "textDocument/inlineValue" => handle_inline_value(&msg),
        "textDocument/inlineCompletion" => handle_inline_completion(&msg),
        "textDocument/prepareCallHierarchy" => {
            let ws = doc_ws(&msg, multi_root);
            handle_prepare_call_hierarchy(&msg, ws, overlay_mgr, &mut conv)
        }
        "callHierarchy/incomingCalls" => {
            let file = uri_to_path(msg["params"]["item"]["uri"].as_str().unwrap_or(""));
            let ws = multi_root.find_workspace_for_path(Path::new(&file));
            handle_call_hierarchy_incoming_calls(&msg, ws, &mut conv)
        }
        "callHierarchy/outgoingCalls" => {
            let file = uri_to_path(msg["params"]["item"]["uri"].as_str().unwrap_or(""));
            let ws = multi_root.find_workspace_for_path(Path::new(&file));
            handle_call_hierarchy_outgoing_calls(&msg, ws, &mut conv)
        }
        "textDocument/prepareTypeHierarchy" => {
            let ws = doc_ws(&msg, multi_root);
            handle_prepare_type_hierarchy(&msg, ws, overlay_mgr, &mut conv)
        }
        "typeHierarchy/supertypes" => {
            handle_type_hierarchy_supertypes(&msg, multi_root.first_workspace())
        }
        "typeHierarchy/subtypes" => {
            handle_type_hierarchy_subtypes(&msg, multi_root.first_workspace())
        }
        "textDocument/selectionRange" => handle_selection_range(&msg, overlay_mgr),
        "textDocument/documentLink" => handle_document_link(&msg, overlay_mgr),
        "documentLink/resolve" => handle_document_link_resolve(&msg),
        "textDocument/documentColor" => handle_document_color(&msg),
        "textDocument/colorPresentation" => handle_color_presentation(&msg),
        "textDocument/semanticTokens/full" => handle_semantic_tokens_full(&msg, overlay_mgr),
        "textDocument/foldingRange" => handle_folding_range(&msg, overlay_mgr),
        "textDocument/codeLens" => handle_code_lens(&msg, overlay_mgr),
        "codeLens/resolve" => handle_code_lens_resolve(&msg),
        "textDocument/diagnostic" => handle_diagnostic(&msg, overlay_mgr),
        "textDocument/moniker" => {
            let ws = doc_ws(&msg, multi_root);
            handle_moniker(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/content" | "workspace/textDocumentContent" => {
            handle_text_document_content(&msg, overlay_mgr)
        }
        "textDocument/prepareRename" => {
            let ws = doc_ws(&msg, multi_root);
            handle_prepare_rename(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/rename" => {
            let ws = doc_ws(&msg, multi_root);
            handle_rename(&msg, ws, overlay_mgr, &mut conv)
        }
        "textDocument/linkedEditingRange" => {
            handle_linked_editing_range(&msg, overlay_mgr, &mut conv)
        }
        "textDocument/formatting" => handle_formatting(&msg, overlay_mgr),
        "textDocument/rangeFormatting" => handle_range_formatting(&msg, overlay_mgr),
        "textDocument/onTypeFormatting" => handle_on_type_formatting(&msg, overlay_mgr),
        "textDocument/codeAction" => handle_code_action(&msg, overlay_mgr),
        "codeAction/resolve" => handle_code_action_resolve(&msg),
        "workspace/executeCommand" => handle_execute_command(&msg, overlay_mgr),

        _ => {
            if let Some(ref req_id) = id {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": { "code": -32601, "message": format!("Method not found: {method}") }
                });
                send_response(writer, &resp)?;
            }
            return Ok(true);
        }
    };

    if let Some(ref req_id) = id {
        if cancelled_requests.lock().unwrap().remove(req_id) {
            let resp = json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "error": { "code": -32800, "message": "Request cancelled" }
            });
            send_response(writer, &resp)?;
        } else {
            let resp = json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": result
            });
            send_response(writer, &resp)?;
        }
    }

    Ok(true)
}

#[inline]
fn doc_ws<'a>(msg: &Value, multi_root: &'a MultiRootWorkspace) -> Option<&'a WorkspaceIndex> {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    multi_root.find_workspace_for_path(Path::new(&file))
}

fn sync_all_open_overlays(
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut crate::overlay::OverlayManager,
) {
    for ws in multi_root.all_workspaces_mut() {
        sync_reloaded_index(ws, overlay_mgr);
    }
}

fn check_lifecycle_and_cancel<W: Write>(
    method: &str,
    id: &Option<Value>,
    lifecycle: &mut LifecycleManager,
    cancelled_requests: &Arc<Mutex<HashSet<Value>>>,
    writer: &mut W,
) -> Result<Option<bool>, io::Error> {
    if method == "exit" {
        return if lifecycle.handle_exit().is_ok() {
            Ok(Some(false))
        } else {
            Err(io::Error::other(
                "exit notification received before shutdown",
            ))
        };
    }
    if lifecycle.is_shutdown() {
        if let Some(ref req_id) = id {
            let resp = json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32600, "message": "Server is shutting down" } });
            send_response(writer, &resp)?;
        }
        return Ok(Some(true));
    }
    if !lifecycle.is_initialized() && method != "initialize" {
        if let Some(ref req_id) = id {
            let resp = json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32002, "message": "Server not initialized" } });
            send_response(writer, &resp)?;
        }
        return Ok(Some(true));
    }
    if let Some(ref req_id) = id {
        if cancelled_requests.lock().unwrap().remove(req_id) {
            let resp = json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32800, "message": "Request cancelled" } });
            let _ = send_response(writer, &resp);
            return Ok(Some(true));
        }
    }
    Ok(None)
}

fn check_file_provenance(multi_root: &MultiRootWorkspace, file: &str, text: &str) -> bool {
    let text_hash = ridx_format::normalized_source_hash(text.as_bytes());
    multi_root
        .find_workspace_for_path(Path::new(file))
        .and_then(|ws| ws.source_hash_for_file(file))
        == Some(text_hash)
}
