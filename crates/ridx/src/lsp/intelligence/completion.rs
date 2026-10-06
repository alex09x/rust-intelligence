/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP completion intelligence handler.

use super::super::protocol::{lsp_completion_kind, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn handle_completion(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
    let utf16_col_0based = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
    let line_1based = line_0based + 1;

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    let mut items = Vec::new();
    let mut seen_names = HashSet::new();

    // Determine prefix under/before cursor
    let mut prefix = String::new();
    if let Some(overlay) = overlay_opt {
        let line_idx = line_0based as usize;
        if let Some(&line_start) = overlay.line_starts.get(line_idx) {
            let byte_pos = overlay.line_char_col_to_byte(line_1based, col_1based);
            let line_slice = &overlay.text[line_start..byte_pos.min(overlay.text.len())];
            for c in line_slice.chars().rev() {
                if c.is_alphanumeric() || c == '_' {
                    prefix.insert(0, c);
                } else {
                    break;
                }
            }
        }
    }

    // 1. Overlay symbols: ONLY in-scope symbols, excluding nested items from other scopes
    if let Some(overlay) = overlay_opt {
        let cursor_byte = overlay.line_char_col_to_byte(line_1based, col_1based);

        // Find enclosing scopes containing cursor
        let active_scope_ids: HashSet<usize> = {
            let mut set = HashSet::new();
            for s in &overlay.scopes {
                if cursor_byte >= s.start_byte && cursor_byte <= s.end_byte {
                    set.insert(s.id);
                    let mut cur = s.parent;
                    while let Some(pid) = cur {
                        set.insert(pid);
                        cur = overlay.scopes.get(pid).and_then(|p| p.parent);
                    }
                }
            }
            set
        };

        for s in &overlay.symbols {
            if !prefix.is_empty() && !s.name.starts_with(&prefix) {
                continue;
            }

            let is_in_scope = if s.scope_id == 0 {
                true
            } else {
                active_scope_ids.contains(&s.scope_id) && s.decl_start_byte <= cursor_byte
            };

            if is_in_scope && seen_names.insert(s.name.clone()) {
                items.push(json!({
                    "label": s.name,
                    "kind": lsp_completion_kind(s.kind),
                    "detail": s.signature
                }));
            }
        }
    }

    // 2. Workspace symbols
    let mut is_capped = false;
    if let Some(ws) = workspace {
        let ws_symbols = ws.search_symbols(&prefix);
        for s in ws_symbols {
            if s.kind == ridx_format::DefKind::Local {
                continue;
            }
            // If the symbol is from the current file, the overlay already handled
            // lexical scope filtering for this buffer.
            if overlay_opt.is_some() {
                let resolved = s.resolved_path();
                let res_str = resolved.to_str().unwrap_or(s.file);
                if res_str == file || s.file == file {
                    continue;
                }
            }
            // Filter out synthetic compiler items (closures, blocks)
            if s.path.contains("{closure}") || s.path.contains("{block}") {
                continue;
            }
            // Filter out nested helpers inside functions, methods, or closures using owner kind
            if let Some((parent_path, _)) = s.path.rsplit_once("::") {
                if let Some(parent_def) = ws.find_definition_by_path(parent_path) {
                    if matches!(
                        parent_def.kind,
                        ridx_format::DefKind::Fn
                            | ridx_format::DefKind::AssocFn
                            | ridx_format::DefKind::Closure
                    ) {
                        continue;
                    }
                }
            }
            if seen_names.insert(s.name.to_string()) {
                items.push(json!({
                    "label": s.name,
                    "kind": lsp_completion_kind(s.kind),
                    "detail": s.signature
                }));
                if items.len() >= 100 {
                    is_capped = true;
                    break;
                }
            }
        }
    }

    json!({
        "isIncomplete": is_capped,
        "items": items
    })
}

pub fn handle_completion_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}
