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

/// The version of `file` the index answering compiled: its buffer's version when the index compiled the buffer's
/// text, `None` while edits since are not compiled yet. A file not open has no version to wait for.
fn covered_version(ov: &crate::overlay::OverlayManager, file: &str) -> Option<Option<i32>> {
    let o = ov.get(file)?;
    Some((!o.is_dirty()).then_some(o.version))
}

fn wanted(msg: &Value) -> Option<(String, i64)> {
    if msg.get(RELEASED).is_some() {
        return None;
    }
    let min = msg["params"]["minVersion"].as_i64()?;
    let uri = msg["params"]["textDocument"]["uri"].as_str()?;
    Some((super::protocol::uri_to_path(uri), min))
}

fn is_covered(ov: &crate::overlay::OverlayManager, file: &str, min: i64) -> bool {
    match covered_version(ov, file) {
        None => true,
        Some(v) => v.is_some_and(|v| i64::from(v) >= min),
    }
}

impl Waiting {
    /// `msg` to dispatch now, or `None` when it waits for a newer index.
    pub fn admit(&mut self, msg: Value, ov: &crate::overlay::OverlayManager) -> Option<Value> {
        let Some((file, min)) = wanted(&msg) else { return Some(msg) };
        if is_covered(ov, &file, min) {
            return Some(msg);
        }
        let timeout = msg["params"]["minVersionTimeoutMs"].as_u64().map_or(DEFAULT_TIMEOUT, Duration::from_millis);
        self.held.push(Held { msg, deadline: Instant::now() + timeout });
        None
    }

    /// The held requests now covered, to dispatch, and the answers of those that timed out.
    pub fn release(&mut self, ov: &crate::overlay::OverlayManager) -> (Vec<Value>, Vec<Value>) {
        let (mut ready, mut expired) = (Vec::new(), Vec::new());
        let now = Instant::now();
        for mut h in std::mem::take(&mut self.held) {
            let Some((file, min)) = wanted(&h.msg) else { continue };
            if is_covered(ov, &file, min) {
                h.msg[RELEASED] = json!(true);
                ready.push(h.msg);
            } else if now >= h.deadline {
                let covered = covered_version(ov, &file).flatten();
                expired.push(json!({
                    "jsonrpc": "2.0",
                    "id": h.msg["id"],
                    "error": { "code": -32803, "message": format!("no index covers version {min} of the document yet"), "data": { "reason": "timeout", "coveredVersion": covered } }
                }));
            } else {
                self.held.push(h);
            }
        }
        (ready, expired)
    }
}

/// What `initialize` answers in `capabilities.experimental`: the `ridx/*` methods and request options this server
/// supports, each with the version of its contract (docs/integration.md §7), for clients to detect them.
pub fn experimental_capabilities() -> Value {
    json!({ "ridx": { "methods": { "ridx/status": 1 }, "requestOptions": { "minVersion": 1 } } })
}

/// `ridx/status`: every root's index, and the given document's coverage.
pub fn status(msg: &Value, mr: &super::MultiRootWorkspace, ov: &crate::overlay::OverlayManager, reindex: &super::reindex::Reindexer, session: &SessionIndexer) -> Value {
    let roots: Vec<Value> = mr
        .all_workspaces()
        .filter_map(|ws| ws.workspace_root().map(|r| (ridx_format::canonicalize_best_effort(r), ws)))
        .map(|(root, ws)| {
            let (building, error) = reindex.status_of(&root);
            let (session_building, session_answers) = session.state_of(&root);
            json!({
                "root": root.display().to_string(),
                "indexed": true,
                "generation": format!("{}.{}", ws.instance_id(), ws.generation()),
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
        out["document"] = match ov.get(&file) {
            Some(o) => json!({ "version": o.version, "coveredVersion": covered_version(ov, &file).flatten(), "building": building }),
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

    /// prod-code (m=19767): a tool that edits and asks at once got the answer of the index before the edit.
    #[test]
    fn test_a_request_waits_for_an_index_of_its_version() {
        let (uri, file) = ("file:///tmp/ridx_fresh/lib.rs", "/tmp/ridx_fresh/lib.rs");
        let saved = "pub fn a() {}\n";
        let mut ov = OverlayManager::new();
        ov.open_with_source_hash(uri, file, 1, saved.to_string(), Some(ridx_format::normalized_source_hash(saved.as_bytes())));
        let mut waiting = Waiting::default();
        assert!(waiting.admit(request(1, 1, json!({})), &ov).is_some(), "version 1 is what the index compiled");
        assert!(waiting.admit(json!({ "id": 2, "method": "textDocument/hover", "params": {} }), &ov).is_some(), "no minVersion: at once");

        let edited = "pub fn a() {}\npub fn b() { a() }\n";
        assert!(ov.apply_content_changes(file, 2, &[json!({ "text": edited })]));
        assert!(waiting.admit(request(3, 2, json!({})), &ov).is_none(), "version 2 is not compiled yet: it waits");
        assert!(waiting.admit(request(4, 2, json!({ "minVersionTimeoutMs": 0 })), &ov).is_none());
        let (ready, expired) = waiting.release(&ov);
        assert!(ready.is_empty());
        assert_eq!(expired.len(), 1, "the one with no time left fails");
        assert_eq!(expired[0]["id"], 4);
        assert_eq!(expired[0]["error"]["code"], -32803);
        assert_eq!(expired[0]["error"]["data"]["reason"], "timeout");
        assert!(expired[0]["error"]["data"]["coveredVersion"].is_null(), "the edit is not compiled");

        // The session index of the edit loads: the buffer's text is what it compiled.
        ov.advance_compiled_baseline(file, edited.to_string());
        let (ready, expired) = waiting.release(&ov);
        assert!(expired.is_empty());
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0]["id"], 3);
        assert!(waiting.admit(ready[0].clone(), &ov).is_some(), "a released request is not held again");
        assert!(waiting.release(&ov).0.is_empty());
    }
}
