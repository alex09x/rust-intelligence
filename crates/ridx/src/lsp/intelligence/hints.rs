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
use super::super::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_inlay_hint(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
) -> LspResult<Value> {
    let params = msg
        .get("params")
        .ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be an object"));
    }

    let uri = params["textDocument"]["uri"]
        .as_str()
        .ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
    if uri.is_empty() {
        return Err(LspError::invalid_params("Empty textDocument.uri"));
    }

    let range = &params["range"];
    if !range.is_object() {
        return Err(LspError::invalid_params("Missing range object"));
    }
    let start_line = range["start"]["line"]
        .as_u64()
        .ok_or_else(|| LspError::invalid_params("Missing range.start.line"))? as u32;
    let end_line = range["end"]["line"]
        .as_u64()
        .ok_or_else(|| LspError::invalid_params("Missing range.end.line"))? as u32;

    let file = uri_to_path(uri);
    let overlay_opt = overlay_mgr.get(&file);
    let text = if let Some(ov) = overlay_opt {
        ov.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return Ok(json!([]));
    };

    let mut hints = Vec::new();

    for (line_idx, line_str) in text.lines().enumerate() {
        let line_0based = line_idx as u32;
        if line_0based < start_line || line_0based > end_line {
            continue;
        }

        let trimmed = line_str.trim_start();
        if let Some(rest) = trimmed.strip_prefix("let ") {
            let (after_mut, had_mut) = if let Some(m) = rest.strip_prefix("mut ") {
                (m, true)
            } else {
                (rest, false)
            };
            let ident_end = after_mut
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(after_mut.len());
            let var_name = &after_mut[..ident_end];

            if !var_name.is_empty() && !after_mut[ident_end..].trim_start().starts_with(':') {
                let indent_chars = line_str.len() - trimmed.len();
                let ident_start_char = indent_chars + 4 + if had_mut { 4 } else { 0 };
                let ident_end_char = ident_start_char + var_name.chars().count();

                let line_1based = line_0based + 1;
                let col_1based = (ident_start_char + 1) as u32;

                let inferred_type = workspace
                    .and_then(|ws| ws.hover_at(&file, line_1based, col_1based))
                    .and_then(|hover| hover.inferred_type)
                    .map(|t| t.to_string());

                if let Some(t) = inferred_type {
                    hints.push(json!({
                        "position": {
                            "line": line_0based,
                            "character": ident_end_char
                        },
                        "label": format!(": {}", t),
                        "kind": 1,
                        "paddingLeft": true
                    }));
                }
            }
        }
    }

    Ok(json!(hints))
}

pub fn handle_inline_value(
    msg: &Value,
    overlay_mgr: Option<&crate::overlay::OverlayManager>,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = crate::lsp::protocol::uri_to_path(uri);
    let sl = msg["params"]["range"]["start"]["line"]
        .as_u64()
        .unwrap_or(0) as usize;
    let el = msg["params"]["range"]["end"]["line"]
        .as_u64()
        .unwrap_or(sl as u64) as usize;

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
                if !crate::lsp::editing::ident::is_rust_keyword(&cur_ident)
                    && !cur_ident.chars().next().is_some_and(|c| c.is_ascii_digit())
                {
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

        if in_ident
            && !crate::lsp::editing::ident::is_rust_keyword(&cur_ident)
            && !cur_ident.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
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

pub fn handle_inline_completion(
    msg: &Value,
    overlay_mgr: Option<&crate::overlay::OverlayManager>,
) -> Value {
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
        let target_byte_offset =
            crate::lsp::editing::ident::utf16_col_to_byte_offset(line_str, target_char_col);
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
            let start_char_col =
                crate::lsp::editing::ident::byte_offset_to_utf16_col(line_str, ident_start_byte)
                    as u32;
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
    let params = msg
        .get("params")
        .ok_or_else(|| LspError::invalid_params("Missing params object"))?;
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
