/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/status`: the roots of the session, their indexes and sessions, and a document's coverage.

use super::{answering_hash, diagnostics_of, owning_root};
use crate::lsp::folders::Folders;
use crate::lsp::fresh::Coverage;
use crate::lsp::presentation::session_diag::Readiness;
use crate::lsp::session_index::SessionIndexer;
use serde_json::{json, Value};
use std::path::PathBuf;

/// `ridx/status`: every root of the session and its index, and the given document's coverage.
pub fn status(
    msg: &Value,
    folders: &Folders,
    mr: &crate::lsp::MultiRootWorkspace,
    ov: &crate::overlay::OverlayManager,
    reindex: &crate::lsp::reindex::Reindexer,
    session: &SessionIndexer,
    flycheck: &crate::lsp::presentation::flycheck::FlycheckState,
) -> Value {
    // The roots with an index loaded, and the client's folders' and those indexed this session without one, being
    // indexed for the first time or whose first index failed (review of a6909dbe: they were missing, with the reason).
    let mut roots: Vec<(PathBuf, Option<String>)> = mr
        .all_workspaces()
        .filter_map(|ws| ws.workspace_root().map(|r| (ridx_format::canonicalize_best_effort(r), Some(format!("{}.{}", ws.instance_id(), ws.generation())))))
        .collect();
    for root in folders.roots().into_iter().chain(reindex.roots()) {
        if !roots.iter().any(|(r, _)| *r == root) {
            roots.push((root, None));
        }
    }
    roots.sort();
    let roots: Vec<Value> = roots
        .into_iter()
        .map(|(root, generation)| {
            let (building, error) = reindex.status_of(&root);
            let (session_building, session_answers) = session.state_of(&root);
            let (changed, building_ms) = reindex.staleness_of(&root);
            json!({
                "root": root.display().to_string(),
                "indexed": generation.is_some(),
                "generation": generation,
                "building": building,
                // A change recorded that no loaded index covers yet: answers from the index may be of the text before
                // it (prod-code, m=20734).
                "pendingChanges": reindex.pending_of(&root),
                // Which files those are (at most 100) and how many, and how long the current run has been going: answers
                // from the loaded index are of the text before them (prod-code #2101).
                "changedFiles": changed.iter().take(100).map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "changedFileCount": changed.len(),
                "buildingForMs": building_ms,
                "error": error,
                "sessionIndex": { "answers": session_answers, "building": session_building },
            })
        })
        .collect();
    let mut out = json!({ "roots": roots });
    if let Some(uri) = msg["params"]["textDocument"]["uri"].as_str() {
        let file = crate::lsp::protocol::uri_to_path(uri);
        // The workspace whose builds the document's diagnostics wait for, as `diagnostics` says (review of 775998ea: a
        // loaded enclosing workspace was idle while the nested one built).
        let root = owning_root(mr, reindex, &file);
        let building = root.as_ref().is_some_and(|r| session.state_of(r).0 || reindex.status_of(r).0);
        let cov = Coverage {
            ov,
            indexed_hash: &|f| answering_hash(mr, f),
            diagnostics: &|f| diagnostics_of(flycheck, ov, mr, reindex, f),
            first_index: &|f| super::first_index_of(f, mr, reindex),
        };
        // Whether the compiler's diagnostics of the buffer's text are known (prod-code, m=20178).
        let diagnostics = match (cov.diagnostics)(&file) {
            Readiness::Ready(v) => json!({ "state": "ready", "version": v }),
            Readiness::Unavailable(v, why) => json!({ "state": "unavailable", "version": v, "reason": why }),
            Readiness::Pending => json!({ "state": "pending", "version": null }),
        };
        out["document"] = match ov.get(&file) {
            Some(o) => json!({ "version": o.version, "coveredVersion": cov.covered_version(&file).flatten(), "building": building, "diagnostics": diagnostics }),
            None => json!({ "version": null, "coveredVersion": null, "building": building, "diagnostics": null }),
        };
    }
    out
}
