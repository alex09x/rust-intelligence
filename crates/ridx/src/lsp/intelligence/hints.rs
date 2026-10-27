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
    let (uri, start_line, _start_col, end_line, _end_col) =
        crate::lsp::params::text_document_range(msg)?;

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

        if let Some((ident_start_byte, _var_name, ident_end_byte)) = find_let_binding(line_str) {
            let line_1based = line_0based + 1;
            let col_1based = line_str[..ident_start_byte].chars().count() as u32 + 1;
            let utf16_char_pos = line_str[..ident_end_byte].encode_utf16().count() as u32;

            let inferred_type = workspace
                .and_then(|ws| ws.hover_at(&file, line_1based, col_1based))
                .and_then(|hover| hover.inferred_type)
                .map(|t| t.to_string());

            if let Some(t) = inferred_type {
                hints.push(json!({
                    "position": {
                        "line": line_0based,
                        "character": utf16_char_pos
                    },
                    "label": format!(": {}", t),
                    "kind": 1,
                    "paddingLeft": true
                }));
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
        return Err(LspError::invalid_params(
            "Params must be an InlayHint object",
        ));
    }

    if params.get("position").is_none() || params.get("label").is_none() {
        return Err(LspError::invalid_params(
            "InlayHint must contain position and label",
        ));
    }

    let mut hint = params.clone();
    if hint.get("tooltip").is_none() {
        hint["tooltip"] = json!("Inferred type by ridx");
    }
    Ok(hint)
}

fn find_let_binding(line_str: &str) -> Option<(usize, &str, usize)> {
    let mut rest = line_str;
    let mut offset = 0;

    loop {
        let trimmed_len = rest.trim_start().len();
        let ws_len = rest.len() - trimmed_len;
        offset += ws_len;
        rest = &rest[ws_len..];

        if rest.starts_with("/*") {
            let end_idx = rest.find("*/")?;
            let comment_len = end_idx + 2;
            offset += comment_len;
            rest = &rest[comment_len..];
        } else {
            break;
        }
    }

    let rest_after_let = rest.strip_prefix("let ")?;
    let mut let_kw_len = 4;
    let mut after_let = rest_after_let;

    if let Some(after_mut) = rest_after_let.strip_prefix("mut ") {
        let_kw_len += 4;
        after_let = after_mut;
    }

    let ws_len = after_let.len() - after_let.trim_start().len();
    let ident_start = offset + let_kw_len + ws_len;
    let trimmed_ident = after_let.trim_start();

    let ident_end_rel = trimmed_ident
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(trimmed_ident.len());
    let var_name = &trimmed_ident[..ident_end_rel];

    if var_name.is_empty() {
        return None;
    }

    if trimmed_ident[ident_end_rel..].trim_start().starts_with(':') {
        return None;
    }

    let ident_end = ident_start + var_name.len();
    Some((ident_start, var_name, ident_end))
}
