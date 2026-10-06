/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP type hierarchy navigation handlers.

use crate::lsp::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

pub fn handle_prepare_type_hierarchy(
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

    let mut items = Vec::new();

    if let Some(def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        if matches!(
            def.kind,
            DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::TyAlias
        ) {
            let overlay = overlay_opt.unwrap();
            let (sc, ec) = (
                overlay.char_col_to_utf16_col(def.start_line, def.start_col),
                overlay.char_col_to_utf16_col(def.end_line, def.end_col),
            );
            let range = json!({
                "start": { "line": def.start_line.saturating_sub(1), "character": sc },
                "end": { "line": def.end_line.saturating_sub(1), "character": ec }
            });
            items.push(json!({
                "name": def.name,
                "kind": lsp_symbol_kind(def.kind),
                "detail": def.signature,
                "uri": uri,
                "range": range,
                "selectionRange": range
            }));
            return json!(items);
        }
    }

    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
                if matches!(
                    def.kind,
                    DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::TyAlias
                ) {
                    let d_path = def.resolved_path();
                    let d_str = d_path.to_str().unwrap_or(def.file);
                    let sc = conv.char_col_to_utf16_col(d_str, def.start_line, def.start_col);
                    let ec = conv.char_col_to_utf16_col(d_str, def.end_line, def.end_col);
                    let range = json!({
                        "start": { "line": def.start_line.saturating_sub(1), "character": sc },
                        "end": { "line": def.end_line.saturating_sub(1), "character": ec }
                    });
                    items.push(json!({
                        "name": def.name,
                        "kind": lsp_symbol_kind(def.kind),
                        "detail": def.signature,
                        "uri": path_to_uri(d_str),
                        "range": range,
                        "selectionRange": range
                    }));
                }
            }
        }
    }

    json!(items)
}

pub fn handle_type_hierarchy_supertypes(msg: &Value, workspace: Option<&WorkspaceIndex>) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");

    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut supertypes = Vec::new();
    let symbols = ws.search_symbols(name);
    for sym in symbols {
        if sym.name == name && matches!(sym.kind, DefKind::Struct | DefKind::Enum) {
            let refs = ws.find_references(sym.path);
            for r in refs {
                if r.kind == DefKind::Impl {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let uri = path_to_uri(r_str);
                    let range = json!({
                        "start": { "line": r.span.start_line().saturating_sub(1), "character": r.span.start_col().saturating_sub(1) },
                        "end": { "line": r.span.end_line().saturating_sub(1), "character": r.span.end_col().saturating_sub(1) }
                    });
                    supertypes.push(json!({
                        "name": format!("impl for {}", name),
                        "kind": 11,
                        "uri": uri,
                        "range": range,
                        "selectionRange": range
                    }));
                }
            }
        }
    }

    json!(supertypes)
}

pub fn handle_type_hierarchy_subtypes(msg: &Value, workspace: Option<&WorkspaceIndex>) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");

    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut subtypes = Vec::new();
    let symbols = ws.search_symbols(name);
    for sym in symbols {
        if sym.name == name && sym.kind == DefKind::Trait {
            let refs = ws.find_references(sym.path);
            for r in refs {
                if r.kind == DefKind::Impl {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let uri = path_to_uri(r_str);
                    let range = json!({
                        "start": { "line": r.span.start_line().saturating_sub(1), "character": r.span.start_col().saturating_sub(1) },
                        "end": { "line": r.span.end_line().saturating_sub(1), "character": r.span.end_col().saturating_sub(1) }
                    });
                    subtypes.push(json!({
                        "name": format!("impl of {}", name),
                        "kind": 23,
                        "uri": uri,
                        "range": range,
                        "selectionRange": range
                    }));
                }
            }
        }
    }

    json!(subtypes)
}
