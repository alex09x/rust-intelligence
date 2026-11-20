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
}
