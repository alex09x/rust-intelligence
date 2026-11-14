/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The workspace's type definitions by short name, built once per index and kept for the few workspaces served
//! lately, for the `typeDefinition` lookups that start from a bare type name.

use super::type_definition::is_type_def_kind;
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub(super) struct WsTypeIndex {
    /// The index it was built from: `instance_id`, `generation` (bumped by a reload) and shard count.
    ws_key: (u64, u64, usize),
    /// `(reader, definition)` indexes of the type definitions under each short name.
    pub(super) by_short_name: HashMap<Box<str>, Vec<(u32, u32)>>,
}

/// The type indexes of the workspaces served lately, most recent last: a multi-root server keeps each root's.
static WS_TYPE_INDEX: OnceLock<Mutex<Vec<Arc<WsTypeIndex>>>> = OnceLock::new();
const WS_TYPE_INDEX_SLOTS: usize = 4;

pub(super) fn clear_type_indexes() {
    if let Some(mut guard) = WS_TYPE_INDEX.get().map(|l| l.lock().unwrap_or_else(|e| e.into_inner())) {
        guard.clear();
    }
}

pub(super) fn get_ws_type_index(ws: &WorkspaceIndex) -> Arc<WsTypeIndex> {
    let ws_key = super::references_gated::ws_cache_key(ws);
    let lock = WS_TYPE_INDEX.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(pos) = guard.iter().position(|idx| idx.ws_key == ws_key) {
        let hit = guard.remove(pos);
        guard.push(Arc::clone(&hit));
        return hit;
    }
    let mut by_short_name: HashMap<Box<str>, Vec<(u32, u32)>> = HashMap::with_capacity(4096);
    for (r_idx, reader) in ws.readers().iter().enumerate() {
        for (d_idx, def) in reader.definitions().iter().enumerate() {
            if !is_type_def_kind(DefKind::from(def.kind)) {
                continue;
            }
            let raw_file = reader.get_file(def.span().file_id());
            if raw_file == "external" || raw_file.is_empty() {
                continue;
            }
            if let Some(short) = reader.get_str(def.path_offset()).rsplit("::").next().filter(|s| !s.is_empty()) {
                by_short_name.entry(short.into()).or_default().push((r_idx as u32, d_idx as u32));
            }
            let name = reader.get_str(def.name_offset());
            if !name.is_empty() && !reader.get_str(def.path_offset()).ends_with(name) {
                by_short_name.entry(name.into()).or_default().push((r_idx as u32, d_idx as u32));
            }
        }
    }
    let built = Arc::new(WsTypeIndex { ws_key, by_short_name });
    // An entry of the same instance under an older generation is never asked again.
    guard.retain(|idx| idx.ws_key.0 != ws_key.0);
    if guard.len() >= WS_TYPE_INDEX_SLOTS {
        guard.remove(0);
    }
    guard.push(Arc::clone(&built));
    built
}
