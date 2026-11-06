/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Syntactic extraction of enclosing `impl`/`trait` scopes and cursor item headers.

use super::action_parse::{byte_to_line0, is_ident_byte, line_starts_of, mask_comments_and_strings, utf16_pos_to_byte, EnclosingContext, ParsedCursorItem};
use super::action_parse_decl::{classify_enclosing_brace, parse_declaration_at};

#[derive(Clone, Copy)]
struct OpenDelim {
    ch: u8,
    open_byte: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct EnclosingSpan {
    pub(crate) ch: u8,
    pub(crate) open_byte: usize,
    pub(crate) close_byte: usize,
}

pub fn parse_item_at_cursor(src: &str, line0: u32, col_u16: u32) -> Option<ParsedCursorItem> {
    if src.is_empty() {
        return None;
    }
    let masked = mask_comments_and_strings(src);
    let starts = line_starts_of(src);
    let cursor_byte = utf16_pos_to_byte(src, &starts, line0, col_u16)?;
    parse_item_with_masked(src, &masked, &starts, cursor_byte)
}

pub fn parse_item_with_masked(src: &str, masked: &str, starts: &[usize], cursor_byte: usize) -> Option<ParsedCursorItem> {
    let mb = masked.as_bytes();
    let enclosing_spans = collect_enclosing_delims(mb, cursor_byte)?;
    for sp in &enclosing_spans {
        if is_macro_delimiter(mb, sp.open_byte) {
            return None;
        }
    }

    let inner_brace = enclosing_spans.iter().rev().find(|s| s.ch == b'{').copied();
    // Cursor inside `(...)` or `[...]` on the item header: only valid if cursor is on the item keyword/name itself,
    // which cannot be inside `(...)` or `[...]`.
    if let Some(last_sp) = enclosing_spans.last() {
        if last_sp.ch != b'{' {
            return None;
        }
    }

    let mut enclosing = EnclosingContext::None;
    let mut in_fn_body = false;
    let mut enclosing_block = None;
    for sp in &enclosing_spans {
        if sp.ch == b'{' {
            let hf = find_prev_item_boundary(mb, sp.open_byte);
            let (_, hkw, _, _, _, _) = scan_outer_attrs_and_docs(src, masked, starts, hf, sp.open_byte);
            if hkw < sp.open_byte && contains_word(&masked[hkw..sp.open_byte], "fn") {
                in_fn_body = true;
            }
        }
    }
    let block_floor = if let Some(br) = inner_brace {
        enclosing_block = Some((br.open_byte, br.close_byte));
        let hdr_floor = find_prev_item_boundary(mb, br.open_byte);
        let (_, hdr_kw_start, cfg_attrs, insert_line0, indent, _) = scan_outer_attrs_and_docs(src, masked, starts, hdr_floor, br.open_byte);
        let hdr_masked = &masked[hdr_kw_start..br.open_byte];
        if contains_word(hdr_masked, "macro_rules") {
            return None;
        }
        enclosing = classify_enclosing_brace(src, masked, hdr_kw_start, br.open_byte, br.close_byte, cfg_attrs, insert_line0, indent);
        if matches!(enclosing, EnclosingContext::ExternBlock) {
            return None;
        }
        br.open_byte + 1
    } else {
        0
    };

    let stmt_floor = find_stmt_start_in_block(mb, block_floor, cursor_byte);
    let (has_doc, kw_start, cfg_attrs, attr_start_line0, indent, first_attr_byte) = scan_outer_attrs_and_docs(src, masked, starts, stmt_floor, src.len());
    if kw_start >= src.len() {
        return None;
    }

    parse_declaration_at(src, masked, kw_start, first_attr_byte, cursor_byte, has_doc, cfg_attrs, attr_start_line0, indent, in_fn_body, enclosing_block, enclosing)
}

pub(crate) fn collect_enclosing_delims(mb: &[u8], cursor_byte: usize) -> Option<Vec<EnclosingSpan>> {
    let mut stack: Vec<OpenDelim> = Vec::new();
    let mut active_before_cursor: Vec<OpenDelim> = Vec::new();
    let mut snapshot_taken = false;
    let mut closed_spans: Vec<EnclosingSpan> = Vec::new();

    for (idx, &b) in mb.iter().enumerate() {
        if !snapshot_taken && idx >= cursor_byte {
            active_before_cursor = stack.clone();
            snapshot_taken = true;
        }
        match b {
            b'{' | b'(' | b'[' => stack.push(OpenDelim { ch: b, open_byte: idx }),
            b'}' | b')' | b']' => {
                let expected = match b {
                    b'}' => b'{',
                    b')' => b'(',
                    _ => b'[',
                };
                let top = stack.pop()?;
                if top.ch != expected {
                    return None;
                }
                if snapshot_taken && active_before_cursor.iter().any(|o| o.open_byte == top.open_byte) {
                    closed_spans.push(EnclosingSpan { ch: top.ch, open_byte: top.open_byte, close_byte: idx });
                }
            }
            _ => {}
        }
    }
    if !snapshot_taken {
        active_before_cursor = stack;
    }
    if closed_spans.len() != active_before_cursor.len() {
        return None;
    }
    closed_spans.sort_by_key(|s| s.open_byte);
    Some(closed_spans)
}

pub(crate) fn is_macro_delimiter(mb: &[u8], open_byte: usize) -> bool {
    let mut i = open_byte;
    while i > 0 && mb[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    if i == 0 || mb[i - 1] != b'!' {
        return false;
    }
    // Exclude `#![...]` inner attributes.
    !(mb[open_byte] == b'[' && i >= 2 && mb[i - 2] == b'#')
}

pub(crate) fn find_prev_item_boundary(mb: &[u8], before_byte: usize) -> usize {
    let mut depth_brace = 0i32;
    let mut depth_paren = 0i32;
    let mut depth_bracket = 0i32;
    let mut depth_angle = 0i32;
    let mut i = before_byte;
    while i > 0 {
        i -= 1;
        let prev = if i > 0 { mb[i - 1] } else { 0 };
        match mb[i] {
            b'}' => {
                if depth_brace == 0 && depth_paren == 0 && depth_bracket == 0 && depth_angle == 0 {
                    return i + 1;
                }
                depth_brace += 1;
            }
            b'{' => {
                if depth_brace == 0 {
                    return i + 1;
                }
                depth_brace -= 1;
            }
            b')' => depth_paren += 1,
            b'(' => depth_paren = (depth_paren - 1).max(0),
            b']' => depth_bracket += 1,
            b'[' => depth_bracket = (depth_bracket - 1).max(0),
            b'>' if depth_brace == 0 && depth_paren == 0 && depth_bracket == 0 && prev != b'-' && prev != b'=' => depth_angle += 1,
            b'<' if depth_brace == 0 && depth_paren == 0 && depth_bracket == 0 => depth_angle = (depth_angle - 1).max(0),
            b';' if depth_brace == 0 && depth_paren == 0 && depth_bracket == 0 && depth_angle == 0 => return i + 1,
            _ => {}
        }
    }
    0
}

fn find_stmt_start_in_block(mb: &[u8], floor: usize, cursor_byte: usize) -> usize {
    let mut brace = 0i32;
    let mut paren = 0i32;
    let mut bracket = 0i32;
    let mut last_boundary = floor;
    let limit = cursor_byte.min(mb.len());
    let mut i = floor;
    while i < limit {
        match mb[i] {
            b'{' => brace += 1,
            b'}' => {
                brace = (brace - 1).max(0);
                if brace == 0 && paren == 0 && bracket == 0 {
                    last_boundary = i + 1;
                }
            }
            b'(' => paren += 1,
            b')' => paren = (paren - 1).max(0),
            b'[' => bracket += 1,
            b']' => bracket = (bracket - 1).max(0),
            b';' if brace == 0 && paren == 0 && bracket == 0 => {
                last_boundary = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    last_boundary
}

pub fn scan_outer_attrs_and_docs(src: &str, masked: &str, starts: &[usize], floor: usize, limit: usize) -> (bool, usize, Vec<String>, u32, String, usize) {
    let sb = src.as_bytes();
    let mb = masked.as_bytes();
    let end = limit.min(sb.len());
    let mut i = floor.min(end);
    let mut has_doc = false;
    let mut cfg_attrs = Vec::new();
    let mut first_attr_or_doc_byte: Option<usize> = None;

    loop {
        while i < end && sb[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end {
            break;
        }
        if sb[i..end].starts_with(b"//") {
            let line_end = sb[i..end].iter().position(|&b| b == b'\n').map(|p| i + p).unwrap_or(end);
            let comment = &src[i..line_end];
            if comment.starts_with("///") && !comment.starts_with("////") {
                has_doc = true;
                first_attr_or_doc_byte.get_or_insert(i);
            }
            i = (line_end + 1).min(end);
            continue;
        }
        if sb[i..end].starts_with(b"/*") {
            let is_outer_doc = sb[i..end].starts_with(b"/**") && !sb[i..end].starts_with(b"/***");
            if is_outer_doc {
                has_doc = true;
                first_attr_or_doc_byte.get_or_insert(i);
            }
            // Masked already turned block comment into spaces; advance until non-space in `mb` or matching `*/`.
            let mut k = i + 2;
            let mut depth = 1usize;
            while k + 1 < end && depth > 0 {
                if sb[k] == b'/' && sb[k + 1] == b'*' {
                    depth += 1;
                    k += 2;
                } else if sb[k] == b'*' && sb[k + 1] == b'/' {
                    depth -= 1;
                    k += 2;
                } else {
                    k += 1;
                }
            }
            i = k.min(end);
            continue;
        }
        if sb[i] == b'#' {
            let mut k = i + 1;
            while k < end && mb[k].is_ascii_whitespace() {
                k += 1;
            }
            let is_inner = k < end && mb[k] == b'!';
            if is_inner {
                k += 1;
                while k < end && mb[k].is_ascii_whitespace() {
                    k += 1;
                }
            }
            if k < end && mb[k] == b'[' {
                if let Some(close_b) = skip_balanced(mb, k, b'[', b']') {
                    let attr_txt = src[i..close_b].trim();
                    if is_inner {
                        // Inner `#![...]` belongs to enclosing module/crate; reset outer accumulator.
                        has_doc = false;
                        cfg_attrs.clear();
                        first_attr_or_doc_byte = None;
                    } else {
                        first_attr_or_doc_byte.get_or_insert(i);
                        let inner = src[k + 1..close_b - 1].trim();
                        let is_doc_kv = inner.strip_prefix("doc").is_some_and(|r| r.trim_start().starts_with('=')) || inner.contains("doc =") || inner.contains("doc=");
                        if is_doc_kv {
                            has_doc = true;
                        }
                        if inner.starts_with("cfg(") {
                            cfg_attrs.push(attr_txt.to_string());
                        }
                    }
                    i = close_b;
                    continue;
                }
            }
        }
        break;
    }

    let kw_clamped = i.min(src.len());
    let first_attr_b = first_attr_or_doc_byte.unwrap_or(kw_clamped);
    let kw_line0 = byte_to_line0(starts, kw_clamped);
    let attr_line0 = first_attr_or_doc_byte.map(|b| byte_to_line0(starts, b)).unwrap_or(kw_line0);
    let ls = starts[kw_line0 as usize];
    let indent: String = src[ls..kw_clamped].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    (has_doc, i, cfg_attrs, attr_line0, indent, first_attr_b)
}

pub fn skip_balanced(mb: &[u8], open_idx: usize, open_ch: u8, close_ch: u8) -> Option<usize> {
    if mb.get(open_idx).copied() != Some(open_ch) {
        return None;
    }
    let mut depth = 1usize;
    let mut i = open_idx + 1;
    while i < mb.len() {
        if mb[i] == open_ch {
            depth += 1;
        } else if mb[i] == close_ch {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

pub fn contains_word(masked_slice: &str, word: &str) -> bool {
    let b = masked_slice.as_bytes();
    let wb = word.as_bytes();
    if wb.is_empty() || b.len() < wb.len() {
        return false;
    }
    for i in 0..=b.len() - wb.len() {
        if &b[i..i + wb.len()] == wb {
            let prev_ok = i == 0 || !is_ident_byte(b[i - 1]);
            let next_ok = i + wb.len() == b.len() || !is_ident_byte(b[i + wb.len()]);
            if prev_ok && next_ok {
                return true;
            }
        }
    }
    false
}
