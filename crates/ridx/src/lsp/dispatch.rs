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

use super::{
    editing::*,
    hierarchy::*,
    intelligence::*,
    lifecycle::{LifecycleManager, ServerState},
    navigation::*,
    presentation::*,
    protocol::{send_response, uri_to_path},
    symbols::*,
    sync_reloaded_index,
    workspace::*,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub fn dispatch_lsp_message<W: Write>(
    lsp_msg: impl Into<super::protocol::LspMessage>,
    writer: &mut W,
    lifecycle: &mut LifecycleManager,
    mr: &mut MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    cancelled_requests: &Arc<Mutex<HashSet<Value>>>,
    index_path: Option<&PathBuf>,
) -> io::Result<bool> {
    let msg = match lsp_msg.into() {
        super::protocol::LspMessage::InternalDiagnostic(gen, diag) => {
            if lifecycle.state() != ServerState::ShutDown && gen == lifecycle.flycheck.current_generation() {
                super::presentation::reconcile::handle_internal_diagnostic(&diag, lifecycle, ov, writer)?;
            }
            return Ok(true);
        }
        super::protocol::LspMessage::Client(m) => m,
    };

    if msg.get("method").is_none() {
        if let Some((Some(ref cmd_id), resp)) = lifecycle.handle_apply_edit_response(&msg) {
            send_response(writer, &json!({ "jsonrpc": "2.0", "id": cmd_id, "result": resp }))?;
        }
        return Ok(true);
    }

    let method = msg["method"].as_str().unwrap_or("");
    let mut id = msg.get("id").cloned();

    if let Some(should_continue) = lifecycle.check_lifecycle_and_cancel(method, &id, cancelled_requests, writer)? {
        return Ok(should_continue);
    }

    // 5. Automatic index reload: fast stat check (sub-microsecond) on manifest
    let mut any_reloaded = false;
    for ws in mr.all_workspaces_mut() {
        if sync_reloaded_index(ws, ov) {
            any_reloaded = true;
        }
    }
    if any_reloaded {
        lifecycle.conv.clear();
        super::hierarchy::clear_impl_table_cache();
        super::navigation::clear_live_db_cache();
    }

    let ra_commands = lifecycle.ra_client_commands();
    let ra_show_references = ra_commands.show_references;
    let conv = &mut lifecycle.conv;

    let result: super::error::LspResult<Value> = match method {
        "initialize" => match lifecycle.handle_initialize(&msg) {
            Ok((capabilities, folders)) => {
                if !folders.is_empty() {
                    if index_path.is_none() {
                        mr.clear();
                    }
                    mr.open_from_folders(&folders);
                } else if index_path.is_none() && mr.is_empty() {
                    if let Ok(cur) = std::env::current_dir() {
                        mr.add_root(cur);
                    }
                }
                let shard_count: usize = mr.all_workspaces_mut().map(|ws| ws.readers().len()).sum();
                super::logger::log_info(&format!("initialized LSP workspace (roots={}, shards={shard_count})", folders.len()));
                Ok(capabilities)
            }
            Err(err) => Err(err),
        },

        "initialized" => {
            mr.warm_background_caches();
            return Ok(true);
        }

        "shutdown" => {
            super::logger::log_info("LSP shutdown requested");
            lifecycle.handle_shutdown()
        }

        "textDocument/didOpen" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let text = msg["params"]["textDocument"]["text"].as_str().unwrap_or("");
            let version = msg["params"]["textDocument"]["version"].as_i64().unwrap_or(1) as i32;
            let file = uri_to_path(uri);
            let expected_hash = mr.source_hash_for_file(&file);
            ov.open_with_source_hash(uri, &file, version, text.to_string(), expected_hash);
            conv.invalidate(&file);
            return Ok(true);
        }

        "textDocument/didChange" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            let version = msg["params"]["textDocument"]["version"].as_i64().unwrap_or(1) as i32;
            if let Some(changes) = msg["params"]["contentChanges"].as_array() {
                if !ov.apply_content_changes(&file, version, changes) {
                    let initial_text = std::fs::read_to_string(&file).unwrap_or_default();
                    let has_provenance = mr.check_file_provenance(&file, &initial_text);
                    ov.open_with_provenance(uri, &file, version, initial_text, has_provenance);
                    let _ = ov.apply_content_changes(&file, version, changes);
                }
            }
            conv.invalidate(&file);
            if !lifecycle.client_pull_diagnostics() {
                publish_reconciled_diagnostics(uri, Some(version), ov, &lifecycle.flycheck, writer)?;
            }
            return Ok(true);
        }

        "textDocument/didClose" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            ov.close_document(&file);
            conv.invalidate(&file);
            return Ok(true);
        }

        "textDocument/didSave" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            ov.save_document(&file);
            conv.invalidate(&file);
            let p = Path::new(&file);
            let ws_root = mr
                .find_workspace_for_path(p)
                .and_then(|ws| ws.workspace_root().map(Path::to_path_buf))
                .or_else(|| p.ancestors().find(|d| d.join("Cargo.toml").is_file()).map(Path::to_path_buf));
            if let Some(ref root) = ws_root {
                lifecycle.flycheck.trigger_check(root);
            }
            return Ok(true);
        }

        "textDocument/publishDiagnostics" | "textDocument/willSave" => return Ok(true),
        "textDocument/willSaveWaitUntil" => Ok(json!([])),

        "$/cancelRequest" => {
            if let Some(cancel_id) = msg.get("params").and_then(|p| p.get("id")) {
                cancelled_requests.lock().unwrap_or_else(|e| e.into_inner()).insert(cancel_id.clone());
            }
            return Ok(true);
        }

        "workspace/didChangeConfiguration" | "$/setTrace" | "window/workDoneProgress/cancel" => return Ok(true),
        "workspace/configuration" => Ok(json!([])),

        "workspace/didChangeWorkspaceFolders" => {
            mr.handle_did_change_workspace_folders(&msg);
            return Ok(true);
        }

        "workspace/didChangeWatchedFiles" => {
            mr.reload_all_if_modified();
            conv.clear();
            return Ok(true);
        }

        "workspace/willCreateFiles" | "workspace/willRenameFiles" | "workspace/willDeleteFiles" => Ok(json!({})),

        "workspace/didCreateFiles" | "workspace/didRenameFiles" | "workspace/didDeleteFiles" => {
            mr.handle_file_event(method, &msg, ov, conv);
            return Ok(true);
        }

        "textDocument/definition" => Ok(handle_definition(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/references" => Ok(handle_references(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/typeDefinition" => Ok(handle_type_definition(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/implementation" => Ok(handle_implementation(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/declaration" => Ok(handle_declaration(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/documentSymbol" => Ok(handle_document_symbol(&msg, ov, conv)),
        "workspace/symbol" => Ok(handle_workspace_symbol(&msg, mr.all_workspaces(), conv)),
        "workspaceSymbol/resolve" | "workspace/symbol/resolve" => Ok(handle_workspace_symbol_resolve(&msg, mr.all_workspaces(), conv)),
        "textDocument/documentHighlight" => Ok(handle_document_highlight(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/hover" => handle_hover(&msg, mr.doc_ws(&msg), ov, conv),
        "textDocument/completion" => handle_completion(&msg, mr.doc_ws(&msg), ov, conv),
        "completionItem/resolve" => handle_completion_resolve(&msg),
        "textDocument/signatureHelp" => handle_signature_help(&msg, mr.doc_ws(&msg), ov, conv),
        "textDocument/inlayHint" => handle_inlay_hint(&msg, mr.doc_ws(&msg), ov),
        "inlayHint/resolve" => handle_inlay_hint_resolve(&msg),
        "textDocument/inlineValue" => Ok(handle_inline_value(&msg, Some(ov))),
        "textDocument/inlineCompletion" => Ok(handle_inline_completion(&msg, Some(ov))),
        "textDocument/prepareCallHierarchy" => handle_prepare_call_hierarchy(&msg, mr.doc_ws(&msg), ov, conv),
        "callHierarchy/incomingCalls" => {
            let f = uri_to_path(msg["params"]["item"]["uri"].as_str().unwrap_or(""));
            Ok(handle_call_hierarchy_incoming_calls(&msg, mr.find_workspace_for_path(Path::new(&f)), ov, conv))
        }
        "callHierarchy/outgoingCalls" => {
            let f = uri_to_path(msg["params"]["item"]["uri"].as_str().unwrap_or(""));
            Ok(handle_call_hierarchy_outgoing_calls(&msg, mr.find_workspace_for_path(Path::new(&f)), ov, conv))
        }
        "textDocument/prepareTypeHierarchy" => handle_prepare_type_hierarchy(&msg, mr.doc_ws(&msg), ov, conv),
        "typeHierarchy/supertypes" => Ok(handle_type_hierarchy_supertypes(&msg, mr.first_workspace())),
        "typeHierarchy/subtypes" => Ok(handle_type_hierarchy_subtypes(&msg, mr.first_workspace())),
        "textDocument/selectionRange" => Ok(handle_selection_range(&msg, ov)),
        "textDocument/documentLink" => Ok(handle_document_link(&msg, ov)),
        "documentLink/resolve" => Ok(handle_document_link_resolve(&msg)),
        "textDocument/documentColor" => Ok(handle_document_color(&msg)),
        "textDocument/colorPresentation" => Ok(handle_color_presentation(&msg)),
        "textDocument/semanticTokens/full" => Ok(handle_semantic_tokens_full(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/semanticTokens/full/delta" => Ok(handle_semantic_tokens_delta(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/semanticTokens/range" => Ok(handle_semantic_tokens_range(&msg, mr.doc_ws(&msg), ov, conv)),
        "textDocument/foldingRange" => Ok(handle_folding_range(&msg, ov)),
        "textDocument/codeLens" => Ok(handle_code_lens_for_client(&msg, mr.doc_ws(&msg), ov, conv, ra_commands)),
        "codeLens/resolve" => {
            let uri = msg["params"]["data"]["uri"].as_str().or_else(|| msg["params"]["textDocument"]["uri"].as_str());
            let ws = uri.and_then(|u| mr.find_workspace_for_path(Path::new(&uri_to_path(u))));
            Ok(handle_code_lens_resolve_for_client(&msg, ws, ov, conv, ra_show_references))
        }
        "textDocument/diagnostic" => Ok(handle_diagnostic_with_flycheck(&msg, ov, Some(&lifecycle.flycheck))),
        "workspace/diagnostic" => Ok(handle_workspace_diagnostic_with_flycheck(&msg, ov, mr, Some(&lifecycle.flycheck))),
        "textDocument/moniker" => Ok(handle_moniker(&msg, mr.doc_ws(&msg), ov, conv)),
        "moniker/resolve" => Ok(handle_moniker_resolve(&msg)),
        "textDocument/content" | "workspace/textDocumentContent" => Ok(handle_text_document_content(&msg, ov)),
        "textDocument/prepareRename" => handle_prepare_rename(&msg, mr.doc_ws(&msg), ov, conv),
        "textDocument/rename" => handle_rename(&msg, mr.doc_ws(&msg), ov, conv),
        "textDocument/linkedEditingRange" => Ok(handle_linked_editing_range(&msg, ov, conv)),
        "textDocument/formatting" => Ok(handle_formatting(&msg, ov)),
        "textDocument/rangeFormatting" => Ok(handle_range_formatting(&msg, ov)),
        "textDocument/onTypeFormatting" => Ok(handle_on_type_formatting(&msg, ov)),
        "textDocument/codeAction" => handle_code_action_with_options(&msg, mr.doc_ws(&msg), ov, lifecycle.client_resolve_code_action_edit()),
        "codeAction/resolve" => handle_code_action_resolve(&msg, mr.resolve_code_action_ws(&msg), ov),
        "workspace/executeCommand" => {
            if let Some(res) = try_dispatch_apply_edits_command(&msg, ov, lifecycle, writer, &mut id)? {
                return Ok(res);
            }
            handle_execute_command(&msg, ov)
        }

        _ => Err(super::error::LspError::method_not_found(format!("Method not found: {method}"))),
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
                    super::logger::log_verbose(&format!("request {method} (id={req_id}) -> ok"));
                    send_response(writer, &json!({ "jsonrpc": "2.0", "id": req_id, "result": val }))?;
                }
                Err(err) => {
                    super::logger::log_error(&format!("request {method} (id={req_id}) -> error {}: {}", err.code, err.message));
                    send_response(writer, &json!({ "jsonrpc": "2.0", "id": req_id, "error": err.to_json() }))?;
                }
            }
        }
    }

    Ok(true)
}
