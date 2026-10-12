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
use crate::lsp::error::LspResult;
use crate::lsp::params::text_document_opt_position;
use crate::lsp::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

#[allow(unused_imports)]
pub(crate) use super::scan::is_call_site;
use super::scan::{find_enclosing_function, find_function_body_end_line, is_call_site_in_lines};

pub fn handle_prepare_call_hierarchy(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> LspResult<Value> {
    let (uri, pos_opt) = text_document_opt_position(msg)?;
    let (line_0based, utf16_col_0based) = match pos_opt {
        Some(p) => p,
        None => return Ok(Value::Array(vec![])),
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
            return Ok(json!(items));
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

    Ok(json!(items))
}

fn find_definition_path_in_workspace<'a>(
    ws: &'a WorkspaceIndex,
    file_path: &Path,
    file_str: &str,
    start_line: u32,
) -> Option<&'a str> {
    if file_str.is_empty() {
        return None;
    }
    for (idx, reader) in ws.readers().iter().enumerate() {
        let rel_file = ws
            .source_root_at(idx)
            .and_then(|root| file_path.strip_prefix(root).ok())
            .and_then(|rel| rel.to_str())
            .unwrap_or(file_str);

        if let Some(fid) = reader.find_file_id(rel_file) {
            if let Some(def) = reader.definitions().iter().find(|d| {
                d.span.file_id == fid
                    && matches!(DefKind::from(d.kind), DefKind::Fn | DefKind::AssocFn)
                    && d.span.start_line == start_line
            }) {
                return Some(reader.get_str(def.path_offset));
            }
        }
    }
    None
}

pub fn handle_call_hierarchy_incoming_calls(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let item = &msg["params"]["item"];
    let Some(item_uri) = item.get("uri").and_then(|u| u.as_str()).filter(|u| !u.is_empty()) else {
        return json!([]);
    };
    let name = item["name"].as_str().unwrap_or("");
    let item_file = uri_to_path(item_uri);
    let item_start_line = item["range"]["start"]["line"].as_u64().unwrap_or(0) as u32 + 1;

    let Some(ws) = workspace else {
        return json!([]);
    };

    let item_path = Path::new(&item_file);
    let mut ref_paths = Vec::new();

    if let Some(path) = find_definition_path_in_workspace(ws, item_path, &item_file, item_start_line) {
        ref_paths.push(path);
    }

    // Include trait or impl counterpart paths for trait methods
    if ref_paths.iter().any(|p| p.contains(" as ")) {
        for p in ref_paths.clone() {
            if let Some(idx) = p.find(" as ") {
                let trait_part = p[idx + 4..].trim_end_matches('>');
                for sym in ws.search_symbols(name) {
                    if sym.path.ends_with(trait_part) && !ref_paths.contains(&sym.path) {
                        ref_paths.push(sym.path);
                    }
                }
            }
        }
    }

    let mut incoming_map: HashMap<String, (Value, Vec<Value>)> = HashMap::new();
    let mut file_cache: HashMap<String, String> = HashMap::new();

    for path in ref_paths {
        let refs = ws.find_references(path);
        for r in refs {
            let r_path = r.resolved_path();
            let r_str = r_path.to_str().unwrap_or(r.file);
            let span = r.span;

            let content = file_cache.entry(r_str.to_string()).or_insert_with(|| {
                if let Some(overlay) = overlay_mgr.get(r_str) {
                    overlay.text.clone()
                } else {
                    std::fs::read_to_string(r_str).unwrap_or_default()
                }
            });

            let file_lines: Vec<&str> = content.lines().collect();

            if !is_call_site_in_lines(&file_lines, span.end_line(), span.end_col()) {
                continue;
            }

            let sc = conv.char_col_to_utf16_col(r_str, span.start_line(), span.start_col());
            let ec = conv.char_col_to_utf16_col(r_str, span.end_line(), span.end_col());
            let call_range = json!({
                "start": { "line": span.start_line().saturating_sub(1), "character": sc },
                "end": { "line": span.end_line().saturating_sub(1), "character": ec }
            });

            let caller_def =
                find_enclosing_function(ws, r_str, span.start_line(), &file_lines);

            let Some((caller, c_body_end)) = caller_def else {
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
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> Value {
    let item = &msg["params"]["item"];
    let Some(uri) = item.get("uri").and_then(|u| u.as_str()).filter(|u| !u.is_empty()) else {
        return json!([]);
    };
    let file = uri_to_path(uri);
    let start_line = item["range"]["start"]["line"]
        .as_u64()
        .map_or(1, |l| (l as u32).saturating_add(1));
    let mut end_line = item["range"]["end"]["line"]
        .as_u64()
        .map_or(u32::MAX, |l| (l as u32).saturating_add(1));

    let Some(ws) = workspace else {
        return json!([]);
    };

    let doc_text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return json!([]);
    };

    let lines: Vec<&str> = doc_text.lines().collect();
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
