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

pub mod color;
pub mod compiler_message;
pub mod diagnostics;
pub mod flycheck;
pub mod folding;
pub mod lens;
pub mod reconcile;
pub mod tokens;
pub mod workspace_diag;

#[cfg(test)]
pub mod selection_range_tests;
#[cfg(test)]
pub mod tests;

pub use color::*;
pub use compiler_message::*;
pub use diagnostics::*;
pub use flycheck::*;
pub use folding::*;
pub use lens::*;
pub use reconcile::*;
pub use tokens::*;
pub use workspace_diag::*;

use super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::{DocumentOverlay, OverlayManager};
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

fn selection_ranges_for_overlay(overlay: &DocumentOverlay, pos_list: &[Value]) -> Value {
    let mut results = Vec::with_capacity(pos_list.len());
    for pos in pos_list {
        let line_0based = pos["line"].as_u64().unwrap_or(0) as u32;
        let utf16_col_0based = pos["character"].as_u64().unwrap_or(0) as u32;
        let byte_pos = overlay.position_to_byte_offset(line_0based, utf16_col_0based);

        let mut matching_scopes: Vec<_> = overlay
            .scopes
            .iter()
            .filter(|s| byte_pos >= s.start_byte && byte_pos <= s.end_byte)
            .collect();

        matching_scopes
            .sort_by_key(|s| (s.end_byte - s.start_byte, std::cmp::Reverse(s.start_byte)));

        let mut chain: Vec<&crate::overlay::parser::Scope> =
            Vec::with_capacity(matching_scopes.len());
        for s in matching_scopes {
            if let Some(prev) = chain.last() {
                let strictly_contains = s.start_byte <= prev.start_byte
                    && s.end_byte >= prev.end_byte
                    && (s.start_byte < prev.start_byte || s.end_byte > prev.end_byte);
                if !strictly_contains {
                    continue;
                }
            }
            chain.push(s);
        }

        let mut current_range: Option<Value> = None;
        for s in chain.into_iter().rev() {
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

        results.push(current_range.unwrap_or_else(|| {
            json!({
                "range": {
                    "start": { "line": line_0based, "character": utf16_col_0based },
                    "end": { "line": line_0based, "character": utf16_col_0based }
                },
                "parent": Value::Null
            })
        }));
    }

    json!(results)
}

pub fn handle_selection_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let Some(pos_list) = msg["params"]["positions"]
        .as_array()
        .filter(|p| !p.is_empty())
    else {
        return json!([]);
    };

    if let Some(overlay) = overlay_mgr.get(&file) {
        return selection_ranges_for_overlay(overlay, pos_list);
    }

    let Ok(content) = std::fs::read_to_string(&file) else {
        return json!([]);
    };
    let overlay = DocumentOverlay::parse_detached(content);
    selection_ranges_for_overlay(&overlay, pos_list)
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

            start_idx = end_pos.max(start_idx + 1);
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

pub fn handle_moniker_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
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
