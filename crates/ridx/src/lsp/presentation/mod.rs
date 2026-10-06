/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP presentation handlers: selection range, document links, diagnostics, moniker, and content.

pub mod diagnostics;
pub mod tokens;

#[cfg(test)]
pub mod tests;

pub use diagnostics::*;
pub use tokens::*;

use super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_selection_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let positions = msg["params"]["positions"].as_array();

    let Some(overlay) = overlay_mgr.get(&file) else {
        return json!([]);
    };

    let mut results = Vec::new();
    if let Some(pos_list) = positions {
        for pos in pos_list {
            let line_0based = pos["line"].as_u64().unwrap_or(0) as u32;
            let utf16_col_0based = pos["character"].as_u64().unwrap_or(0) as u32;
            let byte_pos = overlay.position_to_byte_offset(line_0based, utf16_col_0based);

            let mut matching_scopes: Vec<_> = overlay
                .scopes
                .iter()
                .filter(|s| byte_pos >= s.start_byte && byte_pos <= s.end_byte)
                .collect();

            matching_scopes.sort_by_key(|s| s.end_byte - s.start_byte);

            let mut current_range: Option<Value> = None;
            for s in matching_scopes.into_iter().rev() {
                let (sl, sc) = overlay.byte_offset_to_position(s.start_byte);
                let (el, ec) = overlay.byte_offset_to_position(s.end_byte);
                let r = json!({
                    "range": {
                        "start": { "line": sl, "character": sc },
                        "end": { "line": el, "character": ec }
                    },
                    "parent": current_range
                });
                current_range = Some(r);
            }

            if let Some(range_val) = current_range {
                results.push(range_val);
            }
        }
    }

    json!(results)
}

pub fn handle_document_link(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let doc_text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return json!([]);
    };

    let mut links = Vec::new();
    for (line_idx, line) in doc_text.lines().enumerate() {
        let line_num = line_idx as u32;
        let mut start_idx = 0;
        while let Some(pos) = line[start_idx..].find("https://") {
            let abs_pos = start_idx + pos;
            let end_pos = line[abs_pos..]
                .find(|c: char| c.is_whitespace() || c == ')' || c == '>' || c == ']' || c == '"')
                .map(|p| abs_pos + p)
                .unwrap_or(line.len());

            let url = &line[abs_pos..end_pos];
            let start_char = line[..abs_pos].encode_utf16().count() as u32;
            let end_char = line[..end_pos].encode_utf16().count() as u32;

            links.push(json!({
                "range": {
                    "start": { "line": line_num, "character": start_char },
                    "end": { "line": line_num, "character": end_char }
                },
                "target": url
            }));

            start_idx = end_pos;
        }
    }

    json!(links)
}

pub fn handle_document_link_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}

pub fn handle_moniker(
    msg: &Value,
    _workspace: Option<&WorkspaceIndex>,
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

    if let Some(overlay) = overlay_opt {
        if let Some(def) = overlay.find_definition_at(line_1based, col_1based) {
            return json!([
                {
                    "scheme": "rust",
                    "identifier": format!("{}::{}", def.file, def.name),
                    "unique": "project"
                }
            ]);
        }
    }

    json!([])
}

pub fn handle_text_document_content(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else {
        std::fs::read_to_string(&file).unwrap_or_default()
    };

    json!({ "text": text })
}
