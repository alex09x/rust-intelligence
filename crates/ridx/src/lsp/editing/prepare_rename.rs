/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP prepareRename handler for validating rename targets and computing placeholder ranges.

use super::detached_rename::{try_prepare_detached_rename, DetachedPrepareOutcome};
use super::ident::*;
use crate::lsp::error::{LspError, LspResult};
use crate::lsp::protocol::{uri_to_path, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::WorkspaceIndex;
use serde_json::{json, Value};

pub(super) fn split_line_offsets(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 36 + 1);
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            let end = if i > start && bytes[i - 1] == b'\r' { i - 1 } else { i };
            out.push((start, end));
            start = i + 1;
        }
    }
    if start <= bytes.len() {
        let end = if bytes.len() > start && bytes[bytes.len() - 1] == b'\r' { bytes.len() - 1 } else { bytes.len() };
        out.push((start, end));
    }
    out
}

#[inline]
pub(super) fn get_cached_line(entry: &(String, Vec<(usize, usize)>), line_1based: u32) -> Option<&str> {
    let &(s, e) = entry.1.get(line_1based.saturating_sub(1) as usize)?;
    Some(&entry.0[s..e])
}

#[inline]
pub(super) fn utf16_to_char_col_in_line(line: &str, utf16_col_0based: u32) -> u32 {
    let mut u16_cur = 0u32;
    let mut char_col = 1u32;
    for ch in line.chars() {
        if u16_cur >= utf16_col_0based {
            break;
        }
        u16_cur += ch.len_utf16() as u32;
        char_col += 1;
    }
    char_col
}

#[inline]
pub(super) fn utf16_cols_on_line(line_str: &str, start_col_1based: u32, end_col_1based: u32) -> (u32, u32) {
    let start_idx = start_col_1based.saturating_sub(1) as usize;
    let end_idx = end_col_1based.saturating_sub(1) as usize;
    let s_u16: u32 = line_str.chars().take(start_idx).map(|c| c.len_utf16() as u32).sum();
    let len_u16: u32 = line_str.chars().skip(start_idx).take(end_idx.saturating_sub(start_idx)).map(|c| c.len_utf16() as u32).sum();
    (s_u16, s_u16 + len_u16)
}

fn char_to_utf16_col_in_line(line: &str, char_col_1based: u32) -> u32 {
    line.chars().take(char_col_1based.saturating_sub(1) as usize).map(|c| c.len_utf16() as u32).sum()
}

pub fn handle_prepare_rename(msg: &Value, workspace: Option<&WorkspaceIndex>, overlay_mgr: &OverlayManager, conv: &mut PositionConverter) -> LspResult<Value> {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let line_0based = msg["params"]["position"]["line"].as_u64().unwrap_or(0) as u32;
    let utf16_col_0based = msg["params"]["position"]["character"].as_u64().unwrap_or(0) as u32;
    let line_1based = line_0based + 1;

    let overlay_opt = overlay_mgr.get(&file);
    let disk_text = if overlay_opt.is_none() { std::fs::read_to_string(&file).ok() } else { None };
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
        if let Some(lt_res) = super::rename_special::try_prepare_rename_lifetime(&file, line_1based, col_1based, overlay_mgr, pre, conv) {
            return lt_res;
        }
    }
    super::rename_special::check_cursor_not_keyword(&file, line_1based, col_1based, overlay_mgr, pre, "Cannot prepare rename: no identifier found at this position")?;
    if full_text.is_none_or(|t| t.contains(" as ")) {
        if let Some(alias_val) = super::rename_unindexed::try_prepare_rename_use_as_alias(&file, line_1based, col_1based, overlay_mgr, pre, conv) {
            return Ok(alias_val);
        }
    }

    if let Some(overlay) = overlay_opt {
        let refs = overlay.find_local_binding_references(line_1based, col_1based);
        if let Some((sl, sc, el, ec)) = refs.iter().find(|(sl, sc, el, ec)| line_1based >= *sl && line_1based <= *el && col_1based >= *sc && col_1based <= *ec) {
            let start_char = overlay.char_col_to_utf16_col(*sl, *sc);
            let end_char = overlay.char_col_to_utf16_col(*el, *ec);
            let ident = overlay.get_ident_at(*sl, *sc).unwrap_or_default();
            return Ok(json!({
                "range": {
                    "start": { "line": sl.saturating_sub(1), "character": start_char },
                    "end": { "line": el.saturating_sub(1), "character": end_char }
                },
                "placeholder": ident
            }));
        }
    }

    if let Some(ws) = workspace {
        let is_dirty = overlay_opt.map(|o| o.is_dirty()).unwrap_or(false);
        if !is_dirty {
            let is_self_tok = cur_line.and_then(|l| super::rename_special::extract_full_ident_around_col(l, col_1based)).is_some_and(|(_, tok)| tok == "Self");
            if is_self_tok {
                if let Some(self_res) = super::rename_special::resolve_self_keyword_target(ws, &file, line_1based, col_1based, overlay_mgr, pre) {
                    let (tf, tl, tc) = self_res?;
                    if let Some(res) = ws.check_prepare_rename_at(&tf, tl, tc) {
                        let def = res.map_err(|r| LspError::request_failed(r.prepare_message()))?;
                        super::rename_unindexed::check_unindexed_target_ambiguity(ws, &def).map_err(|r| LspError::request_failed(r.prepare_message()))?;
                        let target_name = def.name.rsplit("::").next().unwrap_or(def.name);
                        let line_s = cur_line.unwrap_or("");
                        let self_col = super::rename_special::extract_full_ident_around_col(line_s, col_1based).map(|(c, _)| c).unwrap_or(col_1based);
                        let sc = overlay_opt.map(|o| o.char_col_to_utf16_col(line_1based, self_col)).unwrap_or_else(|| char_to_utf16_col_in_line(line_s, self_col));
                        return Ok(json!({
                            "range": {
                                "start": { "line": line_0based, "character": sc },
                                "end": { "line": line_0based, "character": sc + 4 }
                            },
                            "placeholder": target_name
                        }));
                    }
                }
            }

            let mut prep_opt = ws.check_prepare_rename_at(&file, line_1based, col_1based);
            if prep_opt.is_none() {
                if let Some(det_res) = try_prepare_detached_rename(ws, &file, line_1based, col_1based, overlay_mgr, pre, conv) {
                    match det_res.map_err(|r| LspError::request_failed(r.prepare_message()))? {
                        DetachedPrepareOutcome::DelegatedTraitItem(tf, tl, tc) => {
                            prep_opt = ws.check_prepare_rename_at(&tf, tl, tc);
                        }
                        DetachedPrepareOutcome::LocalPlaceholder { range, placeholder } => {
                            return Ok(json!({ "range": range, "placeholder": placeholder }));
                        }
                    }
                }
            }

            if let Some(res) = prep_opt {
                let def = res.map_err(|r| LspError::request_failed(r.prepare_message()))?;
                super::rename_unindexed::check_unindexed_target_ambiguity(ws, &def).map_err(|r| LspError::request_failed(r.prepare_message()))?;
                let target_name = def.name.rsplit("::").next().unwrap_or(def.name);
                let ident_col = cur_line
                    .and_then(|s| {
                        find_declaration_ident_col(s, target_name, def.kind)
                            .or_else(|| super::rename_special::extract_full_ident_around_col(s, col_1based).and_then(|(c, tok)| (tok == target_name).then_some(c)))
                    })
                    .unwrap_or(col_1based);
                let sc = if let Some(ov) = overlay_opt {
                    ov.char_col_to_utf16_col(line_1based, ident_col)
                } else if let Some(s) = cur_line {
                    char_to_utf16_col_in_line(s, ident_col)
                } else {
                    conv.char_col_to_utf16_col(&file, line_1based, ident_col)
                };
                let ec = sc + target_name.chars().map(|c| c.len_utf16() as u32).sum::<u32>();

                return Ok(json!({
                    "range": {
                        "start": { "line": line_0based, "character": sc },
                        "end": { "line": line_0based, "character": ec }
                    },
                    "placeholder": target_name
                }));
            }
        }
    }

    Err(LspError::request_failed("Cannot prepare rename: no identifier found at this position"))
}
