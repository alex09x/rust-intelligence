/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP inlay hints, inline values, and inline completions.

use super::super::error::{LspError, LspResult};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_inlay_hint(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager) -> LspResult<Value> {
    super::inlay::handle_inlay_hint(msg, workspace, overlay_mgr)
}

pub fn handle_inline_value(msg: &Value, overlay_mgr: Option<&crate::overlay::OverlayManager>) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = crate::lsp::protocol::uri_to_path(uri);
    let sl = msg["params"]["range"]["start"]["line"].as_u64().unwrap_or(0) as usize;
    let el = msg["params"]["range"]["end"]["line"].as_u64().unwrap_or(sl as u64) as usize;

    let doc_text = if let Some(mgr) = overlay_mgr {
        if let Some(overlay) = mgr.get(&file) {
            overlay.text.clone()
        } else {
            std::fs::read_to_string(&file).unwrap_or_default()
        }
    } else {
        std::fs::read_to_string(&file).unwrap_or_default()
    };

    let mut values = Vec::new();
    for (l, line_str) in doc_text.lines().enumerate().take(el + 1).skip(sl) {
        let mut in_ident = false;
        let mut start_col = 0u32;
        let mut cur_col = 0u32;
        let mut cur_ident = String::new();

        for ch in line_str.chars() {
            let ch_utf16_len = ch.len_utf16() as u32;
            if ch.is_alphanumeric() || ch == '_' {
                if !in_ident {
                    in_ident = true;
                    start_col = cur_col;
                    cur_ident.clear();
                }
                cur_ident.push(ch);
            } else if in_ident {
                in_ident = false;
                if !crate::lsp::editing::ident::is_rust_keyword(&cur_ident) && !cur_ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    values.push(json!({
                        "range": {
                            "start": { "line": l, "character": start_col },
                            "end": { "line": l, "character": cur_col }
                        },
                        "variableName": cur_ident,
                        "caseSensitiveLookup": true
                    }));
                }
            }
            cur_col += ch_utf16_len;
        }

        if in_ident && !crate::lsp::editing::ident::is_rust_keyword(&cur_ident) && !cur_ident.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            values.push(json!({
                "range": {
                    "start": { "line": l, "character": start_col },
                    "end": { "line": l, "character": cur_col }
                },
                "variableName": cur_ident,
                "caseSensitiveLookup": true
            }));
        }
    }
    json!(values)
}

pub fn handle_inline_completion(msg: &Value, overlay_mgr: Option<&crate::overlay::OverlayManager>) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = crate::lsp::protocol::uri_to_path(uri);
    let line = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as usize;
    let target_char_col = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as usize;

    let doc_text = if let Some(mgr) = overlay_mgr {
        if let Some(overlay) = mgr.get(&file) {
            overlay.text.clone()
        } else {
            std::fs::read_to_string(&file).unwrap_or_default()
        }
    } else {
        std::fs::read_to_string(&file).unwrap_or_default()
    };

    let mut items = Vec::new();
    if let Some(line_str) = doc_text.lines().nth(line) {
        let target_byte_offset = crate::lsp::editing::ident::utf16_col_to_byte_offset(line_str, target_char_col);
        let prefix_slice = &line_str[..target_byte_offset.min(line_str.len())];

        let mut ident_start_byte = prefix_slice.len();
        for (b_idx, c) in prefix_slice.char_indices().rev() {
            if c.is_alphanumeric() || c == '_' {
                ident_start_byte = b_idx;
            } else {
                break;
            }
        }

        let prefix = &prefix_slice[ident_start_byte..];
        if !prefix.is_empty() {
            let start_char_col = crate::lsp::editing::ident::byte_offset_to_utf16_col(line_str, ident_start_byte) as u32;
            items.push(json!({
                "insertText": prefix.to_string(),
                "range": {
                    "start": { "line": line, "character": start_char_col },
                    "end": { "line": line, "character": target_char_col as u32 }
                }
            }));
        }
    }
    json!({ "items": items })
}

pub fn handle_inlay_hint_resolve(msg: &Value) -> LspResult<Value> {
    let params = msg.get("params").ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be an InlayHint object"));
    }

    if params.get("position").is_none() || params.get("label").is_none() {
        return Err(LspError::invalid_params("InlayHint must contain position and label"));
    }

    let mut hint = params.clone();
    if hint.get("tooltip").is_none() {
        hint["tooltip"] = json!("Inferred type by ridx");
    }
    Ok(hint)
}
