/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP prepareRename handler for validating rename targets and computing placeholder ranges.

use super::ident::*;
use crate::lsp::error::{LspError, LspResult};
use crate::lsp::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_prepare_rename(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
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
        let refs = overlay.find_local_binding_references(line_1based, col_1based);
        if let Some((sl, sc, el, ec)) = refs.iter().find(|(sl, sc, el, ec)| line_1based >= *sl && line_1based <= *el && col_1based >= *sc && col_1based <= *ec) {
            let start_char = overlay.char_col_to_utf16_col(*sl, *sc);
            let end_char = overlay.char_col_to_utf16_col(*el, *ec);
            let ident = overlay.get_ident_at(*sl, *sc).unwrap_or_default();
            return Ok(json!({
                "range": {
                    "start": { "line": sl.saturating_sub(1), "character": start_char },
                    "end": { "line": el.saturating_sub(1), "character": end_char }
                },
                "placeholder": ident
            }));
        }
    }

    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                let target_name = loc_refs.target_symbol.rsplit("::").next().unwrap_or(loc_refs.target_symbol);
                if target_name.is_empty() {
                    return Err(LspError::request_failed("Cannot prepare rename: no identifier found at this position"));
                }
                if loc_refs.is_external(ws) {
                    return Err(LspError::request_failed(format!("Cannot rename external symbol '{target_name}'")));
                }

                let line_str = if let Some(ov) = overlay_opt {
                    let idx = line_0based as usize;
                    ov.text.lines().nth(idx).map(|s| s.to_string())
                } else {
                    std::fs::read_to_string(&file).ok().and_then(|t| t.lines().nth(line_0based as usize).map(|s| s.to_string()))
                };

                let ident_col = line_str.as_deref().and_then(|s| find_declaration_ident_col(s, target_name, loc_refs.target_kind)).unwrap_or(col_1based);

                let sc = if let Some(ov) = overlay_opt { ov.char_col_to_utf16_col(line_1based, ident_col) } else { conv.char_col_to_utf16_col(&file, line_1based, ident_col) };
                let ec = sc + target_name.chars().count() as u32;

                return Ok(json!({
                    "range": {
                        "start": { "line": line_0based, "character": sc },
                        "end": { "line": line_0based, "character": ec }
                    },
                    "placeholder": target_name
                }));
            }
        }
    }

    Err(LspError::request_failed("Cannot prepare rename: no identifier found at this position"))
}
