//! Thematic LSP request handlers for definition, references, hover, and symbols.

use super::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};

pub fn handle_definition(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let mut result = Value::Null;

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

    // 1. Check live buffer overlay first (resolves local variables, params, unsaved items)
    if let Some(overlay_def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        let def_uri = path_to_uri(&overlay_def.file);
        let (start_char, end_char) = if let Some(overlay) = overlay_opt {
            (
                overlay.char_col_to_utf16_col(overlay_def.start_line, overlay_def.start_col),
                overlay.char_col_to_utf16_col(overlay_def.end_line, overlay_def.end_col),
            )
        } else {
            (
                conv.char_col_to_utf16_col(
                    &overlay_def.file,
                    overlay_def.start_line,
                    overlay_def.start_col,
                ),
                conv.char_col_to_utf16_col(
                    &overlay_def.file,
                    overlay_def.end_line,
                    overlay_def.end_col,
                ),
            )
        };
        result = json!({
            "uri": def_uri,
            "range": {
                "start": {
                    "line": overlay_def.start_line.saturating_sub(1),
                    "character": start_char
                },
                "end": {
                    "line": overlay_def.end_line.saturating_sub(1),
                    "character": end_char
                }
            }
        });
    } else if let Some(ws) = workspace {
        // 2. Fall back to compiled zero-copy workspace index
        // If the current buffer is dirty (contains uncompiled edits), compiled fallback
        // must be suppressed to avoid returning stale cross-file definitions (e.g. from changed imports).
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
                let resolved_file_path = def.resolved_path();
                let resolved_file_str = resolved_file_path.to_str().unwrap_or(def.file);
                let def_uri = path_to_uri(resolved_file_str);

                let (start_line, end_line, start_char, end_char) = if let Some(target_overlay) =
                    overlay_mgr.get(resolved_file_str)
                {
                    let live_start_line = target_overlay.map_compiled_line_to_live(def.start_line);
                    let live_end_line = target_overlay.map_compiled_line_to_live(def.end_line);
                    match (live_start_line, live_end_line) {
                        (Some(sl), Some(el)) => (
                            sl.saturating_sub(1),
                            el.saturating_sub(1),
                            target_overlay.char_col_to_utf16_col(sl, def.start_col),
                            target_overlay.char_col_to_utf16_col(el, def.end_col),
                        ),
                        _ => (u32::MAX, u32::MAX, 0, 0),
                    }
                } else {
                    (
                        def.start_line.saturating_sub(1),
                        def.end_line.saturating_sub(1),
                        conv.char_col_to_utf16_col(
                            resolved_file_str,
                            def.start_line,
                            def.start_col,
                        ),
                        conv.char_col_to_utf16_col(resolved_file_str, def.end_line, def.end_col),
                    )
                };

                if start_line != u32::MAX {
                    result = json!({
                        "uri": def_uri,
                        "range": {
                            "start": {
                                "line": start_line,
                                "character": start_char
                            },
                            "end": {
                                "line": end_line,
                                "character": end_char
                            }
                        }
                    });
                }
            }
        }
    }

    result
}

pub fn handle_references(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let mut result = json!([]);

    if let Some(ws) = workspace {
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
        let include_decl = msg["params"]["context"]["includeDeclaration"]
            .as_bool()
            .unwrap_or(false);

        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                let mut locs = Vec::new();
                if include_decl
                    && !loc_refs.target_file.is_empty()
                    && loc_refs.target_file != "external"
                {
                    let decl_file_path = loc_refs.target_resolved_path();
                    let decl_file_str = decl_file_path.to_str().unwrap_or(loc_refs.target_file);
                    let decl_uri = path_to_uri(decl_file_str);

                    let (decl_start_line, decl_end_line, decl_start_col, decl_end_col) =
                        if let Some(target_overlay) = overlay_mgr.get(decl_file_str) {
                            let live_start =
                                target_overlay.map_compiled_line_to_live(loc_refs.target_line);
                            let end_line_to_map = if loc_refs.target_end_line > 0 {
                                loc_refs.target_end_line
                            } else {
                                loc_refs.target_line
                            };
                            let live_end =
                                target_overlay.map_compiled_line_to_live(end_line_to_map);
                            match (live_start, live_end) {
                                (Some(sl), Some(el)) => {
                                    let sc = target_overlay
                                        .char_col_to_utf16_col(sl, loc_refs.target_col);
                                    let ec = if loc_refs.target_end_line > 0
                                        && loc_refs.target_end_col > 0
                                    {
                                        target_overlay
                                            .char_col_to_utf16_col(el, loc_refs.target_end_col)
                                    } else {
                                        sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                                    };
                                    (sl.saturating_sub(1), el.saturating_sub(1), sc, ec)
                                }
                                _ => (u32::MAX, u32::MAX, 0, 0),
                            }
                        } else {
                            let sl = loc_refs.target_line.saturating_sub(1);
                            let sc = conv.char_col_to_utf16_col(
                                decl_file_str,
                                loc_refs.target_line,
                                loc_refs.target_col,
                            );
                            let el = if loc_refs.target_end_line > 0 {
                                loc_refs.target_end_line.saturating_sub(1)
                            } else {
                                sl
                            };
                            let ec = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0
                            {
                                conv.char_col_to_utf16_col(
                                    decl_file_str,
                                    loc_refs.target_end_line,
                                    loc_refs.target_end_col,
                                )
                            } else {
                                sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                            };
                            (sl, el, sc, ec)
                        };

                    if decl_start_line != u32::MAX {
                        locs.push(json!({
                            "uri": decl_uri,
                            "range": {
                                "start": {
                                    "line": decl_start_line,
                                    "character": decl_start_col
                                },
                                "end": {
                                    "line": decl_end_line,
                                    "character": decl_end_col
                                }
                            }
                        }));
                    }
                }

                for r in loc_refs.references {
                    let span = r.span;
                    let r_file_path = r.resolved_path();
                    let r_file_str = r_file_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_file_str);

                    let (start_line, end_line, start_char, end_char) = if let Some(target_overlay) =
                        overlay_mgr.get(r_file_str)
                    {
                        let live_start =
                            target_overlay.map_compiled_line_to_live(span.start_line());
                        let live_end = target_overlay.map_compiled_line_to_live(span.end_line());
                        match (live_start, live_end) {
                            (Some(sl), Some(el)) => (
                                sl.saturating_sub(1),
                                el.saturating_sub(1),
                                target_overlay.char_col_to_utf16_col(sl, span.start_col()),
                                target_overlay.char_col_to_utf16_col(el, span.end_col()),
                            ),
                            _ => (u32::MAX, u32::MAX, 0, 0),
                        }
                    } else {
                        (
                            span.start_line().saturating_sub(1),
                            span.end_line().saturating_sub(1),
                            conv.char_col_to_utf16_col(
                                r_file_str,
                                span.start_line(),
                                span.start_col(),
                            ),
                            conv.char_col_to_utf16_col(r_file_str, span.end_line(), span.end_col()),
                        )
                    };

                    if start_line != u32::MAX {
                        locs.push(json!({
                            "uri": r_uri,
                            "range": {
                                "start": {
                                    "line": start_line,
                                    "character": start_char
                                },
                                "end": {
                                    "line": end_line,
                                    "character": end_char
                                }
                            }
                        }));
                    }
                }
                result = json!(locs);
            }
        }
    }

    result
}

pub fn handle_hover(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let mut result = Value::Null;

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

    let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);

    // 1. When buffer is clean (!is_dirty), query compiled workspace index first to preserve inferred types
    if !is_dirty {
        if let Some(ws) = workspace {
            if let Some(hover) = ws.hover_at(&file, line_1based, col_1based) {
                let mut md = String::new();
                if let Some(ref def) = hover.definition {
                    md.push_str("```rust\n");
                    if let Some(sig) = def.signature {
                        md.push_str(sig);
                    } else {
                        match def.kind {
                            DefKind::Local => md.push_str(&format!("let {}", def.name)),
                            _ => md.push_str(&format!("{:?} {}", def.kind, def.name)),
                        }
                    }
                    md.push_str("\n```\n\n");
                    md.push_str(&format!(
                        "*Defined in `{}` at {}:{}*\n\n",
                        def.resolved_path().display(),
                        def.start_line,
                        def.start_col
                    ));
                    if let Some(ty) = hover.inferred_type {
                        md.push_str(&format!("**Type:** `{ty}`\n"));
                    }
                } else if let Some(ty) = hover.inferred_type {
                    if let Some(overlay_def) =
                        overlay_mgr.find_definition_at(&file, line_1based, col_1based)
                    {
                        md.push_str("```rust\n");
                        if let Some(ref sig) = overlay_def.signature {
                            md.push_str(sig);
                        } else {
                            match overlay_def.kind {
                                DefKind::Local => md.push_str(&format!("let {}", overlay_def.name)),
                                _ => md.push_str(&format!(
                                    "{:?} {}",
                                    overlay_def.kind, overlay_def.name
                                )),
                            }
                        }
                        md.push_str("\n```\n\n");
                        md.push_str(&format!(
                            "*Defined in `{}` at {}:{}*\n\n",
                            overlay_def.file, overlay_def.start_line, overlay_def.start_col
                        ));
                        md.push_str(&format!("**Type:** `{ty}`\n"));
                    } else {
                        md.push_str(&format!("**Type:** `{ty}`\n"));
                    }
                }

                if !md.is_empty() {
                    result = json!({
                        "contents": {
                            "kind": "markdown",
                            "value": md.trim()
                        }
                    });
                }
            }
        }
    }

    // 2. Fall back to overlay definition if clean hover returned nothing, or if buffer is dirty
    if result.is_null() {
        if let Some(overlay_def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
            let mut md = String::new();
            md.push_str("```rust\n");
            if let Some(ref sig) = overlay_def.signature {
                md.push_str(sig);
            } else {
                match overlay_def.kind {
                    DefKind::Local => md.push_str(&format!("let {}", overlay_def.name)),
                    _ => md.push_str(&format!("{:?} {}", overlay_def.kind, overlay_def.name)),
                }
            }
            md.push_str("\n```\n\n");
            md.push_str(&format!(
                "*Defined in `{}` at {}:{}*",
                overlay_def.file, overlay_def.start_line, overlay_def.start_col
            ));
            result = json!({
                "contents": {
                    "kind": "markdown",
                    "value": md.trim()
                }
            });
        }
    }

    result
}

pub fn handle_workspace_symbol(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    conv: &mut PositionConverter,
) -> Value {
    let mut result = json!([]);

    if let Some(ws) = workspace {
        let query = msg["params"]["query"].as_str().unwrap_or("");
        let symbols = ws.search_symbols(query);
        let mut list = Vec::new();

        for s in symbols.iter().take(100) {
            let span = s.span;
            let res_path = s.resolved_path();
            let res_str = res_path.to_str().unwrap_or(s.file);
            let uri = path_to_uri(res_str);
            let start_char =
                conv.char_col_to_utf16_col(res_str, span.start_line(), span.start_col());
            let end_char = conv.char_col_to_utf16_col(res_str, span.end_line(), span.end_col());
            list.push(json!({
                "name": s.name,
                "kind": lsp_symbol_kind(s.kind),
                "containerName": s.crate_name,
                "location": {
                    "uri": uri,
                    "range": {
                        "start": {
                            "line": span.start_line().saturating_sub(1),
                            "character": start_char
                        },
                        "end": {
                            "line": span.end_line().saturating_sub(1),
                            "character": end_char
                        }
                    }
                }
            }));
        }
        result = json!(list);
    }

    result
}

pub fn handle_document_symbol(
    msg: &Value,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let mut result = json!([]);
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    if let Some(symbols) = overlay_mgr.document_symbols(&file) {
        let overlay = overlay_mgr.get(&file);
        let mut list = Vec::new();
        for sym in symbols {
            let (start_char, end_char) = if let Some(ov) = overlay {
                (
                    ov.char_col_to_utf16_col(sym.start_line, sym.start_col),
                    ov.char_col_to_utf16_col(sym.end_line, sym.end_col),
                )
            } else {
                (
                    conv.char_col_to_utf16_col(&file, sym.start_line, sym.start_col),
                    conv.char_col_to_utf16_col(&file, sym.end_line, sym.end_col),
                )
            };
            list.push(json!({
                "name": sym.name,
                "kind": lsp_symbol_kind(sym.kind),
                "detail": sym.signature,
                "range": {
                    "start": {
                        "line": sym.start_line.saturating_sub(1),
                        "character": start_char
                    },
                    "end": {
                        "line": sym.end_line.saturating_sub(1),
                        "character": end_char
                    }
                },
                "selectionRange": {
                    "start": {
                        "line": sym.start_line.saturating_sub(1),
                        "character": start_char
                    },
                    "end": {
                        "line": sym.end_line.saturating_sub(1),
                        "character": end_char
                    }
                }
            }));
        }
        result = json!(list);
    }

    result
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

    if let Some(ty_str) = ws.type_at(&file, line_1based, col_1based) {
        let trimmed = ty_str.trim();
        let base_type = trimmed
            .trim_start_matches('&')
            .trim_start_matches("mut ")
            .trim_start_matches('*')
            .trim_start_matches("const ")
            .trim_start_matches("mut ")
            .trim_start_matches("dyn ")
            .trim();
        let bare_type = base_type.split('<').next().unwrap_or(base_type).trim();

        if let Some(def) = ws.find_definition_by_path(bare_type).or_else(|| {
            ws.search_symbols(bare_type)
                .into_iter()
                .find(|s| {
                    s.name == bare_type
                        && matches!(
                            s.kind,
                            DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::TyAlias
                        )
                })
                .and_then(|s| ws.find_definition_by_path(s.path))
        }) {
            let resolved_file_path = def.resolved_path();
            let resolved_file_str = resolved_file_path.to_str().unwrap_or(def.file);
            let def_uri = path_to_uri(resolved_file_str);

            let (start_line, end_line, start_char, end_char) = if let Some(target_overlay) =
                overlay_mgr.get(resolved_file_str)
            {
                let live_start_line = target_overlay.map_compiled_line_to_live(def.start_line);
                let live_end_line = target_overlay.map_compiled_line_to_live(def.end_line);
                match (live_start_line, live_end_line) {
                    (Some(sl), Some(el)) => (
                        sl.saturating_sub(1),
                        el.saturating_sub(1),
                        target_overlay.char_col_to_utf16_col(sl, def.start_col),
                        target_overlay.char_col_to_utf16_col(el, def.end_col),
                    ),
                    _ => (u32::MAX, u32::MAX, 0, 0),
                }
            } else {
                (
                    def.start_line.saturating_sub(1),
                    def.end_line.saturating_sub(1),
                    conv.char_col_to_utf16_col(resolved_file_str, def.start_line, def.start_col),
                    conv.char_col_to_utf16_col(resolved_file_str, def.end_line, def.end_col),
                )
            };

            if start_line != u32::MAX {
                return json!({
                    "uri": def_uri,
                    "range": {
                        "start": { "line": start_line, "character": start_char },
                        "end": { "line": end_line, "character": end_char }
                    }
                });
            }
        }
    }

    Value::Null
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

    if let Some(overlay_def) = overlay_mgr.find_definition_at(&file, line_1based, col_1based) {
        if overlay_def.file == file {
            let start_char = if let Some(ov) = overlay_opt {
                ov.char_col_to_utf16_col(overlay_def.start_line, overlay_def.start_col)
            } else {
                conv.char_col_to_utf16_col(&file, overlay_def.start_line, overlay_def.start_col)
            };
            let end_char = if let Some(ov) = overlay_opt {
                ov.char_col_to_utf16_col(overlay_def.end_line, overlay_def.end_col)
            } else {
                conv.char_col_to_utf16_col(&file, overlay_def.end_line, overlay_def.end_col)
            };
            highlights.push(json!({
                "range": {
                    "start": { "line": overlay_def.start_line.saturating_sub(1), "character": start_char },
                    "end": { "line": overlay_def.end_line.saturating_sub(1), "character": end_char }
                },
                "kind": 3 // Write / Definition
            }));
        }
    } else if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(loc_refs) = ws.find_references_at(&file, line_1based, col_1based) {
                let target_file_str = loc_refs.target_file;
                if target_file_str == file || file.ends_with(target_file_str) {
                    let start_char = conv.char_col_to_utf16_col(
                        &file,
                        loc_refs.target_line,
                        loc_refs.target_col,
                    );
                    let end_char = conv.char_col_to_utf16_col(
                        &file,
                        loc_refs.target_end_line,
                        loc_refs.target_end_col,
                    );
                    highlights.push(json!({
                        "range": {
                            "start": { "line": loc_refs.target_line.saturating_sub(1), "character": start_char },
                            "end": { "line": loc_refs.target_end_line.saturating_sub(1), "character": end_char }
                        },
                        "kind": 3
                    }));
                }
                for r in loc_refs.references {
                    let r_file_path = r.resolved_path();
                    let r_file_str = r_file_path.to_str().unwrap_or(r.file);
                    if r_file_str == file || file.ends_with(r_file_str) {
                        let span = r.span;
                        let start_char =
                            conv.char_col_to_utf16_col(&file, span.start_line(), span.start_col());
                        let end_char =
                            conv.char_col_to_utf16_col(&file, span.end_line(), span.end_col());
                        highlights.push(json!({
                            "range": {
                                "start": { "line": span.start_line().saturating_sub(1), "character": start_char },
                                "end": { "line": span.end_line().saturating_sub(1), "character": end_char }
                            },
                            "kind": if r.is_write { 3 } else { 2 } // 3 = Write, 2 = Read
                        }));
                    }
                }
            }
        }
    }

    json!(highlights)
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
            if matches!(r.kind, DefKind::AssocFn | DefKind::Trait | DefKind::Struct) {
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

pub fn handle_workspace_symbol_resolve(msg: &Value, _workspace: Option<&WorkspaceIndex>) -> Value {
    msg.get("params").cloned().unwrap_or(Value::Null)
}

pub fn handle_completion(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    _conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut items = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    // 1. Overlay symbols in open buffer
    if let Some(symbols) = overlay_mgr.document_symbols(&file) {
        for s in symbols {
            if seen_names.insert(s.name.clone()) {
                items.push(json!({
                    "label": s.name,
                    "kind": super::protocol::lsp_completion_kind(s.kind),
                    "detail": s.signature
                }));
            }
        }
    }

    // 2. Workspace symbols
    if let Some(ws) = workspace {
        let ws_symbols = ws.search_symbols("");
        for s in ws_symbols.into_iter().take(100) {
            if seen_names.insert(s.name.to_string()) {
                items.push(json!({
                    "label": s.name,
                    "kind": super::protocol::lsp_completion_kind(s.kind),
                    "detail": s.signature.map(|sig| sig.to_string())
                }));
            }
        }
    }

    json!({
        "isIncomplete": false,
        "items": items
    })
}

pub fn handle_signature_help(
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

    if let Some(ws) = workspace {
        if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based.saturating_sub(1)) {
            if let Some(sig) = def.signature {
                let params: Vec<Value> = if let Some(start) = sig.find('(') {
                    if let Some(end) = sig.rfind(')') {
                        let inner = &sig[start + 1..end];
                        inner
                            .split(',')
                            .map(|p| p.trim())
                            .filter(|p| !p.is_empty())
                            .map(|p| json!({ "label": p }))
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                return json!({
                    "signatures": [
                        {
                            "label": sig,
                            "parameters": params
                        }
                    ],
                    "activeSignature": 0,
                    "activeParameter": 0
                });
            }
        }
    }

    Value::Null
}

pub fn handle_formatting(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let doc_text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return json!([]);
    };

    use std::io::Write as _;
    let mut child = match std::process::Command::new("rustfmt")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return json!([]),
    };

    if let Some(ref mut stdin) = child.stdin {
        let _ = stdin.write_all(doc_text.as_bytes());
    }

    let output = match child.wait_with_output() {
        Ok(o) if o.status.success() => o,
        _ => return json!([]),
    };

    if let Ok(formatted) = String::from_utf8(output.stdout) {
        if formatted != doc_text {
            let line_count = doc_text.lines().count() as u32;
            return json!([
                {
                    "range": {
                        "start": { "line": 0, "character": 0 },
                        "end": { "line": line_count + 1, "character": 0 }
                    },
                    "newText": formatted
                }
            ]);
        }
    }

    json!([])
}

pub fn handle_diagnostic(_msg: &Value, _overlay_mgr: &OverlayManager) -> Value {
    json!({
        "kind": "full",
        "items": []
    })
}

pub fn handle_folding_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let Some(overlay) = overlay_mgr.get(&file) else {
        return json!([]);
    };
    let mut ranges = Vec::new();
    for scope in &overlay.scopes {
        if scope.id == 0 {
            continue;
        }
        let (start_line, _) = overlay.byte_to_char_line_col(scope.start_byte);
        let (end_line, _) = overlay.byte_to_char_line_col(scope.end_byte);
        if end_line > start_line {
            ranges.push(json!({
                "startLine": start_line.saturating_sub(1),
                "endLine": end_line.saturating_sub(1),
                "kind": "region"
            }));
        }
    }
    json!(ranges)
}

pub fn handle_selection_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let positions = msg["params"]["positions"].as_array();
    let Some(overlay) = overlay_mgr.get(&file) else {
        return json!([]);
    };
    let mut results = Vec::new();
    if let Some(pos_list) = positions {
        for pos in pos_list {
            let line_0based = pos["line"].as_u64().unwrap_or(0) as u32;
            let utf16_col_0based = pos["character"].as_u64().unwrap_or(0) as u32;
            let byte_pos = overlay.position_to_byte_offset(line_0based, utf16_col_0based);

            let mut node: Option<Value> = None;
            for scope in overlay.scopes.iter().rev() {
                if byte_pos >= scope.start_byte && byte_pos <= scope.end_byte {
                    let (sl, sc) = overlay.byte_to_char_line_col(scope.start_byte);
                    let (el, ec) = overlay.byte_to_char_line_col(scope.end_byte);
                    let start_char = overlay.char_col_to_utf16_col(sl, sc);
                    let end_char = overlay.char_col_to_utf16_col(el, ec);
                    let range = json!({
                        "start": { "line": sl.saturating_sub(1), "character": start_char },
                        "end": { "line": el.saturating_sub(1), "character": end_char }
                    });
                    node = Some(json!({
                        "range": range,
                        "parent": node
                    }));
                }
            }
            if let Some(n) = node {
                results.push(n);
            }
        }
    }
    json!(results)
}

pub fn handle_inlay_hint(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let Some(ws) = workspace else {
        return json!([]);
    };
    let Some(overlay) = overlay_mgr.get(&file) else {
        return json!([]);
    };
    let mut hints = Vec::new();
    for sym in &overlay.symbols {
        if sym.kind == DefKind::Local {
            if let Some(ty) = ws.type_at(&file, sym.start_line, sym.start_col) {
                let end_char = overlay.char_col_to_utf16_col(sym.end_line, sym.end_col);
                hints.push(json!({
                    "position": {
                        "line": sym.end_line.saturating_sub(1),
                        "character": end_char
                    },
                    "label": format!(": {ty}"),
                    "kind": 1,
                    "paddingLeft": true
                }));
            }
        }
    }
    json!(hints)
}

pub fn handle_semantic_tokens_full(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let Some(overlay) = overlay_mgr.get(&file) else {
        return json!({ "data": [] });
    };
    let mut data = Vec::new();
    let mut prev_line = 0u32;
    let mut prev_char = 0u32;

    let mut sorted_symbols = overlay.symbols.clone();
    sorted_symbols.sort_by_key(|s| (s.start_line, s.start_col));

    for sym in sorted_symbols {
        let line_0based = sym.start_line.saturating_sub(1);
        let char_0based = overlay.char_col_to_utf16_col(sym.start_line, sym.start_col);
        let end_char = overlay.char_col_to_utf16_col(sym.end_line, sym.end_col);
        let length = end_char.saturating_sub(char_0based);

        let delta_line = line_0based.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            char_0based.saturating_sub(prev_char)
        } else {
            char_0based
        };

        let token_type = match sym.kind {
            DefKind::Fn | DefKind::AssocFn => 1,
            DefKind::Struct => 3,
            DefKind::Enum => 4,
            DefKind::Variant => 5,
            DefKind::Trait => 6,
            DefKind::TyAlias => 7,
            _ => 0,
        };

        data.extend_from_slice(&[delta_line, delta_start, length, token_type, 1]);
        prev_line = line_0based;
        prev_char = char_0based;
    }

    json!({ "data": data })
}
