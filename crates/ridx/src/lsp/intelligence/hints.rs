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

use super::super::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_inlay_hint(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let start_line = msg["params"]["range"]["start"]["line"]
        .as_u64()
        .unwrap_or(0) as u32;
    let end_line = msg["params"]["range"]["end"]["line"]
        .as_u64()
        .unwrap_or(u32::MAX as u64) as u32;

    let overlay_opt = overlay_mgr.get(&file);
    let text = if let Some(ov) = overlay_opt {
        ov.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return json!([]);
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

    json!(hints)
}

pub fn handle_inline_value(_msg: &Value) -> Value {
    json!([])
}

pub fn handle_inline_completion(_msg: &Value) -> Value {
    json!({ "items": [] })
}
