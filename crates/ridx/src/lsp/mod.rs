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

mod cargo_root;
pub mod dispatch;
pub mod editing;
pub mod error;
mod folders;
mod fresh;
mod fresh_loop;
pub mod handlers;
pub mod hierarchy;
mod index_progress;
pub mod intelligence;
pub mod lifecycle;
pub mod logger;
pub mod navigation;
mod off_token;
pub mod params;
pub mod presentation;
pub mod protocol;
pub mod reindex;
mod requests;
mod session_index;
mod session_loop;
pub mod symbols;
pub mod workspace;

#[cfg(test)]
pub mod server_tests;
#[cfg(test)]
pub mod tests;

#[allow(unused_imports)]
pub use {
    dispatch::*, editing::*, error::*, handlers::*, hierarchy::*, intelligence::*, lifecycle::*, navigation::*, params::*, presentation::*, protocol::*, symbols::*, workspace::*,
};

use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Takes the text the index compiled as the base of each open buffer it describes: what was typed since is mapped onto
/// it.
pub(crate) fn rebase_open_buffers(ws: &WorkspaceIndex, overlay_mgr: &mut crate::overlay::OverlayManager) {
    overlay_mgr.mark_recorded(|file| ws.source_hash_for_file(file).is_some());
    overlay_mgr.rebase_on_index(|file| ws.units_agree_on_source(file).then(|| ws.source_hash_for_file(file)).flatten());
}

pub(crate) fn sync_reloaded_index(ws: &mut WorkspaceIndex, overlay_mgr: &mut crate::overlay::OverlayManager) -> bool {
    if let Ok(true) = ws.reload_manifest_if_modified() {
        rebase_open_buffers(ws, overlay_mgr);
        true
    } else {
        false
    }
}

/// Runs the LSP server over standard I/O.
pub fn run_lsp_server(index_path: Option<PathBuf>) -> io::Result<()> {
    let reader = io::BufReader::new(io::stdin());
    let stdout = io::stdout();
    run_lsp_server_stream(reader, stdout.lock(), index_path)
}

/// Runs the LSP server over arbitrary reader and writer streams.
pub fn run_lsp_server_stream<R: BufRead + Send + 'static, W: Write>(mut reader: R, writer: W, index_path: Option<PathBuf>) -> io::Result<()> {
    let mut multi_root = MultiRootWorkspace::new();
    if let Some(ref p) = index_path {
        multi_root.add_root(p.clone());
    }

    let mut overlay_mgr = crate::overlay::OverlayManager::new();
    let mut lifecycle = LifecycleManager::new();
    let cancelled_requests: Arc<Mutex<HashSet<Value>>> = Arc::new(Mutex::new(HashSet::new()));

    let (tx, rx) = std::sync::mpsc::sync_channel::<Value>(64);
    let (tx_diag, rx_diag) = std::sync::mpsc::channel::<(u64, Value)>();

    lifecycle.flycheck.set_notifier(move |gen, notif| {
        let _ = tx_diag.send((gen, notif));
    });
    let cancelled_reader = Arc::clone(&cancelled_requests);
    let requests = Arc::new(requests::Requests::default());
    let requests_reader = Arc::clone(&requests);
    let mut writer = requests::RetiringWriter::new(writer, Arc::clone(&requests), Arc::clone(&cancelled_requests));

    std::thread::Builder::new().name("lsp-reader".into()).spawn(move || loop {
        let mut content_length: Option<usize> = None;
        let mut header_line = String::new();

        loop {
            header_line.clear();
            let bytes_read = match reader.read_line(&mut header_line) {
                Ok(b) => b,
                Err(_) => return,
            };
            if bytes_read == 0 {
                return;
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
            requests_reader.read(&msg, &cancelled_reader);
            if tx.send(msg).is_err() {
                return;
            }
        }
    })?;

    let mut index_progress = index_progress::IndexProgress::default();
    let session = session_index::SessionIndexer::default();
    let mut gate = fresh_loop::Gate::default();
    let mut released = std::collections::VecDeque::new();
    loop {
        // A background reindex ended: load it now, rather than on the client's next request.
        let finished = lifecycle.reindex.take_finished();
        // The index of the edited buffers: it answers once built, until the next published index loads.
        let mut touched = Vec::new();
        if session_loop::step(&session, &finished, &mut multi_root, &mut overlay_mgr, &lifecycle.flycheck, &mut touched) && !lifecycle.is_shutdown() {
            dispatch::refresh_after_load(&mut writer, &mut lifecycle, true)?;
        }
        presentation::session_diag::publish_session_files(&touched, &lifecycle, &overlay_mgr, &mut writer)?;
        if !finished.is_empty() {
            dispatch::load_finished_index(&mut writer, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &finished)?;
        }
        let (running, ended) = lifecycle.reindex.progress_snapshot();
        if lifecycle.client_work_done_progress && !lifecycle.is_shutdown() {
            index_progress.sync(&mut writer, running, ended, || lifecycle.next_refresh_id(), |r| lifecycle.reindex.status_of(r).1)?;
        }
        while let Ok((gen, diag)) = rx_diag.try_recv() {
            dispatch::dispatch_lsp_message(
                protocol::LspMessage::InternalDiagnostic(gen, diag),
                &mut writer,
                &mut lifecycle,
                &mut multi_root,
                &mut overlay_mgr,
                &cancelled_requests,
                index_path.as_ref(),
            )?;
        }

        // Requests waiting for an index that covers their document (`minVersion`): the ones covered now go first.
        released.extend(fresh_loop::release_waiting(&mut gate, &mut writer, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &session, &cancelled_requests)?);
        let next = match released.pop_front() {
            Some(msg) => Ok(msg),
            None => rx.recv_timeout(std::time::Duration::from_millis(20)),
        };
        match next {
            Ok(msg) => {
                session_loop::note_edit(&session, &msg, &multi_root, &lifecycle.reindex);
                let Some(msg) = fresh_loop::intercept(msg, &mut gate, &mut writer, &mut lifecycle, &mut multi_root, &mut overlay_mgr, &session, &cancelled_requests)? else {
                    continue;
                };
                // A panic in one handler answers that request with an internal error instead of ending the session:
                // every later request would otherwise go unanswered.
                let id = msg.get("method").and(msg.get("id")).cloned();
                let initialize = (msg["method"] == "initialize" && !lifecycle.is_initialized()).then(|| msg.clone());
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    dispatch::dispatch_lsp_message(
                        protocol::LspMessage::Client(msg),
                        &mut writer,
                        &mut lifecycle,
                        &mut multi_root,
                        &mut overlay_mgr,
                        &cancelled_requests,
                        index_path.as_ref(),
                    )
                }));
                let should_continue = match outcome {
                    Ok(result) => result?,
                    Err(panic) => {
                        let what = panic.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_default();
                        logger::log_info(&format!("request {method} panicked: {what}"));
                        // The panic may have left a live database half updated (a cache taken out, not put back).
                        navigation::clear_live_db_cache();
                        if let Some(id) = &id {
                            protocol::send_response(
                                &mut writer,
                                &serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": format!("{method} failed: {what}") } }),
                            )?;
                        }
                        true
                    }
                };
                // The roots of the session are those of an `initialize` accepted (not one rejected or cancelled).
                if let Some(initialize) = initialize.filter(|_| lifecycle.is_initialized()) {
                    gate.initialized(&initialize, &lifecycle.reindex);
                }
                if !should_continue {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                while let Ok((gen, diag)) = rx_diag.try_recv() {
                    let _ = dispatch::dispatch_lsp_message(
                        protocol::LspMessage::InternalDiagnostic(gen, diag),
                        &mut writer,
                        &mut lifecycle,
                        &mut multi_root,
                        &mut overlay_mgr,
                        &cancelled_requests,
                        index_path.as_ref(),
                    );
                }
                break;
            }
        }
    }

    session.shutdown();
    Ok(())
}
