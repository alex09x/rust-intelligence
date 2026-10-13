/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Server-instance registry for workspace/applyEdit requests and pending responses.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct PendingApplyState {
    pub command_req_id: Option<Value>,
    pub total_count: usize,
    pub combined_edit: Value,
}

pub struct ApplyEditRegistry {
    next_id: AtomicU64,
    pending: Mutex<HashMap<Value, PendingApplyState>>,
}

impl Default for ApplyEditRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplyEditRegistry {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(10000),
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_next_id(&self) -> Value {
        json!(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub fn register_pending(
        &self,
        apply_req_id: Value,
        command_req_id: Option<Value>,
        total_count: usize,
        combined_edit: Value,
    ) {
        let mut lock = self.pending.lock().unwrap();
        lock.insert(
            apply_req_id,
            PendingApplyState {
                command_req_id,
                total_count,
                combined_edit,
            },
        );
    }

    pub fn handle_response(&self, msg: &Value) -> Option<(Option<Value>, Value)> {
        let resp_id = msg.get("id")?;
        let mut lock = self.pending.lock().unwrap();
        let state = lock.remove(resp_id)?;

        let client_applied = msg
            .get("result")
            .and_then(|r| r.get("applied"))
            .and_then(|a| a.as_bool())
            .unwrap_or(false);

        if client_applied {
            Some((
                state.command_req_id,
                json!({
                    "applied": true,
                    "count": state.total_count,
                    "edit": state.combined_edit,
                }),
            ))
        } else {
            Some((
                state.command_req_id,
                json!({ "applied": false, "count": 0, "failure": "client_rejected" }),
            ))
        }
    }

    pub fn clear(&self) {
        self.pending.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_registry_isolation() {
        let (r1, r2) = (ApplyEditRegistry::new(), ApplyEditRegistry::new());
        assert_eq!(r1.get_next_id(), json!(10000));
        assert_eq!(r2.get_next_id(), json!(10000));
        assert_eq!(r1.get_next_id(), json!(10001));
        assert_eq!(r2.get_next_id(), json!(10001));

        r1.register_pending(json!(10000), Some(json!(404)), 2, json!({"test": 1}));
        let ack = json!({ "id": 10000, "result": { "applied": true } });
        assert_eq!(r2.handle_response(&ack), None);
        assert_eq!(
            r1.handle_response(&ack),
            Some((
                Some(json!(404)),
                json!({ "applied": true, "count": 2, "edit": { "test": 1 } })
            ))
        );
    }
}
