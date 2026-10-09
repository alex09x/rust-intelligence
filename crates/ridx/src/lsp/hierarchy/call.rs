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
use crate::lsp::params::text_document_position;
use crate::lsp::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub(crate) use super::scan::is_call_site;
use super::scan::{find_enclosing_function, find_function_body_end_line};

pub fn handle_prepare_call_hierarchy(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let (uri, line_0based, utf16_col_0based) = match text_document_position(msg) {
        Ok(t) => t,
        Err(_) => return Value::Array(vec![]),
    };
    let file = uri_to_path(uri);
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
            let sel_range = json!({
                "start": { "line": def.start_line.saturating_sub(1), "character": sc },
                "end": { "line": def.end_line.saturating_sub(1), "character": ec }
            });
            items.push(json!({
                "name": def.name,
                "kind": lsp_symbol_kind(def.kind),
                "detail": def.signature,
                "uri": uri,
                "range": sel_range,
                "selectionRange": sel_range
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
                    let sel_range = json!({
                        "start": { "line": def.start_line.saturating_sub(1), "character": sc },
                        "end": { "line": def.end_line.saturating_sub(1), "character": ec }
                    });
                    items.push(json!({
                        "name": def.name,
                        "kind": lsp_symbol_kind(def.kind),
                        "detail": def.signature,
                        "uri": path_to_uri(d_str),
                        "range": sel_range,
                        "selectionRange": sel_range
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

    let symbols = ws.search_symbols(name);
    let mut ref_paths = Vec::new();

    for sym in &symbols {
        if sym.name == name
            && matches!(sym.kind, DefKind::Fn | DefKind::AssocFn)
            && sym.resolved_path() == Path::new(&item_file)
            && sym.span.start_line() == item_start_line
        {
            ref_paths.push(sym.path);
        }
    }

    // Include trait or impl counterpart paths for the method
    for other in &symbols {
        if other.name == name
            && matches!(other.kind, DefKind::Fn | DefKind::AssocFn)
            && !ref_paths.contains(&other.path)
        {
            let is_related = ref_paths.iter().any(|p| {
                p.contains(other.path.split("::").next().unwrap_or(""))
                    || other.path.contains(p.split("::").next().unwrap_or(""))
                    || p.contains(" as ")
                    || other.path.contains(" as ")
            });
            if is_related {
                ref_paths.push(other.path);
            }
        }
    }

    let mut incoming_map: HashMap<String, (Value, Vec<Value>)> = HashMap::new();

    for path in ref_paths {
        let refs = ws.find_references(path);
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

            let content = std::fs::read_to_string(r_str).unwrap_or_default();
            let file_lines: Vec<&str> = content.lines().collect();
            let caller_def =
                find_enclosing_function(ws, r_str, span.start_line(), &file_lines);

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
            let c_body_end =
                find_function_body_end_line(&file_lines, caller.span.start_line());
            let c_range = json!({
                "start": { "line": caller.span.start_line().saturating_sub(1), "character": c_sc },
                "end": { "line": c_body_end.saturating_sub(1), "character": 0 }
            });
            let c_sel_range = json!({
                "start": { "line": caller.span.start_line().saturating_sub(1), "character": c_sc },
                "end": { "line": caller.span.end_line().saturating_sub(1), "character": c_ec }
            });
            let caller_item = json!({
                "name": caller.name,
                "kind": lsp_symbol_kind(caller.kind),
                "detail": caller.signature,
                "uri": c_uri,
                "range": c_range,
                "selectionRange": c_sel_range
            });

            let key = format!("{}:{}:{}", caller.path, c_str, caller.span.start_line());
            incoming_map
                .entry(key)
                .or_insert_with(|| (caller_item, Vec::new()))
                .1
                .push(call_range);
        }
    }

    let incoming: Vec<Value> = incoming_map
        .into_values()
        .map(|(from, from_ranges)| {
            json!({
                "from": from,
                "fromRanges": from_ranges
            })
        })
        .collect();

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
    let mut end_line = item["range"]["end"]["line"]
        .as_u64()
        .unwrap_or(u32::MAX as u64) as u32
        + 1;

    let Some(ws) = workspace else {
        return json!([]);
    };

    let Ok(content) = std::fs::read_to_string(&file) else {
        return json!([]);
    };

    let lines: Vec<&str> = content.lines().collect();
    if end_line <= start_line {
        end_line = find_function_body_end_line(&lines, start_line);
    }

    let mut outgoing = Vec::new();

    for line_idx in start_line.saturating_sub(1)..end_line.min(lines.len() as u32) {
        let line_num = line_idx + 1;
        let line_str = lines[line_idx as usize];

        let char_indices: Vec<(usize, char)> = line_str.char_indices().collect();
        let mut i = 0;
        while i < char_indices.len() {
            let (_, ch) = char_indices[i];
            if ch == '_' || ch.is_alphabetic() {
                let start_c = i;
                let start_byte = char_indices[start_c].0;
                while i < char_indices.len()
                    && (char_indices[i].1 == '_' || char_indices[i].1.is_alphanumeric())
                {
                    i += 1;
                }
                let end_byte = if i < char_indices.len() {
                    char_indices[i].0
                } else {
                    line_str.len()
                };
                let ident = &line_str[start_byte..end_byte];

                let line_prefix = &line_str[..start_byte];
                let is_decl = line_prefix.split_whitespace().last() == Some("fn");

                let rest = &line_str[end_byte..];
                let mut full_rest = rest.to_string();
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

                if is_call && !is_decl && !is_rust_keyword(ident) {
                    let col_1based = (start_c + 1) as u32;
                    let end_col_1based = (i + 1) as u32;

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
                                "detail": def.signature,
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
