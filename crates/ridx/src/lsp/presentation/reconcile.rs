/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Diagnostic reconciliation between heuristic editor checks and authoritative flycheck results.

use super::diagnostics::handle_document_diagnostic_text;
use super::flycheck::FlycheckState;
use crate::lsp::protocol::{send_response, uri_to_path};
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::io::Write;

/// Checks if two diagnostic ranges overlap.
pub fn ranges_overlap(r1: &Value, r2: &Value) -> bool {
    let l1_start = r1["start"]["line"].as_u64().unwrap_or(0);
    let l1_end = r1["end"]["line"].as_u64().unwrap_or(l1_start);
    let l2_start = r2["start"]["line"].as_u64().unwrap_or(0);
    let l2_end = r2["end"]["line"].as_u64().unwrap_or(l2_start);

    if l1_start > l2_end || l2_start > l1_end {
        return false;
    }

    if l1_start == l1_end && l2_start == l2_end && l1_start == l2_start {
        let c1_start = r1["start"]["character"].as_u64().unwrap_or(0);
        let c1_end = r1["end"]["character"].as_u64().unwrap_or(c1_start);
        let c2_start = r2["start"]["character"].as_u64().unwrap_or(0);
        let c2_end = r2["end"]["character"].as_u64().unwrap_or(c2_start);

        c1_start.max(c2_start) <= c1_end.min(c2_end)
    } else {
        true
    }
}

/// Reconciles heuristic in-memory diagnostics with authoritative flycheck diagnostics.
///
/// If an authoritative `rustc` diagnostic covers an overlapping range with the same error code,
/// the heuristic entry is suppressed to prevent duplicated error highlights in the editor.
/// Multiple distinct errors on the same line (non-overlapping ranges) are preserved.
pub fn reconcile_diagnostics(heuristic: &[Value], authoritative: &[Value]) -> Vec<Value> {
    let mut result = authoritative.to_vec();
    for h in heuristic {
        let h_range = &h["range"];
        let h_code = h.get("code").and_then(|c| c.as_str());

        let is_shadowed = authoritative.iter().any(|a| {
            let a_range = &a["range"];
            let a_code = a.get("code").and_then(|c| c.as_str());
            a_code == h_code && ranges_overlap(h_range, a_range)
        });

        if !is_shadowed {
            result.push(h.clone());
        }
    }
    result
}

/// Publishes reconciled diagnostics for a document URI to an LSP writer.
///
/// Sends a `textDocument/publishDiagnostics` notification when diagnostics are non-empty,
/// or when previously active diagnostics for this document need to be cleared.
pub fn publish_reconciled_diagnostics<W: Write>(
    uri: &str,
    version: Option<i32>,
    overlay_mgr: &OverlayManager,
    flycheck: &FlycheckState,
    writer: &mut W,
) -> std::io::Result<()> {
    let file = uri_to_path(uri);
    let text = if let Some(ov) = overlay_mgr.get(&file) {
        Some(ov.text.clone())
    } else {
        std::fs::read_to_string(&file).ok()
    };

    let Some(doc_text) = text else {
        if flycheck.clear_published(uri) {
            let clear_notif = json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": {
                    "uri": uri,
                    "version": version,
                    "diagnostics": []
                }
            });
            send_response(writer, &clear_notif)?;
        }
        return Ok(());
    };

    let heuristic = handle_document_diagnostic_text(&doc_text);
    let h_items = heuristic["items"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let auth_items = flycheck.get_latest_diagnostics(uri);
    let merged = reconcile_diagnostics(h_items, &auth_items);

    if !merged.is_empty() {
        flycheck.mark_published(uri);
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "version": version,
                "diagnostics": merged
            }
        });
        send_response(writer, &notif)?;
    } else if flycheck.clear_published(uri) {
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "version": version,
                "diagnostics": []
            }
        });
        send_response(writer, &notif)?;
    }

    Ok(())
}
