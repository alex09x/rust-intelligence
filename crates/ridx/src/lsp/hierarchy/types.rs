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

use super::def_cache::{fallback_json, find_first_target_def, find_first_trait_def};
use super::impl_table::{get_or_build_impl_table, ImplTable};
use super::scan::{get_impl_line_from_file, parse_impl_line};
use crate::lsp::error::LspResult;
use crate::lsp::params::text_document_opt_position;
use crate::lsp::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub fn handle_prepare_type_hierarchy(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
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
        if matches!(def.kind, DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::TyAlias) {
            let overlay = overlay_opt.unwrap();
            let (sc, ec) = (overlay.char_col_to_utf16_col(def.start_line, def.start_col), overlay.char_col_to_utf16_col(def.end_line, def.end_col));
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
            return Ok(json!(items));
        }
    }

    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            if let Some(def) = ws.find_definition_at(&file, line_1based, col_1based) {
                if matches!(def.kind, DefKind::Struct | DefKind::Enum | DefKind::Trait | DefKind::TyAlias) {
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

    Ok(json!(items))
}

pub(crate) fn parse_fallback_impl_path(path: &str) -> (Option<String>, Option<String>) {
    if let Some(inner) = path.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        if let Some((target, tr)) = inner.split_once(" as ") {
            let target_clean = target.rsplit("::").next().unwrap_or(target);
            let tr_clean = tr.rsplit("::").next().unwrap_or(tr);
            return (Some(tr_clean.to_string()), Some(target_clean.to_string()));
        }
    }
    (None, None)
}

struct ImplCandidateQuery<'a> {
    ws: &'a WorkspaceIndex,
    table: Option<&'a ImplTable>,
    query_name: &'a str,
    is_supertype: bool,
    seen: &'a mut HashSet<String>,
    results: &'a mut Vec<Value>,
}

fn process_impl_candidate(q: &mut ImplCandidateQuery<'_>, file_path: &str, start_line_1based: u32, fallback: (Option<String>, Option<String>)) {
    let (trait_opt, target_opt) = if let Some(table) = q.table {
        table.get_or_parse_fallback_line(file_path, start_line_1based, fallback)
    } else {
        get_impl_line_from_file(Path::new(file_path), start_line_1based).map(|l| parse_impl_line(&l)).unwrap_or(fallback)
    };

    let (Some(trait_name), Some(target_name)) = (trait_opt, target_opt) else {
        return;
    };

    let trait_base = trait_name.split('<').next().unwrap_or(&trait_name).trim();
    let trait_short = trait_base.rsplit("::").next().unwrap_or(trait_base);
    if matches!(trait_short, "StructuralPartialEq" | "TrivialClone" | "StructuralEq") {
        return;
    }
    let target_base = target_name.split('<').next().unwrap_or(&target_name).trim();
    let name_base = q.query_name.split('<').next().unwrap_or(q.query_name).trim();

    if q.is_supertype {
        if target_base == name_base || target_base.ends_with(&format!("::{name_base}")) {
            if !q.seen.insert(trait_base.to_string()) {
                return;
            }
            let def_val = if let Some(table) = q.table {
                table.resolve_trait_def(q.ws, trait_base, Path::new(file_path), start_line_1based)
            } else {
                find_first_trait_def(q.ws, trait_base).map(|cd| cd.to_json(trait_base)).unwrap_or_else(|| fallback_json(trait_base, 11, Path::new(file_path), start_line_1based))
            };
            q.results.push(def_val);
        }
    } else if trait_base == name_base || trait_base.ends_with(&format!("::{name_base}")) {
        if !q.seen.insert(target_base.to_string()) {
            return;
        }
        let def_val = if let Some(table) = q.table {
            table.resolve_target_def(q.ws, target_base, Path::new(file_path), start_line_1based)
        } else {
            find_first_target_def(q.ws, target_base).map(|cd| cd.to_json(target_base)).unwrap_or_else(|| fallback_json(target_base, 23, Path::new(file_path), start_line_1based))
        };
        q.results.push(def_val);
    }
}

fn run_impl_reference_fallback(query: &mut ImplCandidateQuery<'_>, match_kind: fn(DefKind) -> bool) {
    if let Some(table) = query.table {
        if let Some(cached_results) = table.get_fallback_results(query.query_name, query.is_supertype) {
            for val in cached_results {
                let name = val["name"].as_str().unwrap_or("");
                if query.seen.insert(name.to_string()) {
                    query.results.push(val);
                }
            }
            return;
        }
    }

    let start_idx = query.results.len();
    let expected_mask = if query.is_supertype { super::type_index::KIND_STRUCT | super::type_index::KIND_ENUM } else { super::type_index::KIND_TRAIT };
    let sym_opt = if let Some(table) = query.table {
        table.find_type_def(query.ws, query.query_name, expected_mask)
    } else {
        query.ws.find_first_symbol(query.query_name, |s| s.name == query.query_name && match_kind(s.kind))
    };

    if let Some(sym) = sym_opt {
        for r in query.ws.find_references(sym.path) {
            let r_path = r.resolved_path();
            let r_file = r_path.to_str().unwrap_or(r.file);
            process_impl_candidate(query, r_file, r.span.start_line(), (None, None));
        }
    }
    if let Some(table) = query.table {
        let added = query.results[start_idx..].to_vec();
        table.insert_fallback_results(query.query_name, query.is_supertype, added);
    }
}

pub fn handle_type_hierarchy_supertypes(msg: &Value, workspace: Option<&WorkspaceIndex>) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");
    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut supertypes = Vec::new();
    let mut seen = HashSet::new();

    // 1. Fast indexed lookup from ImplTable
    let table = get_or_build_impl_table(ws);
    table.find_supertypes(ws, name, &mut seen, &mut supertypes);

    // 2. References to matching struct/enum (for macro expansions and mock indices)
    let mut query = ImplCandidateQuery { ws, table: Some(&table), query_name: name, is_supertype: true, seen: &mut seen, results: &mut supertypes };
    run_impl_reference_fallback(&mut query, |k| matches!(k, DefKind::Struct | DefKind::Enum));

    supertypes.into()
}

pub fn handle_type_hierarchy_subtypes(msg: &Value, workspace: Option<&WorkspaceIndex>) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");
    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut subtypes = Vec::new();
    let mut seen = HashSet::new();

    // 1. Fast indexed lookup from ImplTable
    let table = get_or_build_impl_table(ws);
    table.find_subtypes(ws, name, &mut seen, &mut subtypes);

    // 2. References to matching trait (for macro expansions and mock indices)
    let mut query = ImplCandidateQuery { ws, table: Some(&table), query_name: name, is_supertype: false, seen: &mut seen, results: &mut subtypes };
    run_impl_reference_fallback(&mut query, |k| k == DefKind::Trait);

    subtypes.into()
}

#[cfg(test)]
pub fn baseline_handle_type_hierarchy_supertypes(msg: &Value, workspace: Option<&WorkspaceIndex>) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");
    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut supertypes = Vec::new();
    let mut seen = HashSet::new();
    let mut query = ImplCandidateQuery { ws, table: None, query_name: name, is_supertype: true, seen: &mut seen, results: &mut supertypes };

    for sym in ws.search_symbols("").into_iter().filter(|s| s.kind == DefKind::Impl) {
        let sym_path = sym.resolved_path();
        let sym_file = sym_path.to_str().unwrap_or(sym.file);
        let fallback = parse_fallback_impl_path(sym.path);
        process_impl_candidate(&mut query, sym_file, sym.span.start_line(), fallback);
    }

    run_impl_reference_fallback(&mut query, |k| matches!(k, DefKind::Struct | DefKind::Enum));
    supertypes.into()
}

#[cfg(test)]
pub fn baseline_handle_type_hierarchy_subtypes(msg: &Value, workspace: Option<&WorkspaceIndex>) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");
    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut subtypes = Vec::new();
    let mut seen = HashSet::new();
    let mut query = ImplCandidateQuery { ws, table: None, query_name: name, is_supertype: false, seen: &mut seen, results: &mut subtypes };

    for sym in ws.search_symbols("").into_iter().filter(|s| s.kind == DefKind::Impl) {
        let sym_path = sym.resolved_path();
        let sym_file = sym_path.to_str().unwrap_or(sym.file);
        let fallback = parse_fallback_impl_path(sym.path);
        process_impl_candidate(&mut query, sym_file, sym.span.start_line(), fallback);
    }

    run_impl_reference_fallback(&mut query, |k| k == DefKind::Trait);
    subtypes.into()
}
