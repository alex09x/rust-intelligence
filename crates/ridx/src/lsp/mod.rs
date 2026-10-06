/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lightweight, zero-memory Language Server Protocol (LSP) bridge for Rust Intelligence.
//!
//! Provides ultra-low latency (<50 µs) code intelligence over stdio without running
//! a heavy background daemon or consuming persistent background RAM.

pub mod editing;
pub mod handlers;
pub mod hierarchy;
pub mod intelligence;
pub mod lifecycle;
pub mod navigation;
pub mod presentation;
pub mod protocol;
pub mod symbols;
pub mod workspace;

#[cfg(test)]
pub mod tests;

#[allow(unused_imports)]
pub use {
    editing::*, handlers::*, hierarchy::*, intelligence::*, lifecycle::*, navigation::*,
    presentation::*, protocol::*, symbols::*, workspace::*,
};

use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(crate) fn sync_reloaded_index(
    ws: &mut WorkspaceIndex,
    overlay_mgr: &mut crate::overlay::OverlayManager,
) {
    if let Ok(true) = ws.reload_if_modified() {
        for file in overlay_mgr.open_file_paths() {
            if ws.is_file_reloaded(&file) {
                if let Ok(content) = std::fs::read_to_string(&file) {
                    let disk_hash = ridx_format::normalized_source_hash(content.as_bytes());
                    if ws.source_hash_for_file(&file) == Some(disk_hash) {
                        overlay_mgr.advance_compiled_baseline(&file, content);
                    }
                }
            }
        }
    }
}

/// Runs the LSP server over standard I/O.
pub fn run_lsp_server(index_path: Option<PathBuf>) -> io::Result<()> {
    #[cfg(test)]
    {
        let _ = index_path;
        Ok(())
    }
    #[cfg(not(test))]
    {
        let reader = io::BufReader::new(io::stdin());
        let stdout = io::stdout();
        run_lsp_server_stream(reader, stdout.lock(), index_path)
    }
}

/// Runs the LSP server over arbitrary reader and writer streams.
pub fn run_lsp_server_stream<R: BufRead + Send, W: Write>(
    mut reader: R,
    mut writer: W,
    index_path: Option<PathBuf>,
) -> io::Result<()> {
    let mut multi_root = MultiRootWorkspace::new();
    if let Some(ref p) = index_path {
        multi_root.add_root(p.clone());
    } else if let Ok(auto_ws) = WorkspaceIndex::auto_discover() {
        // Find enclosing workspace dir or path
        let cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        multi_root.add_root(cur);
        let _ = auto_ws;
    }

    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let mut lifecycle = LifecycleManager::new();
    let cancelled_requests: Arc<Mutex<HashSet<Value>>> = Arc::new(Mutex::new(HashSet::new()));

    // Separate message ingestion from request execution so cancellation notices
    // are received concurrently while work or requests are pending or in flight.
    std::thread::scope(|s| -> io::Result<()> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Value>(64);
        let cancelled_reader = Arc::clone(&cancelled_requests);

        s.spawn(move || {
            loop {
                let mut content_length: Option<usize> = None;
                let mut header_line = String::new();

                loop {
                    header_line.clear();
                    let bytes_read = match reader.read_line(&mut header_line) {
                        Ok(b) => b,
                        Err(_) => return,
                    };
                    if bytes_read == 0 {
                        return; // EOF
                    }
                    let trimmed = header_line.trim_end_matches(['\r', '\n']);
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some(len_str) = trimmed.strip_prefix("Content-Length:") {
                        content_length = len_str.trim().parse::<usize>().ok();
                    }
                }

                let Some(len) = content_length else {
                    continue;
                };

                let mut body_buf = vec![0u8; len];
                if reader.read_exact(&mut body_buf).is_err() {
                    return;
                }

                if let Ok(msg) = serde_json::from_slice::<Value>(&body_buf) {
                    if msg.get("method").and_then(|m| m.as_str()) == Some("$/cancelRequest") {
                        if let Some(id) = msg.get("params").and_then(|p| p.get("id")).cloned() {
                            cancelled_reader.lock().unwrap().insert(id);
                        }
                    }
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
            }
        });

        for msg in rx {
            let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
            let id = msg.get("id").cloned();

            // 1. LSP Lifecycle: handle exit notification
            if method == "exit" {
                if lifecycle.handle_exit().is_ok() {
                    return Ok(());
                } else {
                    return Err(io::Error::other(
                        "exit notification received before shutdown",
                    ));
                }
            }

            // 2. Reject requests after shutdown
            if lifecycle.is_shutdown() {
                if let Some(ref req_id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {
                            "code": -32600,
                            "message": "Server is shutting down"
                        }
                    });
                    send_response(&mut writer, &resp)?;
                }
                continue;
            }

            // 3. Enforce initialization state: reject requests before initialize with ServerNotInitialized (-32002)
            if !lifecycle.is_initialized() && method != "initialize" {
                if let Some(ref req_id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {
                            "code": -32002,
                            "message": "Server not initialized"
                        }
                    });
                    send_response(&mut writer, &resp)?;
                }
                continue;
            }

            // 4. Handle request cancellation before dispatch
            if let Some(ref req_id) = id {
                if cancelled_requests.lock().unwrap().remove(req_id) {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {
                            "code": -32800,
                            "message": "Request cancelled"
                        }
                    });
                    let _ = send_response(&mut writer, &resp);
                    continue;
                }
            }

            // 5. Automatically reload indices if files changed on disk and sync open overlay baselines
            multi_root.reload_all_if_modified();
            for ws in multi_root.all_workspaces_mut() {
                sync_reloaded_index(ws, &mut overlay_mgr);
            }

            let mut conv = PositionConverter::new();

            match method {
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
                            send_response(&mut writer, &resp)?;
                        }
                    }
                    Err((code, message)) => {
                        if let Some(ref req_id) = id {
                            let resp = json!({
                                "jsonrpc": "2.0",
                                "id": req_id,
                                "error": { "code": code, "message": message }
                            });
                            let _ = send_response(&mut writer, &resp);
                        }
                    }
                },

                "initialized" => {
                    // Fully initialized notification
                }

                "textDocument/didOpen" => {
                    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                    let text = msg["params"]["textDocument"]["text"].as_str().unwrap_or("");
                    let version = msg["params"]["textDocument"]["version"]
                        .as_i64()
                        .unwrap_or(1) as i32;
                    let file = uri_to_path(uri);
                    overlay_mgr.open_with_provenance(uri, &file, version, text.to_string(), true);
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
                            overlay_mgr.open_with_provenance(
                                uri,
                                &file,
                                version,
                                initial_text,
                                false,
                            );
                            overlay_mgr.apply_content_changes(&file, version, changes);
                        }
                    }
                }

                "textDocument/didSave" => {
                    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                    let file = uri_to_path(uri);
                    if let Ok(content) = std::fs::read(&file) {
                        let hash = ridx_format::normalized_source_hash(&content);
                        overlay_mgr.reconcile_with_disk(&file, hash);
                    }
                    overlay_mgr.save_document(&file);
                }

                "textDocument/didClose" => {
                    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                    let file = uri_to_path(uri);
                    overlay_mgr.close(&file);
                }

                "workspace/didChangeWorkspaceFolders" => {
                    multi_root.handle_did_change_workspace_folders(&msg);
                }

                "workspace/didChangeWatchedFiles" => {
                    multi_root.reload_all_if_modified();
                }

                "workspace/didRenameFiles" => {
                    if let Some(files) = msg["params"]["files"].as_array() {
                        for op in files {
                            let old_uri = op["oldUri"].as_str().unwrap_or("");
                            let new_uri = op["newUri"].as_str().unwrap_or("");
                            let old_path = uri_to_path(old_uri);
                            let new_path = uri_to_path(new_uri);
                            overlay_mgr.rename(&old_path, new_uri, &new_path);
                        }
                    }
                    for ws in multi_root.all_workspaces_mut() {
                        sync_reloaded_index(ws, &mut overlay_mgr);
                    }
                }

                "workspace/didDeleteFiles" => {
                    if let Some(files) = msg["params"]["files"].as_array() {
                        for op in files {
                            let uri = op["uri"].as_str().unwrap_or("");
                            let path = uri_to_path(uri);
                            overlay_mgr.close(&path);
                        }
                    }
                    for ws in multi_root.all_workspaces_mut() {
                        sync_reloaded_index(ws, &mut overlay_mgr);
                    }
                }

                "textDocument/definition" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_definition(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/declaration" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_declaration(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/typeDefinition" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_type_definition(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/implementation" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_implementation(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/references" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_references(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/documentHighlight" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_document_highlight(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/documentSymbol" => {
                    if let Some(ref req_id) = id {
                        let result = handle_document_symbol(&msg, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "workspace/symbol" => {
                    if let Some(ref req_id) = id {
                        let result =
                            handle_workspace_symbol(&msg, multi_root.all_workspaces(), &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "workspaceSymbol/resolve" | "workspace/symbol/resolve" => {
                    if let Some(ref req_id) = id {
                        let result = handle_workspace_symbol_resolve(
                            &msg,
                            multi_root.all_workspaces(),
                            &mut conv,
                        );
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/hover" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_hover(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/completion" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_completion(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "completionItem/resolve" => {
                    if let Some(ref req_id) = id {
                        let result = handle_completion_resolve(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/signatureHelp" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_signature_help(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/formatting" => {
                    if let Some(ref req_id) = id {
                        let result = handle_formatting(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/prepareRename" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_prepare_rename(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/rename" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result = handle_rename(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/prepareCallHierarchy" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result =
                            handle_prepare_call_hierarchy(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "callHierarchy/incomingCalls" => {
                    if let Some(ref req_id) = id {
                        let ws = multi_root.first_workspace();
                        let result = handle_call_hierarchy_incoming_calls(&msg, ws, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "callHierarchy/outgoingCalls" => {
                    if let Some(ref req_id) = id {
                        let ws = multi_root.first_workspace();
                        let result = handle_call_hierarchy_outgoing_calls(&msg, ws);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/prepareTypeHierarchy" => {
                    if let Some(ref req_id) = id {
                        let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                        let file = uri_to_path(uri);
                        let ws = multi_root.find_workspace_for_path(Path::new(&file));
                        let result =
                            handle_prepare_type_hierarchy(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "typeHierarchy/supertypes" => {
                    if let Some(ref req_id) = id {
                        let ws = multi_root.first_workspace();
                        let result = handle_type_hierarchy_supertypes(&msg, ws);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "typeHierarchy/subtypes" => {
                    if let Some(ref req_id) = id {
                        let ws = multi_root.first_workspace();
                        let result = handle_type_hierarchy_subtypes(&msg, ws);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/selectionRange" => {
                    if let Some(ref req_id) = id {
                        let result = handle_selection_range(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/documentLink" => {
                    if let Some(ref req_id) = id {
                        let result = handle_document_link(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "documentLink/resolve" => {
                    if let Some(ref req_id) = id {
                        let result = handle_document_link_resolve(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/semanticTokens/full" => {
                    if let Some(ref req_id) = id {
                        let result = handle_semantic_tokens_full(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/foldingRange" => {
                    if let Some(ref req_id) = id {
                        let result = handle_folding_range(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/codeAction" => {
                    if let Some(ref req_id) = id {
                        let result = handle_code_action(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "codeAction/resolve" => {
                    if let Some(ref req_id) = id {
                        let result = handle_code_action_resolve(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/codeLens" => {
                    if let Some(ref req_id) = id {
                        let result = handle_code_lens(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "codeLens/resolve" => {
                    if let Some(ref req_id) = id {
                        let result = handle_code_lens_resolve(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/inlayHint" => {
                    if let Some(ref req_id) = id {
                        let ws = multi_root.first_workspace();
                        let result = handle_inlay_hint(&msg, ws, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/diagnostic" => {
                    if let Some(ref req_id) = id {
                        let result = handle_diagnostic(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/moniker" => {
                    if let Some(ref req_id) = id {
                        let ws = multi_root.first_workspace();
                        let result = handle_moniker(&msg, ws, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "workspace/executeCommand" => {
                    if let Some(ref req_id) = id {
                        let result = handle_execute_command(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/documentColor" => {
                    if let Some(ref req_id) = id {
                        let result = handle_document_color(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/colorPresentation" => {
                    if let Some(ref req_id) = id {
                        let result = handle_color_presentation(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/linkedEditingRange" => {
                    if let Some(ref req_id) = id {
                        let result = handle_linked_editing_range(&msg, &overlay_mgr, &mut conv);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/inlineValue" => {
                    if let Some(ref req_id) = id {
                        let result = handle_inline_value(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "textDocument/inlineCompletion" => {
                    if let Some(ref req_id) = id {
                        let result = handle_inline_completion(&msg);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "workspace/textDocumentContent" => {
                    if let Some(ref req_id) = id {
                        let result = handle_text_document_content(&msg, &overlay_mgr);
                        let resp = json!({ "jsonrpc": "2.0", "id": req_id, "result": result });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                "shutdown" => {
                    let _ = lifecycle.handle_shutdown();
                    if let Some(ref req_id) = id {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": Value::Null
                        });
                        let _ = send_response(&mut writer, &resp);
                    }
                }

                _ => {
                    // Unknown method: ignore notifications, error for requests
                    if let Some(ref req_id) = id {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "error": {
                                "code": -32601,
                                "message": format!("Method not found: {method}")
                            }
                        });
                        let _ = send_response(&mut writer, &resp);
                    }
                }
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod lsp_stream_tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn format_lsp_msg(val: &serde_json::Value) -> String {
        let body = serde_json::to_string(val).unwrap();
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
    }

    #[test]
    fn test_lsp_server_stream_lifecycle_and_methods() {
        let mut input = String::new();

        // 1. Request before initialize (line 184)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/definition",
            "params": {}
        })));

        // 2. Malformed / missing Content-Length line (line 120)
        input.push_str("Invalid-Header: 123\r\n\r\n");

        // 3. Initialize request with folders when multi_root is empty (lines 213-225)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "initialize",
            "params": {
                "workspaceFolders": [{ "uri": "file:///tmp/mock_folder", "name": "mock" }]
            }
        })));

        // 4. textDocument/didChange on an unopened file (lines 260-271)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": "file:///tmp/unopened_file.rs", "version": 1 },
                "contentChanges": [{ "text": "fn new_content() {}\n" }]
            }
        })));

        // 5. workspace/didRenameFiles (lines 298-311)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "workspace/didRenameFiles",
            "params": {
                "files": [{ "oldUri": "file:///tmp/old.rs", "newUri": "file:///tmp/new.rs" }]
            }
        })));

        // 6. workspace/didDeleteFiles (lines 313-324)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "workspace/didDeleteFiles",
            "params": {
                "files": [{ "uri": "file:///tmp/new.rs" }]
            }
        })));

        // 7. workspace/didChangeWatchedFiles (lines 294-296)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "workspace/didChangeWatchedFiles",
            "params": { "changes": [] }
        })));

        // Exercise all dispatch request handlers (lines 330-705)
        let methods = [
            "textDocument/definition",
            "textDocument/declaration",
            "textDocument/typeDefinition",
            "textDocument/implementation",
            "textDocument/references",
            "textDocument/documentHighlight",
            "textDocument/documentSymbol",
            "workspace/symbol",
            "workspaceSymbol/resolve",
            "textDocument/hover",
            "textDocument/completion",
            "completionItem/resolve",
            "textDocument/signatureHelp",
            "textDocument/formatting",
            "textDocument/prepareRename",
            "textDocument/rename",
            "textDocument/prepareCallHierarchy",
            "callHierarchy/incomingCalls",
            "callHierarchy/outgoingCalls",
            "textDocument/prepareTypeHierarchy",
            "typeHierarchy/supertypes",
            "typeHierarchy/subtypes",
            "workspace/diagnostic",
            "textDocument/diagnostic",
            "textDocument/selectionRange",
            "textDocument/documentLink",
            "documentLink/resolve",
            "textDocument/semanticTokens/full",
            "textDocument/foldingRange",
            "textDocument/codeAction",
            "codeAction/resolve",
            "textDocument/codeLens",
            "codeLens/resolve",
            "textDocument/inlayHint",
            "textDocument/moniker",
            "workspace/executeCommand",
            "textDocument/documentColor",
            "textDocument/colorPresentation",
            "textDocument/linkedEditingRange",
            "textDocument/inlineValue",
            "textDocument/inlineCompletion",
            "workspace/textDocumentContent",
        ];

        let mut req_id = 100;
        for m in methods {
            req_id += 1;
            input.push_str(&format_lsp_msg(&json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "method": m,
                "params": {
                    "textDocument": { "uri": "file:///tmp/unopened_file.rs" },
                    "position": { "line": 0, "character": 0 },
                    "newName": "new_name",
                    "command": "test.cmd"
                }
            })));
        }

        // 8. shutdown (lines 706-716)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "shutdown"
        })));

        // 9. Request after shutdown (lines 160-169)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "textDocument/definition",
            "params": {}
        })));

        // 10. exit (lines 146-148)
        input.push_str(&format_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let reader = Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        let res = run_lsp_server_stream(reader, &mut output, None);
        assert!(res.is_ok());
        let out_str = String::from_utf8_lossy(&output);
        assert!(out_str.contains("Server not initialized"));
        assert!(out_str.contains("Server is shutting down"));
    }
}
