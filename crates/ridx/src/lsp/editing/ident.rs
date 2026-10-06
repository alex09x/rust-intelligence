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
