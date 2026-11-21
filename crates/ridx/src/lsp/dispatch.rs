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

/// The root of the workspace `file` belongs to: the innermost loaded workspace holding it, else the root being indexed
/// for the first time that holds it, else the nearest directory with a `Cargo.toml` (not in a library's sources),
/// else the workspace a single-root session falls back to. Review of 30e62913: a file of an unindexed second root
/// went to the first one, and one of a member crate to that crate rather than to its workspace being indexed.
pub(crate) fn workspace_root_of(mr: &MultiRootWorkspace, reindex: &super::reindex::Reindexer, file: &Path) -> Option<PathBuf> {
    let file = ridx_format::canonicalize_best_effort(file);
    let loaded = mr.all_workspaces().filter_map(|ws| ws.workspace_root()).map(ridx_format::canonicalize_best_effort).filter(|r| file.starts_with(r));
    loaded
        .max_by_key(|r| r.as_os_str().len())
        .or_else(|| reindex.root_for(&file))
        .or_else(|| (!super::reindex::is_cached_source(&file)).then(|| file.ancestors().find(|d| d.join("Cargo.toml").is_file()).map(Path::to_path_buf)).flatten())
        .or_else(|| mr.find_workspace_for_path(&file).and_then(|ws| ws.workspace_root().map(Path::to_path_buf)))
}

/// Loads an index written since it was last read (a background reindex, `ridx index` by hand, a variant switch):
/// rebases the open buffers, drops what was derived from the old one, and has the client fetch again what it shows
/// from it (in Zed the parameter hints of lines saved since stayed missing). Nothing goes out after `shutdown`.
pub(crate) fn reload_changed_index<W: Write>(
    writer: &mut W,
    lifecycle: &mut LifecycleManager,
    mr: &mut MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
) -> io::Result<()> {
    let mut any_reloaded = false;
    for ws in mr.all_workspaces_mut() {
        any_reloaded |= sync_reloaded_index(ws, ov);
    }
    refresh_after_load(writer, lifecycle, any_reloaded)
}

/// Loads what the background runs of `finished` built: a root indexed for the first time joins the workspace (it
/// could not be opened without an index), the others reload.
pub(crate) fn load_finished_index<W: Write>(
    writer: &mut W,
    lifecycle: &mut LifecycleManager,
    mr: &mut MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    finished: &[PathBuf],
) -> io::Result<()> {
    // `add_root` keeps a root it already has. The buffers opened before take the new index as their base, as a reload
    // rebases them: else each read as edited since and answered nothing.
    let mut added = false;
    for root in finished {
        if mr.add_root(root.clone()) {
            added = true;
            if let Some(ws) = mr.all_workspaces().last() {
                super::rebase_open_buffers(ws, ov);
            }
        }
    }
    if added {
        super::logger::log_info(&format!("loaded the first index of {}", finished.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", ")));
    }
    refresh_after_load(writer, lifecycle, added)?;
    reload_changed_index(writer, lifecycle, mr, ov)
}

/// After an index is loaded: what came from the one before is dropped and the client asked to fetch it again.
pub(crate) fn refresh_after_load<W: Write>(writer: &mut W, lifecycle: &mut LifecycleManager, any_reloaded: bool) -> io::Result<()> {
    if any_reloaded {
        lifecycle.conv.clear();
        super::hierarchy::clear_impl_table_cache();
        super::navigation::clear_live_db_cache();
        if !lifecycle.is_shutdown() {
            for refresh in lifecycle.index_refresh_requests() {
                super::protocol::send_response(writer, &refresh)?;
            }
        }
    }
    Ok(())
}

pub fn dispatch_lsp_message<W: Write>(
    lsp_msg: impl Into<super::protocol::LspMessage>,
    writer: &mut W,
    lifecycle: &mut LifecycleManager,
    mr: &mut MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    cancelled_requests: &Arc<Mutex<HashSet<Value>>>,
    index_path: Option<&PathBuf>,
) -> io::Result<bool> {
    let mut msg = match lsp_msg.into() {
        super::protocol::LspMessage::InternalDiagnostic(gen, diag) => {
            if lifecycle.state() != ServerState::ShutDown && lifecycle.flycheck.is_current(gen, &diag) {
                super::presentation::reconcile::handle_internal_diagnostic(&diag, lifecycle, ov, writer)?;
                lifecycle.flycheck.clear_delivered(gen, &diag);
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
    // Not for a request admitted on the indexes refreshed for it (`fresh_loop::PINNED`): they answer it.
    if msg.as_object_mut().and_then(|m| m.remove(super::fresh_loop::PINNED)).is_none() {
        reload_changed_index(writer, lifecycle, mr, ov)?;
    }

    if super::off_token::POSITION_METHODS.contains(&msg["method"].as_str().unwrap_or("")) {
        super::off_token::snap_to_word(&mut msg, ov, &mut lifecycle.conv);
    }
    let method = msg["method"].as_str().unwrap_or("");
    let ra_commands = lifecycle.ra_client_commands();
    let ra_show_references = ra_commands.show_references;
    let conv = &mut lifecycle.conv;
    let off_token = super::off_token::POSITION_METHODS.contains(&method) && super::off_token::is_off_token(&msg, ov, conv);

    let result: super::error::LspResult<Value> = match method {
        _ if off_token => Ok(Value::Null),
        "initialize" => match lifecycle.handle_initialize(&msg) {
            Ok((capabilities, folders)) => {
                let folders = super::cargo_root::workspace_roots(folders);
                if !folders.is_empty() {
                    if index_path.is_none() {
                        mr.clear();
                    }
                    mr.open_from_folders(&folders);
                    for folder in &folders {
                        lifecycle.reindex.queue_first_index(folder);
                    }
                } else if index_path.is_none() && mr.is_empty() {
                    if let Some(cur) = std::env::current_dir().ok().and_then(|cur| super::cargo_root::workspace_roots(vec![cur]).pop()) {
                        lifecycle.reindex.queue_first_index(&cur);
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
            // The compiler's diagnostics of the code as it is, without waiting for a first save (rust-analyzer checks
            // on start too): a crate opened with an unresolved name showed no error at all.
            let roots = super::cargo_root::check_roots(lifecycle.reindex.start_queued(), mr);
            if !roots.is_empty() {
                lifecycle.flycheck.trigger_checks(roots);
            }
            if lifecycle.client_watches_files() {
                send_response(writer, &super::reindex::watch_registration(json!(lifecycle.next_refresh_id())))?;
            }
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
            // Changed on disk since it was indexed: the buffer maps onto the index through the text it was compiled from.
            if let Some(hash) = expected_hash.filter(|_| ov.get(&file).is_some_and(|o| o.compiled_text.is_none())) {
                if let Some(compiled) = crate::indexer::source_snapshots::compiled_source_snapshot(&file, hash) {
                    ov.advance_compiled_baseline(&file, compiled);
                }
            }
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
            super::presentation::session_diag::on_close(uri, &file, lifecycle, ov, writer)?;
            return Ok(true);
        }

        "textDocument/didSave" => {
            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
            let file = uri_to_path(uri);
            ov.save_document(&file);
            conv.invalidate(&file);
            if let Some(ref root) = workspace_root_of(mr, &lifecycle.reindex, Path::new(&file)) {
                // Not again when the watched-file event for this save already started the check of this text.
                if lifecycle.flycheck.mark_saved_for_check(Path::new(&file)) {
                    lifecycle.flycheck.trigger_check(root);
                }
                lifecycle.reindex.trigger_for_change(root, Path::new(&file));
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
            for root in mr.handle_did_change_workspace_folders(&msg) {
                lifecycle.reindex.index_if_missing(&root);
            }
            return Ok(true);
        }

        "workspace/didChangeWatchedFiles" => {
            // The index itself is reloaded before every message (`reload_changed_index`), with the buffers' rebase
            // and the client's refresh; a reload here would take the change from it.
            conv.clear();
            let mut to_check = Vec::new();
            for change in msg["params"]["changes"].as_array().into_iter().flatten() {
                let file = uri_to_path(change["uri"].as_str().unwrap_or(""));
                let p = Path::new(&file);
                if let Some(root) = workspace_root_of(mr, &lifecycle.reindex, p) {
                    lifecycle.reindex.trigger_for_change(&root, p);
                    if super::reindex::is_checked_source(&root, p) && lifecycle.flycheck.mark_changed_for_check(p) && !to_check.contains(&root) {
                        to_check.push(root);
                    }
                }
            }
            // A source whose text differs from the one last checked (a checkout, a pull, an edit elsewhere) is checked
            // again: the compiler's diagnostics of the text before stayed until the next save (in Zed "unused import"
            // on an import used again).
            for root in to_check {
                lifecycle.flycheck.trigger_check(&root);
            }
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
        "experimental/runnables" => Ok(crate::lsp::presentation::runnables::handle_runnables(&msg, mr.doc_ws(&msg), ov)),
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
        m if m.starts_with("ridx/") => super::ridx_methods::dispatch(m, &msg, mr.doc_ws(&msg), ov, conv),
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
