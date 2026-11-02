/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `use ... as` alias rename, generic parameter expansion, module file rename,
//! and LSP `WorkspaceEdit` serialization.

use super::ident::{extract_ident_at_col, is_ident_continue};
use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::mapping::is_byte_in_string_or_comment;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, LocationReferences, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

fn find_use_as_alias_decl_in_lines(lines: &[&str], alias_name: &str) -> Option<(u32, u32)> {
    let pat = format!(" as {alias_name}");
    for (idx, &line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or(line);
        if !code.contains("use ") {
            continue;
        }
        if let Some(pos) = code.find(&pat) {
            let (id_start, id_end, before_as) = (pos + 4, pos + 4 + alias_name.len(), code[..pos].trim_end());
            let after_ok = id_end >= code.len() || !is_ident_continue(code[id_end..].chars().next().unwrap_or(' '));
            let orig_ident = before_as.rsplit(|c: char| !is_ident_continue(c)).next().unwrap_or("");
            if after_ok && !orig_ident.is_empty() && orig_ident != alias_name && alias_name != "_" {
                let col_1 = line[..id_start].chars().count() as u32 + 1;
                return Some(((idx + 1) as u32, col_1));
            }
        }
    }
    None
}

pub fn try_prepare_rename_use_as_alias(
    file: &str,
    line_1based: u32,
    col_1based: u32,
    overlay_mgr: &OverlayManager,
    preloaded: Option<&str>,
    conv: &mut PositionConverter,
) -> Option<Value> {
    let ov = overlay_mgr.get(file);
    let owned_text;
    let text = if let Some(o) = ov {
        o.text.as_str()
    } else if let Some(s) = preloaded {
        s
    } else {
        owned_text = std::fs::read_to_string(file).ok()?;
        owned_text.as_str()
    };
    let cur_line = text.lines().nth(line_1based.saturating_sub(1) as usize)?;
    let (sc_1, cursor_ident) = super::rename_special::extract_full_ident_around_col(cur_line, col_1based)?;
    if cursor_ident == "_" || !text.contains(&format!(" as {cursor_ident}")) {
        return None;
    }
    let lines: Vec<&str> = text.lines().collect();
    let (decl_line_1, decl_col_1) = find_use_as_alias_decl_in_lines(&lines, cursor_ident)?;
    if line_1based == decl_line_1 && col_1based < decl_col_1 {
        return None;
    }
    let ec_1 = sc_1 + cursor_ident.chars().count() as u32;
    let sc = ov.map(|o| o.char_col_to_utf16_col(line_1based, sc_1)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, line_1based, sc_1));
    let ec = ov.map(|o| o.char_col_to_utf16_col(line_1based, ec_1)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, line_1based, ec_1));
    let l0 = line_1based.saturating_sub(1);
    Some(json!({
        "range": { "start": { "line": l0, "character": sc }, "end": { "line": l0, "character": ec } },
        "placeholder": cursor_ident
    }))
}

#[allow(clippy::too_many_arguments)]
pub fn try_rename_use_as_alias(
    ws: Option<&WorkspaceIndex>,
    file: &str,
    line_1based: u32,
    col_1based: u32,
    new_name: &str,
    overlay_mgr: &OverlayManager,
    preloaded: Option<&str>,
    conv: &mut PositionConverter,
) -> Option<HashMap<String, Vec<Value>>> {
    let ov = overlay_mgr.get(file);
    let owned_text;
    let text = if let Some(o) = ov {
        o.text.as_str()
    } else if let Some(s) = preloaded {
        s
    } else {
        owned_text = std::fs::read_to_string(file).ok()?;
        owned_text.as_str()
    };
    let cur_line = text.lines().nth(line_1based.saturating_sub(1) as usize)?;
    let (_, cursor_ident) = super::rename_special::extract_full_ident_around_col(cur_line, col_1based)?;
    if cursor_ident == "_" || !text.contains(&format!(" as {cursor_ident}")) {
        return None;
    }
    let lines: Vec<&str> = text.lines().collect();
    let (decl_line_1, decl_col_1) = find_use_as_alias_decl_in_lines(&lines, cursor_ident)?;
    if line_1based == decl_line_1 && col_1based < decl_col_1 {
        return None;
    }

    let mut changes_map: HashMap<String, Vec<Value>> = HashMap::new();
    let doc_uri = path_to_uri(file);
    let name_chars = cursor_ident.chars().count() as u32;

    for (idx, &ln_str) in lines.iter().enumerate() {
        let ln_1 = (idx + 1) as u32;
        for (b_pos, _) in ln_str.match_indices(cursor_ident) {
            let end_b = b_pos + cursor_ident.len();
            let prev_ok = b_pos == 0 || !is_ident_continue(ln_str[..b_pos].chars().next_back().unwrap_or(' '));
            let next_ok = end_b >= ln_str.len() || !is_ident_continue(ln_str[end_b..].chars().next().unwrap_or(' '));
            if !prev_ok || !next_ok || is_byte_in_string_or_comment(ln_str, b_pos) {
                continue;
            }
            let before = ln_str[..b_pos].trim_end();
            if before.ends_with('.') || (before.ends_with("::") && !before.ends_with("self::")) {
                continue;
            }
            let col_1 = ln_str[..b_pos].chars().count() as u32 + 1;
            let sc = ov.map(|o| o.char_col_to_utf16_col(ln_1, col_1)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, ln_1, col_1));
            let ec = ov.map(|o| o.char_col_to_utf16_col(ln_1, col_1 + name_chars)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, ln_1, col_1 + name_chars));
            let edit = json!({
                "range": { "start": { "line": ln_1 - 1, "character": sc }, "end": { "line": ln_1 - 1, "character": ec } },
                "newText": new_name
            });
            let edits = changes_map.entry(doc_uri.clone()).or_default();
            if !edits.iter().any(|e| e["range"] == edit["range"]) {
                edits.push(edit);
            }
        }
    }

    let decl_is_pub = lines.get(decl_line_1.saturating_sub(1) as usize).is_some_and(|l| l.contains("pub"));
    if let Some(w) = ws.filter(|_| decl_is_pub) {
        if let Some(Ok(loc_refs)) = w.try_find_rename_references_at(file, line_1based, col_1based) {
            for r in loc_refs.references {
                let r_path = r.resolved_path();
                if w.is_external_path(&r_path) {
                    continue;
                }
                let Some(r_str) = r_path.to_str().filter(|&s| s != file) else { continue };
                let Ok(r_text) = std::fs::read_to_string(r_str) else { continue };
                let (r_line, r_col) = (r.span.start_line(), r.span.start_col());
                if r_text.lines().nth(r_line.saturating_sub(1) as usize).and_then(|ln| extract_ident_at_col(ln, r_col)) == Some(cursor_ident) {
                    let sc = conv.char_col_to_utf16_col(r_str, r_line, r_col);
                    let ec = conv.char_col_to_utf16_col(r_str, r_line, r_col + name_chars);
                    let edit = json!({
                        "range": { "start": { "line": r_line - 1, "character": sc }, "end": { "line": r_line - 1, "character": ec } },
                        "newText": new_name
                    });
                    let edits = changes_map.entry(path_to_uri(r_str)).or_default();
                    if !edits.iter().any(|e| e["range"] == edit["range"]) {
                        edits.push(edit);
                    }
                }
            }
        }
    }

    (!changes_map.is_empty()).then_some(changes_map)
}

pub fn expand_generic_param_edits(
    loc_refs: &LocationReferences<'_>,
    new_name: &str,
    overlay_mgr: &OverlayManager,
    conv: &mut PositionConverter,
    changes_map: &mut HashMap<String, Vec<Value>>,
) {
    if !matches!(loc_refs.target_kind, DefKind::TyParam | DefKind::ConstParam) {
        return;
    }
    let decl_path = loc_refs.target_resolved_path();
    let Some(file_str) = decl_path.to_str() else { return };
    let ov = overlay_mgr.get(file_str);
    let owned_text;
    let text = if let Some(o) = ov {
        o.text.as_str()
    } else if let Ok(t) = std::fs::read_to_string(file_str) {
        owned_text = t;
        owned_text.as_str()
    } else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    let target_name = loc_refs.target_symbol.rsplit("::").next().unwrap_or(loc_refs.target_symbol);
    let (s_idx, e_idx) = super::rename_special::find_enclosing_item_range(&lines, loc_refs.target_line.saturating_sub(1) as usize, Some(target_name));
    let doc_uri = path_to_uri(file_str);
    let name_chars = target_name.chars().count() as u32;
    for (idx, &ln_str) in lines.iter().enumerate().take(e_idx + 1).skip(s_idx) {
        let ln_1 = (idx + 1) as u32;
        for (b_pos, _) in ln_str.match_indices(target_name) {
            let end_b = b_pos + target_name.len();
            let prev_ok = b_pos == 0 || !is_ident_continue(ln_str[..b_pos].chars().next_back().unwrap_or(' '));
            let next_ok = end_b >= ln_str.len() || !is_ident_continue(ln_str[end_b..].chars().next().unwrap_or(' '));
            if !prev_ok || !next_ok || is_byte_in_string_or_comment(ln_str, b_pos) {
                continue;
            }
            let before = ln_str[..b_pos].trim_end();
            if before.ends_with('.') || (before.ends_with("::") && !before.ends_with("Self::")) {
                continue;
            }
            let col_1 = ln_str[..b_pos].chars().count() as u32 + 1;
            let sc = ov.map(|o| o.char_col_to_utf16_col(ln_1, col_1)).unwrap_or_else(|| conv.char_col_to_utf16_col(file_str, ln_1, col_1));
            let ec = ov.map(|o| o.char_col_to_utf16_col(ln_1, col_1 + name_chars)).unwrap_or_else(|| conv.char_col_to_utf16_col(file_str, ln_1, col_1 + name_chars));
            let edit = json!({
                "range": { "start": { "line": ln_1 - 1, "character": sc }, "end": { "line": ln_1 - 1, "character": ec } },
                "newText": new_name
            });
            let edits = changes_map.entry(doc_uri.clone()).or_default();
            if !edits.iter().any(|e| e["range"] == edit["range"]) {
                edits.push(edit);
            }
        }
    }
}

pub fn detect_module_file_rename(decl_file: &str, decl_lines: &[&str], decl_line_1based: u32, target_name: &str, new_name: &str) -> Option<(String, String)> {
    let idx = decl_line_1based.saturating_sub(1) as usize;
    let line = *decl_lines.get(idx)?;
    let code = line.split("//").next().unwrap_or(line).trim();
    if !code.ends_with(';') || !code.contains(&format!("mod {target_name};")) {
        return None;
    }
    if (idx.saturating_sub(2)..idx).any(|i| decl_lines.get(i).is_some_and(|l| l.contains("#[path"))) {
        return None;
    }
    let decl_p = Path::new(decl_file);
    let parent = decl_p.parent()?;
    let stem = decl_p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let mut candidates = Vec::new();
    if !matches!(stem, "lib" | "main" | "mod") {
        candidates.push(parent.join(stem).join(format!("{target_name}.rs")));
    }
    candidates.push(parent.join(format!("{target_name}.rs")));
    for old_p in candidates {
        if old_p.is_file() {
            let new_p = old_p.with_file_name(format!("{new_name}.rs"));
            return Some((path_to_uri(&old_p.to_string_lossy()), path_to_uri(&new_p.to_string_lossy())));
        }
    }
    None
}

pub fn build_workspace_edit_json(changes_map: HashMap<String, Vec<Value>>, mod_file_rename: Option<(String, String)>, overlay_mgr: &OverlayManager) -> Value {
    let has_versioned_overlay = changes_map.keys().any(|u| overlay_mgr.get(&uri_to_path(u)).is_some_and(|ov| ov.version > 1));
    if mod_file_rename.is_none() && !has_versioned_overlay {
        return json!({ "changes": changes_map });
    }
    let mut sorted_uris: Vec<&String> = changes_map.keys().collect();
    sorted_uris.sort();
    let mut doc_changes = Vec::with_capacity(sorted_uris.len() + usize::from(mod_file_rename.is_some()));
    for doc_uri in sorted_uris {
        let edits = &changes_map[doc_uri];
        let doc_path = uri_to_path(doc_uri);
        let version = overlay_mgr.get(&doc_path).map(|ov| json!(ov.version)).unwrap_or(Value::Null);
        doc_changes.push(json!({ "textDocument": { "uri": doc_uri, "version": version }, "edits": edits }));
    }
    if let Some((old_uri, new_uri)) = mod_file_rename {
        doc_changes.push(json!({ "kind": "rename", "oldUri": old_uri, "newUri": new_uri }));
        return json!({ "documentChanges": doc_changes });
    }
    json!({ "changes": changes_map, "documentChanges": doc_changes })
}

pub(super) fn extract_impl_header_owners(lines: &[&str]) -> Vec<Option<(String, bool)>> {
    let mut out = vec![None; lines.len()];
    let (mut depth, mut pending): (i32, Option<(String, bool)>) = (0, None);
    let mut stack: Vec<(i32, (String, bool))> = Vec::new();
    for (idx, &raw) in lines.iter().enumerate() {
        let ln = raw.split("//").next().unwrap_or(raw);
        let t = ln.trim_start();
        if depth == 0 && (t.starts_with("impl ") || t.starts_with("impl<") || t.starts_with("unsafe impl ") || t.starts_with("unsafe impl<")) {
            let mut hdr = String::new();
            for &h_ln in &lines[idx..(idx + 6).min(lines.len())] {
                let hc = h_ln.split("//").next().unwrap_or(h_ln);
                if let Some(b) = hc.find('{') {
                    hdr.push_str(&hc[..b]);
                    break;
                }
                hdr.push_str(hc);
                hdr.push(' ');
            }
            let body = hdr.trim_start().strip_prefix("unsafe").unwrap_or(&hdr).trim_start().strip_prefix("impl").unwrap_or("").trim_start();
            let after_gen = if body.starts_with('<') {
                let (mut gd, mut end_g) = (0i32, 0usize);
                for (bi, ch) in body.char_indices() {
                    gd += i32::from(ch == '<') - i32::from(ch == '>');
                    if gd == 0 {
                        end_g = bi + 1;
                        break;
                    }
                }
                body[end_g..].trim_start()
            } else {
                body
            };
            let head = after_gen.split(" where ").next().unwrap_or(after_gen);
            let (raw_owner, is_tr) = match head.split_once(" for ") {
                Some((tr, _)) => (tr.trim(), true),
                None => (head.trim(), false),
            };
            let clean = raw_owner.split('<').next().unwrap_or(raw_owner).trim().trim_start_matches('&').trim_start_matches("mut ").trim();
            let short = clean.rsplit("::").next().unwrap_or(clean).trim();
            if !short.is_empty() {
                pending = Some((short.to_string(), is_tr));
            }
        }
        for b in ln.bytes() {
            if b == b'{' {
                depth += 1;
                if let Some(own) = pending.take() {
                    stack.push((depth, own));
                }
            } else if b == b'}' {
                while stack.last().is_some_and(|(d, _)| *d == depth) {
                    stack.pop();
                }
                depth = (depth - 1).max(0);
            }
        }
        out[idx] = stack.last().map(|(_, own)| own.clone());
    }
    out
}
