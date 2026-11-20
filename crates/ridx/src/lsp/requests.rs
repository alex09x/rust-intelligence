/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which requests are read and not answered yet, for `$/cancelRequest`.
//!
//! The reader marks a cancelled id in the set dispatch checks (before handling a request, and after, for one cancelled
//! while it ran). A cancel names a request sent before it, and the reader reads in order: it marks the id only while a
//! request with that id is read and not answered; a cancel of one answered already (or never sent) is ignored, as the
//! protocol says. The loop tells when a request is answered, and the mark of its id goes then. A later request reusing
//! the id is never cancelled by the cancel of an earlier one (review of 2045537b: by an id history, a late cancel of an
//! answered request cancelled the next request with its id, and an early one was lost).

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
pub struct Requests {
    /// Requests read and not answered, per id (one, unless a client reuses an id while it is pending).
    pending: Mutex<HashMap<String, usize>>,
}

fn is_request(msg: &Value) -> bool {
    msg.get("method").is_some() && msg.get("id").is_some_and(|id| !id.is_null())
}

impl Requests {
    /// The reader read `msg`.
    pub fn read(&self, msg: &Value, cancelled: &Mutex<HashSet<Value>>) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if msg["method"] == "$/cancelRequest" {
            let id = &msg["params"]["id"];
            if pending.get(&id.to_string()).is_some_and(|&n| n > 0) {
                cancelled.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone());
            }
        } else if is_request(msg) {
            *pending.entry(msg["id"].to_string()).or_default() += 1;
        }
    }

    /// The request with this id was answered: its id's mark goes with the last one pending.
    pub fn answered(&self, id: &Value, cancelled: &Mutex<HashSet<Value>>) {
        let key = id.to_string();
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let Some(n) = pending.get_mut(&key) else { return };
        *n -= 1;
        if *n == 0 {
            pending.remove(&key);
            cancelled.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        }
    }
}

/// The server's writer: a response retires its request before its bytes go out, so a client that reuses the id after
/// reading it finds the old one gone (review of 8a28e8ff: retired after the write, a late cancel of the old request
/// was taken for the new one). Every response of every path (dispatch, `ridx/status`, held requests ended) passes here.
pub struct RetiringWriter<W: std::io::Write> {
    inner: W,
    pending: Vec<u8>,
    requests: std::sync::Arc<Requests>,
    cancelled: std::sync::Arc<Mutex<HashSet<Value>>>,
}

impl<W: std::io::Write> RetiringWriter<W> {
    pub fn new(inner: W, requests: std::sync::Arc<Requests>, cancelled: std::sync::Arc<Mutex<HashSet<Value>>>) -> Self {
        Self { inner, pending: Vec::new(), requests, cancelled }
    }

    /// The responses among the frames written since the last flush retire their requests.
    fn retire_responses(&self) {
        let mut rest = self.pending.as_slice();
        while let Some(at) = rest.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&rest[..at]);
            let Some(len) = header.lines().find_map(|l| l.strip_prefix("Content-Length:")).and_then(|v| v.trim().parse::<usize>().ok()) else { return };
            let body = &rest[at + 4..];
            let Some(frame) = body.get(..len) else { return };
            if let Ok(msg) = serde_json::from_slice::<Value>(frame) {
                if msg.get("method").is_none() && msg.get("id").is_some_and(|id| !id.is_null()) {
                    self.requests.answered(&msg["id"], &self.cancelled);
                }
            }
            rest = &body[len..];
        }
    }
}

impl<W: std::io::Write> std::io::Write for RetiringWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.retire_responses();
        self.inner.write_all(&self.pending)?;
        self.pending.clear();
        self.inner.flush()
    }
}

impl<W: std::io::Write> Drop for RetiringWriter<W> {
    fn drop(&mut self) {
        let _ = std::io::Write::flush(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_a_cancel_marks_only_a_request_still_pending() {
        let (r, cancelled) = (Requests::default(), Mutex::new(HashSet::new()));
        let req = |id: u64| json!({ "id": id, "method": "textDocument/hover", "params": {} });
        let cancel = |id: u64| json!({ "method": "$/cancelRequest", "params": { "id": id } });
        r.read(&cancel(7), &cancelled);
        assert!(cancelled.lock().unwrap().is_empty(), "a cancel of a request not sent is ignored");
        r.read(&req(7), &cancelled);
        r.read(&cancel(7), &cancelled);
        assert!(cancelled.lock().unwrap().contains(&json!(7)), "pending: marked");
        r.answered(&json!(7), &cancelled);
        assert!(cancelled.lock().unwrap().is_empty(), "answered: the mark goes");
        r.read(&cancel(7), &cancelled);
        r.read(&req(7), &cancelled);
        assert!(cancelled.lock().unwrap().is_empty(), "a late cancel of the answered one does not touch the next with its id");
    }

    /// Review of 8a28e8ff: the client reads a response and reuses its id; the request was retired only after the write,
    /// and a late cancel of it marked the new one.
    #[test]
    fn test_a_response_retires_its_request_before_it_goes_out() {
        let requests = std::sync::Arc::new(Requests::default());
        let cancelled = std::sync::Arc::new(Mutex::new(HashSet::new()));
        requests.read(&json!({ "id": 7, "method": "textDocument/hover", "params": {} }), &cancelled);
        let mut out = Vec::new();
        {
            let mut w = RetiringWriter::new(&mut out, requests.clone(), cancelled.clone());
            crate::lsp::protocol::send_response(&mut w, &json!({ "jsonrpc": "2.0", "id": 7, "result": null })).unwrap();
            // The bytes are out now: a late cancel of 7 and a new 7 come after.
            requests.read(&json!({ "method": "$/cancelRequest", "params": { "id": 7 } }), &cancelled);
            requests.read(&json!({ "id": 7, "method": "textDocument/hover", "params": {} }), &cancelled);
        }
        assert!(String::from_utf8_lossy(&out).contains("\"id\":7"));
        assert!(cancelled.lock().unwrap().is_empty(), "the new 7 is not cancelled");
    }
}
