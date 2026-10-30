/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Server readiness and warmup protocol orchestration for symmetric LSP benchmarking.

use super::transport::{read_lsp_msg, resolve_file_uri, send_lsp_msg};
use super::types::SymmetricScenario;
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// Opens scenario documents via `textDocument/didOpen` simulating an active editor workspace.
pub fn open_scenario_documents<W: Write>(w: &mut W, root: &Path, scenarios: &[SymmetricScenario]) {
    let mut opened_uris = HashSet::new();
    for sc in scenarios {
        if let Some((f, _, _)) = &sc.location {
            let uri = resolve_file_uri(f, Some(root));
            if opened_uris.insert(uri.clone()) {
                let p = Path::new(f);
                let abs_path = if p.is_relative() { root.join(p) } else { p.to_path_buf() };
                if let Ok(content) = std::fs::read_to_string(&abs_path) {
                    let did_open = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/didOpen",
                        "params": {
                            "textDocument": {
                                "uri": uri,
                                "languageId": "rust",
                                "version": 1,
                                "text": content
                            }
                        }
                    });
                    let _ = send_lsp_msg(w, &did_open);
                }
            }
        }
    }
}

/// Orchestrates warmup and waits until the LSP server has reached a quiescent/ready state.
/// For rust-analyzer: listens for `experimental/serverStatus` (`quiescent: true`) and answers
/// incoming `window/workDoneProgress/create` requests.
/// Returns the elapsed warmup time in microseconds.
pub fn wait_for_server_readiness<R: BufRead, W: Write>(server_name: &str, reader: &mut R, writer: &mut W, root: &Path, scenarios: &[SymmetricScenario], timeout: Duration) -> u128 {
    let t_warmup_start = Instant::now();

    // 1. Open all scenario buffers
    open_scenario_documents(writer, root, scenarios);

    // 2. Server-specific readiness detection
    if server_name == "rust-analyzer" {
        wait_for_rust_analyzer_quiescent(reader, writer, timeout);
    } else {
        // For ridx: send an initial symbol query to confirm responsiveness
        let warmup_symbol = scenarios.first().map(|s| s.target.as_str()).unwrap_or("main");
        let warmup_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "workspace/symbol",
            "params": { "query": warmup_symbol }
        });
        let _ = send_lsp_msg(writer, &warmup_req);
        let _ = super::transport::read_lsp_response_with_writer(reader, writer, 99);
    }

    t_warmup_start.elapsed().as_micros()
}

pub fn wait_for_rust_analyzer_quiescent<R: BufRead, W: Write>(reader: &mut R, writer: &mut W, timeout: Duration) {
    let start = Instant::now();
    let mut probe_id = 9000i64;
    let mut last_probe = Instant::now();

    // Send an initial probe so RA has a pending request if already idle
    probe_id += 1;
    let probe_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": probe_id,
        "method": "workspace/symbol",
        "params": { "query": "a" }
    });
    let _ = send_lsp_msg(writer, &probe_req);

    while start.elapsed() < timeout {
        let msg = match read_lsp_msg(reader) {
            Some(m) => m,
            None => break,
        };

        // Automatically reply to server-initiated requests (e.g. workDoneProgress/create)
        if msg.get("method").is_some() {
            if let Some(srv_id) = msg.get("id") {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": srv_id,
                    "result": null
                });
                let _ = send_lsp_msg(writer, &resp);
            }

            let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
            if method == "experimental/serverStatus" {
                if let Some(quiescent) = msg.get("params").and_then(|p| p.get("quiescent")).and_then(|q| q.as_bool()) {
                    if quiescent {
                        break;
                    }
                }
            }
        } else if let Some(id) = msg.get("id").and_then(|i| i.as_i64()) {
            if id == probe_id && last_probe.elapsed() > Duration::from_millis(1500) {
                probe_id += 1;
                let probe = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": probe_id,
                    "method": "workspace/symbol",
                    "params": { "query": "a" }
                });
                let _ = send_lsp_msg(writer, &probe);
                last_probe = Instant::now();
            }
        }
    }
}
