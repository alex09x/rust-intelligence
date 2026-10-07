/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP semantic tokens, folding ranges, code lenses, and color presentation.

use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

pub fn handle_document_color(_msg: &Value) -> Value {
    json!([])
}

pub fn handle_color_presentation(_msg: &Value) -> Value {
    json!([])
}

pub fn handle_semantic_tokens_full(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut data = Vec::new();
    if let Some(overlay) = overlay_mgr.get(&file) {
        let mut prev_line = 0;
        let mut prev_col = 0;

        for sym in &overlay.symbols {
            let line_0based = sym.start_line.saturating_sub(1);
            let col_0based = overlay.char_col_to_utf16_col(sym.start_line, sym.start_col);
            let length = overlay
                .char_col_to_utf16_col(sym.end_line, sym.end_col)
                .saturating_sub(col_0based);

            let delta_line = line_0based.saturating_sub(prev_line);
            let delta_col = if delta_line == 0 {
                col_0based.saturating_sub(prev_col)
            } else {
                col_0based
            };

            let token_type = match sym.kind {
                ridx_format::DefKind::Fn => 0,
                ridx_format::DefKind::Struct => 1,
                ridx_format::DefKind::Enum => 2,
                ridx_format::DefKind::Trait => 3,
                ridx_format::DefKind::Const => 4,
                ridx_format::DefKind::Mod => 5,
                _ => 6,
            };

            data.extend_from_slice(&[delta_line, delta_col, length, token_type, 0]);

            prev_line = line_0based;
            prev_col = col_0based;
        }
    }

    json!({ "data": data })
}

pub fn handle_semantic_tokens_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let start_line = msg["params"]["range"]["start"]["line"]
        .as_u64()
        .unwrap_or(0) as u32;
    let end_line = msg["params"]["range"]["end"]["line"]
        .as_u64()
        .unwrap_or(u32::MAX as u64) as u32;

    let mut data = Vec::new();
    if let Some(overlay) = overlay_mgr.get(&file) {
        let mut prev_line = 0;
        let mut prev_col = 0;

        for sym in &overlay.symbols {
            let line_0based = sym.start_line.saturating_sub(1);
            if line_0based < start_line || line_0based > end_line {
                continue;
            }
            let col_0based = overlay.char_col_to_utf16_col(sym.start_line, sym.start_col);
            let length = overlay
                .char_col_to_utf16_col(sym.end_line, sym.end_col)
                .saturating_sub(col_0based);

            let delta_line = line_0based.saturating_sub(prev_line);
            let delta_col = if delta_line == 0 {
                col_0based.saturating_sub(prev_col)
            } else {
                col_0based
            };

            let token_type = match sym.kind {
                ridx_format::DefKind::Fn => 0,
                ridx_format::DefKind::Struct => 1,
                ridx_format::DefKind::Enum => 2,
                ridx_format::DefKind::Trait => 3,
                ridx_format::DefKind::Const => 4,
                ridx_format::DefKind::Mod => 5,
                _ => 6,
            };

            data.extend_from_slice(&[delta_line, delta_col, length, token_type, 0]);

            prev_line = line_0based;
            prev_col = col_0based;
        }
    }

    json!({ "data": data })
}

pub fn handle_folding_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut ranges = Vec::new();
    if let Some(overlay) = overlay_mgr.get(&file) {
        for s in &overlay.scopes {
            let (sl, sc) = overlay.byte_offset_to_position(s.start_byte);
            let (el, ec) = overlay.byte_offset_to_position(s.end_byte);
            if el > sl {
                ranges.push(json!({
                    "startLine": sl,
                    "startCharacter": sc,
                    "endLine": el,
                    "endCharacter": ec,
                    "kind": "region"
                }));
            }
        }
    }

    json!(ranges)
}

pub fn handle_code_lens(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut lenses = Vec::new();
    if let Some(overlay) = overlay_mgr.get(&file) {
        for sym in &overlay.symbols {
            if sym.start_line == sym.end_line {
                let sl = sym.start_line.saturating_sub(1);
                let sc = overlay.char_col_to_utf16_col(sym.start_line, sym.start_col);
                let ec = overlay.char_col_to_utf16_col(sym.end_line, sym.end_col);
                lenses.push(json!({
                    "range": {
                        "start": { "line": sl, "character": sc },
                        "end": { "line": sl, "character": ec }
                    },
                    "command": {
                        "title": format!("References: {}", sym.name),
                        "command": "ridx.showReferences"
                    }
                }));
            }
        }
    }

    json!(lenses)
}

pub fn handle_code_lens_resolve(msg: &Value) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}
