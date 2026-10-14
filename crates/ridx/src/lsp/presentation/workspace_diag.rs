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
use crate::lsp::protocol::{path_to_uri, uri_to_path};
use crate::lsp::workspace::MultiRootWorkspace;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::path::Path;

pub fn handle_workspace_diagnostic(
    msg: &Value,
    overlay_mgr: &OverlayManager,
    multi_root: &MultiRootWorkspace,
) -> Value {
    handle_workspace_diagnostic_with_flycheck(msg, overlay_mgr, multi_root, None)
}

pub fn handle_workspace_diagnostic_with_flycheck(
    msg: &Value,
    overlay_mgr: &OverlayManager,
    multi_root: &MultiRootWorkspace,
    flycheck: Option<&super::flycheck::FlycheckState>,
) -> Value {
    let mut reports = Vec::new();
    let mut seen_uris = std::collections::HashSet::new();

    let build_items = |uri: &str, text: &str| -> Vec<Value> {
        let diag_resp = handle_document_diagnostic_text(text);
        let h_items = diag_resp
            .get("items")
            .and_then(|i| i.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        if let Some(fc) = flycheck {
            let auth = fc.get_latest_diagnostics(uri);
            let baseline = fc.get_diagnostic_baseline(uri);
            let mapped_auth = super::reconcile::map_authoritative_diagnostics_to_live(
                &auth,
                baseline.as_deref(),
                text,
                uri,
            );
            super::reconcile::reconcile_diagnostics(h_items, &mapped_auth)
        } else {
            h_items.to_vec()
        }
    };

    // 1. Diagnostics for open document overlays
    for file_path in overlay_mgr.open_file_paths() {
        if let Some(overlay) = overlay_mgr.get(&file_path) {
            seen_uris.insert(overlay.uri.clone());
            let items = build_items(&overlay.uri, &overlay.text);
            reports.push(json!({
                "uri": overlay.uri,
                "version": overlay.version,
                "kind": "full",
                "items": items,
            }));
        }
    }

    // 2. Discover closed source files known to the workspace index
    for ws in multi_root.all_workspaces() {
        for (i, reader) in ws.readers().iter().enumerate() {
            if ws.is_reader_external(i) {
                continue;
            }
            let origin = ws
                .source_root_at(i)
                .or_else(|| ws.workspace_root())
                .or_else(|| ws.paths().get(i).and_then(|p| p.parent()));
            for fid in 0..reader.files_count() {
                let rel = reader.get_file(fid);
                if rel.is_empty() {
                    continue;
                }
                let path = if Path::new(rel).is_absolute() {
                    std::path::PathBuf::from(rel)
                } else if let Some(root) = origin {
                    root.join(rel)
                } else {
                    std::path::PathBuf::from(rel)
                };
                let path_str = path.to_string_lossy().to_string();
                let uri = path_to_uri(&path_str);
                if seen_uris.insert(uri.clone()) {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        let items = build_items(&uri, &text);
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

    // 3. Any previousResultIds sent by the client
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
                        let items = build_items(uri, &text);
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
