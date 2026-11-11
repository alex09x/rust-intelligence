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
mod lens_runnable_hints;
mod lens_runnables;
mod lens_targets;
pub mod reconcile;
pub mod selection;
pub mod tokens;
mod tokens_attr;
mod tokens_detached;
mod tokens_doc;
mod tokens_doc_macro;
mod tokens_fixture;
mod tokens_gated;
mod tokens_inactive;
mod tokens_keywords;
#[cfg(test)]
mod tokens_rules_tests;
pub mod tokens_scan;
pub mod workspace_diag;

#[cfg(test)]
pub mod lens_filter_tests;
#[cfg(test)]
pub mod lens_overlay_tests;
#[cfg(test)]
pub mod lens_tests;
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
pub use selection::*;
pub use tokens::*;
pub use workspace_diag::*;

use super::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

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
            let end_pos = line[abs_pos..].find(|c: char| c.is_whitespace() || c == ')' || c == '>' || c == ']' || c == '"').map(|p| abs_pos + p).unwrap_or(line.len());

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

pub fn handle_moniker(msg: &Value, _workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
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

    let text = if let Some(overlay) = overlay_mgr.get(&file) { overlay.text.clone() } else { std::fs::read_to_string(&file).unwrap_or_default() };

    json!({ "text": text })
}
