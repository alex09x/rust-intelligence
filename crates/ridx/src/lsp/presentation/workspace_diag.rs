/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP workspace diagnostics collection across open overlays and known files.

use super::diagnostics::handle_document_diagnostic_text;
use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

pub fn handle_workspace_diagnostic(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let mut reports = Vec::new();
    let mut seen_uris = std::collections::HashSet::new();

    for file_path in overlay_mgr.open_file_paths() {
        if let Some(overlay) = overlay_mgr.get(&file_path) {
            seen_uris.insert(overlay.uri.clone());
            let diag_resp = handle_document_diagnostic_text(&overlay.text);
            let items = diag_resp.get("items").cloned().unwrap_or_else(|| json!([]));
            reports.push(json!({
                "uri": overlay.uri,
                "version": overlay.version,
                "kind": "full",
                "items": items,
            }));
        }
    }

    if let Some(prev_ids) = msg
        .get("params")
        .and_then(|p| p.get("previousResultIds"))
        .and_then(|p| p.as_array())
    {
        for prev in prev_ids {
            if let Some(uri) = prev.get("uri").and_then(|u| u.as_str()) {
                if seen_uris.insert(uri.to_string()) {
                    let file = uri_to_path(uri);
                    if let Ok(text) = std::fs::read_to_string(&file) {
                        let diag_resp = handle_document_diagnostic_text(&text);
                        let items = diag_resp.get("items").cloned().unwrap_or_else(|| json!([]));
                        reports.push(json!({
                            "uri": uri,
                            "version": Value::Null,
                            "kind": "full",
                            "items": items,
                        }));
                    }
                }
            }
        }
    }

    json!({ "items": reports })
}
