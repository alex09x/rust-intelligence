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
    let (uri, line_0based, utf16_col_0based) = crate::lsp::params::text_document_position(msg)?;
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

    // Extract line slice up to cursor
    let line_slice = if let Some(overlay) = overlay_opt {
        let idx = line_0based as usize;
        overlay.line_starts.get(idx).map_or_else(String::new, |&s| {
            let bp = overlay.line_char_col_to_byte(line_1based, col_1based);
            overlay.text[s..bp.min(overlay.text.len())].to_string()
        })
    } else if let Ok(c) = std::fs::read_to_string(&file) {
        c.lines().nth(line_0based as usize).map_or_else(String::new, |l| {
            l.chars().take(col_1based.saturating_sub(1) as usize).collect()
        })
    } else {
        String::new()
    };

    // Determine prefix under/before cursor
    let mut prefix = String::new();
    for c in line_slice.chars().rev() {
        if c.is_alphanumeric() || c == '_' {
            prefix.insert(0, c);
        } else {
            break;
        }
    }

    let trimmed_before = line_slice[..line_slice.len() - prefix.len()].trim_end();
    let is_dot = trimmed_before.ends_with('.') && !trimmed_before.ends_with("..");
    let is_colon = trimmed_before.ends_with("::");

    // Unified scoped completion after '.' (receiver methods) or '::' (associated items)
    if is_dot || is_colon {
        if let Some(ws) = workspace {
            // Fast path: try O(1) owner-index and autoderef lookup (<100 µs)
            let owner_ctx = super::owner_completion::OwnerCompletionContext {
                file: &file,
                line_1based,
                line_slice: &line_slice,
                prefix: &prefix,
                is_dot,
                is_colon,
                trimmed_before,
            };
            if let Some(owner_items) = super::owner_completion::try_owner_completion(ws, &owner_ctx) {
                return Ok(json!({
                    "isIncomplete": false,
                    "items": owner_items
                }));
            }

            // Fallback path: streaming symbol traversal with early-stopping and early-exit
            let (fallback_items, is_capped) =
                super::fallback_completion::try_fallback_completion(ws, &owner_ctx, overlay_opt);
            return Ok(json!({
                "isIncomplete": is_capped,
                "items": fallback_items
            }));
        }
        return Ok(json!({
            "isIncomplete": false,
            "items": []
        }));
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
    let (p_cur, p_sym) = (std::path::Path::new(cur_file), std::path::Path::new(sym_file));
    p_cur == p_sym
        || (p_cur.parent() == p_sym.parent()
            && p_cur.parent().is_some()
            && p_cur
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s == "mod" || s == "lib"))
}

