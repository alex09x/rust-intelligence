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
use std::collections::{BTreeMap, HashMap};

#[cfg(test)]
pub(crate) use super::scan::is_call_site;
#[allow(unused_imports)]
use super::scan::{find_definition_path_in_workspace, find_enclosing_function, find_function_body_end_line, is_call_site_in_lines};
use super::value_use::is_function_value_use_in_lines;

fn make_call_item(name: &str, kind: DefKind, sig: Option<&str>, uri: &str, coords: (u32, u32, u32, u32)) -> Value {
    let r = json!({
        "start": { "line": coords.0.saturating_sub(1), "character": coords.1 },
        "end": { "line": coords.2.saturating_sub(1), "character": coords.3 }
    });
    json!({ "name": name, "kind": lsp_symbol_kind(kind), "detail": sig, "uri": uri, "range": r, "selectionRange": r })
}

pub fn handle_prepare_call_hierarchy(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
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
            let (sc, ec) = (overlay.char_col_to_utf16_col(def.start_line, def.start_col), overlay.char_col_to_utf16_col(def.end_line, def.end_col));
            items.push(make_call_item(&def.name, def.kind, def.signature.as_deref(), uri, (def.start_line, sc, def.end_line, ec)));
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
                    items.push(make_call_item(def.name, def.kind, def.signature, &path_to_uri(d_str), (def.start_line, sc, def.end_line, ec)));
                }
            }
        }
    }

    Ok(json!(items))
}

pub fn handle_call_hierarchy_incoming_calls(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let item = &msg["params"]["item"];
    let Some(item_uri) = item.get("uri").and_then(|u| u.as_str()).filter(|u| !u.is_empty()) else {
        return json!([]);
    };
    let item_file = uri_to_path(item_uri);
    let item_start_line = item["range"]["start"]["line"].as_u64().unwrap_or(0) as u32 + 1;

    let Some(ws) = workspace else {
        return json!([]);
    };

    // The item is resolved by position: the definition at the start of its selection range (its
    // name, as prepareCallHierarchy answers it) and the references of that definition within its
    // compilation unit. The item's path alone is ambiguous: a crate-root item of a test target
    // (`test`, `main`) shares its path with same-named items of other shards, and references
    // recorded by sibling units spell the target path differently. A position that resolves to
    // no function falls back to the path of the function starting on the item's line.
    let name_pos = if item["selectionRange"]["start"]["line"].is_u64() { &item["selectionRange"]["start"] } else { &item["range"]["start"] };
    let name_line = name_pos["line"].as_u64().unwrap_or(0) as u32 + 1;
    let name_col = conv.utf16_col_to_char_col(&item_file, name_line, name_pos["character"].as_u64().unwrap_or(0) as u32);
    let refs = match ws.find_references_at(&item_file, name_line, name_col) {
        Some(loc) if matches!(loc.target_kind, DefKind::Fn | DefKind::AssocFn) => loc.references,
        _ => find_definition_path_in_workspace(ws, &item_file, item_start_line).map(|path| ws.find_references(path)).unwrap_or_default(),
    };

    let item_name = item["name"].as_str().unwrap_or("");
    let mut incoming_map: BTreeMap<String, (Value, Vec<Value>)> = BTreeMap::new();
    let mut file_cache: HashMap<String, String> = HashMap::new();

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

        let is_call = is_call_site_in_lines(&file_lines, span.end_line(), span.end_col());
        if !is_call
            && !(matches!(r.kind, DefKind::Fn | DefKind::AssocFn)
                && !r.is_macro_expansion
                && is_function_value_use_in_lines(&file_lines, item_name, span.start_line(), span.start_col(), span.end_line(), span.end_col()))
        {
            continue;
        }

        let sc = conv.char_col_to_utf16_col(r_str, span.start_line(), span.start_col());
        let ec = conv.char_col_to_utf16_col(r_str, span.end_line(), span.end_col());
        let call_range = json!({
            "start": { "line": span.start_line().saturating_sub(1), "character": sc },
            "end": { "line": span.end_line().saturating_sub(1), "character": ec }
        });

        let caller_def = find_enclosing_function(ws, r_str, span.start_line(), &file_lines);

        let Some((caller, c_body_end)) = caller_def else {
            continue;
        };

        let c_path = caller.resolved_path();
        let c_str = c_path.to_str().unwrap_or(caller.file);
        let c_uri = path_to_uri(c_str);
        let c_sc = conv.char_col_to_utf16_col(c_str, caller.span.start_line(), caller.span.start_col());
        let c_ec = conv.char_col_to_utf16_col(c_str, caller.span.end_line(), caller.span.end_col());
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
        incoming_map.entry(key).or_insert_with(|| (caller_item, Vec::new())).1.push(call_range);
    }

    let incoming: Vec<Value> = incoming_map
        .into_values()
        .map(|(from, mut from_ranges)| {
            from_ranges.sort_by(|a, b| {
                let a_key = (
                    a["start"]["line"].as_u64().unwrap_or(0),
                    a["start"]["character"].as_u64().unwrap_or(0),
                    a["end"]["line"].as_u64().unwrap_or(0),
                    a["end"]["character"].as_u64().unwrap_or(0),
                );
                let b_key = (
                    b["start"]["line"].as_u64().unwrap_or(0),
                    b["start"]["character"].as_u64().unwrap_or(0),
                    b["end"]["line"].as_u64().unwrap_or(0),
                    b["end"]["character"].as_u64().unwrap_or(0),
                );
                a_key.cmp(&b_key)
            });
            from_ranges.dedup();
            json!({
                "from": from,
                "fromRanges": from_ranges
            })
        })
        .collect();

    json!(incoming)
}

pub fn handle_call_hierarchy_outgoing_calls(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> Value {
    let item = &msg["params"]["item"];
    let Some(uri) = item.get("uri").and_then(|u| u.as_str()).filter(|u| !u.is_empty()) else {
        return json!([]);
    };
    let file = uri_to_path(uri);
    let start_line = item["range"]["start"]["line"].as_u64().map_or(1, |l| (l as u32).saturating_add(1));
    let mut end_line = item["range"]["end"]["line"].as_u64().map_or(u32::MAX, |l| (l as u32).saturating_add(1));

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
                while i < char_indices.len() && (char_indices[i].1 == '_' || char_indices[i].1.is_alphanumeric()) {
                    i += 1;
                }
                let end_byte = if i < char_indices.len() { char_indices[i].0 } else { line_str.len() };
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
                let is_call = trimmed.starts_with('(') || (trimmed.starts_with("::<") && trimmed.contains('('));

                if is_call && !is_decl && !is_rust_keyword(ident) {
                    let col_1based = (start_c + 1) as u32;
                    let end_col_1based = (i + 1) as u32;

                    if let Some(def) = ws.find_definition_at(&file, line_num, col_1based) {
                        if matches!(def.kind, DefKind::Fn | DefKind::AssocFn | DefKind::Variant | DefKind::VariantCtor) {
                            if def.file == file && def.start_line == start_line && line_num == start_line {
                                continue;
                            }
                            let target_path = def.resolved_path();
                            let target_str = target_path.to_str().unwrap_or(def.file);
                            let target_uri = path_to_uri(target_str);

                            let t_sc = conv.char_col_to_utf16_col(target_str, def.start_line, def.start_col);
                            let t_ec = conv.char_col_to_utf16_col(target_str, def.end_line, def.end_col);
                            let c_sc = conv.char_col_to_utf16_col(&file, line_num, col_1based);
                            let c_ec = conv.char_col_to_utf16_col(&file, line_num, end_col_1based);
                            let call_range = json!({
                                "start": { "line": line_idx, "character": c_sc },
                                "end": { "line": line_idx, "character": c_ec }
                            });

                            let target_item = make_call_item(def.name, def.kind, def.signature, &target_uri, (def.start_line, t_sc, def.end_line, t_ec));

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
