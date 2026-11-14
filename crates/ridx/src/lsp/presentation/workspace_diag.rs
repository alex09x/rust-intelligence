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
//!
//! Clients poll `workspace/diagnostic` continuously (Zed asks again as soon as an answer arrives), so an answer
//! for an unchanged workspace must be cheap: every report carries a `resultId` (a hash of its items), a file whose
//! items match the id the client sent back is reported `unchanged`, and a closed file is read and checked again
//! only when its size, modification time or compiler diagnostics change.

use super::diagnostics::handle_document_diagnostic_text;
use crate::lsp::protocol::{path_to_uri, uri_to_path};
use crate::lsp::workspace::MultiRootWorkspace;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

pub fn handle_workspace_diagnostic(msg: &Value, overlay_mgr: &OverlayManager, multi_root: &MultiRootWorkspace) -> Value {
    handle_workspace_diagnostic_with_flycheck(msg, overlay_mgr, multi_root, None)
}

/// What a closed file's items were computed from, and the items.
struct ClosedEntry {
    stamp: (Option<SystemTime>, u64, u64),
    items: Vec<Value>,
}

/// Bounded like the other per-path caches: a workspace with more files than this is checked afresh past it.
const MAX_CLOSED_ENTRIES: usize = 16_384;

fn closed_cache() -> &'static Mutex<HashMap<PathBuf, ClosedEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, ClosedEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hash_of<T: Hash + ?Sized>(v: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// The `resultId` of a report with `items`.
pub(crate) fn result_id(items: &[Value]) -> String {
    format!("{:016x}", hash_of(&serde_json::to_string(items).unwrap_or_default()))
}

/// The ids the client holds per document (`previousResultIds`).
fn previous_ids(msg: &Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(prev) = msg["params"]["previousResultIds"].as_array() {
        for p in prev {
            if let (Some(uri), Some(id)) = (p["uri"].as_str(), p["value"].as_str()) {
                out.insert(uri.to_string(), id.to_string());
            }
        }
    }
    out
}

/// A full report, or `unchanged` when the client already holds these items.
fn report(uri: &str, version: Value, items: Vec<Value>, prev: &HashMap<String, String>) -> Value {
    let id = result_id(&items);
    if prev.get(uri) == Some(&id) {
        return json!({ "uri": uri, "version": version, "kind": "unchanged", "resultId": id });
    }
    json!({ "uri": uri, "version": version, "kind": "full", "resultId": id, "items": items })
}

pub fn handle_workspace_diagnostic_with_flycheck(
    msg: &Value,
    overlay_mgr: &OverlayManager,
    multi_root: &MultiRootWorkspace,
    flycheck: Option<&super::flycheck::FlycheckState>,
) -> Value {
    let prev = previous_ids(msg);
    let mut reports = Vec::new();
    let mut seen_uris = HashSet::new();

    let build_items = |uri: &str, text: &str| -> Vec<Value> {
        let diag_resp = handle_document_diagnostic_text(text);
        let h_items = diag_resp.get("items").and_then(|i| i.as_array()).map(|a| a.as_slice()).unwrap_or(&[]);
        if let Some(fc) = flycheck {
            let auth = fc.get_latest_diagnostics(uri);
            let baseline = fc.get_diagnostic_baseline(uri);
            let mapped_auth = super::reconcile::map_authoritative_diagnostics_to_live(&auth, baseline.as_deref(), text, uri);
            super::reconcile::reconcile_diagnostics(h_items, &mapped_auth)
        } else {
            h_items.to_vec()
        }
    };
    let auth_hash = |uri: &str| flycheck.map_or(0, |fc| hash_of(&serde_json::to_string(&fc.get_latest_diagnostics(uri)).unwrap_or_default()));

    // A closed file's items, recomputed only when the file or its compiler diagnostics changed.
    let closed_items = |path: &Path, uri: &str| -> Option<Vec<Value>> {
        let meta = std::fs::metadata(path).ok()?;
        let stamp = (meta.modified().ok(), meta.len(), auth_hash(uri));
        if let Some(e) = closed_cache().lock().unwrap_or_else(|e| e.into_inner()).get(path) {
            if e.stamp == stamp {
                return Some(e.items.clone());
            }
        }
        let text = std::fs::read_to_string(path).ok()?;
        let items = build_items(uri, &text);
        let mut cache = closed_cache().lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= MAX_CLOSED_ENTRIES {
            cache.clear();
        }
        cache.insert(path.to_path_buf(), ClosedEntry { stamp, items: items.clone() });
        Some(items)
    };

    // 1. Diagnostics for open document overlays
    for file_path in overlay_mgr.open_file_paths() {
        if let Some(overlay) = overlay_mgr.get(&file_path) {
            seen_uris.insert(overlay.uri.clone());
            let items = build_items(&overlay.uri, &overlay.text);
            reports.push(report(&overlay.uri, json!(overlay.version), items, &prev));
        }
    }

    // 2. Discover closed source files known to the workspace index
    for ws in multi_root.all_workspaces() {
        for (i, reader) in ws.readers().iter().enumerate() {
            if ws.is_reader_external(i) {
                continue;
            }
            let origin = ws.source_root_at(i).or_else(|| ws.workspace_root()).or_else(|| ws.paths().get(i).and_then(|p| p.parent()));
            for fid in 0..reader.files_count() {
                let rel = reader.get_file(fid);
                if rel.is_empty() {
                    continue;
                }
                let path = if Path::new(rel).is_absolute() {
                    PathBuf::from(rel)
                } else if let Some(root) = origin {
                    root.join(rel)
                } else {
                    PathBuf::from(rel)
                };
                // Workspace crate readers can reference external files (e.g. macro expansions in dependencies).
                if ridx_format::is_external_dependency_path(&path) || ws.is_external_path(&path) {
                    continue;
                }
                let uri = path_to_uri(&path.to_string_lossy());
                if seen_uris.insert(uri.clone()) {
                    if let Some(items) = closed_items(&path, &uri) {
                        reports.push(report(&uri, Value::Null, items, &prev));
                    }
                }
            }
        }
    }

    // 3. Any previousResultIds sent by the client
    for uri in prev.keys() {
        if seen_uris.insert(uri.clone()) {
            let file = uri_to_path(uri);
            if let Some(items) = closed_items(Path::new(&file), uri) {
                reports.push(report(uri, Value::Null, items, &prev));
            }
        }
    }

    json!({ "items": reports })
}
