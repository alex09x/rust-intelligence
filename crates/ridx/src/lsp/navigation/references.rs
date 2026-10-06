/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP reference tracking and document highlight handlers.

use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_references(
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
    let include_decl = msg["params"]["context"]["includeDeclaration"]
        .as_bool()
        .unwrap_or(true);

    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    let mut results = Vec::new();

    // 1. Overlay local references
    if let Some(overlay) = overlay_opt {
        let local_refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !local_refs.is_empty() {
            for (sl, sc, el, ec) in local_refs {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                results.push(json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": sl.saturating_sub(1), "character": start_char },
                        "end": { "line": el.saturating_sub(1), "character": end_char }
                    }
                }));
            }
            return json!(results);
        }
    }

    // 2. Workspace compiled references
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                if include_decl
                    && !loc_refs.target_file.is_empty()
                    && loc_refs.target_file != "external"
                {
                    let decl_path = loc_refs.target_resolved_path();
                    let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                    let decl_uri = path_to_uri(decl_str);
                    let sc = conv.char_col_to_utf16_col(
                        decl_str,
                        loc_refs.target_line,
                        loc_refs.target_col,
                    );
                    let ec = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0 {
                        conv.char_col_to_utf16_col(
                            decl_str,
                            loc_refs.target_end_line,
                            loc_refs.target_end_col,
                        )
                    } else {
                        sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                    };
                    let sl = loc_refs.target_line.saturating_sub(1);
                    let el = if loc_refs.target_end_line > 0 {
                        loc_refs.target_end_line.saturating_sub(1)
                    } else {
                        sl
                    };
                    results.push(json!({
                        "uri": decl_uri,
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": el, "character": ec }
                        }
                    }));
                }

                for r in loc_refs.references {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);
                    let span = r.span;
                    let sc = conv.char_col_to_utf16_col(r_str, span.start_line(), span.start_col());
                    let ec = conv.char_col_to_utf16_col(r_str, span.end_line(), span.end_col());
                    results.push(json!({
                        "uri": r_uri,
                        "range": {
                            "start": { "line": span.start_line().saturating_sub(1), "character": sc },
                            "end": { "line": span.end_line().saturating_sub(1), "character": ec }
                        }
                    }));
                }
            }
        }
    }

    json!(results)
}

pub fn handle_document_highlight(
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

    let mut highlights = Vec::new();

    // 1. Overlay local references
    if let Some(overlay) = overlay_opt {
        let local_refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !local_refs.is_empty() {
            for (idx, (sl, sc, el, ec)) in local_refs.into_iter().enumerate() {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                let kind = if idx == 0 { 3 } else { 1 };
                highlights.push(json!({
                    "range": {
                        "start": { "line": sl.saturating_sub(1), "character": start_char },
                        "end": { "line": el.saturating_sub(1), "character": end_char }
                    },
                    "kind": kind
                }));
            }
            return json!(highlights);
        }
    }

    // 2. Workspace compiled references in the current document
    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                let decl_path = loc_refs.target_resolved_path();
                let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                if decl_str == file || decl_str.ends_with(&file) || file.ends_with(decl_str) {
                    let sc = conv.char_col_to_utf16_col(
                        &file,
                        loc_refs.target_line,
                        loc_refs.target_col,
                    );
                    let ec = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0 {
                        conv.char_col_to_utf16_col(
                            &file,
                            loc_refs.target_end_line,
                            loc_refs.target_end_col,
                        )
                    } else {
                        sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                    };
                    let sl = loc_refs.target_line.saturating_sub(1);
                    let el = if loc_refs.target_end_line > 0 {
                        loc_refs.target_end_line.saturating_sub(1)
                    } else {
                        sl
                    };
                    highlights.push(json!({
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": el, "character": ec }
                        },
                        "kind": 3
                    }));
                }

                for r in loc_refs.references {
                    let r_path = r.resolved_path();
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    if r_str == file || r_str.ends_with(&file) || file.ends_with(r_str) {
                        let span = r.span;
                        let sc =
                            conv.char_col_to_utf16_col(&file, span.start_line(), span.start_col());
                        let ec = conv.char_col_to_utf16_col(&file, span.end_line(), span.end_col());
                        highlights.push(json!({
                            "range": {
                                "start": { "line": span.start_line().saturating_sub(1), "character": sc },
                                "end": { "line": span.end_line().saturating_sub(1), "character": ec }
                            },
                            "kind": 1
                        }));
                    }
                }
            }
        }
    }

    json!(highlights)
}
