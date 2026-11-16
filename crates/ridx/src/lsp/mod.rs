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

pub mod dispatch;
pub mod editing;
pub mod error;
pub mod handlers;
pub mod hierarchy;
pub mod intelligence;
pub mod lifecycle;
pub mod logger;
pub mod navigation;
mod off_token;
pub mod params;
pub mod presentation;
pub mod protocol;
pub mod reindex;
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

pub(crate) fn sync_reloaded_index(ws: &mut WorkspaceIndex, overlay_mgr: &mut crate::overlay::OverlayManager) -> bool {
    if let Ok(true) = ws.reload_manifest_if_modified() {
        for file in overlay_mgr.open_file_paths() {
            if let Ok(content) = std::fs::read_to_string(&file) {
                let disk_hash = ridx_format::normalized_source_hash(content.as_bytes());
                if ws.source_hash_for_file(&file) == Some(disk_hash) {
                    overlay_mgr.advance_compiled_baseline(&file, content);
                }
            }
        }
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
pub fn run_lsp_server_stream<R: BufRead + Send + 'static, W: Write>(mut reader: R, mut writer: W, index_path: Option<PathBuf>) -> io::Result<()> {
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
            if msg["method"] == "$/cancelRequest" {
                let id_raw = msg["params"]["id"].clone();
                cancelled_reader.lock().unwrap().insert(id_raw);
            }
            if tx.send(msg).is_err() {
                return;
            }
        }
    })?;

    loop {
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

        match rx.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(msg) => {
                // A panic in one handler answers that request with an internal error instead of ending the session:
                // every later request would otherwise go unanswered.
                let id = msg.get("method").and(msg.get("id")).cloned();
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
                        if let Some(id) = id {
                            protocol::send_response(
                                &mut writer,
                                &serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": format!("{method} failed: {what}") } }),
                            )?;
                        }
                        true
                    }
                };
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

    Ok(())
}
