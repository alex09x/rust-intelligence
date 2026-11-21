/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Answers a tool can trust right after an edit (`minVersion`).
//!
//! An editor is served at once, from the newest index mapped through the edits since. A tool that edits and then
//! asks would get that answer silently stale (prod-code, m=19767). With `params.minVersion: N` a request waits until
//! the index answering compiled version `N` (or later) of its document, or fails with `-32803` after
//! `params.minVersionTimeoutMs` (30 s by default). The server loop's side, and `ridx/status`, are in `fresh_loop`.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

struct Held {
    msg: Value,
    deadline: Instant,
}

/// Requests waiting for an index that covers their document's version.
#[derive(Default)]
pub struct Waiting {
    held: Vec<Held>,
    /// The deadlines of requests released to dispatch, by id: one no longer covered when its turn comes (another index
    /// loaded meanwhile) waits again until that same deadline (review of e535e9cf).
    released: HashMap<String, Instant>,
}

/// What a document's coverage is judged by: the open buffers, and the source hash the index answering for a file
/// recorded for it, `None` when no loaded index answers for it or its units disagree (`fresh_loop::answering_hash`).
pub struct Coverage<'a> {
    pub ov: &'a crate::overlay::OverlayManager,
    pub indexed_hash: &'a dyn Fn(&str) -> Option<u64>,
    /// Whether the compiler's diagnostics of a buffer are known (`session_diag::readiness`): a `textDocument/diagnostic`
    /// waits for those, not for the index (prod-code, m=20178: the answer before them read as clean).
    pub diagnostics: &'a dyn Fn(&str) -> Readiness,
}

use crate::lsp::presentation::session_diag::Readiness;

fn is_diagnostic(msg: &Value) -> bool {
    msg["method"] == "textDocument/diagnostic"
}

/// Whether `msg`, waiting for version `min` of `file`, goes now: a `textDocument/diagnostic` once the compiler's
/// diagnostics of that version are known, any other request once the index answering covers it.
fn goes_now(msg: &Value, cov: &Coverage, file: &str, min: i64) -> bool {
    if !is_diagnostic(msg) || cov.ov.get(file).is_none() {
        return cov.is_covered(file, min);
    }
    matches!((cov.diagnostics)(file), Readiness::Ready(v) if i64::from(v) >= min)
}

/// The answer of a `textDocument/diagnostic` waiting for version `min` whose compile of that version gave no
/// diagnostics: an error saying why, never an empty list (prod-code, m=20178).
pub(super) fn unavailable(msg: &Value, cov: &Coverage) -> Option<Value> {
    let (file, min) = wanted(msg).filter(|_| is_diagnostic(msg))?;
    match (cov.diagnostics)(&file) {
        Readiness::Unavailable(v, why) if i64::from(v) >= min => {
            let data = json!({ "reason": "diagnosticsUnavailable", "detail": why, "version": v });
            Some(response(&msg["id"], -32803, format!("the compile of version {v} of the document gave no diagnostics ({why})"), data))
        }
        _ => None,
    }
}

impl Coverage<'_> {
    /// The version of `file` the answering index compiled: the buffer's version when that index recorded the hash of
    /// the buffer's text, `None` otherwise (edits not compiled yet, or no index answering for the file any more: review
    /// of a6909dbe, a removed workspace kept a clean overlay). A file not open has no version to wait for.
    pub fn covered_version(&self, file: &str) -> Option<Option<i32>> {
        let o = self.ov.get(file)?;
        Some(((self.indexed_hash)(file) == Some(o.content_hash)).then_some(o.version))
    }

    fn is_covered(&self, file: &str, min: i64) -> bool {
        self.covered_version(file).is_none_or(|v| v.is_some_and(|v| i64::from(v) >= min))
    }
}

pub(super) fn response(id: &Value, code: i64, message: String, data: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message, "data": data } })
}

/// The document and version a request waits for. Only a request (with an `id`) waits: a notification carrying the
/// fields is handled at once (review of a6909dbe: a `didChange` waited for the version it was bringing).
pub(super) fn wanted(msg: &Value) -> Option<(String, i64)> {
    if msg.get("id").is_none_or(Value::is_null) || msg.get("method").is_none() {
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

    /// The deadline of the request with this id released to dispatch, taken out: the loop takes it on every path that
    /// request goes, so none is left to a later request reusing the id.
    pub fn take_released(&mut self, id: &Value) -> Option<Instant> {
        self.released.remove(&id.to_string())
    }

    /// `msg` to dispatch now, or `None` when it waits for a newer index. A request released before (`released`, its
    /// first deadline) is checked again against the index answering now.
    pub fn admit(&mut self, msg: Value, cov: &Coverage, released: Option<Instant>) -> Option<Value> {
        let Some((file, min)) = wanted(&msg) else { return Some(msg) };
        if goes_now(&msg, cov, &file, min) {
            return Some(msg);
        }
        let timeout = msg["params"]["minVersionTimeoutMs"].as_u64().map_or(DEFAULT_TIMEOUT, Duration::from_millis);
        self.held.push(Held { deadline: released.unwrap_or_else(|| Instant::now() + timeout), msg });
        None
    }

    /// The held requests now covered, to dispatch (through `admit` again), and the answers of the others that ended:
    /// cancelled (`-32800`, its id taken out of `cancelled`), then timed out (`-32803`), in that order, before coverage
    /// counts: a deadline passed is a timeout even when the index arrived since (review of a6909dbe).
    pub fn release(&mut self, cov: &Coverage, cancelled: &std::sync::Mutex<std::collections::HashSet<Value>>) -> (Vec<Value>, Vec<Value>) {
        let (mut ready, mut answers) = (Vec::new(), Vec::new());
        let now = Instant::now();
        let mut cancelled = cancelled.lock().unwrap_or_else(|e| e.into_inner());
        for h in std::mem::take(&mut self.held) {
            let Some((file, min)) = wanted(&h.msg) else { continue };
            if cancelled.remove(&h.msg["id"]) {
                answers.push(response(&h.msg["id"], -32800, "Request cancelled".into(), Value::Null));
            } else if now >= h.deadline && is_diagnostic(&h.msg) {
                // The version whose diagnostics are known, older than the one asked for, if any.
                let known = match (cov.diagnostics)(&file) {
                    Readiness::Ready(v) => Some(v),
                    _ => None,
                };
                let data = json!({ "reason": "timeout", "coveredVersion": known });
                answers.push(response(&h.msg["id"], -32803, format!("the compiler's diagnostics of version {min} of the document are not known yet"), data));
            } else if now >= h.deadline {
                let data = json!({ "reason": "timeout", "coveredVersion": cov.covered_version(&file).flatten() });
                answers.push(response(&h.msg["id"], -32803, format!("no index covers version {min} of the document yet"), data));
            } else if let Some(answer) = unavailable(&h.msg, cov) {
                answers.push(answer);
            } else if goes_now(&h.msg, cov, &file, min) {
                self.released.insert(h.msg["id"].to_string(), h.deadline);
                ready.push(h.msg);
            } else {
                self.held.push(h);
            }
        }
        (ready, answers)
    }

    /// The answers of the held requests `refused` answers now (their index failed: review of b3d0d293), taken out. A
    /// cancelled one, or one past its deadline, is left to `release`, which answers those first.
    pub fn refuse(&mut self, refused: impl Fn(&Value) -> Option<Value>, cancelled: &std::sync::Mutex<std::collections::HashSet<Value>>) -> Vec<Value> {
        let now = Instant::now();
        let cancelled = cancelled.lock().unwrap_or_else(|e| e.into_inner());
        let mut answers = Vec::new();
        self.held.retain(|h| {
            if cancelled.contains(&h.msg["id"]) || now >= h.deadline {
                return true;
            }
            refused(&h.msg).map(|a| answers.push(a)).is_none()
        });
        answers
    }

    /// Answers every held request with `code` (the server shut down while they waited).
    pub fn drain(&mut self, code: i64, message: &str) -> Vec<Value> {
        self.released.clear();
        std::mem::take(&mut self.held).into_iter().map(|h| response(&h.msg["id"], code, message.to_string(), Value::Null)).collect()
    }
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
                Coverage { ov: &ov, indexed_hash: &|_| indexed.get(), diagnostics: &|_| Readiness::Pending }
            };
        }
        assert!(waiting.admit(request(1, 1, json!({})), &cov!(), None).is_some(), "version 1 is what the index compiled");
        assert!(waiting.admit(json!({ "id": 2, "method": "textDocument/hover", "params": {} }), &cov!(), None).is_some(), "no minVersion: at once");
        let note = json!({ "method": "textDocument/didChange", "params": { "textDocument": { "uri": uri, "version": 3 }, "minVersion": 3 } });
        assert!(waiting.admit(note, &cov!(), None).is_some(), "a notification never waits");

        indexed.set(None);
        assert!(waiting.admit(request(9, 1, json!({})), &cov!(), None).is_none(), "no index answers for the file any more: not covered");
        cancelled.lock().unwrap().insert(json!(9));
        let (ready, answers) = waiting.release(&cov!(), &cancelled);
        assert!(ready.is_empty() && answers[0]["id"] == 9 && answers[0]["error"]["code"] == -32800, "{answers:?}");
        assert!(cancelled.lock().unwrap().is_empty(), "the cancellation is used up, so a reused id is not cancelled");
        indexed.set(Some(hash(saved)));

        assert!(ov.apply_content_changes(file, 2, &[json!({ "text": edited })]));
        assert!(waiting.admit(request(3, 2, json!({})), &cov!(), None).is_none(), "version 2 is not compiled yet: it waits");
        assert!(waiting.admit(request(4, 2, json!({ "minVersionTimeoutMs": 0 })), &cov!(), None).is_none());
        // The index of the edit arrives after the deadline of 4 passed: 4 still times out, 3 is released.
        indexed.set(Some(hash(edited)));
        let (ready, answers) = waiting.release(&cov!(), &cancelled);
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!((answers[0]["id"].clone(), answers[0]["error"]["code"].clone()), (json!(4), json!(-32803)));
        assert_eq!(answers[0]["error"]["data"]["coveredVersion"], 2);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0]["id"], 3);
        // Review of e535e9cf: before its turn, another index loads that does not cover it: it waits again.
        indexed.set(Some(hash(saved)));
        assert!(
            {
                let d = waiting.take_released(&ready[0]["id"]);
                waiting.admit(ready[0].clone(), &cov!(), d)
            }
            .is_none(),
            "checked again against the index answering now"
        );
        indexed.set(Some(hash(edited)));
        let (ready, _) = waiting.release(&cov!(), &cancelled);
        assert!({
            let d = waiting.take_released(&ready[0]["id"]);
            waiting.admit(ready[0].clone(), &cov!(), d)
        }
        .is_some());

        indexed.set(None);
        assert!(waiting.admit(request(5, 2, json!({})), &cov!(), None).is_none());
        let drained = waiting.drain(-32600, "shutting down");
        assert!(drained.len() == 1 && drained[0]["error"]["code"] == -32600 && waiting.is_empty());
    }

    /// prod-code, m=20178: a pull with `minVersion` answered `[]` before the buffer's version was compiled; it waits for
    /// the compiler's diagnostics of that version, not for the index, and is told when there are none.
    #[test]
    fn test_a_pull_waits_for_the_diagnostics_of_its_version() {
        use std::cell::Cell;
        let (uri, file) = ("file:///tmp/ridx_fresh/lib.rs", "/tmp/ridx_fresh/lib.rs");
        let mut ov = OverlayManager::new();
        ov.open_with_source_hash(uri, file, 1, "x".into(), None);
        let state: Cell<Option<Readiness>> = Cell::new(Some(Readiness::Pending));
        let take = || {
            let s = state.take().unwrap();
            let copy = match &s {
                Readiness::Ready(v) => Readiness::Ready(*v),
                Readiness::Unavailable(v, w) => Readiness::Unavailable(*v, w),
                Readiness::Pending => Readiness::Pending,
            };
            state.set(Some(s));
            copy
        };
        let cancelled = std::sync::Mutex::new(std::collections::HashSet::new());
        let mut waiting = Waiting::default();
        let pull = |id: u64| json!({ "jsonrpc": "2.0", "id": id, "method": "textDocument/diagnostic", "params": { "textDocument": { "uri": uri }, "minVersion": 1 } });
        let cov = Coverage { ov: &ov, indexed_hash: &|_| Some(ridx_format::normalized_source_hash(b"x")), diagnostics: &|_| take() };
        assert!(waiting.admit(pull(1), &cov, None).is_none(), "the index covers it, its diagnostics are not known");
        state.set(Some(Readiness::Ready(1)));
        let (ready, answers) = waiting.release(&cov, &cancelled);
        assert!(answers.is_empty() && ready.len() == 1 && ready[0]["id"] == 1);
        state.set(Some(Readiness::Unavailable(1, "notChecked")));
        let answer = unavailable(&pull(2), &cov).expect("an answer, not an empty list");
        assert_eq!((answer["error"]["code"].clone(), answer["error"]["data"]["reason"].clone()), (json!(-32803), json!("diagnosticsUnavailable")));
        assert_eq!(answer["error"]["data"]["detail"], "notChecked");
        assert!(waiting.admit(pull(3), &cov, None).is_none());
        let (ready, answers) = waiting.release(&cov, &cancelled);
        assert!(ready.is_empty() && answers[0]["id"] == 3 && answers[0]["error"]["data"]["reason"] == "diagnosticsUnavailable", "a held one too");
    }
}
