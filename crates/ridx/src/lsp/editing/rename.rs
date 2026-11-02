/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Symbol rename operations, identifier validation, and linked editing ranges.

use super::ident::*;
use super::ref_span::span_covers_other_identifier;
use crate::lsp::error::{LspError, LspResult};
use crate::lsp::protocol::{path_to_uri, uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;

pub use super::prepare_rename::handle_prepare_rename;
use super::prepare_rename::{get_cached_line, split_line_offsets, utf16_cols_on_line, utf16_to_char_col_in_line};

fn to_utf16_col(ov: Option<&crate::overlay::DocumentOverlay>, conv: &mut PositionConverter, file: &str, line: u32, col: u32) -> u32 {
    ov.map(|o| o.char_col_to_utf16_col(line, col)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, line, col))
}

pub fn handle_rename(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
    let new_name = msg["params"]["newName"].as_str().unwrap_or("");
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
    let utf16_col_0based = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
    let line_1based = line_0based + 1;

    let overlay_opt = overlay_mgr.get(&file);
    let mut disk_text = if overlay_opt.is_none() { std::fs::read_to_string(&file).ok() } else { None };
    let pre = disk_text.as_deref();
    let full_text = overlay_opt.map(|o| o.text.as_str()).or(pre);
    let cur_line = full_text.and_then(|t| t.lines().nth(line_0based as usize));

    let col_1based = if let Some(overlay) = overlay_opt {
        overlay.utf16_col_to_char_col(line_1based, utf16_col_0based)
    } else if let Some(line_s) = cur_line {
        utf16_to_char_col_in_line(line_s, utf16_col_0based)
    } else {
        conv.utf16_col_to_char_col(&file, line_1based, utf16_col_0based)
    };

    if cur_line.is_none_or(|l| l.contains('\'')) {
        if let Some(lt_res) = super::rename_special::try_rename_lifetime(&file, line_1based, col_1based, new_name, overlay_mgr, pre, conv) {
            let mut lt_map = lt_res?;
            validate_and_dedup_edits(&mut lt_map)?;
            return Ok(super::rename_unindexed::build_workspace_edit_json(lt_map, None, overlay_mgr));
        }
    }
    super::rename_special::check_cursor_not_keyword(&file, line_1based, col_1based, overlay_mgr, pre, "No renameable symbol found at this position")?;

    if !is_valid_rust_ident(new_name) {
        return Err(LspError::request_failed(format!("Invalid name '{new_name}': not a valid Rust identifier")));
    }

    if full_text.is_none_or(|t| t.contains(" as ")) {
        if let Some(mut alias_map) = super::rename_unindexed::try_rename_use_as_alias(workspace, &file, line_1based, col_1based, new_name, overlay_mgr, pre, conv) {
            validate_and_dedup_edits(&mut alias_map)?;
            return Ok(super::rename_unindexed::build_workspace_edit_json(alias_map, None, overlay_mgr));
        }
    }

    let mut changes_map: HashMap<String, Vec<Value>> = HashMap::new();
    let mut mod_file_rename: Option<(String, String)> = None;

    if let Some(overlay) = overlay_opt {
        let local_refs = overlay.find_local_binding_references(line_1based, col_1based);
        if !local_refs.is_empty() {
            let ov_lines: Vec<&str> = overlay.text.lines().collect();
            let old_ident = overlay.get_ident_at(line_1based, col_1based).unwrap_or_default();
            let mut edits = Vec::new();
            for (sl, sc, el, ec) in local_refs {
                let start_char = overlay.char_col_to_utf16_col(sl, sc);
                let end_char = overlay.char_col_to_utf16_col(el, ec);
                let new_text = super::rename_special::local_edit_new_text(&ov_lines, sl, sc, ec, &old_ident, new_name);
                let edit = json!({
                    "range": {
                        "start": { "line": sl.saturating_sub(1), "character": start_char },
                        "end": { "line": el.saturating_sub(1), "character": end_char }
                    },
                    "newText": new_text
                });
                if !edits.iter().any(|e: &Value| e["range"] == edit["range"]) {
                    edits.push(edit);
                }
            }
            changes_map.insert(uri.to_string(), edits);
            validate_and_dedup_edits(&mut changes_map)?;
            return Ok(super::rename_unindexed::build_workspace_edit_json(changes_map, None, overlay_mgr));
        }
    }

    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            let tok_is_self = cur_line.and_then(|l| extract_ident_at_col(l, col_1based)).is_some_and(|t| t == "Self");
            let mut loc_res_opt = if tok_is_self {
                if let Some(self_res) = super::rename_special::resolve_self_keyword_target(ws, &file, line_1based, col_1based, overlay_mgr, pre) {
                    let (tf, tl, tc) = self_res?;
                    ws.try_find_rename_references_at(&tf, tl, tc)
                } else {
                    ws.try_find_rename_references_at(&file, line_1based, col_1based)
                }
            } else {
                ws.try_find_rename_references_at(&file, line_1based, col_1based)
            };
            if loc_res_opt.is_none() {
                if let Some((tf, tl, tc)) =
                    super::detached_rename::collect_detached_rename_edits(ws, &file, line_1based, col_1based, new_name, overlay_mgr, pre, conv, &mut changes_map)
                        .map_err(|r| LspError::request_failed(r.message()))?
                {
                    loc_res_opt = ws.try_find_rename_references_at(&tf, tl, tc);
                }
            }
            if let Some(loc_res) = loc_res_opt {
                let loc_refs = loc_res.map_err(|r| LspError::request_failed(r.message()))?;
                let target_name = loc_refs.target_symbol.rsplit("::").next().unwrap_or(loc_refs.target_symbol);
                let mut file_cache: HashMap<String, (String, Vec<(usize, usize)>)> = HashMap::new();
                if let Some(owned) = disk_text.take() {
                    let offs = split_line_offsets(&owned);
                    file_cache.insert(file.clone(), (owned, offs));
                }
                let decl_path = loc_refs.target_resolved_path();
                let decl_str = decl_path.to_str().unwrap_or(loc_refs.target_file);
                let mut decl_final_line = None;

                if !loc_refs.target_file.is_empty() && loc_refs.target_file != "external" {
                    let decl_uri = path_to_uri(decl_str);
                    let decl_ov = overlay_mgr.get(decl_str);
                    if let Some(dov) = decl_ov {
                        if dov.is_dirty() && dov.has_modified_imports() {
                            return Err(LspError::request_failed("Cannot rename symbol in file with modified imports; save document first"));
                        }
                    }

                    let mapped_line = if let Some(dov) = decl_ov.filter(|d| d.is_dirty()) {
                        dov.map_compiled_to_live(loc_refs.target_line, loc_refs.target_col)
                            .map(|p| p.0)
                            .ok_or_else(|| LspError::request_failed("Cannot map symbol position in modified file"))?
                    } else {
                        loc_refs.target_line
                    };

                    let (final_line, ident_col, decl_u16) = if let Some(dov) = decl_ov {
                        let decl_lines_ref: Vec<&str> = dov.text.lines().collect();
                        let (fl, ic) = find_declaration_ident_in_lines(&decl_lines_ref, mapped_line, target_name, loc_refs.target_kind)
                            .ok_or_else(|| LspError::request_failed("Cannot find declaration identifier at mapped position"))?;
                        if loc_refs.target_kind == DefKind::Mod {
                            mod_file_rename = super::rename_unindexed::detect_module_file_rename(decl_str, &decl_lines_ref, fl, target_name, new_name);
                        }
                        if dov.get_ident_at(fl, ic).as_deref() != Some(target_name) {
                            return Err(LspError::request_failed(format!("Identifier at mapped location does not match '{target_name}'")));
                        }
                        let ec_char = ic + target_name.chars().count() as u32;
                        (fl, ic, (dov.char_col_to_utf16_col(fl, ic), dov.char_col_to_utf16_col(fl, ec_char)))
                    } else {
                        let entry = file_cache.entry(decl_str.to_string()).or_insert_with(|| {
                            let t = std::fs::read_to_string(decl_str).unwrap_or_default();
                            let offs = split_line_offsets(&t);
                            (t, offs)
                        });
                        let decl_lines_ref: Vec<&str> = entry.1.iter().map(|&(s, e)| &entry.0[s..e]).collect();
                        let (fl, ic) = find_declaration_ident_in_lines(&decl_lines_ref, mapped_line, target_name, loc_refs.target_kind)
                            .ok_or_else(|| LspError::request_failed("Cannot find declaration identifier at mapped position"))?;
                        if loc_refs.target_kind == DefKind::Mod {
                            mod_file_rename = super::rename_unindexed::detect_module_file_rename(decl_str, &decl_lines_ref, fl, target_name, new_name);
                        }
                        let ec_char = ic + target_name.chars().count() as u32;
                        let u16_pair = get_cached_line(entry, fl)
                            .map(|ls| utf16_cols_on_line(ls, ic, ec_char))
                            .unwrap_or_else(|| (to_utf16_col(None, conv, decl_str, fl, ic), to_utf16_col(None, conv, decl_str, fl, ec_char)));
                        (fl, ic, u16_pair)
                    };
                    let _ = ident_col;
                    decl_final_line = Some(final_line);
                    let (sc, ec) = decl_u16;
                    let sl = final_line.saturating_sub(1);
                    changes_map.entry(decl_uri).or_default().push(json!({
                        "range": { "start": { "line": sl, "character": sc }, "end": { "line": sl, "character": ec } },
                        "newText": new_name
                    }));
                }

                for r in &loc_refs.references {
                    let r_path = r.resolved_path();
                    if ws.is_external_path(&r_path) {
                        continue;
                    }
                    let r_str = r_path.to_str().unwrap_or(r.file);
                    let r_uri = path_to_uri(r_str);

                    let ref_ov = overlay_mgr.get(r_str);
                    let (mut mapped_line, mut mapped_col) = if let Some(rov) = ref_ov.filter(|o| o.is_dirty()) {
                        if rov.has_modified_imports() {
                            return Err(LspError::request_failed("Cannot rename symbol in caller file with modified imports; save document first"));
                        }
                        let pos = rov
                            .map_compiled_to_live(r.span.start_line(), r.span.start_col())
                            .ok_or_else(|| LspError::request_failed("Cannot map symbol position in modified file"))?;
                        if let Some(local_def) = rov.find_definition_at(pos.0, pos.1) {
                            if r_str != decl_str || decl_final_line != Some(local_def.start_line) {
                                return Err(LspError::request_failed("Symbol shadowed by local definition"));
                            }
                        }
                        pos
                    } else {
                        (r.span.start_line(), r.span.start_col())
                    };

                    let cached_entry = if ref_ov.is_none() {
                        Some(file_cache.entry(r_str.to_string()).or_insert_with(|| {
                            let t = std::fs::read_to_string(r_str).unwrap_or_default();
                            let offs = split_line_offsets(&t);
                            (t, offs)
                        }))
                    } else {
                        None
                    };

                    let mut occ_name: Option<String> = if let Some(rov) = ref_ov {
                        rov.get_ident_at(mapped_line, mapped_col)
                    } else {
                        cached_entry.as_deref().and_then(|e| get_cached_line(e, mapped_line)).and_then(|line| extract_ident_at_col(line, mapped_col).map(String::from))
                    };

                    if occ_name.as_deref() != Some(target_name)
                        && !r.is_macro_expansion
                        && r.span.start_line() > 0
                        && (r.span.end_line() > r.span.start_line() || r.span.end_col().saturating_sub(r.span.start_col()) as usize > target_name.chars().count())
                    {
                        let lines_ref: Vec<&str> = if let Some(rov) = ref_ov {
                            rov.text.lines().collect()
                        } else if let Some(e) = cached_entry.as_deref() {
                            let max_l = (r.span.end_line().max(mapped_line) as usize).min(e.1.len());
                            e.1[..max_l].iter().map(|&(s, en)| &e.0[s..en]).collect()
                        } else {
                            Vec::new()
                        };
                        let max_line = (r.span.end_line().max(mapped_line) as usize).min(lines_ref.len());
                        if let Some((dl, dc)) = find_declaration_ident_in_lines(&lines_ref[..max_line], mapped_line, target_name, r.kind) {
                            mapped_line = dl;
                            mapped_col = dc;
                            occ_name = Some(target_name.to_string());
                        }
                    }

                    let occ_str = match occ_name {
                        Some(s) if !s.is_empty() => s,
                        _ => {
                            if r.is_macro_expansion
                                || span_covers_other_identifier(ref_ov.is_none().then_some(r_str), (r.span.start_line(), r.span.start_col(), r.span.end_line(), r.span.end_col()))
                            {
                                continue;
                            }
                            return Err(LspError::request_failed("Failed to locate identifier at reference position"));
                        }
                    };
                    if occ_str != target_name {
                        continue;
                    }

                    let end_col = mapped_col + occ_str.chars().count() as u32;
                    let cached_line = cached_entry.as_deref().and_then(|e| get_cached_line(e, mapped_line));
                    if let Some(line_str) = cached_line {
                        if loc_refs.target_kind != DefKind::Macro && line_str.chars().nth(end_col.saturating_sub(1) as usize) == Some('!') {
                            continue;
                        }
                        if r_str == decl_str && matches!(loc_refs.target_kind, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::Trait) {
                            let prefix: String = line_str.chars().take(mapped_col.saturating_sub(1) as usize).collect();
                            if let Some(before_colons) = prefix.strip_suffix("::") {
                                let seg = before_colons.rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or("");
                                if !seg.is_empty() && !matches!(seg, "self" | "super" | "crate") && !loc_refs.target_path.contains(&format!("{seg}::")) {
                                    continue;
                                }
                            }
                        }
                    }

                    let (sc, ec) = if let Some(line_str) = cached_line {
                        utf16_cols_on_line(line_str, mapped_col, end_col)
                    } else {
                        (to_utf16_col(ref_ov, conv, r_str, mapped_line, mapped_col), to_utf16_col(ref_ov, conv, r_str, mapped_line, end_col))
                    };
                    let sl = mapped_line.saturating_sub(1);
                    let edit_new_text = if matches!(loc_refs.target_kind, DefKind::Field | DefKind::Local) {
                        let lines_ref: Vec<&str> = if let Some(rov) = ref_ov {
                            rov.text.lines().collect()
                        } else if let Some(e) = cached_entry.as_deref() {
                            e.1.iter().map(|&(s, en)| &e.0[s..en]).collect()
                        } else {
                            Vec::new()
                        };
                        let is_decl = r_str == decl_str && Some(mapped_line) == decl_final_line;
                        if loc_refs.target_kind == DefKind::Field {
                            super::rename_special::field_edit_new_text(&lines_ref, mapped_line, mapped_col, end_col, is_decl, target_name, new_name)
                        } else if is_decl {
                            new_name.to_string()
                        } else {
                            super::rename_special::local_edit_new_text(&lines_ref, mapped_line, mapped_col, end_col, target_name, new_name)
                        }
                    } else {
                        new_name.to_string()
                    };
                    let edits = changes_map.entry(r_uri).or_default();
                    let edit = json!({
                        "range": { "start": { "line": sl, "character": sc }, "end": { "line": sl, "character": ec } },
                        "newText": edit_new_text
                    });
                    if !edits.iter().any(|e| e["range"] == edit["range"]) {
                        edits.push(edit);
                    }
                }

                super::rename_unindexed::expand_generic_param_edits(&loc_refs, new_name, overlay_mgr, conv, &mut changes_map);
                super::rename_unindexed::expand_unindexed_cargo_target_edits(ws, &loc_refs, target_name, new_name, conv, &mut changes_map)
                    .map_err(|r| LspError::request_failed(r.message()))?;
            }
        }
    }

    if changes_map.is_empty() {
        return Err(LspError::request_failed("No renameable symbol found at this position"));
    }

    validate_and_dedup_edits(&mut changes_map)?;
    Ok(super::rename_unindexed::build_workspace_edit_json(changes_map, mod_file_rename, overlay_mgr))
}

pub(crate) fn validate_and_dedup_edits(changes_map: &mut HashMap<String, Vec<Value>>) -> Result<(), LspError> {
    for (doc_uri, edits) in changes_map {
        edits.sort_by(|a, b| {
            let a_sl = a["range"]["start"]["line"].as_u64().unwrap_or(0);
            let b_sl = b["range"]["start"]["line"].as_u64().unwrap_or(0);
            let a_sc = a["range"]["start"]["character"].as_u64().unwrap_or(0);
            let b_sc = b["range"]["start"]["character"].as_u64().unwrap_or(0);
            (a_sl, a_sc).cmp(&(b_sl, b_sc))
        });
        let mut deduped: Vec<Value> = Vec::with_capacity(edits.len());
        for edit in edits.drain(..) {
            if let Some(prev) = deduped.last() {
                let prev_el = prev["range"]["end"]["line"].as_u64().unwrap_or(0);
                let prev_ec = prev["range"]["end"]["character"].as_u64().unwrap_or(0);
                let cur_sl = edit["range"]["start"]["line"].as_u64().unwrap_or(0);
                let cur_sc = edit["range"]["start"]["character"].as_u64().unwrap_or(0);
                if (cur_sl, cur_sc) < (prev_el, prev_ec) {
                    if prev["range"] == edit["range"] {
                        continue;
                    }
                    let doc_file = uri_to_path(doc_uri);
                    return Err(LspError::internal_error(format!("Overlapping rename edits in file: {doc_file}")));
                }
            }
            deduped.push(edit);
        }
        *edits = deduped;
    }
    Ok(())
}
