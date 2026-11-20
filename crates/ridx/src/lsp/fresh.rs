/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Answers a tool can trust right after an edit, and what the server's indexes cover (`ridx/status`).
//!
//! An editor is served at once, from the newest index mapped through the edits since. A tool that edits and then
//! asks would get that answer silently stale (prod-code, m=19767). With `params.minVersion: N` a request waits until
//! the index answering compiled version `N` (or later) of its document, or fails with `-32803` after
//! `params.minVersionTimeoutMs` (30 s by default). A document is covered when the index compiled its current text:
//! the overlay's compiled text is the buffer's.

use super::session_index::SessionIndexer;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Marks a request released from waiting, so that it is not held again.
const RELEASED: &str = "__ridxReleased";

struct Held {
    msg: Value,
    deadline: Instant,
}

/// Requests waiting for an index that covers their document's version.
#[derive(Default)]
pub struct Waiting {
    held: Vec<Held>,
}

/// What a document's coverage is judged by: the open buffers, and the source hash the answering index recorded for a
/// file (`MultiRootWorkspace::source_hash_for_file`), `None` when no loaded index answers for it.
pub struct Coverage<'a> {
    pub ov: &'a crate::overlay::OverlayManager,
    pub indexed_hash: &'a dyn Fn(&str) -> Option<u64>,
}

impl Coverage<'_> {
    /// The version of `file` the answering index compiled: the buffer's version when that index recorded the hash of
    /// the buffer's text, `None` otherwise (edits not compiled yet, or no index answering for the file any more: review
    /// of a6909dbe, a removed workspace kept a clean overlay). A file not open has no version to wait for.
    fn covered_version(&self, file: &str) -> Option<Option<i32>> {
        let o = self.ov.get(file)?;
        Some(((self.indexed_hash)(file) == Some(o.content_hash)).then_some(o.version))
    }

    fn is_covered(&self, file: &str, min: i64) -> bool {
        self.covered_version(file).is_none_or(|v| v.is_some_and(|v| i64::from(v) >= min))
    }
}

fn response(id: &Value, code: i64, message: String, data: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message, "data": data } })
}

/// The document and version a request waits for. Only a request (with an `id`) waits: a notification carrying the
/// fields is handled at once (review of a6909dbe: a `didChange` waited for the version it was bringing).
fn wanted(msg: &Value) -> Option<(String, i64)> {
    if msg.get(RELEASED).is_some() || msg.get("id").is_none_or(Value::is_null) || msg.get("method").is_none() {
        return None;
    }
    let min = msg["params"]["minVersion"].as_i64()?;
    let uri = msg["params"]["textDocument"]["uri"].as_str()?;
    Some((super::protocol::uri_to_path(uri), min))
}

impl Waiting {
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// `msg` to dispatch now, or `None` when it waits for a newer index.
    pub fn admit(&mut self, msg: Value, cov: &Coverage) -> Option<Value> {
        let Some((file, min)) = wanted(&msg) else { return Some(msg) };
        if cov.is_covered(&file, min) {
            return Some(msg);
        }
        let timeout = msg["params"]["minVersionTimeoutMs"].as_u64().map_or(DEFAULT_TIMEOUT, Duration::from_millis);
        self.held.push(Held { msg, deadline: Instant::now() + timeout });
        None
    }

    /// The held requests now covered, to dispatch, and the answers of the others that ended: cancelled (`-32800`, its
    /// id taken out of `cancelled`), then timed out (`-32803`), in that order, before coverage counts: a deadline passed
    /// is a timeout even when the index arrived since (review of a6909dbe).
    pub fn release(&mut self, cov: &Coverage, cancelled: &std::sync::Mutex<std::collections::HashSet<Value>>) -> (Vec<Value>, Vec<Value>) {
        let (mut ready, mut answers) = (Vec::new(), Vec::new());
        let now = Instant::now();
        let mut cancelled = cancelled.lock().unwrap_or_else(|e| e.into_inner());
        for mut h in std::mem::take(&mut self.held) {
            let Some((file, min)) = wanted(&h.msg) else { continue };
            if cancelled.remove(&h.msg["id"]) {
                answers.push(response(&h.msg["id"], -32800, "Request cancelled".into(), Value::Null));
            } else if now >= h.deadline {
                let data = json!({ "reason": "timeout", "coveredVersion": cov.covered_version(&file).flatten() });
                answers.push(response(&h.msg["id"], -32803, format!("no index covers version {min} of the document yet"), data));
            } else if cov.is_covered(&file, min) {
                h.msg[RELEASED] = json!(true);
                ready.push(h.msg);
            } else {
                self.held.push(h);
            }
        }
        (ready, answers)
    }

    /// Answers every held request with `code` (the server shut down while they waited).
    pub fn drain(&mut self, code: i64, message: &str) -> Vec<Value> {
        std::mem::take(&mut self.held).into_iter().map(|h| response(&h.msg["id"], code, message.to_string(), Value::Null)).collect()
    }
}

/// The server loop's turn for waiting requests: after `shutdown` they are answered `-32600`; otherwise the published
/// index is reloaded if it changed (coverage counts against the index that answers now) and the ended or covered ones
/// come out. Returns the requests to dispatch.
pub(crate) fn release_waiting<W: std::io::Write>(
    waiting: &mut Waiting,
    writer: &mut W,
    lifecycle: &mut super::LifecycleManager,
    mr: &mut super::MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    cancelled: &std::sync::Mutex<std::collections::HashSet<Value>>,
) -> std::io::Result<Vec<Value>> {
    if waiting.is_empty() {
        return Ok(Vec::new());
    }
    if lifecycle.is_shutdown() {
        for answer in waiting.drain(-32600, "the server is shutting down") {
            super::protocol::send_response(writer, &answer)?;
        }
        return Ok(Vec::new());
    }
    super::dispatch::reload_changed_index(writer, lifecycle, mr, ov)?;
    let (ready, answers) = waiting.release(&Coverage { ov, indexed_hash: &|f| mr.source_hash_for_file(f) }, cancelled);
    for answer in answers {
        super::protocol::send_response(writer, &answer)?;
    }
    Ok(ready)
}

/// Handles `msg` before the ordinary dispatch when it is `ridx/status` or a request to hold for `minVersion`; returns it
/// back to dispatch otherwise. Only for a request (with an `id`) of an initialized server not shut down and not
/// cancelled: the rest goes through dispatch's own checks (review of a6909dbe: status answered before `initialize`,
/// after `shutdown` and to a notification, and a held request survived `shutdown`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn intercept<W: std::io::Write>(
    msg: Value,
    waiting: &mut Waiting,
    writer: &mut W,
    lifecycle: &mut super::LifecycleManager,
    mr: &mut super::MultiRootWorkspace,
    ov: &mut crate::overlay::OverlayManager,
    session: &SessionIndexer,
    cancelled: &std::sync::Mutex<std::collections::HashSet<Value>>,
) -> std::io::Result<Option<Value>> {
    let id = msg.get("id").filter(|id| !id.is_null());
    let is_cancelled = id.is_some_and(|id| cancelled.lock().unwrap_or_else(|e| e.into_inner()).contains(id));
    if id.is_none() || is_cancelled || !lifecycle.is_initialized() || lifecycle.is_shutdown() {
        return Ok(Some(msg));
    }
    if msg["method"] == "ridx/status" {
        // The published index as it is now, also when the client only polls this (review of a6909dbe).
        super::dispatch::reload_changed_index(writer, lifecycle, mr, ov)?;
        let result = status(&msg, mr, ov, &lifecycle.reindex, session);
        super::protocol::send_response(writer, &json!({ "jsonrpc": "2.0", "id": msg["id"], "result": result }))?;
        return Ok(None);
    }
    Ok(waiting.admit(msg, &Coverage { ov, indexed_hash: &|f| mr.source_hash_for_file(f) }))
}

/// What `initialize` answers in `capabilities.experimental`: the `ridx/*` methods and request options this server
/// supports, each with the version of its contract (docs/integration.md §7), for clients to detect them.
pub fn experimental_capabilities() -> Value {
    json!({ "ridx": { "methods": { "ridx/status": 1 }, "requestOptions": { "minVersion": 1 } } })
}

/// `ridx/status`: every root's index, and the given document's coverage.
pub fn status(msg: &Value, mr: &super::MultiRootWorkspace, ov: &crate::overlay::OverlayManager, reindex: &super::reindex::Reindexer, session: &SessionIndexer) -> Value {
    // The roots with an index loaded, and those being indexed for the first time or whose first index failed (review
    // of a6909dbe: they were missing, with the reason of the failure).
    let mut roots: Vec<(std::path::PathBuf, Option<String>)> = mr
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
        let cov = Coverage { ov, indexed_hash: &|f| mr.source_hash_for_file(f) };
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
    use crate::overlay::OverlayManager;

    fn request(id: u64, min: i64, extra: Value) -> Value {
        let mut params = json!({ "textDocument": { "uri": "file:///tmp/ridx_fresh/lib.rs" }, "position": { "line": 0, "character": 7 }, "minVersion": min });
        if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        json!({ "jsonrpc": "2.0", "id": id, "method": "textDocument/references", "params": params })
    }

    /// prod-code (m=19778): clients detect the `ridx/*` methods from `initialize`.
    #[test]
    fn test_initialize_lists_the_ridx_methods() {
        let mut lc = crate::lsp::lifecycle::LifecycleManager::new();
        let (caps, _) = lc.handle_initialize(&json!({ "id": 1, "method": "initialize", "params": { "capabilities": {} } })).unwrap();
        let ridx = &caps["capabilities"]["experimental"]["ridx"];
        assert_eq!(ridx["methods"]["ridx/status"], 1, "{caps}");
        assert_eq!(ridx["requestOptions"]["minVersion"], 1, "{caps}");
    }

    /// prod-code (m=19767): a tool that edits and asks at once got the answer of the index before the edit. Coverage is
    /// the answering index's recorded hash of the buffer's text (review of a6909dbe).
    #[test]
    fn test_a_request_waits_for_an_index_of_its_version() {
        use std::cell::Cell;
        let (uri, file) = ("file:///tmp/ridx_fresh/lib.rs", "/tmp/ridx_fresh/lib.rs");
        let (saved, edited) = ("pub fn a() {}\n", "pub fn a() {}\npub fn b() { a() }\n");
        let hash = |t: &str| ridx_format::normalized_source_hash(t.as_bytes());
        let mut ov = OverlayManager::new();
        ov.open_with_source_hash(uri, file, 1, saved.to_string(), Some(hash(saved)));
        let indexed: Cell<Option<u64>> = Cell::new(Some(hash(saved)));
        let cancelled = std::sync::Mutex::new(std::collections::HashSet::new());
        let mut waiting = Waiting::default();
        macro_rules! cov {
            () => {
                Coverage { ov: &ov, indexed_hash: &|_| indexed.get() }
            };
        }
        assert!(waiting.admit(request(1, 1, json!({})), &cov!()).is_some(), "version 1 is what the index compiled");
        assert!(waiting.admit(json!({ "id": 2, "method": "textDocument/hover", "params": {} }), &cov!()).is_some(), "no minVersion: at once");
        let note = json!({ "method": "textDocument/didChange", "params": { "textDocument": { "uri": uri, "version": 3 }, "minVersion": 3 } });
        assert!(waiting.admit(note, &cov!()).is_some(), "a notification never waits");

        indexed.set(None);
        assert!(waiting.admit(request(9, 1, json!({})), &cov!()).is_none(), "no index answers for the file any more: not covered");
        cancelled.lock().unwrap().insert(json!(9));
        let (ready, answers) = waiting.release(&cov!(), &cancelled);
        assert!(ready.is_empty() && answers[0]["id"] == 9 && answers[0]["error"]["code"] == -32800, "{answers:?}");
        assert!(cancelled.lock().unwrap().is_empty(), "the cancellation is used up, so a reused id is not cancelled");
        indexed.set(Some(hash(saved)));

        assert!(ov.apply_content_changes(file, 2, &[json!({ "text": edited })]));
        assert!(waiting.admit(request(3, 2, json!({})), &cov!()).is_none(), "version 2 is not compiled yet: it waits");
        assert!(waiting.admit(request(4, 2, json!({ "minVersionTimeoutMs": 0 })), &cov!()).is_none());
        // The index of the edit arrives after the deadline of 4 passed: 4 still times out, 3 is released.
        indexed.set(Some(hash(edited)));
        let (ready, answers) = waiting.release(&cov!(), &cancelled);
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!((answers[0]["id"].clone(), answers[0]["error"]["code"].clone()), (json!(4), json!(-32803)));
        assert_eq!(answers[0]["error"]["data"]["coveredVersion"], 2);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0]["id"], 3);
        assert!(waiting.admit(ready[0].clone(), &cov!()).is_some(), "a released request is not held again");

        indexed.set(None);
        assert!(waiting.admit(request(5, 2, json!({})), &cov!()).is_none());
        let drained = waiting.drain(-32600, "shutting down");
        assert!(drained.len() == 1 && drained[0]["error"]["code"] == -32600 && waiting.is_empty());
    }
}
