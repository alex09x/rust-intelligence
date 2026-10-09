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
                super::presentation::reconcile::handle_internal_diagnostic(
                    &diag,
                    lifecycle,
                    overlay_mgr,
                    writer,
                )?;
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

    let result: super::error::LspResult<Value> = match method {
        "initialize" => match lifecycle.handle_initialize(&msg) {
            Ok((capabilities, folders)) => {
                if !folders.is_empty() {
                    if index_path.is_none() {
                        multi_root.clear();
                    }
                    multi_root.open_from_folders(&folders);
                }
                Ok(capabilities)
            }
            Err(err) => Err(err),
        },

        "initialized" => return Ok(true),

        "shutdown" => lifecycle.handle_shutdown(),

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
            if !lifecycle.client_pull_diagnostics() {
                publish_reconciled_diagnostics(
                    uri,
                    Some(version),
                    overlay_mgr,
                    &lifecycle.flycheck,
                    writer,
                )?;
            }
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
        "textDocument/willSaveWaitUntil" => Ok(json!([])),

        "workspace/didChangeConfiguration" | "$/setTrace" | "window/workDoneProgress/cancel" => {
            return Ok(true)
        }
        "workspace/configuration" => Ok(json!([])),

        "workspace/didChangeWorkspaceFolders" => {
            multi_root.handle_did_change_workspace_folders(&msg);
            return Ok(true);
        }

        "workspace/didChangeWatchedFiles" => {
            multi_root.reload_all_if_modified();
            conv.clear();
            return Ok(true);
        }

        "workspace/willCreateFiles" | "workspace/willRenameFiles" | "workspace/willDeleteFiles" => {
            Ok(json!({}))
        }

        "workspace/didCreateFiles" | "workspace/didRenameFiles" | "workspace/didDeleteFiles" => {
            multi_root.handle_file_event(method, &msg, overlay_mgr, conv);
            return Ok(true);
        }

        "textDocument/definition" => {
            Ok(handle_definition(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv))
        }
        "textDocument/references" => {
            Ok(handle_references(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv))
        }
        "textDocument/typeDefinition" => {
            Ok(handle_type_definition(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv))
        }
        "textDocument/implementation" => {
            Ok(handle_implementation(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv))
        }
        "textDocument/declaration" => {
            Ok(handle_declaration(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv))
        }
        "textDocument/documentSymbol" => Ok(handle_document_symbol(&msg, overlay_mgr, conv)),
        "workspace/symbol" => Ok(handle_workspace_symbol(&msg, multi_root.all_workspaces(), conv)),
        "workspaceSymbol/resolve" | "workspace/symbol/resolve" => {
            Ok(handle_workspace_symbol_resolve(&msg, multi_root.all_workspaces(), conv))
        }
        "textDocument/documentHighlight" => {
            Ok(handle_document_highlight(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv))
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
        "textDocument/inlineValue" => Ok(handle_inline_value(&msg, Some(overlay_mgr))),
        "textDocument/inlineCompletion" => Ok(handle_inline_completion(&msg, Some(overlay_mgr))),
        "textDocument/prepareCallHierarchy" => {
            Ok(handle_prepare_call_hierarchy(&msg, multi_root.doc_ws(&msg), overlay_mgr, conv))
        }
        "callHierarchy/incomingCalls" => {
            let f = uri_to_path(msg["params"]["item"]["uri"].as_str().unwrap_or(""));
            Ok(handle_call_hierarchy_incoming_calls(
                &msg,
                multi_root.find_workspace_for_path(Path::new(&f)),
                overlay_mgr,
                conv,
            ))
        }
        "callHierarchy/outgoingCalls" => {
            let f = uri_to_path(msg["params"]["item"]["uri"].as_str().unwrap_or(""));
            Ok(handle_call_hierarchy_outgoing_calls(
                &msg,
                multi_root.find_workspace_for_path(Path::new(&f)),
                overlay_mgr,
                conv,
            ))
        }
        "textDocument/prepareTypeHierarchy" => {
            let ws = multi_root.doc_ws(&msg);
            Ok(handle_prepare_type_hierarchy(&msg, ws, overlay_mgr, conv))
        }
        "typeHierarchy/supertypes" => {
            Ok(handle_type_hierarchy_supertypes(&msg, multi_root.first_workspace()))
        }
        "typeHierarchy/subtypes" => {
            Ok(handle_type_hierarchy_subtypes(&msg, multi_root.first_workspace()))
        }
        "textDocument/selectionRange" => Ok(handle_selection_range(&msg, overlay_mgr)),
        "textDocument/documentLink" => Ok(handle_document_link(&msg, overlay_mgr)),
        "documentLink/resolve" => Ok(handle_document_link_resolve(&msg)),
        "textDocument/documentColor" => Ok(handle_document_color(&msg)),
        "textDocument/colorPresentation" => Ok(handle_color_presentation(&msg)),
        "textDocument/semanticTokens/full" => {
            let ws = multi_root.doc_ws(&msg);
            Ok(handle_semantic_tokens_full(&msg, ws, overlay_mgr, conv))
        }
        "textDocument/semanticTokens/full/delta" => {
            let ws = multi_root.doc_ws(&msg);
            Ok(handle_semantic_tokens_delta(&msg, ws, overlay_mgr, conv))
        }
        "textDocument/semanticTokens/range" => {
            let ws = multi_root.doc_ws(&msg);
            Ok(handle_semantic_tokens_range(&msg, ws, overlay_mgr, conv))
        }
        "textDocument/foldingRange" => Ok(handle_folding_range(&msg, overlay_mgr)),
        "textDocument/codeLens" => {
            let ws = multi_root.doc_ws(&msg);
            Ok(handle_code_lens(&msg, ws, overlay_mgr))
        }
        "codeLens/resolve" => Ok(handle_code_lens_resolve(&msg)),
        "textDocument/diagnostic" => {
            Ok(handle_diagnostic_with_flycheck(&msg, overlay_mgr, Some(&lifecycle.flycheck)))
        }
        "workspace/diagnostic" => Ok(handle_workspace_diagnostic_with_flycheck(
            &msg,
            overlay_mgr,
            multi_root,
            Some(&lifecycle.flycheck),
        )),
        "textDocument/moniker" => {
            let ws = multi_root.doc_ws(&msg);
            Ok(handle_moniker(&msg, ws, overlay_mgr, conv))
        }
        "moniker/resolve" => Ok(handle_moniker_resolve(&msg)),
        "textDocument/content" | "workspace/textDocumentContent" => {
            Ok(handle_text_document_content(&msg, overlay_mgr))
        }
        "textDocument/prepareRename" => {
            let ws = multi_root.doc_ws(&msg);
            handle_prepare_rename(&msg, ws, overlay_mgr, conv)
        }
        "textDocument/rename" => {
            let ws = multi_root.doc_ws(&msg);
            handle_rename(&msg, ws, overlay_mgr, conv)
        }
        "textDocument/linkedEditingRange" => Ok(handle_linked_editing_range(&msg, overlay_mgr, conv)),
        "textDocument/formatting" => Ok(handle_formatting(&msg, overlay_mgr)),
        "textDocument/rangeFormatting" => Ok(handle_range_formatting(&msg, overlay_mgr)),
        "textDocument/onTypeFormatting" => Ok(handle_on_type_formatting(&msg, overlay_mgr)),
        "textDocument/codeAction" => {
            let ws = multi_root.doc_ws(&msg);
            handle_code_action(&msg, ws, overlay_mgr)
        }
        "codeAction/resolve" => {
            let uri = msg["params"]
                .get("data")
                .and_then(|d| d.get("uri"))
                .and_then(|u| u.as_str())
                .or_else(|| {
                    msg["params"]
                        .get("edit")
                        .and_then(|e| e.get("changes"))
                        .and_then(|c| c.as_object())
                        .and_then(|m| m.keys().next().map(|s| s.as_str()))
                });
            let ws = uri.and_then(|u| {
                let p = uri_to_path(u);
                multi_root.find_workspace_for_path(Path::new(&p))
            });
            handle_code_action_resolve(&msg, ws, overlay_mgr)
        }
        "workspace/executeCommand" => {
            if let Some(res) = try_dispatch_apply_edits_command(&msg, overlay_mgr, writer, &mut id)?
            {
                return Ok(res);
            }
            handle_execute_command(&msg, overlay_mgr)
        }

        _ => Err(super::error::LspError::method_not_found(format!(
            "Method not found: {method}"
        ))),
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
            match result {
                Ok(val) => {
                    send_response(
                        writer,
                        &json!({ "jsonrpc": "2.0", "id": req_id, "result": val }),
                    )?;
                }
                Err(err) => {
                    send_response(
                        writer,
                        &json!({ "jsonrpc": "2.0", "id": req_id, "error": err.to_json() }),
                    )?;
                }
            }
        }
    }

    Ok(true)
}
