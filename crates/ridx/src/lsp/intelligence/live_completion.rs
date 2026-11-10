/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Narrows a `receiver.` completion list to the members of the receiver's inferred type.
//!
//! The owner index names a receiver by its head type, so it offers every method of every impl of that head. The
//! live inference knows the receiver's full type and the traits in scope, so its member list decides which fields
//! and methods stay; snippets and keyword items are left as the owner path built them.

use std::collections::HashSet;

use serde_json::Value;

use super::owner_completion::item;
use crate::lsp::navigation::global_live_db;
use crate::lsp::protocol::lsp_completion_kind;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;

/// LSP completion kinds of the items a receiver's type decides: Method, Function, Field.
const MEMBER_KINDS: [u64; 3] = [2, 3, 5];

/// The member name an item completes: its filter text when the label carries a note, its label otherwise.
fn member_name(it: &Value) -> &str {
    it["filterText"].as_str().or_else(|| it["label"].as_str()).unwrap_or_default()
}

/// Whether `it` is a plain field or method of the receiver (not a snippet, not a call through `iter()`).
fn is_member(it: &Value) -> bool {
    it["kind"].as_u64().is_some_and(|k| MEMBER_KINDS.contains(&k)) && !it["label"].as_str().unwrap_or_default().contains('.')
}

/// Keeps the members of `items` that the type inferred for the receiver before `cursor_byte` has, and appends the
/// members the owner path missed. `items` is returned untouched when the receiver's type cannot be inferred.
pub(super) fn refine_dot_items(ws: &WorkspaceIndex, file: &str, cursor_byte: u32, overlay_mgr: &OverlayManager, prefix: &str, mut items: Vec<Value>) -> Vec<Value> {
    let cands = {
        let mut db = global_live_db().lock().unwrap_or_else(|e| e.into_inner());
        db.dot_candidates(ws, file, cursor_byte, Some(overlay_mgr))
    };
    if cands.is_empty() {
        return items;
    }
    let live: HashSet<&str> = cands.iter().map(|c| c.label.as_str()).collect();
    items.retain(|it| !is_member(it) || live.contains(member_name(it)));
    let mut have: HashSet<String> = items.iter().filter(|it| is_member(it)).map(|it| member_name(it).to_string()).collect();
    for c in &cands {
        if c.label.starts_with(prefix) && have.insert(c.label.clone()) {
            let note = c.from_trait.as_deref().map(item::trait_note);
            items.push(item::item(&c.label, note.as_deref(), lsp_completion_kind(c.kind), c.detail.as_deref().unwrap_or_default()));
        }
    }
    item::number_sort_text(&mut items);
    items
}
