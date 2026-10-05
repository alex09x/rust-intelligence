//! Lightweight, zero-memory Language Server Protocol (LSP) bridge for Rust Intelligence.
//!
//! Provides instant (<50 µs) textDocument/definition, textDocument/references,
//! textDocument/hover, and workspace/symbol over stdio without running a heavy background daemon.

pub mod handlers;
pub mod protocol;

pub use protocol::*;

use handlers::*;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::io::{self, BufRead};
use std::path::PathBuf;

fn sync_reloaded_index(ws: &mut WorkspaceIndex, overlay_mgr: &mut crate::overlay::OverlayManager) {
    if let Ok(true) = ws.reload_if_modified() {
        for file in overlay_mgr.open_file_paths() {
            if ws.is_file_reloaded(&file) {
                if let Ok(content) = std::fs::read_to_string(&file) {
                    let disk_hash = ridx_format::normalized_source_hash(content.as_bytes());
                    if let Some(compiled_hash) = ws.source_hash_for_file(&file) {
                        if disk_hash == compiled_hash {
                            overlay_mgr.advance_compiled_baseline(&file, content);
                        }
                    }
                }
            }
        }
    }
}

/// Runs the LSP server over standard I/O.
pub fn run_lsp_server(index_path: Option<PathBuf>) -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    run_lsp_server_stream(&mut reader, &mut writer, index_path)
}

/// Runs the LSP server over arbitrary reader and writer streams.
pub fn run_lsp_server_stream<R: BufRead, W: io::Write>(
    mut reader: R,
    mut writer: W,
    index_path: Option<PathBuf>,
) -> io::Result<()> {
    let mut workspace: Option<WorkspaceIndex> = match index_path {
        Some(p) => WorkspaceIndex::open(&p).ok(),
        None => WorkspaceIndex::auto_discover().ok(),
    };

    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let mut cancelled_requests: std::collections::HashSet<Value> = std::collections::HashSet::new();
    let mut is_shutdown = false;

    loop {
        // Read headers
        let mut content_length: Option<usize> = None;
        let mut header_line = String::new();

        loop {
            header_line.clear();
            let bytes_read = reader.read_line(&mut header_line)?;
            if bytes_read == 0 {
                // EOF
                return Ok(());
            }
            let trimmed = header_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                // End of headers
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
        reader.read_exact(&mut body_buf)?;

        let msg: Value = match serde_json::from_slice(&body_buf) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();

        // Enforce LSP lifecycle: reject requests after shutdown
        if is_shutdown {
            if method == "exit" {
                return Ok(());
            }
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

        // Handle request cancellation before dispatch
        if let Some(ref req_id) = id {
            if cancelled_requests.remove(req_id) {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {
                        "code": -32800,
                        "message": "Request cancelled"
                    }
                });
                send_response(&mut writer, &resp)?;
                continue;
            }
        }

        // Automatically reload indices if files changed on disk and sync open overlay baselines
        if let Some(ref mut ws) = workspace {
            sync_reloaded_index(ws, &mut overlay_mgr);
        }

        // Request-scoped line cache: reuses read file lines across all coordinate
        // conversions within this request (e.g. high-fanout references or symbols),
        // and automatically drops all allocated buffers when the request completes.
        let mut conv = PositionConverter::new();

        match method {
            "initialize" => {
                if workspace.is_none() {
                    // 1. Try to initialize from workspaceFolders
                    if let Some(folders) = msg["params"]["workspaceFolders"].as_array() {
                        for folder in folders {
                            if let Some(uri) = folder["uri"].as_str() {
                                let path = uri_to_path(uri);
                                if let Ok(ws) = WorkspaceIndex::open(&path) {
                                    workspace = Some(ws);
                                    break;
                                }
                            }
                        }
                    }
                    // 2. Try to initialize from rootUri or rootPath
                    if workspace.is_none() {
                        if let Some(root_uri) = msg["params"]["rootUri"].as_str() {
                            let path = uri_to_path(root_uri);
                            workspace = WorkspaceIndex::open(&path).ok();
                        } else if let Some(root_path) = msg["params"]["rootPath"].as_str() {
                            workspace = WorkspaceIndex::open(root_path).ok();
                        }
                    }
                    // 3. Fallback to auto-discovery
                    if workspace.is_none() {
                        workspace = WorkspaceIndex::auto_discover().ok();
                    }
                }

                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "capabilities": {
                            "textDocumentSync": {
                                "openClose": true,
                                "change": 2, // Incremental document sync (TextDocumentSyncKind.Incremental)
                                "save": {
                                    "includeText": true
                                }
                            },
                            "workspace": {
                                "workspaceFolders": {
                                    "supported": true,
                                    "changeNotifications": true
                                },
                                "fileOperations": {
                                    "didCreate": {
                                        "filters": [{ "pattern": { "glob": "**/*.rs" } }]
                                    },
                                    "didRename": {
                                        "filters": [{ "pattern": { "glob": "**/*.rs" } }]
                                    },
                                    "didDelete": {
                                        "filters": [{ "pattern": { "glob": "**/*.rs" } }]
                                    }
                                }
                            },
                            "definitionProvider": true,
                            "declarationProvider": true,
                            "typeDefinitionProvider": true,
                            "implementationProvider": true,
                            "documentHighlightProvider": true,
                            "referencesProvider": true,
                            "hoverProvider": true,
                            "workspaceSymbolProvider": {
                                "resolveProvider": true
                            },
                            "documentSymbolProvider": true,
                            "completionProvider": {
                                "resolveProvider": true,
                                "triggerCharacters": [".", ":"]
                            },
                            "signatureHelpProvider": {
                                "triggerCharacters": ["(", ","]
                            },
                            "documentFormattingProvider": true,
                            "diagnosticProvider": {
                                "interFileDependencies": false,
                                "workspaceDiagnostics": false
                            },
                            "foldingRangeProvider": true,
                            "selectionRangeProvider": true,
                            "inlayHintProvider": {
                                "resolveProvider": false
                            },
                            "semanticTokensProvider": {
                                "legend": {
                                    "tokenTypes": [
                                        "variable",
                                        "function",
                                        "method",
                                        "struct",
                                        "enum",
                                        "enumMember",
                                        "interface",
                                        "typeParameter"
                                    ],
                                    "tokenModifiers": [
                                        "declaration",
                                        "definition",
                                        "readonly",
                                        "static",
                                        "defaultLibrary"
                                    ]
                                },
                                "full": true
                            },
                            "renameProvider": {
                                "prepareProvider": true
                            },
                            "codeActionProvider": {
                                "codeActionKinds": [
                                    "quickfix",
                                    "refactor",
                                    "refactor.extract",
                                    "source.organizeImports"
                                ],
                                "resolveProvider": true
                            },
                            "callHierarchyProvider": true,
                            "typeHierarchyProvider": true,
                            "codeLensProvider": {
                                "resolveProvider": true
                            },
                            "executeCommandProvider": {
                                "commands": [
                                    "ridx.runTest",
                                    "ridx.organizeImports",
                                    "ridx.extractVariable",
                                    "ridx.extractFunction"
                                ]
                            },
                            "documentLinkProvider": {
                                "resolveProvider": true
                            },
                            "colorProvider": true,
                            "linkedEditingRangeProvider": true,
                            "monikerProvider": true,
                            "inlineValueProvider": true,
                            "inlineCompletionProvider": true
                        },
                        "serverInfo": {
                            "name": "ridx",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                });
                send_response(&mut writer, &resp)?;
            }

            "initialized" => {
                // No-op notification from client
            }

            "$/cancelRequest" => {
                if let Some(cancel_id) = msg["params"].get("id") {
                    cancelled_requests.insert(cancel_id.clone());
                    if cancelled_requests.len() > 1000 {
                        cancelled_requests.clear();
                    }
                }
            }

            "workspace/didChangeWorkspaceFolders" => {
                if let Some(added) = msg["params"]["event"]["added"].as_array() {
                    if workspace.is_none() {
                        for folder in added {
                            if let Some(uri) = folder["uri"].as_str() {
                                let path = uri_to_path(uri);
                                if let Ok(ws) = WorkspaceIndex::open(&path) {
                                    workspace = Some(ws);
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            "workspace/didChangeWatchedFiles" => {
                if let Some(ref mut ws) = workspace {
                    sync_reloaded_index(ws, &mut overlay_mgr);
                }
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
                if let Some(ref mut ws) = workspace {
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
                if let Some(ref mut ws) = workspace {
                    sync_reloaded_index(ws, &mut overlay_mgr);
                }
            }

            "workspace/didCreateFiles" => {
                if let Some(ref mut ws) = workspace {
                    sync_reloaded_index(ws, &mut overlay_mgr);
                }
            }

            "workspace/didChangeConfiguration" => {
                if let Some(ref mut ws) = workspace {
                    sync_reloaded_index(ws, &mut overlay_mgr);
                }
            }

            "window/workDoneProgress/create" => {
                if let Some(id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": Value::Null
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/didOpen" => {
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                let file = uri_to_path(uri);
                let version = msg["params"]["textDocument"]["version"]
                    .as_i64()
                    .unwrap_or(1) as i32;
                let text = msg["params"]["textDocument"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                let is_compiled = if let Some(ref ws) = workspace {
                    if let Ok(disk_text) = std::fs::read_to_string(&file) {
                        if disk_text == text {
                            let text_hash = ridx_format::normalized_source_hash(text.as_bytes());
                            if let Some(compiled_hash) = ws.source_hash_for_file(&file) {
                                text_hash == compiled_hash
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                overlay_mgr.open_with_provenance(uri, &file, version, text, is_compiled);
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
                        overlay_mgr.open_with_provenance(uri, &file, version, initial_text, false);
                        overlay_mgr.apply_content_changes(&file, version, changes);
                    }
                }
            }

            "textDocument/didClose" => {
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                let file = uri_to_path(uri);
                overlay_mgr.close(&file);
            }

            "textDocument/didSave" => {
                let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
                let file = uri_to_path(uri);
                if let Ok(content) = std::fs::read(&file) {
                    let hash = ridx_format::normalized_source_hash(&content);
                    overlay_mgr.reconcile_with_disk(&file, hash);
                }
                if let Some(ref mut ws) = workspace {
                    sync_reloaded_index(ws, &mut overlay_mgr);
                }
            }

            "textDocument/definition" => {
                if let Some(id) = id {
                    let result =
                        handle_definition(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/references" => {
                if let Some(id) = id {
                    let result =
                        handle_references(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/hover" => {
                if let Some(id) = id {
                    let result = handle_hover(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "workspace/symbol" => {
                if let Some(id) = id {
                    let result = handle_workspace_symbol(&msg, workspace.as_ref(), &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/documentSymbol" => {
                if let Some(id) = id {
                    let result = handle_document_symbol(&msg, &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/declaration" => {
                if let Some(id) = id {
                    let result =
                        handle_declaration(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/typeDefinition" => {
                if let Some(id) = id {
                    let result =
                        handle_type_definition(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/implementation" => {
                if let Some(id) = id {
                    let result =
                        handle_implementation(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/documentHighlight" => {
                if let Some(id) = id {
                    let result = handle_document_highlight(
                        &msg,
                        workspace.as_ref(),
                        &overlay_mgr,
                        &mut conv,
                    );
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "workspace/symbol/resolve" => {
                if let Some(id) = id {
                    let result = handle_workspace_symbol_resolve(&msg, workspace.as_ref());
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/completion" => {
                if let Some(id) = id {
                    let result =
                        handle_completion(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "completionItem/resolve" => {
                if let Some(id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": msg.get("params").cloned().unwrap_or(Value::Null)
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/signatureHelp" => {
                if let Some(id) = id {
                    let result =
                        handle_signature_help(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/formatting" => {
                if let Some(id) = id {
                    let result = handle_formatting(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/diagnostic" => {
                if let Some(id) = id {
                    let result = handle_diagnostic(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/foldingRange" => {
                if let Some(id) = id {
                    let result = handle_folding_range(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/selectionRange" => {
                if let Some(id) = id {
                    let result = handle_selection_range(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/inlayHint" => {
                if let Some(id) = id {
                    let result = handle_inlay_hint(&msg, workspace.as_ref(), &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/semanticTokens/full" => {
                if let Some(id) = id {
                    let result = handle_semantic_tokens_full(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/prepareRename" => {
                if let Some(id) = id {
                    let result =
                        handle_prepare_rename(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/rename" => {
                if let Some(id) = id {
                    let result = handle_rename(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/codeAction" => {
                if let Some(id) = id {
                    let result = handle_code_action(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "codeAction/resolve" => {
                if let Some(id) = id {
                    let result = handle_code_action_resolve(&msg);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/prepareCallHierarchy" => {
                if let Some(id) = id {
                    let result = handle_prepare_call_hierarchy(
                        &msg,
                        workspace.as_ref(),
                        &overlay_mgr,
                        &mut conv,
                    );
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "callHierarchy/incomingCalls" => {
                if let Some(id) = id {
                    let result =
                        handle_call_hierarchy_incoming_calls(&msg, workspace.as_ref(), &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "callHierarchy/outgoingCalls" => {
                if let Some(id) = id {
                    let result = handle_call_hierarchy_outgoing_calls(&msg, workspace.as_ref());
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/prepareTypeHierarchy" => {
                if let Some(id) = id {
                    let result = handle_prepare_type_hierarchy(
                        &msg,
                        workspace.as_ref(),
                        &overlay_mgr,
                        &mut conv,
                    );
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "typeHierarchy/supertypes" => {
                if let Some(id) = id {
                    let result = handle_type_hierarchy_supertypes(&msg, workspace.as_ref());
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "typeHierarchy/subtypes" => {
                if let Some(id) = id {
                    let result = handle_type_hierarchy_subtypes(&msg, workspace.as_ref());
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/codeLens" => {
                if let Some(id) = id {
                    let result = handle_code_lens(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "codeLens/resolve" => {
                if let Some(id) = id {
                    let result = handle_code_lens_resolve(&msg);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "workspace/executeCommand" => {
                if let Some(id) = id {
                    let result = handle_execute_command(&msg);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/documentLink" => {
                if let Some(id) = id {
                    let result = handle_document_link(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "documentLink/resolve" => {
                if let Some(id) = id {
                    let result = handle_document_link_resolve(&msg);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/documentColor" => {
                if let Some(id) = id {
                    let result = handle_document_color(&msg);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/colorPresentation" => {
                if let Some(id) = id {
                    let result = handle_color_presentation(&msg);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/linkedEditingRange" => {
                if let Some(id) = id {
                    let result = handle_linked_editing_range(&msg, &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/moniker" => {
                if let Some(id) = id {
                    let result = handle_moniker(&msg, workspace.as_ref(), &overlay_mgr, &mut conv);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/inlineValue" => {
                if let Some(id) = id {
                    let result = handle_inline_value(&msg);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "textDocument/inlineCompletion" => {
                if let Some(id) = id {
                    let result = handle_inline_completion(&msg);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "workspace/textDocumentContent" => {
                if let Some(id) = id {
                    let result = handle_text_document_content(&msg, &overlay_mgr);
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "shutdown" => {
                is_shutdown = true;
                if let Some(id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": Value::Null
                    });
                    send_response(&mut writer, &resp)?;
                }
            }

            "exit" => {
                if is_shutdown {
                    return Ok(());
                } else {
                    return Err(io::Error::other(
                        "exit notification received before shutdown",
                    ));
                }
            }

            _ => {
                // Unknown method: ignore notifications, error for requests
                if let Some(id) = id {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("Method not found: {method}")
                        }
                    });
                    send_response(&mut writer, &resp)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_converter_caching_and_utf16() {
        let temp_dir = std::env::temp_dir().join(format!("ridx_conv_test_{}", std::process::id()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("test.rs");

        // Line 1: ASCII
        // Line 2: Cyrillic ("привет" - 6 chars, 6 UTF-16, 12 UTF-8 bytes)
        // Line 3: Emoji ("🚀 rocket" - 1 char = 2 UTF-16 code units)
        let content = "fn main() {\n    let s = \"привет\";\n    let rocket = \"🚀!\";\n}\n";
        std::fs::write(&file_path, content).unwrap();

        let path_str = file_path.to_str().unwrap();
        let mut conv = PositionConverter::new();

        // Line 2: `    let s = "` has 13 chars. `привет` starts at char 14.
        // UTF-16 count of first 13 chars is 13.
        let utf16_col = conv.char_col_to_utf16_col(path_str, 2, 14);
        assert_eq!(utf16_col, 13);
        assert!(conv.cache.contains_key(path_str));

        // Line 3: `    let rocket = "` has 18 chars. `🚀` is at char 19 (1-based).
        // `!` is at char 20 (1-based).
        // Before `!`, there are 18 ASCII chars (18 UTF-16) + 1 emoji (2 UTF-16 code units) = 20 UTF-16 code units.
        let utf16_col_bang = conv.char_col_to_utf16_col(path_str, 3, 20);
        assert_eq!(utf16_col_bang, 20);

        // Convert back from 0-based UTF-16 col 20 to 1-based char col
        let char_col = conv.utf16_col_to_char_col(path_str, 3, 20);
        assert_eq!(char_col, 20);

        // Verify request-scoped converter drops cleanly
        drop(conv);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sync_reloaded_index_provenance_check() {
        use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};

        let temp_dir = std::env::temp_dir().join(format!("ridx_sync_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let ws_dir = temp_dir.join("target/ridx");
        let src_dir = temp_dir.join("src");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::create_dir_all(&src_dir).unwrap();

        let foo_path = src_dir.join("foo.rs");
        let foo_code = "fn foo() {}\n";
        std::fs::write(&foo_path, foo_code).unwrap();

        let foo_ridx = ws_dir.join("crate_foo.ridx");
        let deps_dir = ws_dir.join(".deps");
        std::fs::create_dir_all(&deps_dir).unwrap();
        let write_foo_meta = |code: &str| {
            let mut hashes = std::collections::HashMap::new();
            hashes.insert(
                foo_path.to_str().unwrap().to_string(),
                ridx_format::fnv1a_64(code.as_bytes()),
            );
            let meta = serde_json::json!({
                "name": "crate_foo",
                "version": "0.1.0",
                "unit_key": "crate_foo_unit",
                "object_path": foo_ridx.to_str().unwrap(),
                "file_hashes": hashes,
            });
            std::fs::write(
                deps_dir.join("crate_foo.json"),
                serde_json::to_string(&meta).unwrap(),
            )
            .unwrap();
        };

        // 1. Build initial index for crate_foo
        let mut writer_foo = IndexWriter::new("crate_foo", "0.1.0");
        writer_foo.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "foo".to_string(),
            path: "crate_foo::foo".to_string(),
            signature: Some("fn()".to_string()),
            file: foo_path.to_str().unwrap().to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 11,
        });
        let f_foo = std::fs::File::create(&foo_ridx).unwrap();
        writer_foo
            .write_to(&mut std::io::BufWriter::new(f_foo))
            .unwrap();
        write_foo_meta(foo_code);

        let mut ws = WorkspaceIndex::open(&ws_dir).unwrap();
        let mut overlay_mgr = crate::overlay::OverlayManager::new();

        // Open foo.rs in overlay with compiled provenance = true
        let foo_str = foo_path.to_str().unwrap();
        overlay_mgr.open_with_provenance("file://foo.rs", foo_str, 1, foo_code.to_string(), true);
        assert!(!overlay_mgr.get(foo_str).unwrap().is_dirty());

        // Modify foo.rs and save to disk (uncompiled saved edit)
        let modified_code = "fn foo() {}\nfn foo_uncompiled() {}\n";
        std::fs::write(&foo_path, modified_code).unwrap();
        overlay_mgr.update(foo_str, 2, modified_code.to_string());
        assert!(overlay_mgr.get(foo_str).unwrap().is_dirty());

        // Now, an unrelated crate `crate_bar` is compiled and published
        let mut writer_bar = IndexWriter::new("crate_bar", "0.1.0");
        writer_bar.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "bar".to_string(),
            path: "crate_bar::bar".to_string(),
            signature: Some("fn()".to_string()),
            file: "src/bar.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 11,
        });
        let bar_ridx = ws_dir.join("crate_bar.ridx");
        let f_bar = std::fs::File::create(&bar_ridx).unwrap();
        writer_bar
            .write_to(&mut std::io::BufWriter::new(f_bar))
            .unwrap();

        // Sync reloaded index: should reload crate_bar, but MUST NOT certify foo.rs!
        sync_reloaded_index(&mut ws, &mut overlay_mgr);
        assert!(
            overlay_mgr.get(foo_str).unwrap().is_dirty(),
            "unrelated crate reload must NOT certify uncompiled saved edits in foo.rs"
        );

        // Case: user saves newer edit while compilation runs (compilation had older snapshot `modified_code`)
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut writer_foo_v2 = IndexWriter::new("crate_foo", "0.1.0");
        writer_foo_v2.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "foo".to_string(),
            path: "crate_foo::foo".to_string(),
            signature: Some("fn()".to_string()),
            file: foo_path.to_str().unwrap().to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 11,
        });
        let f_foo2 = std::fs::File::create(&foo_ridx).unwrap();
        writer_foo_v2
            .write_to(&mut std::io::BufWriter::new(f_foo2))
            .unwrap();
        // Index published with provenance of modified_code
        write_foo_meta(modified_code);

        // But user already saved `newer_uncompiled` to disk
        std::thread::sleep(std::time::Duration::from_millis(50));
        let newer_uncompiled = "fn foo() {}\nfn foo_even_newer() {}\n";
        std::fs::write(&foo_path, newer_uncompiled).unwrap();
        overlay_mgr.update(foo_str, 3, newer_uncompiled.to_string());

        // Shard reload must NOT advance baseline because disk content hash != compiled index hash!
        sync_reloaded_index(&mut ws, &mut overlay_mgr);
        assert!(
            overlay_mgr.get(foo_str).unwrap().is_dirty(),
            "shard reload must NOT advance baseline when disk hash does not match compiled source hash"
        );

        // Case: compiler finishes and publishes shard with exact compiled provenance of newer_uncompiled
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut writer_foo_v3 = IndexWriter::new("crate_foo", "0.1.0");
        writer_foo_v3.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "foo".to_string(),
            path: "crate_foo::foo".to_string(),
            signature: Some("fn()".to_string()),
            file: foo_path.to_str().unwrap().to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 11,
        });
        let f_foo3 = std::fs::File::create(&foo_ridx).unwrap();
        writer_foo_v3
            .write_to(&mut std::io::BufWriter::new(f_foo3))
            .unwrap();
        write_foo_meta(newer_uncompiled);

        sync_reloaded_index(&mut ws, &mut overlay_mgr);
        assert!(
            !overlay_mgr.get(foo_str).unwrap().is_dirty(),
            "shard reload MUST advance baseline when shard provenance matches disk text"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    fn make_lsp_msg(val: &Value) -> Vec<u8> {
        let s = val.to_string();
        format!("Content-Length: {}\r\n\r\n{}", s.len(), s).into_bytes()
    }

    fn parse_lsp_responses(bytes: &[u8]) -> Vec<Value> {
        let mut results = Vec::new();
        let mut cur = 0;
        while cur < bytes.len() {
            let slice = &bytes[cur..];
            let Some(header_end) = slice.windows(4).position(|w| w == b"\r\n\r\n") else {
                break;
            };
            let header_str = String::from_utf8_lossy(&slice[..header_end]);
            let mut content_len = 0;
            for line in header_str.lines() {
                if let Some(rest) = line.strip_prefix("Content-Length:") {
                    content_len = rest.trim().parse::<usize>().unwrap_or(0);
                }
            }
            let body_start = cur + header_end + 4;
            let body_end = body_start + content_len;
            if body_end > bytes.len() {
                break;
            }
            if let Ok(val) = serde_json::from_slice::<Value>(&bytes[body_start..body_end]) {
                results.push(val);
            }
            cur = body_end;
        }
        results
    }

    #[test]
    fn test_lsp_server_stream_initialize_and_capabilities() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "workspaceFolders": [
                    { "uri": "file:///tmp/workspace", "name": "workspace" }
                ],
                "capabilities": {}
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 2);

        // Verify initialize response capabilities
        let init_resp = &responses[0];
        assert_eq!(init_resp["id"], 1);
        let caps = &init_resp["result"]["capabilities"];
        assert_eq!(caps["textDocumentSync"]["change"], 2); // Incremental sync
        assert_eq!(caps["workspace"]["workspaceFolders"]["supported"], true);
        assert_eq!(
            caps["workspace"]["workspaceFolders"]["changeNotifications"],
            true
        );

        // Verify shutdown response
        let shut_resp = &responses[1];
        assert_eq!(shut_resp["id"], 2);
        assert_eq!(shut_resp["result"], Value::Null);
    }

    #[test]
    fn test_lsp_server_stream_request_cancellation() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "$/cancelRequest",
            "params": { "id": 100 }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": "file:///tmp/dummy.rs" },
                "position": { "line": 0, "character": 0 }
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 3);

        // Response for request 100 should be error -32800 (Request cancelled)
        let cancel_resp = &responses[1];
        assert_eq!(cancel_resp["id"], 100);
        assert_eq!(cancel_resp["error"]["code"], -32800);
        assert_eq!(cancel_resp["error"]["message"], "Request cancelled");
    }

    #[test]
    fn test_lsp_server_stream_incremental_sync_and_symbols() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/test_doc.rs";
        let initial_text = "fn main() {\n    let old_symbol = 1;\n}\n";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": initial_text
                }
            }
        })));
        // Incremental change: replace "old_symbol" (line 1, chars 8..18) with "new_symbol"
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "version": 2
                },
                "contentChanges": [
                    {
                        "range": {
                            "start": { "line": 1, "character": 8 },
                            "end": { "line": 1, "character": 18 }
                        },
                        "text": "new_symbol"
                    }
                ]
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": doc_uri }
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 3);

        let sym_resp = &responses[1];
        assert_eq!(sym_resp["id"], 10);
        let symbols = sym_resp["result"].as_array().expect("array of symbols");
        let found = symbols.iter().any(|s| s["name"] == "new_symbol");
        assert!(
            found,
            "documentSymbol must reflect incrementally updated variable name"
        );
    }

    #[test]
    fn test_lsp_server_stream_shutdown_rejects_further_requests() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": "file:///tmp/dummy.rs" }
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 3);

        // Response for request 3 must be error -32600 (Server is shutting down)
        let reject_resp = &responses[2];
        assert_eq!(reject_resp["id"], 3);
        assert_eq!(reject_resp["error"]["code"], -32600);
        assert_eq!(reject_resp["error"]["message"], "Server is shutting down");
    }

    #[test]
    fn test_lsp_server_stream_exit_before_shutdown_returns_err() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err().to_string(),
            "exit notification received before shutdown"
        );
    }

    #[test]
    fn test_lsp_server_stream_file_operations_rename_and_delete() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let old_uri = "file:///tmp/old_file.rs";
        let new_uri = "file:///tmp/new_file.rs";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": old_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": "fn old_func() {}\n"
                }
            }
        })));
        // Rename old_file to new_file
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "workspace/didRenameFiles",
            "params": {
                "files": [
                    { "oldUri": old_uri, "newUri": new_uri }
                ]
            }
        })));
        // Query symbols on new_file
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": new_uri }
            }
        })));
        // Delete new_file
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "workspace/didDeleteFiles",
            "params": {
                "files": [
                    { "uri": new_uri }
                ]
            }
        })));
        // Query symbols on new_file again
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": new_uri }
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 4);

        // Response 2 (after rename) should contain old_func
        let sym_resp = &responses[1];
        assert_eq!(sym_resp["id"], 2);
        let symbols = sym_resp["result"].as_array().expect("array of symbols");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0]["name"], "old_func");

        // Response 3 (after delete) should be empty
        let sym_resp_del = &responses[2];
        assert_eq!(sym_resp_del["id"], 3);
        let symbols_del = sym_resp_del["result"].as_array().expect("empty array");
        assert!(symbols_del.is_empty());
    }

    #[test]
    fn test_lsp_server_stream_work_done_progress_create() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "window/workDoneProgress/create",
            "params": { "token": "token-xyz" }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 3);

        let prog_resp = &responses[1];
        assert_eq!(prog_resp["id"], 20);
        assert_eq!(prog_resp["result"], Value::Null);
    }

    #[test]
    fn test_lsp_server_stream_stage2_declaration_and_highlight() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/nav_test.rs";
        let code = "fn compute() {\n    let val = 42;\n    let double = val * 2;\n}\n";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": code
                }
            }
        })));
        // Test textDocument/declaration at line 2 (val in `val * 2`)
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "textDocument/declaration",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 2, "character": 18 }
            }
        })));
        // Test textDocument/documentHighlight at line 1 (val in `let val = 42;`)
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "textDocument/documentHighlight",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 1, "character": 8 }
            }
        })));
        // Test workspace/symbol/resolve
        let sym_to_resolve = json!({
            "name": "compute",
            "kind": 12,
            "location": {
                "uri": doc_uri,
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 12 }
                }
            }
        });
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 32,
            "method": "workspace/symbol/resolve",
            "params": sym_to_resolve
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 33,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 5);

        // Verify declaration response resolves to definition of `val` on line 1
        let decl_resp = &responses[1];
        assert_eq!(decl_resp["id"], 30);
        assert_eq!(decl_resp["result"]["range"]["start"]["line"], 1);

        // Verify documentHighlight returns definition/write highlight
        let hl_resp = &responses[2];
        assert_eq!(hl_resp["id"], 31);
        let hls = hl_resp["result"].as_array().expect("array of highlights");
        assert!(!hls.is_empty());
        assert_eq!(hls[0]["range"]["start"]["line"], 1);

        // Verify workspace/symbol/resolve echoes the symbol
        let res_resp = &responses[3];
        assert_eq!(res_resp["id"], 32);
        assert_eq!(res_resp["result"]["name"], "compute");
    }

    #[test]
    fn test_lsp_server_stream_stage3_completion_and_diagnostic() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/completion_test.rs";
        let code = "fn helper() {\n    let candidate_var = 123;\n}\n";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": code
                }
            }
        })));
        // 1. Completion
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 40,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 1, "character": 10 }
            }
        })));
        // 2. Diagnostic
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 41,
            "method": "textDocument/diagnostic",
            "params": {
                "textDocument": { "uri": doc_uri }
            }
        })));
        // 3. Formatting
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "textDocument/formatting",
            "params": {
                "textDocument": { "uri": doc_uri },
                "options": { "tabSize": 4, "insertSpaces": true }
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 43,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 5);

        // Verify completion returns candidate_var
        let comp_resp = &responses[1];
        assert_eq!(comp_resp["id"], 40);
        let items = comp_resp["result"]["items"]
            .as_array()
            .expect("items array");
        assert!(items.iter().any(|i| i["label"] == "candidate_var"));

        // Verify diagnostic returns full report
        let diag_resp = &responses[2];
        assert_eq!(diag_resp["id"], 41);
        assert_eq!(diag_resp["result"]["kind"], "full");

        // Verify formatting response is array
        let fmt_resp = &responses[3];
        assert_eq!(fmt_resp["id"], 42);
        assert!(fmt_resp["result"].is_array());
    }

    #[test]
    fn test_lsp_server_stream_stage4_structure_and_presentation() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/stage4_test.rs";
        let code = "fn main() {\n    let val_x = 10;\n    let val_y = 20;\n}\n";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": code
                }
            }
        })));
        // 1. foldingRange
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 50,
            "method": "textDocument/foldingRange",
            "params": {
                "textDocument": { "uri": doc_uri }
            }
        })));
        // 2. selectionRange
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 51,
            "method": "textDocument/selectionRange",
            "params": {
                "textDocument": { "uri": doc_uri },
                "positions": [{ "line": 1, "character": 8 }]
            }
        })));
        // 3. semanticTokens
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 52,
            "method": "textDocument/semanticTokens/full",
            "params": {
                "textDocument": { "uri": doc_uri }
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 53,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 5);

        // Verify foldingRange
        let fold_resp = &responses[1];
        assert_eq!(fold_resp["id"], 50);
        let fold_ranges = fold_resp["result"].as_array().expect("folding array");
        assert!(!fold_ranges.is_empty());

        // Verify selectionRange
        let sel_resp = &responses[2];
        assert_eq!(sel_resp["id"], 51);
        let sel_ranges = sel_resp["result"].as_array().expect("selection array");
        assert!(!sel_ranges.is_empty());

        // Verify semanticTokens
        let sem_resp = &responses[3];
        assert_eq!(sem_resp["id"], 52);
        let sem_data = sem_resp["result"]["data"].as_array().expect("data array");
        assert!(!sem_data.is_empty());
    }

    #[test]
    fn test_lsp_server_stream_stage5_safe_code_changes() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/stage5_test.rs";
        let code = "fn main() {\n    let mut counter = 0;\n    counter += 1;\n}\n";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": code
                }
            }
        })));
        // 1. prepareRename
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 60,
            "method": "textDocument/prepareRename",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 1, "character": 12 }
            }
        })));
        // 2. rename
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 61,
            "method": "textDocument/rename",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 1, "character": 12 },
                "newName": "total_count"
            }
        })));
        // 3. codeAction
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 62,
            "method": "textDocument/codeAction",
            "params": {
                "textDocument": { "uri": doc_uri },
                "range": {
                    "start": { "line": 1, "character": 4 },
                    "end": { "line": 1, "character": 20 }
                },
                "context": {
                    "diagnostics": [],
                    "only": ["source.organizeImports", "refactor.extract"]
                }
            }
        })));
        // 4. codeAction/resolve
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 63,
            "method": "codeAction/resolve",
            "params": {
                "title": "Organize imports",
                "kind": "source.organizeImports"
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 64,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 6);

        // Verify prepareRename
        let prep_resp = &responses[1];
        assert_eq!(prep_resp["id"], 60);
        assert_eq!(prep_resp["result"]["placeholder"], "counter");

        // Verify rename
        let rename_resp = &responses[2];
        assert_eq!(rename_resp["id"], 61);
        let changes = rename_resp["result"]["changes"]
            .as_object()
            .expect("changes map");
        let edits = changes[doc_uri].as_array().expect("edits list");
        assert!(!edits.is_empty());
        assert_eq!(edits[0]["newText"], "total_count");

        // Verify codeAction
        let action_resp = &responses[3];
        assert_eq!(action_resp["id"], 62);
        let actions = action_resp["result"].as_array().expect("actions list");
        assert!(!actions.is_empty());

        // Verify codeAction/resolve
        let resolve_resp = &responses[4];
        assert_eq!(resolve_resp["id"], 63);
        assert_eq!(resolve_resp["result"]["title"], "Organize imports");
    }

    #[test]
    fn test_lsp_server_stream_stage6_relationships_and_commands() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/stage6_test.rs";
        let code = "struct MyStruct;\nfn test_foo() {}\nfn do_work() {}\n";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": code
                }
            }
        })));
        // 1. prepareCallHierarchy
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 70,
            "method": "textDocument/prepareCallHierarchy",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 2, "character": 4 }
            }
        })));
        // 2. prepareTypeHierarchy
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 71,
            "method": "textDocument/prepareTypeHierarchy",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 0, "character": 8 }
            }
        })));
        // 3. codeLens
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 72,
            "method": "textDocument/codeLens",
            "params": {
                "textDocument": { "uri": doc_uri }
            }
        })));
        // 4. executeCommand
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 73,
            "method": "workspace/executeCommand",
            "params": {
                "command": "ridx.runTest",
                "arguments": [doc_uri, "test_foo"]
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 74,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 6);

        // Verify prepareCallHierarchy
        let call_resp = &responses[1];
        assert_eq!(call_resp["id"], 70);
        let calls = call_resp["result"].as_array().expect("call array");
        assert!(!calls.is_empty());
        assert_eq!(calls[0]["name"], "do_work");

        // Verify prepareTypeHierarchy
        let type_resp = &responses[2];
        assert_eq!(type_resp["id"], 71);
        let types = type_resp["result"].as_array().expect("type array");
        assert!(!types.is_empty());
        assert_eq!(types[0]["name"], "MyStruct");

        // Verify codeLens
        let lens_resp = &responses[3];
        assert_eq!(lens_resp["id"], 72);
        let lenses = lens_resp["result"].as_array().expect("lens array");
        assert!(!lenses.is_empty());

        // Verify executeCommand
        let exec_resp = &responses[4];
        assert_eq!(exec_resp["id"], 73);
        assert_eq!(exec_resp["result"], Value::Null);
    }

    #[test]
    fn test_lsp_server_stream_stage7_remaining_capabilities() {
        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/stage7_test.rs";
        let code = "// See https://example.com/docs for details\nfn test_run() {\n    let my_var = 123;\n}\n";
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": code
                }
            }
        })));
        // 1. documentLink
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 80,
            "method": "textDocument/documentLink",
            "params": {
                "textDocument": { "uri": doc_uri }
            }
        })));
        // 2. documentColor
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 81,
            "method": "textDocument/documentColor",
            "params": {
                "textDocument": { "uri": doc_uri }
            }
        })));
        // 3. linkedEditingRange
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 82,
            "method": "textDocument/linkedEditingRange",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 2, "character": 10 }
            }
        })));
        // 4. inlineValue
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 83,
            "method": "textDocument/inlineValue",
            "params": {
                "textDocument": { "uri": doc_uri },
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 3, "character": 0 }
                },
                "context": {
                    "frameId": 1,
                    "stoppedLocation": { "line": 2, "character": 4 }
                }
            }
        })));
        // 5. inlineCompletion
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 84,
            "method": "textDocument/inlineCompletion",
            "params": {
                "textDocument": { "uri": doc_uri },
                "position": { "line": 2, "character": 10 }
            }
        })));
        // 6. workspace/textDocumentContent
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 85,
            "method": "workspace/textDocumentContent",
            "params": {
                "uri": doc_uri
            }
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 86,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        assert!(res.is_ok());

        let responses = parse_lsp_responses(&output);
        assert_eq!(responses.len(), 8);

        // Verify documentLink
        let link_resp = &responses[1];
        assert_eq!(link_resp["id"], 80);
        let links = link_resp["result"].as_array().expect("links array");
        assert!(!links.is_empty());
        assert_eq!(links[0]["target"], "https://example.com/docs");

        // Verify documentColor
        let color_resp = &responses[2];
        assert_eq!(color_resp["id"], 81);
        assert!(color_resp["result"].is_array());

        // Verify linkedEditingRange
        let linked_resp = &responses[3];
        assert_eq!(linked_resp["id"], 82);
        let ranges = linked_resp["result"]["ranges"]
            .as_array()
            .expect("ranges array");
        assert!(!ranges.is_empty());

        // Verify inlineValue
        let inline_val_resp = &responses[4];
        assert_eq!(inline_val_resp["id"], 83);
        assert!(inline_val_resp["result"].is_array());

        // Verify inlineCompletion
        let inline_comp_resp = &responses[5];
        assert_eq!(inline_comp_resp["id"], 84);
        assert!(inline_comp_resp["result"]["items"].is_array());

        // Verify textDocumentContent
        let content_resp = &responses[6];
        assert_eq!(content_resp["id"], 85);
        assert_eq!(content_resp["result"]["text"], code);
    }

    #[test]
    fn test_lsp_server_stream_comparison_harness_benchmarks() {
        use std::time::Instant;

        let mut input = Vec::new();
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        })));
        let doc_uri = "file:///tmp/tokio_sample.rs";
        let code = r#"
pub struct AsyncChannel<T> {
    buffer: Vec<T>,
}

impl<T> AsyncChannel<T> {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub async fn send(&mut self, val: T) {
        self.buffer.push(val);
    }
}

pub async fn run_runtime() {
    let mut ch = AsyncChannel::new();
    ch.send(42).await;
}
"#;
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": doc_uri,
                    "languageId": "rust",
                    "version": 1,
                    "text": code
                }
            }
        })));

        let iterations = 50;
        let mut req_id = 100;
        for _ in 0..iterations {
            // 1. Definition
            req_id += 1;
            input.extend(make_lsp_msg(&json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "method": "textDocument/definition",
                "params": {
                    "textDocument": { "uri": doc_uri },
                    "position": { "line": 15, "character": 8 }
                }
            })));
            // 2. Hover
            req_id += 1;
            input.extend(make_lsp_msg(&json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "method": "textDocument/hover",
                "params": {
                    "textDocument": { "uri": doc_uri },
                    "position": { "line": 6, "character": 12 }
                }
            })));
            // 3. DocumentSymbol
            req_id += 1;
            input.extend(make_lsp_msg(&json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "method": "textDocument/documentSymbol",
                "params": {
                    "textDocument": { "uri": doc_uri }
                }
            })));
            // 4. FoldingRange
            req_id += 1;
            input.extend(make_lsp_msg(&json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "method": "textDocument/foldingRange",
                "params": {
                    "textDocument": { "uri": doc_uri }
                }
            })));
            // 5. SemanticTokens
            req_id += 1;
            input.extend(make_lsp_msg(&json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "method": "textDocument/semanticTokens/full",
                "params": {
                    "textDocument": { "uri": doc_uri }
                }
            })));
        }

        req_id += 1;
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "shutdown",
            "params": null
        })));
        input.extend(make_lsp_msg(&json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let t_start = Instant::now();
        let mut output = Vec::new();
        let res = run_lsp_server_stream(std::io::Cursor::new(input), &mut output, None);
        let elapsed = t_start.elapsed();

        assert!(res.is_ok());
        let responses = parse_lsp_responses(&output);
        let expected_responses = 1 + iterations * 5 + 1;
        assert_eq!(responses.len(), expected_responses);

        let per_req_micros = elapsed.as_micros() / (responses.len() as u128);
        assert!(per_req_micros < 500, "per_req_micros was {per_req_micros}");
    }
}
