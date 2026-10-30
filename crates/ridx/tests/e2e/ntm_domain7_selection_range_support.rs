/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Shared assertions for the textDocument/selectionRange integration tests (Domain 7).

use serde_json::Value;

/// Every parent strictly contains its child, the innermost range contains the cursor, and the
/// chain ends at the whole file, `(0, 0)` to `(last_line, 0)`.
pub fn assert_chain_invariants(item: &Value, cursor: (u32, u32), last_line: u64) {
    let pos = |v: &Value| (v["line"].as_u64().unwrap(), v["character"].as_u64().unwrap());
    let cursor = (cursor.0 as u64, cursor.1 as u64);
    let (start, end) = (pos(&item["range"]["start"]), pos(&item["range"]["end"]));
    assert!(start <= cursor && cursor <= end, "innermost {start:?}..{end:?} must contain {cursor:?}");
    let mut child = item;
    while child["parent"].is_object() {
        let parent = &child["parent"];
        let (cs, ce) = (pos(&child["range"]["start"]), pos(&child["range"]["end"]));
        let (ps, pe) = (pos(&parent["range"]["start"]), pos(&parent["range"]["end"]));
        assert!(ps <= cs && pe >= ce && (ps, pe) != (cs, ce), "{ps:?}..{pe:?} must strictly contain {cs:?}..{ce:?}");
        child = parent;
    }
    assert_eq!(pos(&child["range"]["start"]), (0, 0));
    assert_eq!(pos(&child["range"]["end"]), (last_line, 0));
}
