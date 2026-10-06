/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Rust identifier validation, keyword detection, and declaration span resolution.

use crate::overlay::mapping::is_byte_in_string_or_comment;
use crate::overlay::parser::{is_rust_ident_continue, is_rust_ident_start};
use ridx_format::DefKind;

pub fn is_ident_start(c: char) -> bool {
    is_rust_ident_start(c)
}

pub fn is_ident_continue(c: char) -> bool {
    is_rust_ident_continue(c)
}

pub fn is_valid_rust_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_ident_start(c) => {}
        _ => return false,
    }
    for c in chars {
        if !is_ident_continue(c) {
            return false;
        }
    }
    !is_rust_keyword(s)
}

pub fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    )
}

/// Locates the 1-based character column where `target_name` begins on `line`.
pub fn find_declaration_ident_col(
    line: &str,
    target_name: &str,
    target_kind: DefKind,
) -> Option<u32> {
    let kw = match target_kind {
        DefKind::Fn => "fn",
        DefKind::Struct => "struct",
        DefKind::Enum => "enum",
        DefKind::Trait => "trait",
        DefKind::TyAlias => "type",
        DefKind::Const => "const",
        DefKind::Static => "static",
        DefKind::Mod => "mod",
        _ => "",
    };

    let kw_after_pos = if !kw.is_empty() {
        line.find(kw).map(|pos| pos + kw.len())
    } else {
        None
    };

    let mut first_match = None;
    let mut search_from = 0;
    while let Some(rel_pos) = line[search_from..].find(target_name) {
        let abs_pos = search_from + rel_pos;
        let end_pos = abs_pos + target_name.len();

        let prev_ok = if abs_pos == 0 {
            true
        } else {
            !is_ident_continue(line[..abs_pos].chars().next_back().unwrap_or(' '))
        };
        let next_ok = if end_pos >= line.len() {
            true
        } else {
            !is_ident_continue(line[end_pos..].chars().next().unwrap_or(' '))
        };

        if prev_ok && next_ok && !is_byte_in_string_or_comment(line, abs_pos) {
            let col = line[..abs_pos].chars().count() as u32 + 1;
            if let Some(after_kw) = kw_after_pos {
                if abs_pos >= after_kw {
                    return Some(col);
                }
            }
            if first_match.is_none() {
                first_match = Some(col);
            }
        }
        search_from = end_pos;
    }

    first_match
}

/// Locates (line_1based, col_1based) of `target_name` in multiline declaration text.
pub fn find_declaration_ident_in_lines(
    lines: &[&str],
    start_line_1based: u32,
    target_name: &str,
    target_kind: DefKind,
) -> Option<(u32, u32)> {
    let start_idx = start_line_1based.saturating_sub(1) as usize;
    for (offset, line) in lines.iter().skip(start_idx).take(10).enumerate() {
        let curr_line = start_line_1based + offset as u32;
        if let Some(col) = find_declaration_ident_col(line, target_name, target_kind) {
            return Some((curr_line, col));
        }
    }
    None
}

/// Extracts identifier string at 1-based character column.
pub fn extract_ident_at_col(line: &str, col_1based: u32) -> Option<&str> {
    let char_idx = col_1based.saturating_sub(1) as usize;
    let mut byte_start = None;
    for (current_char_idx, (b_idx, c)) in line.char_indices().enumerate() {
        if current_char_idx == char_idx {
            if is_ident_start(c) || is_ident_continue(c) {
                byte_start = Some(b_idx);
            }
            break;
        }
    }
    let start = byte_start?;
    let mut end = start;
    for (b_idx, c) in line[start..].char_indices() {
        if is_ident_continue(c) {
            end = start + b_idx + c.len_utf8();
        } else {
            break;
        }
    }
    if end > start {
        Some(&line[start..end])
    } else {
        None
    }
}

pub fn utf16_col_to_byte_offset(s: &str, utf16_col: usize) -> usize {
    let mut cur = 0;
    for (byte_idx, ch) in s.char_indices() {
        if cur >= utf16_col {
            return byte_idx;
        }
        cur += ch.len_utf16();
    }
    s.len()
}

pub fn byte_offset_to_utf16_col(s: &str, byte_offset: usize) -> usize {
    let mut cur = 0;
    for (byte_idx, ch) in s.char_indices() {
        if byte_idx >= byte_offset {
            break;
        }
        cur += ch.len_utf16();
    }
    cur
}

/// Applies a slice of LSP TextEdits from a WorkspaceEdit to the original document text.
/// Edits refer to the same snapshot, so they are sorted descending and applied in reverse order.
pub fn apply_workspace_text_edits_to_content(
    content: &str,
    edits: &[serde_json::Value],
) -> Option<String> {
    if edits.is_empty() {
        return Some(content.to_string());
    }

    struct ParsedEdit {
        start_byte: usize,
        end_byte: usize,
        new_text: String,
    }

    let mut line_starts = Vec::new();
    let mut current_offset = 0;
    line_starts.push(0);
    for b in content.bytes() {
        current_offset += 1;
        if b == b'\n' {
            line_starts.push(current_offset);
        }
    }

    let get_byte_offset = |line: usize, utf16_col: usize| -> usize {
        if line >= line_starts.len() {
            return content.len();
        }
        let line_start = line_starts[line];
        let next_start = if line + 1 < line_starts.len() {
            line_starts[line + 1]
        } else {
            content.len()
        };
        let line_slice = &content[line_start..next_start];
        let line_text = line_slice.trim_end_matches(['\r', '\n']);
        let col_offset = utf16_col_to_byte_offset(line_text, utf16_col);
        (line_start + col_offset).min(content.len())
    };

    let mut parsed: Vec<ParsedEdit> = Vec::with_capacity(edits.len());
    for e in edits {
        let range = e.get("range")?;
        let sl = range.get("start")?.get("line")?.as_u64()? as usize;
        let sc = range.get("start")?.get("character")?.as_u64()? as usize;
        let el = range.get("end")?.get("line")?.as_u64()? as usize;
        let ec = range.get("end")?.get("character")?.as_u64()? as usize;

        let new_text = e
            .get("newText")
            .or_else(|| e.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        let start_byte = get_byte_offset(sl, sc);
        let end_byte = get_byte_offset(el, ec);
        if start_byte > end_byte {
            return None;
        }
        parsed.push(ParsedEdit {
            start_byte,
            end_byte,
            new_text,
        });
    }

    parsed.sort_by(|a, b| {
        b.start_byte
            .cmp(&a.start_byte)
            .then_with(|| b.end_byte.cmp(&a.end_byte))
    });

    for i in 0..parsed.len().saturating_sub(1) {
        if parsed[i + 1].end_byte > parsed[i].start_byte {
            return None;
        }
    }

    let mut result = content.to_string();
    for edit in parsed {
        if edit.start_byte <= result.len() && edit.end_byte <= result.len() {
            result.replace_range(edit.start_byte..edit.end_byte, &edit.new_text);
        } else {
            return None;
        }
    }

    Some(result)
}
