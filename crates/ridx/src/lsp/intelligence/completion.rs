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

/// The completion list at the cursor, each item carrying the edit accepting it makes (see [`completion_edits`]).
///
/// [`completion_edits`]: super::completion_edits
pub fn handle_completion(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
    let mut response = completion_list(msg, workspace, overlay_mgr, conv)?;
    let (uri, line_0based, utf16_col_0based) = crate::lsp::params::text_document_position(msg)?;
    let file = uri_to_path(uri);
    let scopes;
    let text: &str = if let Some(overlay) = overlay_mgr.get(&file) {
        &overlay.text
    } else if let Some(s) = super::scope_cache::get_or_parse_file_scopes(&file, None) {
        scopes = s;
        &scopes.text
    } else {
        return Ok(response);
    };
    let Some(line_start) = line_start_byte(text, line_0based as usize) else { return Ok(response) };
    let line_end = text[line_start..].find('\n').map_or(text.len(), |i| line_start + i);
    let line = &text[line_start..line_end];
    let cursor = utf16_to_byte(line, utf16_col_0based);
    let site = super::completion_edits::EditSite {
        before: &line[..cursor],
        after: &line[cursor..],
        line_0based,
        col_utf16: utf16_col_0based,
        in_use_item: super::completion_edits::in_use_item(text, line_start + cursor),
        doc_text: text,
        file: std::path::Path::new(&file),
    };
    super::completion_edits::apply(&mut response, &site);
    Ok(response)
}

/// Whether an item of `kind` is marked with its declaration (for its call's arguments and its doc comment): any
/// item, not a local binding.
fn is_documented_kind(kind: ridx_format::DefKind) -> bool {
    !matches!(kind, ridx_format::DefKind::Local | ridx_format::DefKind::TyParam | ridx_format::DefKind::LifetimeParam | ridx_format::DefKind::ConstParam)
}

fn line_start_byte(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return Some(0);
    }
    text.match_indices('\n').nth(line - 1).map(|(i, _)| i + 1)
}

/// The byte offset in `line` of the UTF-16 column `col`, clamped to the line.
fn utf16_to_byte(line: &str, col: u32) -> usize {
    let mut units = 0u32;
    for (i, c) in line.char_indices() {
        if units >= col {
            return i;
        }
        units += c.len_utf16() as u32;
    }
    line.len()
}

fn completion_list(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
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

    // Guard against out-of-bounds positions beyond end-of-file
    if let Some(overlay) = overlay_opt {
        if line_0based as usize >= overlay.line_starts.len() {
            return Ok(json!({
                "isIncomplete": false,
                "items": []
            }));
        }
    } else if let Some(scopes) = super::scope_cache::get_or_parse_file_scopes(&file, None) {
        if line_0based as usize >= scopes.line_starts.len() {
            return Ok(json!({
                "isIncomplete": false,
                "items": []
            }));
        }
    }

    // Extract line slice up to cursor
    let line_slice = if let Some(overlay) = overlay_opt {
        let idx = line_0based as usize;
        overlay.line_starts.get(idx).map_or_else(String::new, |&s| {
            let bp = overlay.line_char_col_to_byte(line_1based, col_1based);
            overlay.text[s..bp.min(overlay.text.len())].to_string()
        })
    } else if let Some(scopes) = super::scope_cache::get_or_parse_file_scopes(&file, None) {
        let line_idx = line_0based as usize;
        let line_start = scopes.line_starts.get(line_idx).copied().unwrap_or(0);
        let next_start = scopes.line_starts.get(line_idx + 1).copied().unwrap_or(scopes.text.len());
        let line = &scopes.text[line_start..next_start];
        line.chars().take(col_1based.saturating_sub(1) as usize).collect()
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
            let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
            let owner_ctx = super::owner_completion::OwnerCompletionContext {
                file: &file,
                line_1based,
                line_slice: &line_slice,
                prefix: &prefix,
                is_dot,
                is_colon,
                trimmed_before,
                overlay_text: overlay_opt.map(|o| o.text.as_str()),
                is_dirty,
            };
            // After `.` the receiver's inferred type decides the list; the owner index answers when it cannot be inferred.
            let cursor_byte = if is_dot { crate::lsp::navigation::byte_offset_from_conv_or_overlay(&file, line_1based, col_1based, overlay_mgr, conv) } else { None };
            if let Some(items) = cursor_byte.and_then(|off| super::live_completion::live_dot_items(ws, &owner_ctx, off, overlay_mgr)) {
                return Ok(json!({
                    "isIncomplete": false,
                    "items": items
                }));
            }
            if let Some(owner_items) = super::owner_completion::try_owner_completion(ws, &owner_ctx) {
                return Ok(json!({
                    "isIncomplete": false,
                    "items": owner_items
                }));
            }

            // Fallback path: streaming symbol traversal with early-stopping and early-exit
            let (fallback_items, is_capped) = super::fallback_completion::try_fallback_completion(ws, &owner_ctx, overlay_opt);
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

            let is_in_scope = if s.scope_id == 0 { true } else { active_scope_ids.contains(&s.scope_id) && s.decl_start_byte <= cursor_byte };

            if is_in_scope && seen_names.insert(s.name.clone()) {
                let mut item = json!({
                    "label": s.name,
                    "kind": lsp_completion_kind(s.kind),
                    "detail": s.signature
                });
                if is_documented_kind(s.kind) {
                    let line = overlay.text[..s.decl_start_byte.min(overlay.text.len())].matches('\n').count() as u32 + 1;
                    super::completion_edits::mark_decl(&mut item, std::path::Path::new(&file), line);
                }
                items.push(item);
            }
        }
    }

    // 2. Workspace symbols
    let mut is_capped = false;
    if let Some(ws) = workspace {
        let current_crate = ws.crate_name_for_file(&file);
        let ws_symbols = if prefix.is_empty() { ws.search_local_symbols(&prefix) } else { ws.search_symbols(&prefix) };
        let doc_text = overlay_opt.map(|o| o.text.as_str()).unwrap_or("");
        let mut public_paths = std::collections::HashMap::new();
        let mut seen_imports = HashSet::new();

        for s in ws_symbols {
            // Only what a name brings in: no impl block, associated item, field, local or another module's import.
            if !super::completion_import::importable(&s) {
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
                    if matches!(parent_def.kind, ridx_format::DefKind::Fn | ridx_format::DefKind::AssocFn | ridx_format::DefKind::Closure) {
                        continue;
                    }
                }
            }
            let is_same_module = is_same_file || is_symbol_in_current_module(&file, s.file);
            // A name from elsewhere comes with its `use`, by a path that surely reaches it; a name the file already
            // imports (by whatever path) is offered as it is, without a second, clashing import.
            let import_path = if is_same_module || super::completion_import::name_imported(doc_text, s.name) {
                None
            } else {
                match super::completion_import::import_path(&s, current_crate, &mut public_paths) {
                    Some(p) => Some(p),
                    None => continue,
                }
            };
            let in_scope = import_path.is_none();
            if (in_scope && seen_names.insert(s.name.to_string())) || (!in_scope && !seen_names.contains(s.name) && seen_imports.insert(import_path.clone())) {
                let import_path = import_path.unwrap_or_default();
                let auto_import_edit = if in_scope { None } else { overlay_opt.and_then(|ov| super::auto_import::compute_auto_import_edit(&ov.text, &import_path)) };

                let detail = if let Some(sig) = s.signature { sig.to_string() } else { s.path.to_string() };

                let mut item = json!({
                    "label": s.name,
                    "kind": lsp_completion_kind(s.kind),
                    "detail": detail
                });

                if let Some(edit) = auto_import_edit {
                    // As rust-analyzer labels an import completion: the editor shows `detail` next to the name, so
                    // the same name by several paths reads `HashSet (use std::collections::HashSet)`.
                    item["labelDetails"] = json!({ "detail": format!(" (use {import_path})"), "description": detail });
                    item["additionalTextEdits"] = json!([edit]);
                }
                if is_documented_kind(s.kind) {
                    let decl_file = crate::lsp::navigation::type_definition_parse::on_disk(s.resolved_path());
                    super::completion_edits::mark_decl(&mut item, &decl_file, s.span.start_line);
                }

                items.push(item);
                if items.len() >= 100 {
                    is_capped = true;
                    break;
                }
            }
        }
    }

    // Among the names that come with an import, the standard library's first.
    let import_rank = |i: &Value| match i["labelDetails"]["detail"].as_str() {
        None => 0,
        Some(d) if d.starts_with(" (use std::") => 1,
        Some(_) => 2,
    };
    items.sort_by_key(import_rank);

    Ok(json!({
        "isIncomplete": is_capped,
        "items": items
    }))
}

pub fn handle_completion_resolve(msg: &Value) -> LspResult<Value> {
    let params = msg.get("params").ok_or_else(|| LspError::invalid_params("Missing params object"))?;
    if !params.is_object() {
        return Err(LspError::invalid_params("Params must be a CompletionItem object"));
    }

    let label = params.get("label").and_then(|l| l.as_str()).ok_or_else(|| LspError::invalid_params("CompletionItem must contain a label"))?;
    if label.is_empty() {
        return Err(LspError::invalid_params("CompletionItem label cannot be empty"));
    }

    let mut resolved = params.clone();
    if resolved.get("documentation").is_none() && !super::completion_docs::resolve_documentation(&mut resolved) {
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
    p_cur == p_sym || (p_cur.parent() == p_sym.parent() && p_cur.parent().is_some() && p_cur.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s == "mod" || s == "lib"))
}
