/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Specialized rename handlers: lifetimes (`'a`), `Self` keyword, `use ... as` aliases,
//! struct field/local shorthand (`Foo { x }`), generic parameters (`T`), and module file renames.

use super::ident::{is_ident_continue, is_ident_start, is_valid_rust_ident};
use crate::lsp::error::{LspError, LspResult};
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::mapping::is_byte_in_string_or_comment;
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, WorkspaceIndex};
use serde_json::{json, Value};
use std::collections::HashMap;

fn char_col_to_byte(line: &str, col_1based: u32) -> usize {
    let target = col_1based.saturating_sub(1) as usize;
    line.char_indices().nth(target).map(|(b, _)| b).unwrap_or(line.len())
}

fn find_item_end_line(lines: &[&str], start_idx: usize) -> usize {
    let mut depth = 0i32;
    let mut seen_open = false;
    for (idx, &line) in lines.iter().enumerate().skip(start_idx) {
        let code = line.split("//").next().unwrap_or(line);
        for (b_idx, ch) in code.char_indices() {
            if is_byte_in_string_or_comment(code, b_idx) {
                continue;
            }
            if ch == '{' {
                depth += 1;
                seen_open = true;
            } else if ch == '}' {
                depth -= 1;
                if seen_open && depth <= 0 {
                    return idx;
                }
            } else if ch == ';' && !seen_open && depth == 0 {
                return idx;
            }
        }
    }
    lines.len().saturating_sub(1)
}

fn extract_lifetime_at_col(line: &str, col_1based: u32) -> Option<(u32, u32, &str)> {
    let b_pos = char_col_to_byte(line, col_1based);
    if b_pos >= line.len() || is_byte_in_string_or_comment(line, b_pos) {
        return None;
    }
    let bytes = line.as_bytes();
    let mut quote_byte = None;
    if bytes[b_pos] == b'\'' {
        quote_byte = Some(b_pos);
    } else {
        let mut s = b_pos;
        while s > 0 && is_ident_continue(bytes[s - 1] as char) {
            s -= 1;
        }
        if s > 0 && bytes[s - 1] == b'\'' {
            quote_byte = Some(s - 1);
        }
    }
    let qb = quote_byte?;
    if qb > 0 && is_ident_continue(bytes[qb - 1] as char) {
        return None;
    }
    let id_start = qb + 1;
    if id_start >= line.len() || !is_ident_start(bytes[id_start] as char) {
        return None;
    }
    let mut id_end = id_start;
    while id_end < line.len() && is_ident_continue(bytes[id_end] as char) {
        id_end += 1;
    }
    if id_end < line.len() && bytes[id_end] == b'\'' {
        return None; // character literal like 'a'
    }
    let start_col = line[..qb].chars().count() as u32 + 1;
    let end_col = line[..id_end].chars().count() as u32 + 1;
    Some((start_col, end_col, &line[id_start..id_end]))
}

pub(super) fn find_enclosing_item_range(lines: &[&str], cursor_idx: usize, needle: Option<&str>) -> (usize, usize) {
    let mut candidates = Vec::new();
    for idx in (0..=cursor_idx).rev() {
        let trimmed = lines[idx].split("//").next().unwrap_or(lines[idx]).trim_start();
        let is_item =
            ["pub ", "pub(", "fn ", "async fn ", "unsafe fn ", "const fn ", "struct ", "enum ", "union ", "trait ", "impl", "type "].iter().any(|p| trimmed.starts_with(p));
        if is_item {
            let end_idx = find_item_end_line(lines, idx);
            if end_idx >= cursor_idx {
                candidates.push((idx, end_idx));
            }
        }
    }
    if let Some(n) = needle {
        for &(s, e) in &candidates {
            let header_end = (s + 5).min(e);
            if (s..=header_end).any(|l| lines[l].contains(n)) {
                return (s, e);
            }
        }
    }
    candidates.first().copied().unwrap_or((0, lines.len().saturating_sub(1)))
}

pub(super) fn extract_full_ident_around_col(line: &str, col_1based: u32) -> Option<(u32, &str)> {
    let b_pos = char_col_to_byte(line, col_1based);
    if b_pos >= line.len() || is_byte_in_string_or_comment(line, b_pos) {
        return None;
    }
    let bytes = line.as_bytes();
    if !is_ident_continue(bytes[b_pos] as char) {
        return None;
    }
    let mut s = b_pos;
    while s > 0 && is_ident_continue(bytes[s - 1] as char) {
        s -= 1;
    }
    let mut e = b_pos;
    while e < bytes.len() && is_ident_continue(bytes[e] as char) {
        e += 1;
    }
    let start_col = line[..s].chars().count() as u32 + 1;
    Some((start_col, &line[s..e]))
}

fn load_source_text<'a>(file: &str, overlay_mgr: &'a OverlayManager, preloaded: Option<&'a str>, scratch: &'a mut Option<String>) -> Option<&'a str> {
    if let Some(o) = overlay_mgr.get(file) {
        return Some(o.text.as_str());
    }
    if let Some(s) = preloaded {
        return Some(s);
    }
    *scratch = std::fs::read_to_string(file).ok();
    scratch.as_deref()
}

pub fn check_cursor_not_keyword(file: &str, line_1based: u32, col_1based: u32, overlay_mgr: &OverlayManager, preloaded: Option<&str>, err_msg: &str) -> LspResult<()> {
    let mut scratch = None;
    let Some(text) = load_source_text(file, overlay_mgr, preloaded, &mut scratch) else {
        return Ok(());
    };
    let Some(line) = text.lines().nth(line_1based.saturating_sub(1) as usize) else { return Ok(()) };
    let b_pos = char_col_to_byte(line, col_1based);
    if b_pos >= line.len() || is_byte_in_string_or_comment(line, b_pos) {
        return Ok(());
    }
    let bytes = line.as_bytes();
    let is_ascii_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    if !is_ascii_id(bytes[b_pos]) {
        return Ok(());
    }
    let mut s = b_pos;
    while s > 0 && is_ascii_id(bytes[s - 1]) {
        s -= 1;
    }
    let mut e = b_pos;
    while e < bytes.len() && is_ascii_id(bytes[e]) {
        e += 1;
    }
    if line.get(s.saturating_sub(2)..s) == Some("r#") || (s > 0 && bytes[s - 1] == b'\'') {
        return Ok(());
    }
    let tok = &line[s..e];
    if tok != "Self" && super::ident::is_rust_keyword(tok) {
        return Err(LspError::request_failed(err_msg));
    }
    Ok(())
}

pub fn try_prepare_rename_lifetime(
    file: &str,
    line_1based: u32,
    col_1based: u32,
    overlay_mgr: &OverlayManager,
    preloaded: Option<&str>,
    conv: &mut PositionConverter,
) -> Option<LspResult<Value>> {
    let ov = overlay_mgr.get(file);
    let mut scratch = None;
    let text = load_source_text(file, overlay_mgr, preloaded, &mut scratch)?;
    let line = text.lines().nth(line_1based.saturating_sub(1) as usize)?;
    let (sc_1, ec_1, lt_name) = extract_lifetime_at_col(line, col_1based)?;
    if matches!(lt_name, "static" | "_") {
        return Some(Err(LspError::request_failed(format!("Cannot rename reserved lifetime ''{lt_name}'"))));
    }
    let sc = ov.map(|o| o.char_col_to_utf16_col(line_1based, sc_1)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, line_1based, sc_1));
    let ec = ov.map(|o| o.char_col_to_utf16_col(line_1based, ec_1)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, line_1based, ec_1));
    let l0 = line_1based.saturating_sub(1);
    Some(Ok(json!({
        "range": { "start": { "line": l0, "character": sc }, "end": { "line": l0, "character": ec } },
        "placeholder": format!("'{lt_name}")
    })))
}

pub fn try_rename_lifetime(
    file: &str,
    line_1based: u32,
    col_1based: u32,
    new_name: &str,
    overlay_mgr: &OverlayManager,
    preloaded: Option<&str>,
    conv: &mut PositionConverter,
) -> Option<LspResult<HashMap<String, Vec<Value>>>> {
    let ov = overlay_mgr.get(file);
    let mut scratch = None;
    let text = load_source_text(file, overlay_mgr, preloaded, &mut scratch)?;
    let line = text.lines().nth(line_1based.saturating_sub(1) as usize)?;
    let (_, _, lt_name) = extract_lifetime_at_col(line, col_1based)?;
    if matches!(lt_name, "static" | "_") {
        return Some(Err(LspError::request_failed(format!("Cannot rename reserved lifetime ''{lt_name}'"))));
    }
    let lines: Vec<&str> = text.lines().collect();
    let clean_new = new_name.strip_prefix('\'').unwrap_or(new_name);
    if !is_valid_rust_ident(clean_new) || matches!(clean_new, "static" | "_") {
        return Some(Err(LspError::request_failed(format!("Invalid lifetime name '{new_name}'"))));
    }
    let pat = format!("'{lt_name}");
    let (s_idx, e_idx) = find_enclosing_item_range(&lines, line_1based.saturating_sub(1) as usize, Some(&pat));
    let mut edits = Vec::new();
    for (idx, &ln_str) in lines.iter().enumerate().take(e_idx + 1).skip(s_idx) {
        let ln_1 = (idx + 1) as u32;
        for (b_pos, _) in ln_str.match_indices(&pat) {
            let col_1 = ln_str[..b_pos].chars().count() as u32 + 1;
            if let Some((sc_1, ec_1, found)) = extract_lifetime_at_col(ln_str, col_1) {
                if found == lt_name {
                    let sc = ov.map(|o| o.char_col_to_utf16_col(ln_1, sc_1)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, ln_1, sc_1));
                    let ec = ov.map(|o| o.char_col_to_utf16_col(ln_1, ec_1)).unwrap_or_else(|| conv.char_col_to_utf16_col(file, ln_1, ec_1));
                    edits.push(json!({
                        "range": { "start": { "line": ln_1 - 1, "character": sc }, "end": { "line": ln_1 - 1, "character": ec } },
                        "newText": format!("'{clean_new}")
                    }));
                }
            }
        }
    }
    let mut map = HashMap::new();
    map.insert(path_to_uri(file), edits);
    Some(Ok(map))
}

pub fn resolve_self_keyword_target(
    ws: &WorkspaceIndex,
    file: &str,
    line_1based: u32,
    col_1based: u32,
    overlay_mgr: &OverlayManager,
    preloaded: Option<&str>,
) -> Option<LspResult<(String, u32, u32)>> {
    let mut scratch = None;
    let text = load_source_text(file, overlay_mgr, preloaded, &mut scratch)?;
    let lines: Vec<&str> = text.lines().collect();
    let cur_idx = line_1based.saturating_sub(1) as usize;
    let cur_line = *lines.get(cur_idx)?;
    if extract_full_ident_around_col(cur_line, col_1based).map(|(_, s)| s) != Some("Self") {
        return None;
    }
    let mut depth = 0i32;
    let mut type_name: Option<String> = None;
    for idx in (0..=cur_idx).rev() {
        let code = lines[idx].split("//").next().unwrap_or(lines[idx]);
        if idx < cur_idx {
            for b in code.bytes().rev() {
                if b == b'}' {
                    depth += 1;
                } else if b == b'{' {
                    depth -= 1;
                }
            }
        }
        if depth <= 0 {
            let trimmed = code.trim_start();
            if trimmed.starts_with("trait ") || trimmed.starts_with("pub trait ") || trimmed.starts_with("pub(crate) trait ") {
                return Some(Err(LspError::request_failed("Cannot rename 'Self' inside a trait definition")));
            }
            let rest = trimmed.strip_prefix("unsafe ").map(str::trim_start).unwrap_or(trimmed);
            if let Some(after_impl) = rest.strip_prefix("impl") {
                if after_impl.starts_with(|c: char| c.is_whitespace() || c == '<') {
                    let before_brace = after_impl.split('{').next().unwrap_or(after_impl).split(" where ").next().unwrap_or(after_impl);
                    let rhs = before_brace.rsplit_once(" for ").map(|(_, r)| r).unwrap_or(before_brace).trim();
                    let stripped = if rhs.starts_with('<') { rhs.split_once('>').map(|(_, r)| r.trim()).unwrap_or(rhs) } else { rhs };
                    let clean = stripped.trim_start_matches('&').trim_start_matches("mut ").split(['<', ' ', '{', '(']).next().unwrap_or("").rsplit("::").next().unwrap_or("");
                    if !clean.is_empty() {
                        type_name = Some(clean.to_string());
                        break;
                    }
                }
            }
            for kw in ["struct ", "enum ", "union "] {
                if let Some((_, after_kw)) = rest.split_once(kw) {
                    let clean = after_kw.trim().split(['<', ' ', '{', '(', ';']).next().unwrap_or("");
                    if !clean.is_empty() {
                        type_name = Some(clean.to_string());
                        break;
                    }
                }
            }
            if type_name.is_some() {
                break;
            }
        }
    }
    let t_name = type_name?;
    for (idx, &ln) in lines.iter().enumerate() {
        let code = ln.split("//").next().unwrap_or(ln);
        for (kw, kind) in [("struct", DefKind::Struct), ("enum", DefKind::Enum), ("union", DefKind::Union)] {
            if code.contains(kw) && code.contains(&t_name) {
                if let Some(col) = super::ident::find_declaration_ident_col(code, &t_name, kind) {
                    return Some(Ok((file.to_string(), (idx + 1) as u32, col)));
                }
            }
        }
    }
    if let Some(sym) = ws.find_first_symbol(&t_name, |s| s.name == t_name && matches!(s.kind, DefKind::Struct | DefKind::Enum | DefKind::Union)) {
        if let Some(p) = sym.resolved_path().to_str() {
            return Some(Ok((p.to_string(), sym.span.start_line(), sym.span.start_col())));
        }
    }
    None
}

pub fn field_edit_new_text(lines: &[&str], line_1based: u32, start_col_1based: u32, end_col_1based: u32, is_decl: bool, target_name: &str, new_name: &str) -> String {
    if is_decl {
        return new_name.to_string();
    }
    let Some(&line) = lines.get(line_1based.saturating_sub(1) as usize) else {
        return new_name.to_string();
    };
    let sb = char_col_to_byte(line, start_col_1based);
    let eb = char_col_to_byte(line, end_col_1based);
    let before = line[..sb].trim_end();
    let after = line[eb..].trim_start();
    let is_dot = before.ends_with('.');
    let is_colon = after.starts_with(':') && !after.starts_with("::");
    if !is_dot && !is_colon && (before.ends_with('{') || before.ends_with(',')) && (after.starts_with('}') || after.starts_with(',') || after.is_empty()) {
        return format!("{new_name}: {target_name}");
    }
    new_name.to_string()
}

pub fn local_edit_new_text(lines: &[&str], line_1based: u32, start_col_1based: u32, end_col_1based: u32, old_name: &str, new_name: &str) -> String {
    let idx = line_1based.saturating_sub(1) as usize;
    let Some(&line) = lines.get(idx) else {
        return new_name.to_string();
    };
    let sb = char_col_to_byte(line, start_col_1based);
    let eb = char_col_to_byte(line, end_col_1based);
    let before = line[..sb].trim_end();
    let after = line[eb..].trim_start();
    if !(before.ends_with('{') || before.ends_with(',')) || !(after.starts_with('}') || after.starts_with(',') || after.is_empty()) {
        return new_name.to_string();
    }
    let min_idx = idx.saturating_sub(6);
    let mut joined = String::new();
    for &l in &lines[min_idx..idx] {
        joined.push_str(l.split("//").next().unwrap_or(l));
        joined.push(' ');
    }
    joined.push_str(&line[..sb]);
    let mut depth = 0i32;
    let mut open_brace_pos = None;
    for (b_i, ch) in joined.char_indices().rev() {
        if ch == '}' {
            depth += 1;
        } else if ch == '{' {
            if depth == 0 {
                open_brace_pos = Some(b_i);
                break;
            }
            depth -= 1;
        }
    }
    if let Some(ob) = open_brace_pos {
        let prefix = joined[..ob].trim_end();
        let last_tok = prefix.rsplit(|c: char| !is_ident_continue(c) && c != ':').next().unwrap_or("");
        let short_tok = last_tok.rsplit("::").next().unwrap_or(last_tok);
        let before_tok = prefix[..prefix.len().saturating_sub(last_tok.len())].trim_end();
        let is_upper = short_tok.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        let is_block_kw = before_tok.ends_with("->") || ["match", "if", "while", "for", "struct", "enum", "union", "trait", "impl"].iter().any(|kw| before_tok.ends_with(kw));
        if is_upper && !is_block_kw {
            return format!("{old_name}: {new_name}");
        }
    }
    new_name.to_string()
}
