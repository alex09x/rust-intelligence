/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP completion intelligence handler.

use super::super::error::{LspError, LspResult};
use super::super::protocol::{lsp_completion_kind, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn handle_completion(
    msg: &Value,
    workspace: Option<&WorkspaceIndex>,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
) -> LspResult<Value> {
    let params = msg
        .get("params")
        .ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be an object"));
    }

    let uri = params["textDocument"]["uri"]
        .as_str()
        .ok_or_else(|| LspError::invalid_params("Missing textDocument.uri"))?;
    if uri.is_empty() {
        return Err(LspError::invalid_params("Empty textDocument.uri"));
    }

    let pos = &params["position"];
    if !pos.is_object() {
        return Err(LspError::invalid_params("Missing position object"));
    }
    let line_0based = pos["line"]
        .as_u64()
        .ok_or_else(|| LspError::invalid_params("Missing position.line"))? as u32;
    let utf16_col_0based = pos["character"]
        .as_u64()
        .ok_or_else(|| LspError::invalid_params("Missing position.character"))? as u32;
    let line_1based = line_0based + 1;

    let file = uri_to_path(uri);
    let overlay_opt = overlay_mgr.get(&file);
    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    let mut items = Vec::new();
    let mut seen_names = HashSet::new();

    // Determine prefix under/before cursor
    let mut prefix = String::new();
    if let Some(overlay) = overlay_opt {
        let line_idx = line_0based as usize;
        if let Some(&line_start) = overlay.line_starts.get(line_idx) {
            let byte_pos = overlay.line_char_col_to_byte(line_1based, col_1based);
            let line_slice = &overlay.text[line_start..byte_pos.min(overlay.text.len())];
            for c in line_slice.chars().rev() {
                if c.is_alphanumeric() || c == '_' {
                    prefix.insert(0, c);
                } else {
                    break;
                }
            }
        }
    }

    // 1. Overlay symbols: ONLY in-scope symbols, excluding nested items from other scopes
    if let Some(overlay) = overlay_opt {
        let cursor_byte = overlay.line_char_col_to_byte(line_1based, col_1based);

        // Find enclosing scopes containing cursor
        let active_scope_ids: HashSet<usize> = {
            let mut set = HashSet::new();
            for s in &overlay.scopes {
                if cursor_byte >= s.start_byte && cursor_byte <= s.end_byte {
                    set.insert(s.id);
                    let mut cur = s.parent;
                    while let Some(pid) = cur {
                        set.insert(pid);
                        cur = overlay.scopes.get(pid).and_then(|p| p.parent);
                    }
                }
            }
            set
        };

        for s in &overlay.symbols {
            if s.name.contains("{closure") || s.name.contains("{block}") {
                continue;
            }
            if !prefix.is_empty() && !s.name.starts_with(&prefix) {
                continue;
            }

            let is_in_scope = if s.scope_id == 0 {
                true
            } else {
                active_scope_ids.contains(&s.scope_id) && s.decl_start_byte <= cursor_byte
            };

            if is_in_scope && seen_names.insert(s.name.clone()) {
                items.push(json!({
                    "label": s.name,
                    "kind": lsp_completion_kind(s.kind),
                    "detail": s.signature
                }));
            }
        }
    }

    // 2. Workspace symbols
    let mut is_capped = false;
    if let Some(ws) = workspace {
        let current_crate = ws.crate_name_for_file(&file);
        let ws_symbols = ws.search_symbols(&prefix);

        for s in ws_symbols {
            if s.kind == ridx_format::DefKind::Local {
                continue;
            }
            let resolved = s.resolved_path();
            let res_str = resolved.to_str().unwrap_or(s.file);
            let is_same_file = res_str == file || s.file == file;
            if overlay_opt.is_some() && is_same_file {
                continue;
            }

            // Filter out synthetic compiler items (closures, blocks)
            if s.path.contains("{closure}") || s.path.contains("{block}") {
                continue;
            }
            // Filter out nested helpers inside functions, methods, or closures using owner kind
            if let Some((parent_path, _)) = s.path.rsplit_once("::") {
                if let Some(parent_def) = ws.find_definition_by_path(parent_path) {
                    if matches!(
                        parent_def.kind,
                        ridx_format::DefKind::Fn
                            | ridx_format::DefKind::AssocFn
                            | ridx_format::DefKind::Closure
                    ) {
                        continue;
                    }
                }
            }
            if seen_names.insert(s.name.to_string()) {
                // Normalize symbol import path for same-crate items (e.g. my_crate::foo -> crate::foo)
                let import_path = if let Some(cname) = current_crate {
                    let prefix = format!("{cname}::");
                    if s.path.starts_with(&prefix) {
                        format!("crate::{}", &s.path[prefix.len()..])
                    } else {
                        s.path.to_string()
                    }
                } else {
                    s.path.to_string()
                };

                let is_same_module = is_same_file || is_symbol_in_current_module(&file, s.file);

                let auto_import_edit = if is_same_module {
                    None
                } else {
                    overlay_opt.and_then(|ov| {
                        super::auto_import::compute_auto_import_edit(&ov.text, &import_path)
                    })
                };

                let detail = if let Some(sig) = s.signature {
                    sig.to_string()
                } else {
                    s.path.to_string()
                };

                let mut item = json!({
                    "label": s.name,
                    "kind": lsp_completion_kind(s.kind),
                    "detail": detail
                });

                if let Some(edit) = auto_import_edit {
                    item["labelDetails"] = json!({
                        "description": format!("{import_path} (auto-import)")
                    });
                    item["additionalTextEdits"] = json!([edit]);
                }

                items.push(item);
                if items.len() >= 100 {
                    is_capped = true;
                    break;
                }
            }
        }
    }

    Ok(json!({
        "isIncomplete": is_capped,
        "items": items
    }))
}

pub fn handle_completion_resolve(msg: &Value) -> LspResult<Value> {
    let params = msg
        .get("params")
        .ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be a CompletionItem object"));
    }

    let label = params
        .get("label")
        .and_then(|l| l.as_str())
        .ok_or_else(|| LspError::invalid_params("CompletionItem must contain a label"))?;
    if label.is_empty() {
        return Err(LspError::invalid_params("CompletionItem label cannot be empty"));
    }

    let mut resolved = params.clone();
    if resolved.get("documentation").is_none() {
        if let Some(detail) = resolved.get("detail").and_then(|d| d.as_str()) {
            resolved["documentation"] = json!({
                "kind": "markdown",
                "value": format!("```rust\n{detail}\n```")
            });
        } else if let Some(data) = resolved.get("data").and_then(|d| d.as_str()) {
            resolved["documentation"] = json!({
                "kind": "markdown",
                "value": data
            });
        }
    }
    Ok(resolved)
}

fn is_symbol_in_current_module(cur_file: &str, sym_file: &str) -> bool {
    if cur_file == sym_file {
        return true;
    }
    let p_cur = std::path::Path::new(cur_file);
    let p_sym = std::path::Path::new(sym_file);
    if let (Some(cur_parent), Some(sym_parent)) = (p_cur.parent(), p_sym.parent()) {
        if cur_parent == sym_parent {
            if let Some(stem) = p_cur.file_stem().and_then(|s| s.to_str()) {
                if stem == "mod" || stem == "lib" {
                    return true;
                }
            }
        }
    }
    false
}
