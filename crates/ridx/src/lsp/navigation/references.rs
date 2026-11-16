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

/// Whether the cursor of `file` (its editor buffer when open) sits in a literal on a word other than `name`.
fn literal_word_is_not(overlay: Option<&crate::overlay::DocumentOverlay>, file: &str, line: u32, col: u32, name: &str) -> bool {
    let text = overlay.map(|o| o.text.clone()).or_else(|| std::fs::read_to_string(file).ok()).unwrap_or_default();
    super::cursor_literal::literal_word_is_not(&text, line, col, name)
}

pub fn handle_references(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
    let utf16_col_0based = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
    let line_1based = line_0based + 1;
    let include_decl = msg["params"]["context"]["includeDeclaration"].as_bool().unwrap_or(true);

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
            let decl_pos = overlay.find_definition_at(line_1based, col_1based).map(|d| (d.start_line, d.start_col));
            for (sl, sc, el, ec) in local_refs {
                if !include_decl && decl_pos == Some((sl, sc)) {
                    continue;
                }
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
        let (query_line, query_col) = if let Some(overlay) = overlay_opt.filter(|o| o.is_dirty()) {
            if overlay.imports_changed_at(line_1based, col_1based) {
                (0, 0)
            } else if let Some((cl, cc)) = overlay.map_live_to_compiled(line_1based, col_1based) {
                (cl, cc)
            } else {
                (0, 0)
            }
        } else {
            (line_1based, col_1based)
        };

        if query_line > 0 {
            let found = ws.find_references_at(&file, query_line, query_col);
            if let Some(loc_refs) = found.filter(|r| !literal_word_is_not(overlay_opt, &file, line_1based, col_1based, r.target_symbol)) {
                let mut seen_out = std::collections::HashSet::new();
                if include_decl && !loc_refs.target_file.is_empty() && loc_refs.target_file != "external" {
                    let decl_path = super::type_definition_parse::on_disk(loc_refs.target_resolved_path());
                    let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                    let decl_uri = path_to_uri(decl_str);
                    let (sl, el, sc, ec) = if let Some(target_ov) = overlay_mgr.get(decl_str).filter(|o| o.is_dirty()) {
                        let s = target_ov.map_compiled_to_live(loc_refs.target_line, loc_refs.target_col);
                        let e = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0 {
                            target_ov.map_compiled_to_live(loc_refs.target_end_line, loc_refs.target_end_col)
                        } else {
                            s
                        };
                        match (s, e) {
                            (Some(s), Some(e)) => {
                                let sc = target_ov.char_col_to_utf16_col(s.0, s.1);
                                let ec = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0 {
                                    target_ov.char_col_to_utf16_col(e.0, e.1)
                                } else {
                                    sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                                };
                                (s.0.saturating_sub(1), e.0.saturating_sub(1), sc, ec)
                            }
                            _ => (loc_refs.target_line.saturating_sub(1), loc_refs.target_line.saturating_sub(1), 0, 0),
                        }
                    } else {
                        let sc = conv.char_col_to_utf16_col(decl_str, loc_refs.target_line, loc_refs.target_col);
                        let ec = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0 {
                            conv.char_col_to_utf16_col(decl_str, loc_refs.target_end_line, loc_refs.target_end_col)
                        } else {
                            sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                        };
                        let sl = loc_refs.target_line.saturating_sub(1);
                        let el = if loc_refs.target_end_line > 0 { loc_refs.target_end_line.saturating_sub(1) } else { sl };
                        (sl, el, sc, ec)
                    };
                    if seen_out.insert((decl_uri.clone(), sl, sc, el, ec)) {
                        results.push(json!({
                            "uri": decl_uri,
                            "range": {
                                "start": { "line": sl, "character": sc },
                                "end": { "line": el, "character": ec }
                            }
                        }));
                    }
                }

                let gated = super::references_gated::gated_file_refs(ws, &loc_refs);
                for (path, line, sc, ec) in gated.into_iter().chain(super::references_inactive::inactive_item_refs(ws, &loc_refs)) {
                    let r_uri = path_to_uri(path.to_str().unwrap_or_default());
                    if seen_out.insert((r_uri.clone(), line, sc, line, ec)) {
                        results.push(json!({ "uri": r_uri, "range": { "start": { "line": line, "character": sc }, "end": { "line": line, "character": ec } } }));
                    }
                }
                for r in loc_refs.references {
                    let r_path = super::type_definition_parse::on_disk(r.resolved_path());
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);
                    let span = r.span;
                    let (sl, el, sc, ec) = if let Some(target_ov) = overlay_mgr.get(r_str).filter(|o| o.is_dirty()) {
                        let s = target_ov.map_compiled_to_live(span.start_line(), span.start_col());
                        let e = target_ov.map_compiled_to_live(span.end_line(), span.end_col());
                        match (s, e) {
                            (Some(s), Some(e)) => {
                                let sc = target_ov.char_col_to_utf16_col(s.0, s.1);
                                let ec = target_ov.char_col_to_utf16_col(e.0, e.1);
                                (s.0.saturating_sub(1), e.0.saturating_sub(1), sc, ec)
                            }
                            _ => continue,
                        }
                    } else {
                        let sc = conv.char_col_to_utf16_col(r_str, span.start_line(), span.start_col());
                        let ec = conv.char_col_to_utf16_col(r_str, span.end_line(), span.end_col());
                        (span.start_line().saturating_sub(1), span.end_line().saturating_sub(1), sc, ec)
                    };
                    if seen_out.insert((r_uri.clone(), sl, sc, el, ec)) {
                        results.push(json!({
                            "uri": r_uri,
                            "range": {
                                "start": { "line": sl, "character": sc },
                                "end": { "line": el, "character": ec }
                            }
                        }));
                    }
                }
            }
        }
    }

    json!(results)
}

pub fn handle_document_highlight(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
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
            let found = ws.find_references_at(&file, line_1based, col_1based);
            let in_literal = found.as_ref().is_some_and(|r| literal_word_is_not(overlay_opt, &file, line_1based, col_1based, r.target_symbol));
            if in_literal {
                // The word under the cursor in a literal names nothing the macro resolved.
            } else if let Some(loc_refs) = found {
                let decl_path = loc_refs.target_resolved_path();
                let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                let declared_here = decl_str == file || decl_str.ends_with(&file) || file.ends_with(decl_str);
                // A derive's generated `fn default` is "declared" on `#[derive(Default)]`, which spells no `default`.
                let name = loc_refs.target_symbol.strip_prefix("r#").unwrap_or(loc_refs.target_symbol);
                let spelled = conv.get_line(&file, loc_refs.target_line).is_some_and(|l| {
                    let at: String = l.chars().skip(loc_refs.target_col.saturating_sub(1) as usize).collect();
                    at.strip_prefix("r#").unwrap_or(&at).starts_with(name)
                });
                if declared_here && spelled {
                    let sc = conv.char_col_to_utf16_col(&file, loc_refs.target_line, loc_refs.target_col);
                    let ec = if loc_refs.target_end_line > 0 && loc_refs.target_end_col > 0 {
                        conv.char_col_to_utf16_col(&file, loc_refs.target_end_line, loc_refs.target_end_col)
                    } else {
                        sc + (loc_refs.target_symbol.encode_utf16().count() as u32)
                    };
                    let sl = loc_refs.target_line.saturating_sub(1);
                    let el = if loc_refs.target_end_line > 0 { loc_refs.target_end_line.saturating_sub(1) } else { sl };
                    highlights.push(json!({
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end": { "line": el, "character": ec }
                        },
                        "kind": 3
                    }));
                }

                for r in loc_refs.references {
                    let r_path = super::type_definition_parse::on_disk(r.resolved_path());
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    if r_str == file || r_str.ends_with(&file) || file.ends_with(r_str) {
                        let span = r.span;
                        let sc = conv.char_col_to_utf16_col(&file, span.start_line(), span.start_col());
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
                super::highlight_ext::dedup_highlights(&mut highlights);
                let doc_text = overlay_opt.map(|o| o.text.clone()).or_else(|| std::fs::read_to_string(&file).ok());
                if let Some(ref text) = doc_text {
                    let target_crate = loc_refs.target_reader_idx.and_then(|i| ws.readers().get(i)).map_or("", |r| r.crate_name());
                    super::highlight_ext::augment_resolved_highlights(
                        &file,
                        text,
                        (loc_refs.target_symbol, loc_refs.target_path, target_crate),
                        loc_refs.target_kind,
                        conv,
                        &mut highlights,
                    );
                }
            } else {
                let doc_text = overlay_opt.map(|o| o.text.clone()).or_else(|| std::fs::read_to_string(&file).ok());
                if let Some(ref text) = doc_text {
                    super::highlight_ext::fallback_cursor_highlights(&file, text, line_1based, col_1based, conv, &mut highlights);
                }
            }
        }
    }

    json!(highlights)
}
