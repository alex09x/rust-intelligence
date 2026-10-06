/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP navigation handlers: definition, declaration, type definition, and implementation.

pub mod references;

#[cfg(test)]
pub mod tests;

pub use references::*;

use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

pub fn handle_definition(
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

    // 1. Overlay definition
    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        let uri = path_to_uri(&def.file);
        let ov = overlay_opt.unwrap();
        let start_char = ov.char_col_to_utf16_col(def.start_line, def.start_col);
        let end_char = ov.char_col_to_utf16_col(def.end_line, def.end_col);
        return json!({
            "uri": uri,
            "range": {
                "start": { "line": def.start_line.saturating_sub(1), "character": start_char },
                "end": { "line": def.end_line.saturating_sub(1), "character": end_char }
            }
        });
    }

    // 2. Workspace definition
    if let Some(ws) = workspace {
        let (query_line, query_col) = if let Some(overlay) = overlay_opt.filter(|o| o.is_dirty()) {
            if overlay.has_modified_imports() {
                return Value::Null;
            }
            if let Some((cl, cc)) = overlay.map_live_to_compiled(line_1based, col_1based) {
                (cl, cc)
            } else {
                return Value::Null;
            }
        } else {
            (line_1based, col_1based)
        };

        if let Some(def) = ws.find_definition_at(&file, query_line, query_col) {
            let resolved_path = def.resolved_path();
            let path_str = resolved_path.to_str().unwrap_or(def.file);
            let uri = path_to_uri(path_str);
            let (start_line, start_col, end_line, end_col) =
                if let Some(target_ov) = overlay_mgr.get(path_str).filter(|o| o.is_dirty()) {
                    let sl = target_ov
                        .map_compiled_to_live(def.start_line, def.start_col)
                        .unwrap_or((def.start_line, def.start_col));
                    let el = target_ov
                        .map_compiled_to_live(def.end_line, def.end_col)
                        .unwrap_or((def.end_line, def.end_col));
                    (sl.0, sl.1, el.0, el.1)
                } else {
                    (def.start_line, def.start_col, def.end_line, def.end_col)
                };

            let start_char = conv.char_col_to_utf16_col(path_str, start_line, start_col);
            let end_char = conv.char_col_to_utf16_col(path_str, end_line, end_col);
            return json!({
                "uri": uri,
                "range": {
                    "start": { "line": start_line.saturating_sub(1), "character": start_char },
                    "end": { "line": end_line.saturating_sub(1), "character": end_char }
                }
            });
        }
    }

    Value::Null
}

pub fn handle_declaration(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    handle_definition(msg, workspace, overlay_mgr, conv)
}

pub fn handle_type_definition(
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

    let Some(ws) = workspace else {
        return Value::Null;
    };

    let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
    if is_dirty {
        return Value::Null;
    }

    if let Some(ty) = ws.type_at(&file, line_1based, col_1based) {
        if let Some(def) = ws.search_symbols(ty).into_iter().next() {
            let resolved_path = def.resolved_path();
            let path_str = resolved_path.to_str().unwrap_or(def.file);
            let uri = path_to_uri(path_str);
            let start_char =
                conv.char_col_to_utf16_col(path_str, def.span.start_line(), def.span.start_col());
            let end_char =
                conv.char_col_to_utf16_col(path_str, def.span.end_line(), def.span.end_col());
            return json!({
                "uri": uri,
                "range": {
                    "start": { "line": def.span.start_line().saturating_sub(1), "character": start_char },
                    "end": { "line": def.span.end_line().saturating_sub(1), "character": end_char }
                }
            });
        }
    }

    Value::Null
}

pub fn handle_implementation(
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

    let Some(ws) = workspace else {
        return json!([]);
    };

    let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
    if is_dirty {
        return json!([]);
    }

    let mut results = Vec::new();
    if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
        let refs = ws.find_references(def.path);
        for r in refs {
            let is_impl = if r.kind == DefKind::Impl {
                true
            } else {
                let resolved_path = r.resolved_path();
                let path_str = resolved_path.to_str().unwrap_or(r.file);
                if let Ok(content) = std::fs::read_to_string(path_str) {
                    let line_text = content
                        .lines()
                        .nth(r.span.start_line().saturating_sub(1) as usize)
                        .unwrap_or("");
                    let trimmed = line_text.trim();
                    trimmed.starts_with("impl")
                        || (trimmed.contains("impl ")
                            && !trimmed.contains("where ")
                            && !trimmed.contains("<T:"))
                } else {
                    false
                }
            };

            if is_impl {
                let resolved_path = r.resolved_path();
                let path_str = resolved_path.to_str().unwrap_or(r.file);
                let uri = path_to_uri(path_str);
                let span = r.span;
                let start_char =
                    conv.char_col_to_utf16_col(path_str, span.start_line(), span.start_col());
                let end_char =
                    conv.char_col_to_utf16_col(path_str, span.end_line(), span.end_col());
                results.push(json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": span.start_line().saturating_sub(1), "character": start_char },
                        "end": { "line": span.end_line().saturating_sub(1), "character": end_char }
                    }
                }));
            }
        }
    }

    json!(results)
}
