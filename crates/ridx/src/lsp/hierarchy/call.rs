/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP call hierarchy navigation handlers.

use crate::lsp::editing::is_rust_keyword;
use crate::lsp::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn is_call_site(file: &str, span_end_line: u32, span_end_col: u32) -> bool {
    let content = std::fs::read_to_string(file).ok();
    let Some(ref text) = content else {
        return false;
    };
    let lines: Vec<&str> = text.lines().collect();
    let l_idx = span_end_line.saturating_sub(1) as usize;
    if l_idx >= lines.len() {
        return false;
    }
    let char_offset = span_end_col.saturating_sub(1) as usize;
    let first_line: String = lines[l_idx].chars().skip(char_offset).collect();
    let mut rest = first_line.clone();
    if rest.trim().is_empty() {
        for next_l in lines.iter().skip(l_idx + 1).take(3) {
            rest.push('\n');
            rest.push_str(next_l);
            if !next_l.trim().is_empty() {
                break;
            }
        }
    }
    let trimmed = rest.trim_start();
    if trimmed.starts_with('(') {
        return true;
    }
    if trimmed.starts_with("::<") {
        if let Some(pos) = trimmed.find('(') {
            let between = &trimmed[3..pos];
            if between.contains('>') {
                return true;
            }
        }
    }
    false
}

pub fn handle_prepare_call_hierarchy(
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
        if matches!(def.kind, DefKind::Fn | DefKind::AssocFn) {
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
                if matches!(def.kind, DefKind::Fn | DefKind::AssocFn) {
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

pub fn handle_call_hierarchy_incoming_calls(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    conv: &mut PositionConverter,
) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");
    let item_uri = item["uri"].as_str().unwrap_or("");
    let item_file = uri_to_path(item_uri);
    let item_start_line = item["range"]["start"]["line"].as_u64().unwrap_or(0) as u32 + 1;

    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut incoming = Vec::new();
    let symbols = ws.search_symbols(name);

    for sym in symbols {
        if sym.name == name
            && matches!(sym.kind, DefKind::Fn | DefKind::AssocFn)
            && sym.resolved_path() == Path::new(&item_file)
            && sym.span.start_line() == item_start_line
        {
            let refs = ws.find_references(sym.path);
            for r in refs {
                let r_path = r.resolved_path();
                let r_str = r_path.to_str().unwrap_or(r.file);
                let span = r.span;

                if !is_call_site(r_str, span.end_line(), span.end_col()) {
                    continue;
                }

                let sc = conv.char_col_to_utf16_col(r_str, span.start_line(), span.start_col());
                let ec = conv.char_col_to_utf16_col(r_str, span.end_line(), span.end_col());
                let call_range = json!({
                    "start": { "line": span.start_line().saturating_sub(1), "character": sc },
                    "end": { "line": span.end_line().saturating_sub(1), "character": ec }
                });

                let caller_def = ws.search_symbols("").into_iter().find(|d| {
                    d.file == r.file
                        && matches!(d.kind, DefKind::Fn | DefKind::AssocFn)
                        && d.span.start_line() <= span.start_line()
                        && d.span.end_line() >= span.end_line()
                });

                let Some(caller) = caller_def else {
                    continue;
                };

                let c_path = caller.resolved_path();
                let c_str = c_path.to_str().unwrap_or(caller.file);
                let c_uri = path_to_uri(c_str);
                let c_sc = conv.char_col_to_utf16_col(
                    c_str,
                    caller.span.start_line(),
                    caller.span.start_col(),
                );
                let c_ec = conv.char_col_to_utf16_col(
                    c_str,
                    caller.span.end_line(),
                    caller.span.end_col(),
                );
                let c_range = json!({
                    "start": { "line": caller.span.start_line().saturating_sub(1), "character": c_sc },
                    "end": { "line": caller.span.end_line().saturating_sub(1), "character": c_ec }
                });
                let caller_item = json!({
                    "name": caller.name,
                    "kind": lsp_symbol_kind(caller.kind),
                    "uri": c_uri,
                    "range": c_range,
                    "selectionRange": c_range
                });

                incoming.push(json!({
                    "from": caller_item,
                    "fromRanges": [call_range]
                }));
            }
        }
    }

    json!(incoming)
}

pub fn handle_call_hierarchy_outgoing_calls(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    conv: &mut PositionConverter,
) -> Value {
    let item = &msg["params"]["item"];
    let uri = item["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let start_line = item["range"]["start"]["line"].as_u64().unwrap_or(0) as u32 + 1;
    let end_line = item["range"]["end"]["line"]
        .as_u64()
        .unwrap_or(u32::MAX as u64) as u32
        + 1;

    let Some(ws) = workspace else {
        return json!([]);
    };

    let Ok(content) = std::fs::read_to_string(&file) else {
        return json!([]);
    };

    let mut outgoing = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    for line_idx in start_line.saturating_sub(1)..end_line.min(lines.len() as u32) {
        let line_num = line_idx + 1;
        let line_str = lines[line_idx as usize];

        let chars: Vec<char> = line_str.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '_' || chars[i].is_alphabetic() {
                let start_c = i;
                while i < chars.len() && (chars[i] == '_' || chars[i].is_alphanumeric()) {
                    i += 1;
                }
                let ident: String = chars[start_c..i].iter().collect();

                let line_prefix = &line_str[..start_c];
                let is_decl = line_prefix.split_whitespace().last() == Some("fn");

                let rest: String = chars[i..].iter().collect();
                let mut full_rest = rest.clone();
                if full_rest.trim().is_empty() {
                    for next_l in lines.iter().skip(line_idx as usize + 1).take(3) {
                        full_rest.push('\n');
                        full_rest.push_str(next_l);
                        if !next_l.trim().is_empty() {
                            break;
                        }
                    }
                }
                let trimmed = full_rest.trim_start();
                let is_call = trimmed.starts_with('(')
                    || (trimmed.starts_with("::<") && trimmed.contains('('));

                if is_call && !is_decl && !is_rust_keyword(&ident) {
                    let col_1based = (start_c + 1) as u32;
                    let end_col_1based = (start_c + ident.len() + 1) as u32;

                    if let Some(def) = ws.find_definition_at(&file, line_num, col_1based) {
                        if matches!(def.kind, DefKind::Fn | DefKind::AssocFn) {
                            if def.file == file
                                && def.start_line == start_line
                                && line_num == start_line
                            {
                                continue;
                            }
                            let target_path = def.resolved_path();
                            let target_str = target_path.to_str().unwrap_or(def.file);
                            let target_uri = path_to_uri(target_str);

                            let t_sc = conv.char_col_to_utf16_col(
                                target_str,
                                def.start_line,
                                def.start_col,
                            );
                            let t_ec =
                                conv.char_col_to_utf16_col(target_str, def.end_line, def.end_col);
                            let target_range = json!({
                                "start": { "line": def.start_line.saturating_sub(1), "character": t_sc },
                                "end": { "line": def.end_line.saturating_sub(1), "character": t_ec }
                            });

                            let c_sc = conv.char_col_to_utf16_col(&file, line_num, col_1based);
                            let c_ec = conv.char_col_to_utf16_col(&file, line_num, end_col_1based);
                            let call_range = json!({
                                "start": { "line": line_idx, "character": c_sc },
                                "end": { "line": line_idx, "character": c_ec }
                            });

                            let target_item = json!({
                                "name": def.name,
                                "kind": lsp_symbol_kind(def.kind),
                                "uri": target_uri,
                                "range": target_range,
                                "selectionRange": target_range
                            });

                            outgoing.push(json!({
                                "to": target_item,
                                "fromRanges": [call_range]
                            }));
                        }
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    json!(outgoing)
}
