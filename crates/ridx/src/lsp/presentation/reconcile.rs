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

/// The text of `file` as last saved, which compiler diagnostics are placed in for a client that moves them through
/// unsaved edits itself (see `compiler_merge`).
pub fn saved_text(overlay_mgr: &OverlayManager, file: &str) -> Option<String> {
    std::fs::read_to_string(file).ok().or_else(|| overlay_mgr.get(file).and_then(|ov| ov.disk_text.clone()))
}

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

/// Maps compiler diagnostics from the text the check read (`baseline_text`) to `live_text`, by line shifts; callers
/// pass the file as saved (see [`saved_text`]).
///
/// Diagnostics whose lines fall into modified, inserted, or deleted regions are dropped.
/// If `baseline_text` is None or equals `live_text`, diagnostics are returned as-is.
pub fn map_authoritative_diagnostics_to_text(auth_items: &[Value], baseline_text: Option<&str>, live_text: &str, uri: &str) -> Vec<Value> {
    let Some(base_text) = baseline_text else {
        return auth_items.to_vec();
    };

    if base_text == live_text {
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

        let live_start = crate::overlay::map_text_line_to_live(base_text, live_text, sl as u32 + 1);
        let live_end = crate::overlay::map_text_line_to_live(base_text, live_text, el as u32 + 1);

        match (live_start, live_end) {
            (Some(new_sl), Some(new_el)) => {
                let mut d = item.clone();
                d["range"]["start"]["line"] = json!(new_sl.saturating_sub(1));
                d["range"]["start"]["character"] = json!(sc);
                d["range"]["end"]["line"] = json!(new_el.saturating_sub(1));
                d["range"]["end"]["character"] = json!(ec);

                if let Some(related) = d.get_mut("relatedInformation").and_then(|r| r.as_array_mut()) {
                    for rel in related {
                        if rel["location"]["uri"].as_str() == Some(uri) {
                            if let (Some(r_sl), Some(r_el)) = (rel["location"]["range"]["start"]["line"].as_u64(), rel["location"]["range"]["end"]["line"].as_u64()) {
                                if let (Some(m_sl), Some(m_el)) = (
                                    crate::overlay::map_text_line_to_live(base_text, live_text, r_sl as u32 + 1),
                                    crate::overlay::map_text_line_to_live(base_text, live_text, r_el as u32 + 1),
                                ) {
                                    rel["location"]["range"]["start"]["line"] = json!(m_sl.saturating_sub(1));
                                    rel["location"]["range"]["end"]["line"] = json!(m_el.saturating_sub(1));
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
pub fn publish_reconciled_diagnostics<W: Write>(uri: &str, version: Option<i32>, overlay_mgr: &OverlayManager, flycheck: &FlycheckState, writer: &mut W) -> std::io::Result<()> {
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
    let h_items = heuristic["items"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let merged = super::compiler_merge::merge_with_compiler(uri, h_items, text, || saved_text(overlay_mgr, &file), flycheck);

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

/// Dispatches an internal compiler diagnostic notification from flycheck.
///
/// Updates the cached latest diagnostics and text baseline. If the client declared
/// pull diagnostics capability, suppresses push publishDiagnostics and emits
/// `workspace/diagnostic/refresh` (if refreshSupport is enabled). Otherwise, pushes
/// reconciled diagnostics to the client.
pub fn handle_internal_diagnostic<W: Write>(
    diag: &Value,
    lifecycle: &crate::lsp::lifecycle::LifecycleManager,
    overlay_mgr: &OverlayManager,
    writer: &mut W,
) -> std::io::Result<()> {
    let uri = diag["params"]["uri"].as_str().unwrap_or("");
    let diags = diag["params"]["diagnostics"].as_array().cloned().unwrap_or_default();
    // The check's key names the path as it is, not a URI to decode (review of 2810515d).
    let file = super::compiler_merge::check_path(uri).to_string_lossy().into_owned();
    // The text the check read, sent with its diagnostics; the buffer's idea of the disk may be newer.
    let baseline = (!diags.is_empty())
        .then(|| diag["params"]["baseline"].as_str().map(str::to_string))
        .flatten()
        .or_else(|| (!diags.is_empty()).then(|| overlay_mgr.get(&file).and_then(|ov| ov.disk_text.clone())).flatten())
        .or_else(|| (!diags.is_empty()).then(|| std::fs::read_to_string(&file).ok()).flatten());
    lifecycle.flycheck.set_latest_diagnostics_with_baseline(uri, diags, baseline);
    if lifecycle.client_pull_diagnostics() {
        if lifecycle.client_refresh_diagnostics() {
            let refresh = json!({
                "jsonrpc": "2.0",
                "method": "workspace/diagnostic/refresh",
                "id": lifecycle.next_refresh_id(),
                "params": {}
            });
            send_response(writer, &refresh)?;
        }
    } else {
        super::session_diag::publish_check(uri, lifecycle, overlay_mgr, writer)?;
    }
    Ok(())
}
