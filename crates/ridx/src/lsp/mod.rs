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
pub mod server_tests;
#[cfg(test)]
pub mod tests;

#[allow(unused_imports)]
pub use {
    dispatch::*, editing::*, handlers::*, hierarchy::*, intelligence::*, lifecycle::*,
    navigation::*, presentation::*, protocol::*, symbols::*, workspace::*,
};

use ridx_format::WorkspaceIndex;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(crate) fn sync_reloaded_index(
    ws: &mut WorkspaceIndex,
    overlay_mgr: &mut crate::overlay::OverlayManager,
) {
    if let Ok(true) = ws.reload_if_modified() {
        for file in overlay_mgr.open_file_paths() {
            if let Ok(content) = std::fs::read_to_string(&file) {
                let disk_hash = ridx_format::normalized_source_hash(content.as_bytes());
                if ws.source_hash_for_file(&file) == Some(disk_hash) {
                    overlay_mgr.advance_compiled_baseline(&file, content);
                }
            }
        }
    }
}

/// Runs the LSP server over standard I/O.
pub fn run_lsp_server(index_path: Option<PathBuf>) -> io::Result<()> {
    let reader = io::BufReader::new(io::stdin());
    let stdout = io::stdout();
    run_lsp_server_stream(reader, stdout.lock(), index_path)
}

/// Runs the LSP server over arbitrary reader and writer streams.
pub fn run_lsp_server_stream<R: BufRead + Send + 'static, W: Write>(
    mut reader: R,
    mut writer: W,
    index_path: Option<PathBuf>,
) -> io::Result<()> {
    let mut multi_root = MultiRootWorkspace::new();
    if let Some(ref p) = index_path {
        multi_root.add_root(p.clone());
    } else if let Ok(auto_ws) = WorkspaceIndex::auto_discover() {
        let cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        multi_root.add_root(cur);
        let _ = auto_ws;
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

    std::thread::Builder::new()
        .name("lsp-reader".into())
        .spawn(move || loop {
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
                let should_continue = dispatch::dispatch_lsp_message(
                    protocol::LspMessage::Client(msg),
                    &mut writer,
                    &mut lifecycle,
                    &mut multi_root,
                    &mut overlay_mgr,
                    &cancelled_requests,
                    index_path.as_ref(),
                )?;
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
