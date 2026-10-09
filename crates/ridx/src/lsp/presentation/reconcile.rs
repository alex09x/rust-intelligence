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

/// Maps authoritative compiler diagnostics to live buffer coordinates based on overlay line shifts.
///
/// Diagnostics whose lines fall into modified, inserted, or deleted regions are dropped.
/// If the overlay is not dirty, diagnostics are returned as-is.
/// If the overlay has no compiled baseline to compare against, authoritative diagnostics
/// are omitted to avoid sending stale coordinates.
pub fn map_authoritative_diagnostics_to_live(
    auth_items: &[Value],
    overlay: Option<&crate::overlay::DocumentOverlay>,
    uri: &str,
) -> Vec<Value> {
    let Some(ov) = overlay else {
        return auth_items.to_vec();
    };

    let baseline = ov.compiled_text.as_deref().or(ov.disk_text.as_deref());
    let Some(base_text) = baseline else {
        return auth_items.to_vec();
    };

    if base_text == ov.text {
        return auth_items.to_vec();
    }

    let mut mapped = Vec::with_capacity(auth_items.len());
    for item in auth_items {
        let range = &item["range"];
        let Some(sl) = range["start"]["line"].as_u64() else {
            continue;
        };
        let Some(sc) = range["start"]["character"].as_u64() else {
            continue;
        };
        let Some(el) = range["end"]["line"].as_u64() else {
            continue;
        };
        let Some(ec) = range["end"]["character"].as_u64() else {
            continue;
        };

        let live_start = ov.map_compiled_to_live(sl as u32 + 1, sc as u32 + 1);
        let live_end = ov.map_compiled_to_live(el as u32 + 1, ec as u32 + 1);

        match (live_start, live_end) {
            (Some((new_sl, _)), Some((new_el, _))) => {
                let mut d = item.clone();
                d["range"]["start"]["line"] = json!(new_sl.saturating_sub(1));
                d["range"]["start"]["character"] = json!(sc);
                d["range"]["end"]["line"] = json!(new_el.saturating_sub(1));
                d["range"]["end"]["character"] = json!(ec);

                if let Some(related) = d
                    .get_mut("relatedInformation")
                    .and_then(|r| r.as_array_mut())
                {
                    for rel in related {
                        if rel["location"]["uri"].as_str() == Some(uri) {
                            if let (Some(r_sl), Some(r_el)) = (
                                rel["location"]["range"]["start"]["line"].as_u64(),
                                rel["location"]["range"]["end"]["line"].as_u64(),
                            ) {
                                if let (Some(m_sl), Some(m_el)) = (
                                    ov.map_compiled_line_to_live(r_sl as u32 + 1),
                                    ov.map_compiled_line_to_live(r_el as u32 + 1),
                                ) {
                                    rel["location"]["range"]["start"]["line"] =
                                        json!(m_sl.saturating_sub(1));
                                    rel["location"]["range"]["end"]["line"] =
                                        json!(m_el.saturating_sub(1));
                                }
                            }
                        }
                    }
                }

                mapped.push(d);
            }
            _ => {
                // Diagnostic line falls in modified/inserted/deleted region; drop it.
                continue;
            }
        }
    }
    mapped
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
    let ov_opt = overlay_mgr.get(&file);
    let file_text;
    let effective_text = if let Some(ov) = ov_opt {
        Some(ov.text.as_str())
    } else {
        file_text = std::fs::read_to_string(&file).ok();
        file_text.as_deref()
    };

    let Some(text) = effective_text else {
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

    let heuristic = handle_document_diagnostic_text(text);
    let h_items = heuristic["items"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let raw_auth = flycheck.get_latest_diagnostics(uri);
    let auth_items = map_authoritative_diagnostics_to_live(&raw_auth, ov_opt, uri);
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
