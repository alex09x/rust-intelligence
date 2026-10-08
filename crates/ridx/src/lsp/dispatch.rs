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
use super::lifecycle::{LifecycleManager, ServerState};
use super::navigation::*;
use super::presentation::*;
use super::protocol::{send_response, uri_to_path};
use super::symbols::*;
use super::sync_reloaded_index;
use super::workspace::*;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub fn dispatch_lsp_message<W: Write>(
    lsp_msg: impl Into<super::protocol::LspMessage>,
    writer: &mut W,
    lifecycle: &mut LifecycleManager,
    multi_root: &mut MultiRootWorkspace,
    overlay_mgr: &mut crate::overlay::OverlayManager,
    cancelled_requests: &Arc<Mutex<HashSet<Value>>>,
    index_path: Option<&PathBuf>,
) -> io::Result<bool> {
    let msg = match lsp_msg.into() {
        super::protocol::LspMessage::InternalDiagnostic(gen, diag) => {
            if lifecycle.state() != ServerState::ShutDown
                && gen == lifecycle.flycheck.current_generation()
            {
                send_response(writer, &diag)?;
            }
            return Ok(true);
        }
        super::protocol::LspMessage::Client(m) => m,
    };

    if msg.get("method").is_none() {
        if let Some((Some(ref cmd_id), resp)) = handle_apply_edit_response(&msg) {
            send_response(
                writer,
                &json!({ "jsonrpc": "2.0", "id": cmd_id, "result": resp }),
            )?;
        }
        return Ok(true);
    }

    let method = msg["method"].as_str().unwrap_or("");
    let mut id = msg.get("id").cloned();

    if let Some(should_continue) =
        lifecycle.check_lifecycle_and_cancel(method, &id, cancelled_requests, writer)?
    {
        return Ok(should_continue);
    }

    // 5. Automatic index reload: fast stat check (sub-microsecond) on manifest
    let mut any_reloaded = false;
    for ws in multi_root.all_workspaces_mut() {
        if sync_reloaded_index(ws, overlay_mgr) {
            any_reloaded = true;
        }
    }
    if any_reloaded {
        lifecycle.conv.clear();
    }

    let conv = &mut lifecycle.conv;

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
                    send_response(
                        writer,
                        &json!({ "jsonrpc": "2.0", "id": req_id, "result": capabilities }),
                    )?;
                }
                return Ok(true);
            }
            Err((code, message)) => {
                if let Some(ref req_id) = id {
                    let _ = send_response(
                        writer,
                        &json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": code, "message": message } }),
                    );
                }
                return Ok(true);
            }
        },

        "initialized" => return Ok(true),

        "shutdown" => {
            let _ = lifecycle.handle_shutdown();
            if let Some(ref req_id) = id {
                send_response(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": req_id, "result": Value::Null }),
                )?;
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
            let has_provenance = multi_root.check_file_provenance(&file, text);
            overlay_mgr.open_with_provenance(uri, &file, version, text.to_string(), has_provenance);
            conv.invalidate(&file);
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
                    let has_provenance = multi_root.check_file_provenance(&file, &initial_text);
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
            conv.invalidate(&file);
            return Ok(true);
        }

        "textDocument/didClose" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            overlay_mgr.close_document(&file);
            conv.invalidate(&file);
            return Ok(true);
        }

        "textDocument/didSave" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            overlay_mgr.save_document(&file);
            conv.invalidate(&file);
            let p = Path::new(&file);
            let ws_root = multi_root
                .find_workspace_for_path(p)
                .and_then(|ws| ws.workspace_root().map(Path::to_path_buf))
                .or_else(|| {
                    p.ancestors()
                        .find(|d| d.join("Cargo.toml").is_file())
                        .map(Path::to_path_buf)
                });
            if let Some(ref root) = ws_root {
                lifecycle.flycheck.trigger_check(root);
            }
            return Ok(true);
        }

        "textDocument/publishDiagnostics" | "textDocument/willSave" => return Ok(true),
        "textDocument/willSaveWaitUntil" => json!([]),

        "workspace/didChangeConfiguration" | "$/setTrace" | "window/workDoneProgress/cancel" => {
            return Ok(true)
        }
        "workspace/configuration" => json!([]),

        "workspace/didChangeWorkspaceFolders" => {
            multi_root.handle_did_change_workspace_folders(&msg);
            return Ok(true);
        }

        "workspace/didChangeWatchedFiles" => {
            multi_root.reload_all_if_modified();
            conv.clear();
            return Ok(true);
        }

        "workspace/didCreateFiles" => {
            for ws in multi_root.all_workspaces_mut() {
                sync_reloaded_index(ws, overlay_mgr);
            }
            conv.clear();
            return Ok(true);
        }

        "workspace/willCreateFiles" | "workspace/willRenameFiles" | "workspace/willDeleteFiles" => {
            json!({})
        }

        "workspace/didRenameFiles" => {
            if let Some(files) = msg["params"]["files"].as_array() {
                for op in files {
                    let old_uri = op["oldUri"].as_str().unwrap_or("");
                    let new_uri = op["newUri"].as_str().unwrap_or("");
                    overlay_mgr.rename(&uri_to_path(old_uri), new_uri, &uri_to_path(new_uri));
                }
            }
            for ws in multi_root.all_workspaces_mut() {
                sync_reloaded_index(ws, overlay_mgr);
            }
            return Ok(true);
        }

        "workspace/didDeleteFiles" => {
            if let Some(files) = msg["params"]["files"].as_array() {
                for op in files {
                    let uri = op["uri"].as_str().unwrap_or("");
                    overlay_mgr.close(&uri_to_path(uri));
                }
            }
            for ws in multi_root.all_workspaces_mut() {
                sync_reloaded_index(ws, overlay_mgr);
            }
            return Ok(true);
        }

        "textDocument/definition" => {
            handle_definition(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "textDocument/references" => {
            handle_references(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "textDocument/typeDefinition" => {
            handle_type_definition(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "textDocument/implementation" => {
            handle_implementation(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "textDocument/declaration" => {
            handle_declaration(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "textDocument/documentSymbol" => handle_document_symbol(&msg, overlay_mgr, conv),
        "workspace/symbol" => handle_workspace_symbol(&msg, multi_root.all_workspaces(), conv),
        "workspaceSymbol/resolve" | "workspace/symbol/resolve" => {
            handle_workspace_symbol_resolve(&msg, multi_root.all_workspaces(), conv)
        }
        "textDocument/documentHighlight" => {
            handle_document_highlight(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "textDocument/hover" => handle_hover(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv),
        "textDocument/completion" => {
            handle_completion(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "completionItem/resolve" => handle_completion_resolve(&msg),
        "textDocument/signatureHelp" => {
            handle_signature_help(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "textDocument/inlayHint" => handle_inlay_hint(&msg, multi_root.doc_ws(&msg), overlay_mgr),
        "inlayHint/resolve" => handle_inlay_hint_resolve(&msg),
        "textDocument/inlineValue" => handle_inline_value(&msg, Some(overlay_mgr)),
        "textDocument/inlineCompletion" => handle_inline_completion(&msg, Some(overlay_mgr)),
        "textDocument/prepareCallHierarchy" => {
            handle_prepare_call_hierarchy(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv)
        }
        "callHierarchy/incomingCalls" => {
            let f = uri_to_path(msg["params"]["item"]["uri"].as_str().unwrap_or(""));
            handle_call_hierarchy_incoming_calls(
                &msg,
                multi_root.find_workspace_for_path(Path::new(&f)),
                conv,
            )
        }
        "callHierarchy/outgoingCalls" => {
            let f = uri_to_path(msg["params"]["item"]["uri"].as_str().unwrap_or(""));
            handle_call_hierarchy_outgoing_calls(
                &msg,
                multi_root.find_workspace_for_path(Path::new(&f)),
                conv,
            )
        }
        "textDocument/prepareTypeHierarchy" => {
            let ws = multi_root.doc_ws(&msg);
            handle_prepare_type_hierarchy(&msg, ws, overlay_mgr, conv)
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
        "textDocument/semanticTokens/full" | "textDocument/semanticTokens/full/delta" => {
            handle_semantic_tokens_full(&msg, overlay_mgr)
        }
        "textDocument/semanticTokens/range" => handle_semantic_tokens_range(&msg, overlay_mgr),
        "textDocument/foldingRange" => handle_folding_range(&msg, overlay_mgr),
        "textDocument/codeLens" => handle_code_lens(&msg, overlay_mgr),
        "codeLens/resolve" => handle_code_lens_resolve(&msg),
        "textDocument/diagnostic" => handle_diagnostic(&msg, overlay_mgr),
        "workspace/diagnostic" => handle_workspace_diagnostic(&msg, overlay_mgr, multi_root),
        "textDocument/moniker" => {
            let ws = multi_root.doc_ws(&msg);
            handle_moniker(&msg, ws, overlay_mgr, conv)
        }
        "moniker/resolve" => handle_moniker_resolve(&msg),
        "textDocument/content" | "workspace/textDocumentContent" => {
            handle_text_document_content(&msg, overlay_mgr)
        }
        "textDocument/prepareRename" => {
            let ws = multi_root.doc_ws(&msg);
            handle_prepare_rename(&msg, ws, overlay_mgr, conv)
        }
        "textDocument/rename" => {
            let ws = multi_root.doc_ws(&msg);
            handle_rename(&msg, ws, overlay_mgr, conv)
        }
        "textDocument/linkedEditingRange" => handle_linked_editing_range(&msg, overlay_mgr, conv),
        "textDocument/formatting" => handle_formatting(&msg, overlay_mgr),
        "textDocument/rangeFormatting" => handle_range_formatting(&msg, overlay_mgr),
        "textDocument/onTypeFormatting" => handle_on_type_formatting(&msg, overlay_mgr),
        "textDocument/codeAction" => handle_code_action(&msg, overlay_mgr),
        "codeAction/resolve" => handle_code_action_resolve(&msg),
        "workspace/executeCommand" => {
            if let Some(res) = try_dispatch_apply_edits_command(&msg, overlay_mgr, writer, &mut id)?
            {
                return Ok(res);
            }
            handle_execute_command(&msg, overlay_mgr)
        }

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
            send_response(
                writer,
                &json!({ "jsonrpc": "2.0", "id": req_id, "result": result }),
            )?;
        }
    }

    Ok(true)
}
