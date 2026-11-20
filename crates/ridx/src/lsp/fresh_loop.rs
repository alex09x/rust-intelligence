/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The server loop's side of `minVersion` (`fresh`) and `ridx/status`: what is answered before the ordinary dispatch,
//! and against which index.

use super::fresh::{Coverage, Waiting};
use super::session_index::SessionIndexer;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The hash of `file` the index answering for it recorded, when all its units agree on it: one unit compiled from the
/// buffer's text and another from the text before is no coverage (review of e535e9cf).
pub(crate) fn answering_hash(mr: &super::MultiRootWorkspace, file: &str) -> Option<u64> {
    let ws = mr.find_workspace_for_path(Path::new(file))?;
    ws.units_agree_on_source(file).then(|| ws.source_hash_for_file(file)).flatten()
}

/// The indexes as they are now: a root of the session whose first index was published since (by `ridx index` run by
/// anyone; review of e535e9cf) is loaded, and a loaded one whose manifest changed is reloaded.
fn refresh<W: Write>(writer: &mut W, lifecycle: &mut super::LifecycleManager, mr: &mut super::MultiRootWorkspace, ov: &mut crate::overlay::OverlayManager) -> io::Result<()> {
    let loaded: HashSet<PathBuf> = mr.all_workspaces().filter_map(|ws| ws.workspace_root()).map(ridx_format::canonicalize_best_effort).collect();
    for root in lifecycle.reindex.roots() {
        if !loaded.contains(&root) && root.join("target/ridx/ridx_manifest.json").is_file() && mr.add_root(root.clone()) {
            if let Some(ws) = mr.all_workspaces().last() {
                super::rebase_open_buffers(ws, ov);
            }
        }
    }
    super::dispatch::reload_changed_index(writer, lifecycle, mr, ov)
}

/// The server loop's turn for waiting requests: after `shutdown` they are answered `-32600`; otherwise the indexes are
/// refreshed and the ended or covered ones come out. Returns the requests to dispatch (through `intercept` again).
pub(crate) fn release_waiting<W: Write>(
    waiting: &mut Waiting,
    writer: &mut W,
    lifecycle: &mut super::LifecycleManager,
    mr: &mut super::MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    cancelled: &Mutex<HashSet<Value>>,
) -> io::Result<Vec<Value>> {
    if waiting.is_empty() {
        return Ok(Vec::new());
    }
    if lifecycle.is_shutdown() {
        for answer in waiting.drain(-32600, "the server is shutting down") {
            super::protocol::send_response(writer, &answer)?;
        }
        return Ok(Vec::new());
    }
    refresh(writer, lifecycle, mr, ov)?;
    let (ready, answers) = waiting.release(&Coverage { ov, indexed_hash: &|f| answering_hash(mr, f) }, cancelled);
    for answer in answers {
        super::protocol::send_response(writer, &answer)?;
    }
    Ok(ready)
}

/// Handles `msg` before the ordinary dispatch when it is `$/cancelRequest`, `ridx/status`, or a request to hold for
/// `minVersion`; returns it to dispatch otherwise.
///
/// A `$/cancelRequest` is taken here, in order. For a request taken already (answered, `-32800` from the mark the reader
/// set, or waiting here, ended now) the mark goes, so a later request reusing the id is not cancelled (review of
/// e535e9cf: dispatch marked it again after the answer); for one still to come it stays.
///
/// `ridx/status` and holding are only for a request (with an `id`) of an initialized server not shut down and not
/// cancelled: the rest goes through dispatch's own checks (review of a6909dbe). Both look at the indexes refreshed
/// first, the ones dispatch then answers from (review of e535e9cf: a request was admitted on the index before).
#[allow(clippy::too_many_arguments)]
pub(crate) fn intercept<W: Write>(
    msg: Value,
    waiting: &mut Waiting,
    writer: &mut W,
    lifecycle: &mut super::LifecycleManager,
    mr: &mut super::MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    session: &SessionIndexer,
    cancelled: &Mutex<HashSet<Value>>,
) -> io::Result<Option<Value>> {
    match msg["method"].as_str() {
        Some("$/cancelRequest") => {
            let id = &msg["params"]["id"];
            let (answer, taken) = waiting.cancel(id);
            if taken {
                cancelled.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
            }
            if let Some(answer) = answer {
                super::protocol::send_response(writer, &answer)?;
            }
            return Ok(None);
        }
        // Removed folders leave `ridx/status` (review of e535e9cf: they came back from the reindex history).
        Some("workspace/didChangeWorkspaceFolders") => {
            for item in msg["params"]["event"]["removed"].as_array().into_iter().flatten() {
                let path = PathBuf::from(super::protocol::uri_to_path(item["uri"].as_str().unwrap_or("")));
                for root in super::cargo_root::workspace_roots(vec![path.clone()]).into_iter().chain([path]) {
                    lifecycle.reindex.forget(&ridx_format::canonicalize_best_effort(&root));
                }
            }
            return Ok(Some(msg));
        }
        _ => {}
    }
    let id = msg.get("id").filter(|id| !id.is_null());
    if let Some(id) = id.filter(|_| msg.get("method").is_some()) {
        waiting.note_request(id);
    }
    let is_cancelled = id.is_some_and(|id| cancelled.lock().unwrap_or_else(|e| e.into_inner()).contains(id));
    if id.is_none() || is_cancelled || !lifecycle.is_initialized() || lifecycle.is_shutdown() {
        return Ok(Some(msg));
    }
    if msg["method"] == "ridx/status" {
        refresh(writer, lifecycle, mr, ov)?;
        let result = status(&msg, mr, ov, &lifecycle.reindex, session);
        super::protocol::send_response(writer, &json!({ "jsonrpc": "2.0", "id": msg["id"], "result": result }))?;
        return Ok(None);
    }
    if super::fresh::wanted(&msg).is_none() {
        return Ok(Some(msg));
    }
    refresh(writer, lifecycle, mr, ov)?;
    Ok(waiting.admit(msg, &Coverage { ov, indexed_hash: &|f| answering_hash(mr, f) }))
}

/// What `initialize` answers in `capabilities.experimental`: the `ridx/*` methods and request options this server
/// supports, each with the version of its contract (docs/integration.md §7), for clients to detect them.
pub fn experimental_capabilities() -> Value {
    json!({ "ridx": { "methods": { "ridx/status": 1 }, "requestOptions": { "minVersion": 1 } } })
}

/// `ridx/status`: every root of the session and its index, and the given document's coverage.
pub fn status(msg: &Value, mr: &super::MultiRootWorkspace, ov: &crate::overlay::OverlayManager, reindex: &super::reindex::Reindexer, session: &SessionIndexer) -> Value {
    // The roots with an index loaded, and those being indexed for the first time or whose first index failed (review
    // of a6909dbe: they were missing, with the reason of the failure).
    let mut roots: Vec<(PathBuf, Option<String>)> = mr
        .all_workspaces()
        .filter_map(|ws| ws.workspace_root().map(|r| (ridx_format::canonicalize_best_effort(r), Some(format!("{}.{}", ws.instance_id(), ws.generation())))))
        .collect();
    for root in reindex.roots() {
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
            json!({
                "root": root.display().to_string(),
                "indexed": generation.is_some(),
                "generation": generation,
                "building": building,
                "error": error,
                "sessionIndex": { "answers": session_answers, "building": session_building },
            })
        })
        .collect();
    let mut out = json!({ "roots": roots });
    if let Some(uri) = msg["params"]["textDocument"]["uri"].as_str() {
        let file = super::protocol::uri_to_path(uri);
        let root = super::dispatch::workspace_root_of(mr, reindex, Path::new(&file));
        let building = root.as_ref().is_some_and(|r| session.state_of(r).0 || reindex.status_of(r).0);
        let cov = Coverage { ov, indexed_hash: &|f| answering_hash(mr, f) };
        out["document"] = match ov.get(&file) {
            Some(o) => json!({ "version": o.version, "coveredVersion": cov.covered_version(&file).flatten(), "building": building }),
            None => json!({ "version": null, "coveredVersion": null, "building": building }),
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// prod-code (m=19778): clients detect the `ridx/*` methods from `initialize`.
    #[test]
    fn test_initialize_lists_the_ridx_methods() {
        let mut lc = crate::lsp::lifecycle::LifecycleManager::new();
        let (caps, _) = lc.handle_initialize(&json!({ "id": 1, "method": "initialize", "params": { "capabilities": {} } })).unwrap();
        let ridx = &caps["capabilities"]["experimental"]["ridx"];
        assert_eq!(ridx["methods"]["ridx/status"], 1, "{caps}");
        assert_eq!(ridx["requestOptions"]["minVersion"], 1, "{caps}");
    }
}
