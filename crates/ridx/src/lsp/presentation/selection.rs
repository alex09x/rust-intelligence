/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `textDocument/selectionRange`: the token under the cursor, then the enclosing blocks,
//! function bodies and items, up to the whole file.

use crate::lsp::protocol::uri_to_path;
use crate::overlay::parser::{is_rust_ident_continue, is_rust_ident_start};
use crate::overlay::{DocumentOverlay, OverlayManager};
use serde_json::{json, Value};

/// Byte index inside `line` of the character at UTF-16 column `col`, or `line.len()` when the
/// column is the end of the line. `None` past the end of the line.
fn utf16_col_to_byte(line: &str, col: u32) -> Option<usize> {
    let mut units = 0u32;
    for (byte_idx, ch) in line.char_indices() {
        if units >= col {
            return Some(byte_idx);
        }
        units += ch.len_utf16() as u32;
        if units > col {
            // Inside a surrogate pair: snap to the character that holds it.
            return Some(byte_idx);
        }
    }
    (units == col).then_some(line.len())
}

/// Number of leading bytes of `s` that continue an identifier.
fn ident_continue_len(s: &str) -> usize {
    s.chars().take_while(|&c| is_rust_ident_continue(c)).map(char::len_utf8).sum()
}

/// Length of the identifier (plain or `r#raw`) at the start of `s`, if there is one.
fn ident_len(s: &str) -> Option<usize> {
    let (prefix, rest) = match s.strip_prefix("r#") {
        Some(rest) => (2, rest),
        None => (0, s),
    };
    let first = rest.chars().next().filter(|&c| is_rust_ident_start(c))?;
    Some(prefix + first.len_utf8() + ident_continue_len(&rest[first.len_utf8()..]))
}

/// Length of the token at the start of `s`: an identifier or a lifetime `'a`.
fn token_len(s: &str) -> Option<usize> {
    match s.strip_prefix('\'') {
        Some(rest) => ident_len(rest).map(|n| n + 1),
        None => ident_len(s),
    }
}

/// The opaque span (comment or literal) that `pos` lies in: strictly after its start and at
/// most at its end, so the end boundary still selects it.
fn opaque_span_at(spans: &[(usize, usize)], pos: usize) -> Option<(usize, usize)> {
    let idx = spans.partition_point(|&(start, _)| start < pos);
    let &(start, end) = spans.get(idx.checked_sub(1)?)?;
    (start < pos && pos <= end).then_some((start, end))
}

/// End of the opaque span that starts at or before `pos` and extends past it.
fn opaque_end_after(spans: &[(usize, usize)], pos: usize) -> Option<usize> {
    let idx = spans.partition_point(|&(start, _)| start <= pos);
    let &(_, end) = spans.get(idx.checked_sub(1)?)?;
    (pos < end).then_some(end)
}

/// Byte range of the token under the cursor, if the cursor is on one.
///
/// A token is an identifier (keywords and `r#raw` included) or a lifetime; a cursor on the end
/// boundary of such a token still selects it. A cursor inside a comment or a string/char
/// literal selects that whole comment or literal, as rust-analyzer's token node does.
/// Whitespace, punctuation and numbers give `None`.
fn token_range_at(overlay: &DocumentOverlay, line_0based: u32, utf16_col_0based: u32) -> Option<(usize, usize)> {
    let line_idx = line_0based as usize;
    if line_idx >= overlay.line_starts.len() {
        return None;
    }
    let line_start = overlay.line_starts[line_idx];
    let raw_end = overlay.line_starts.get(line_idx + 1).copied().unwrap_or(overlay.text.len());
    let line = overlay.text[line_start..raw_end].trim_end_matches(['\r', '\n']);
    let target_rel = utf16_col_to_byte(line, utf16_col_0based)?;

    if let Some(span) = opaque_span_at(&overlay.opaque, line_start + target_rel) {
        return Some(span);
    }

    let mut best = None;
    let mut i = 0usize;
    while i < line.len() && i <= target_rel {
        if let Some(end) = opaque_end_after(&overlay.opaque, line_start + i) {
            i = end.saturating_sub(line_start).min(line.len());
            continue;
        }
        let Some(len) = token_len(&line[i..]) else {
            i += line[i..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let end = i + len;
        if target_rel < end {
            return Some((line_start + i, line_start + end));
        }
        if target_rel == end {
            best = Some((line_start + i, line_start + end));
        }
        i = end;
    }
    best
}

fn selection_ranges_for_overlay(overlay: &DocumentOverlay, pos_list: &[Value]) -> Value {
    let mut results = Vec::with_capacity(pos_list.len());
    for pos in pos_list {
        let line_0based = pos["line"].as_u64().unwrap_or(0) as u32;
        let utf16_col_0based = pos["character"].as_u64().unwrap_or(0) as u32;
        let byte_pos = overlay.position_to_byte_offset(line_0based, utf16_col_0based);

        let mut candidates: Vec<(usize, usize)> = Vec::with_capacity(overlay.scopes.len() * 2 + 1);
        candidates.extend(token_range_at(overlay, line_0based, utf16_col_0based));
        for s in &overlay.scopes {
            if let Some(body_start) = s.body_start_byte {
                if byte_pos >= body_start && byte_pos <= s.end_byte {
                    candidates.push((body_start, s.end_byte));
                }
            }
            if byte_pos >= s.sel_start_byte && byte_pos <= s.end_byte {
                candidates.push((s.sel_start_byte, s.end_byte));
            }
        }
        candidates.sort_by_key(|&(start, end)| (end.saturating_sub(start), std::cmp::Reverse(start)));

        // Innermost first; every further range must strictly contain the previous one.
        let mut chain: Vec<(usize, usize)> = Vec::with_capacity(candidates.len());
        for (start, end) in candidates {
            if let Some(&(prev_start, prev_end)) = chain.last() {
                let strictly_contains = start <= prev_start && end >= prev_end && (start, end) != (prev_start, prev_end);
                if !strictly_contains {
                    continue;
                }
            }
            chain.push((start, end));
        }

        let mut current_range: Option<Value> = None;
        for (start_byte, end_byte) in chain.into_iter().rev() {
            let (sl, sc) = overlay.byte_offset_to_position(start_byte);
            let (el, ec) = overlay.byte_offset_to_position(end_byte);
            current_range = Some(json!({
                "range": {
                    "start": { "line": sl, "character": sc },
                    "end": { "line": el, "character": ec }
                },
                "parent": current_range
            }));
        }

        results.push(current_range.unwrap_or_else(|| {
            json!({
                "range": {
                    "start": { "line": line_0based, "character": utf16_col_0based },
                    "end": { "line": line_0based, "character": utf16_col_0based }
                },
                "parent": Value::Null
            })
        }));
    }

    json!(results)
}

pub fn handle_selection_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);
    let Some(pos_list) = msg["params"]["positions"].as_array().filter(|p| !p.is_empty()) else {
        return json!([]);
    };

    if let Some(overlay) = overlay_mgr.get(&file) {
        return selection_ranges_for_overlay(overlay, pos_list);
    }

    let Ok(content) = std::fs::read_to_string(&file) else {
        return json!([]);
    };
    let overlay = DocumentOverlay::parse_detached(content);
    selection_ranges_for_overlay(&overlay, pos_list)
}
