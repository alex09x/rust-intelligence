/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP symbol handlers: document symbols, multi-root workspace symbols, and symbol resolution.

pub mod disk_cache;
pub mod document_symbol;
pub(crate) mod document_symbol_tree;

pub use document_symbol::handle_document_symbol;

use super::protocol::{lsp_symbol_kind, path_to_uri, PositionConverter};
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub fn handle_workspace_symbol<'a>(msg: &Value, workspaces: impl Iterator<Item = &'a WorkspaceIndex>, conv: &mut PositionConverter) -> Value {
    let query = msg["params"]["query"].as_str().unwrap_or("");
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for ws in workspaces {
        // rust-analyzer's default `workspace.symbol.search.limit`.
        let symbols = ws.search_workspace_symbols(query, WORKSPACE_SYMBOL_LIMIT);
        for s in symbols {
            let resolved_path = s.resolved_path();
            let path_str = resolved_path.to_str().unwrap_or(s.file);
            let dedupe_key = (s.name.to_string(), path_str.to_string(), s.span.start_line());
            if !seen.insert(dedupe_key) {
                continue;
            }

            let uri = path_to_uri(path_str);
            let need_utf16_conv = !s.name.is_ascii() || s.span.start_line() <= 2;
            let start_char = if need_utf16_conv { conv.char_col_to_utf16_col(path_str, s.span.start_line(), s.span.start_col()) } else { s.span.start_col().saturating_sub(1) };
            let end_char = if need_utf16_conv { conv.char_col_to_utf16_col(path_str, s.span.end_line(), s.span.end_col()) } else { s.span.end_col().saturating_sub(1) };
            // A crate root, listed under the crate's name or its Cargo target's dashed name (`search-stdin`).
            let crate_root = s.kind == ridx_format::DefKind::Mod && s.path == s.name && (s.name == s.crate_name || s.name.replace('-', "_") == s.crate_name);
            results.push(json!({
                // rust-analyzer lists a raw identifier by its name (`async` for `r#async`).
                "name": s.name.strip_prefix("r#").unwrap_or(s.name),
                "kind": workspace_symbol_kind(s.kind, crate_root),
                "location": {
                    "uri": uri,
                    "range": {
                        "start": { "line": s.span.start_line().saturating_sub(1), "character": start_char },
                        "end": { "line": s.span.end_line().saturating_sub(1), "character": end_char }
                    }
                },
                "containerName": s.path.rsplit("::").nth(1).unwrap_or("")
            }));

            if results.len() >= WORKSPACE_SYMBOL_LIMIT {
                break;
            }
        }
        if results.len() >= WORKSPACE_SYMBOL_LIMIT {
            break;
        }
    }

    json!(results)
}

const WORKSPACE_SYMBOL_LIMIT: usize = 128;

/// The LSP kind rust-analyzer's symbol index gives an item: associated functions are functions, associated types
/// are type aliases (`TypeParameter`), statics are constants, and a crate's root module is a package.
fn workspace_symbol_kind(kind: ridx_format::DefKind, crate_root: bool) -> u32 {
    use ridx_format::DefKind;
    match kind {
        DefKind::AssocFn => 12,
        DefKind::AssocTy => 26,
        DefKind::Static => 14,
        DefKind::Mod if crate_root => 4,
        k => lsp_symbol_kind(k),
    }
}

pub fn handle_workspace_symbol_resolve<'a>(msg: &Value, workspaces: impl Iterator<Item = &'a WorkspaceIndex>, conv: &mut PositionConverter) -> Value {
    let mut item = msg.get("params").cloned().unwrap_or(Value::Null);
    if item.is_null() {
        return Value::Null;
    }

    // If location is missing or explicitly null, resolve it and populate containerName
    if item.get("location").is_none_or(Value::is_null) {
        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
        for ws in workspaces {
            let symbols = ws.search_symbols(name);
            if let Some(s) = symbols.into_iter().find(|sym| sym.name == name) {
                let resolved_path = s.resolved_path();
                let path_str = resolved_path.to_str().unwrap_or(s.file);
                let uri = path_to_uri(path_str);
                let start_char = conv.char_col_to_utf16_col(path_str, s.span.start_line(), s.span.start_col());
                let end_char = conv.char_col_to_utf16_col(path_str, s.span.end_line(), s.span.end_col());
                item["location"] = json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": s.span.start_line().saturating_sub(1), "character": start_char },
                        "end": { "line": s.span.end_line().saturating_sub(1), "character": end_char }
                    }
                });
                item["containerName"] = json!(s.path.rsplit("::").nth(1).unwrap_or(""));
                break;
            }
        }
    }

    item
}

#[cfg(test)]
pub mod tests;
