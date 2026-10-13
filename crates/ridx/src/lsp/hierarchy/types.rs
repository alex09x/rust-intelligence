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

use super::scan::{get_impl_line_from_file, parse_impl_line};
use crate::lsp::error::LspResult;
use crate::lsp::params::text_document_opt_position;
use crate::lsp::protocol::{lsp_symbol_kind, path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub fn handle_prepare_type_hierarchy(
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
            return Ok(json!(items));
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
    query_name: &'a str,
    is_supertype: bool,
    seen: &'a mut HashSet<String>,
    results: &'a mut Vec<Value>,
}

fn process_impl_candidate(
    q: &mut ImplCandidateQuery<'_>,
    file_path: &str,
    start_line_1based: u32,
    fallback: (Option<String>, Option<String>),
) {
    let (trait_opt, target_opt) = get_impl_line_from_file(Path::new(file_path), start_line_1based)
        .map(|l| parse_impl_line(&l))
        .unwrap_or(fallback);

    let (Some(trait_name), Some(target_name)) = (trait_opt, target_opt) else {
        return;
    };

    let trait_base = trait_name.split('<').next().unwrap_or(&trait_name).trim();
    let target_base = target_name.split('<').next().unwrap_or(&target_name).trim();
    let name_base = q.query_name.split('<').next().unwrap_or(q.query_name).trim();

    if q.is_supertype {
        if target_base == name_base || target_base.ends_with(&format!("::{name_base}")) {
            if !q.seen.insert(trait_base.to_string()) {
                return;
            }
            let (item_uri, item_range) = if let Some(t_def) = q
                .ws
                .find_first_symbol(trait_base, |d| d.name == trait_base && d.kind == DefKind::Trait)
            {
                let p = t_def.resolved_path();
                let s = p.to_str().unwrap_or(t_def.file);
                let r = json!({
                    "start": { "line": t_def.span.start_line().saturating_sub(1), "character": t_def.span.start_col().saturating_sub(1) },
                    "end": { "line": t_def.span.end_line().saturating_sub(1), "character": t_def.span.end_col().saturating_sub(1) }
                });
                (path_to_uri(s), r)
            } else {
                let uri = path_to_uri(file_path);
                let r = json!({
                    "start": { "line": start_line_1based.saturating_sub(1), "character": 0 },
                    "end": { "line": start_line_1based.saturating_sub(1), "character": 0 }
                });
                (uri, r)
            };
            q.results.push(json!({
                "name": trait_base,
                "kind": 11,
                "uri": item_uri,
                "range": item_range,
                "selectionRange": item_range
            }));
        }
    } else if trait_base == name_base || trait_base.ends_with(&format!("::{name_base}")) {
        if !q.seen.insert(target_base.to_string()) {
            return;
        }
        let (item_uri, item_range, kind) = if let Some(t_def) = q
            .ws
            .find_first_symbol(target_base, |d| d.name == target_base && matches!(d.kind, DefKind::Struct | DefKind::Enum))
        {
            let p = t_def.resolved_path();
            let s = p.to_str().unwrap_or(t_def.file);
            let r = json!({
                "start": { "line": t_def.span.start_line().saturating_sub(1), "character": t_def.span.start_col().saturating_sub(1) },
                "end": { "line": t_def.span.end_line().saturating_sub(1), "character": t_def.span.end_col().saturating_sub(1) }
            });
            (path_to_uri(s), r, lsp_symbol_kind(t_def.kind))
        } else {
            let uri = path_to_uri(file_path);
            let r = json!({
                "start": { "line": start_line_1based.saturating_sub(1), "character": 0 },
                "end": { "line": start_line_1based.saturating_sub(1), "character": 0 }
            });
            (uri, r, 23)
        };
        q.results.push(json!({
            "name": target_base,
            "kind": kind,
            "uri": item_uri,
            "range": item_range,
            "selectionRange": item_range
        }));
    }
}

fn run_impl_reference_fallback(
    query: &mut ImplCandidateQuery<'_>,
    match_kind: fn(DefKind) -> bool,
) {
    if let Some(sym) = query.ws.find_first_symbol(query.query_name, |s| {
        s.name == query.query_name && match_kind(s.kind)
    }) {
        for r in query.ws.find_references(sym.path) {
            let r_path = r.resolved_path();
            let r_file = r_path.to_str().unwrap_or(r.file);
            process_impl_candidate(query, r_file, r.span.start_line(), (None, None));
        }
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
    let table = super::impl_table::get_or_build_impl_table(ws);
    table.find_supertypes(ws, name, &mut seen, &mut supertypes);

    // 2. References to matching struct/enum (for macro expansions and mock indices)
    let mut query = ImplCandidateQuery {
        ws,
        query_name: name,
        is_supertype: true,
        seen: &mut seen,
        results: &mut supertypes,
    };
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
    let table = super::impl_table::get_or_build_impl_table(ws);
    table.find_subtypes(ws, name, &mut seen, &mut subtypes);

    // 2. References to matching trait (for macro expansions and mock indices)
    let mut query = ImplCandidateQuery {
        ws,
        query_name: name,
        is_supertype: false,
        seen: &mut seen,
        results: &mut subtypes,
    };
    run_impl_reference_fallback(&mut query, |k| k == DefKind::Trait);

    subtypes.into()
}

#[cfg(test)]
pub fn baseline_handle_type_hierarchy_supertypes(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");
    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut supertypes = Vec::new();
    let mut seen = HashSet::new();
    let mut query = ImplCandidateQuery {
        ws,
        query_name: name,
        is_supertype: true,
        seen: &mut seen,
        results: &mut supertypes,
    };

    for sym in ws.search_symbols("").into_iter().filter(|s| s.kind == DefKind::Impl) {
        let sym_path = sym.resolved_path();
        let sym_file = sym_path.to_str().unwrap_or(sym.file);
        let fallback = parse_fallback_impl_path(sym.path);
        process_impl_candidate(
            &mut query,
            sym_file,
            sym.span.start_line(),
            fallback,
        );
    }

    run_impl_reference_fallback(&mut query, |k| matches!(k, DefKind::Struct | DefKind::Enum));
    supertypes.into()
}

#[cfg(test)]
pub fn baseline_handle_type_hierarchy_subtypes(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
) -> Value {
    let item = &msg["params"]["item"];
    let name = item["name"].as_str().unwrap_or("");
    let Some(ws) = workspace else {
        return json!([]);
    };

    let mut subtypes = Vec::new();
    let mut seen = HashSet::new();
    let mut query = ImplCandidateQuery {
        ws,
        query_name: name,
        is_supertype: false,
        seen: &mut seen,
        results: &mut subtypes,
    };

    for sym in ws.search_symbols("").into_iter().filter(|s| s.kind == DefKind::Impl) {
        let sym_path = sym.resolved_path();
        let sym_file = sym_path.to_str().unwrap_or(sym.file);
        let fallback = parse_fallback_impl_path(sym.path);
        process_impl_candidate(
            &mut query,
            sym_file,
            sym.span.start_line(),
            fallback,
        );
    }

    run_impl_reference_fallback(&mut query, |k| k == DefKind::Trait);
    subtypes.into()
}
