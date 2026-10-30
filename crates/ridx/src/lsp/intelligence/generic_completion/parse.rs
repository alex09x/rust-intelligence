/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Declaration header and where-clause parsing for generic parameters and trait bounds.

use super::types::GenericParam;
use crate::lsp::intelligence::self_completion::{clean_trait_name, find_keyword};

/// Strips comments and string literals from a declaration header, replacing them with spaces.
pub fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'/' && i + 1 < len {
            if bytes[i + 1] == b'/' {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                out.push(' ');
                continue;
            } else if bytes[i + 1] == b'*' {
                i += 2;
                let mut depth = 1;
                while i + 1 < len && depth > 0 {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                out.push(' ');
                continue;
            }
        }
        if bytes[i] == b'"' {
            out.push('"');
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == b'"' {
                    out.push('"');
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Splits a string by a delimiter character at depth 0 of angle brackets `<...>` and parens `(...)`.
pub fn split_at_depth_zero(s: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth_angle: usize = 0;
    let mut depth_paren: usize = 0;
    let mut start = 0;

    for (idx, c) in s.char_indices() {
        match c {
            '<' => depth_angle += 1,
            '>' => depth_angle = depth_angle.saturating_sub(1),
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            _ if c == delimiter && depth_angle == 0 && depth_paren == 0 => {
                parts.push(&s[start..idx]);
                start = idx + c.len_utf8();
            }
            _ => {}
        }
    }
    if start <= s.len() {
        parts.push(&s[start..]);
    }
    parts
}

/// Finds the index of a single colon `:` (not `::`) at depth 0 of angle brackets and parens.
pub fn find_colon_at_depth_zero(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth_angle: usize = 0;
    let mut depth_paren: usize = 0;
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            b'<' => {
                depth_angle += 1;
                i += 1;
            }
            b'>' => {
                depth_angle = depth_angle.saturating_sub(1);
                i += 1;
            }
            b'(' => {
                depth_paren += 1;
                i += 1;
            }
            b')' => {
                depth_paren = depth_paren.saturating_sub(1);
                i += 1;
            }
            b':' if depth_angle == 0 && depth_paren == 0 => {
                if (i + 1 < len && bytes[i + 1] == b':') || (i > 0 && bytes[i - 1] == b':') {
                    i += 2;
                    continue;
                }
                return Some(i);
            }
            _ => i += 1,
        }
    }
    None
}

/// Parses a bounds string (`Trait1 + Trait2 + 'static`) into a list of clean trait names.
pub fn parse_bounds(bounds_str: &str) -> Vec<String> {
    let mut bounds = Vec::new();
    for part in split_at_depth_zero(bounds_str, '+') {
        let trimmed = part.trim();
        if trimmed.is_empty() || trimmed.starts_with('\'') || trimmed.starts_with('?') {
            continue;
        }
        let clean = clean_trait_name(trimmed);
        if !clean.is_empty() && !bounds.contains(&clean) {
            bounds.push(clean);
        }
    }
    bounds
}

/// Parses generic parameters from `<...>` and any `where` clause in a declaration header.
pub fn parse_generic_params_from_header(header: &str) -> Vec<GenericParam> {
    let clean = strip_comments(header);

    let kw_list = ["fn", "impl", "trait", "struct", "enum", "type"];
    let mut decl = None;
    for &kw in &kw_list {
        let mut search_from = 0;
        while let Some(pos) = find_keyword(&clean[search_from..], kw) {
            let actual_pos = search_from + pos;
            if decl.map(|(p, _)| actual_pos > p).unwrap_or(true) {
                decl = Some((actual_pos, kw));
            }
            search_from = actual_pos + kw.len();
        }
    }

    let Some((decl_pos, kw)) = decl else {
        return Vec::new();
    };

    let decl_slice = &clean[decl_pos + kw.len()..];
    let limit = match kw {
        "fn" => decl_slice.find('(').unwrap_or(decl_slice.len()),
        "impl" => find_keyword(decl_slice, "for").unwrap_or_else(|| decl_slice.find('{').unwrap_or(decl_slice.len())),
        "trait" => decl_slice.find(':').unwrap_or_else(|| decl_slice.find('{').unwrap_or(decl_slice.len())),
        _ => decl_slice.find('{').unwrap_or(decl_slice.len()),
    };

    let before_limit = &decl_slice[..limit];
    let mut params = Vec::new();

    if let Some(angle_start) = before_limit.find('<') {
        let bytes = decl_slice.as_bytes();
        let mut depth = 0;
        let mut angle_end = None;
        for (offset, &b) in bytes[angle_start..].iter().enumerate() {
            if b == b'<' {
                depth += 1;
            } else if b == b'>' {
                depth -= 1;
                if depth == 0 {
                    angle_end = Some(angle_start + offset);
                    break;
                }
            }
        }

        if let Some(end) = angle_end {
            let inner = &decl_slice[angle_start + 1..end];
            for chunk in split_at_depth_zero(inner, ',') {
                let trimmed = chunk.trim();
                if trimmed.is_empty() || trimmed.starts_with('\'') || trimmed.starts_with("const ") {
                    continue;
                }
                let no_default = trimmed.split('=').next().unwrap_or(trimmed).trim();
                let (name, inline_bounds) = if let Some(colon) = find_colon_at_depth_zero(no_default) {
                    (no_default[..colon].trim(), parse_bounds(&no_default[colon + 1..]))
                } else {
                    (no_default, Vec::new())
                };

                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    params.push(GenericParam::with_bounds(name, inline_bounds));
                }
            }
        }
    }

    if let Some(where_pos) = find_keyword(decl_slice, "where") {
        let where_text = &decl_slice[where_pos + 5..];
        for pred in split_at_depth_zero(where_text, ',') {
            let mut pred_trimmed = pred.trim();
            if pred_trimmed.is_empty() {
                continue;
            }
            if pred_trimmed.starts_with("for<") {
                if let Some(gt) = pred_trimmed.find('>') {
                    pred_trimmed = pred_trimmed[gt + 1..].trim();
                }
            }
            if let Some(colon) = find_colon_at_depth_zero(pred_trimmed) {
                let target = pred_trimmed[..colon].trim();
                let bounds = parse_bounds(&pred_trimmed[colon + 1..]);
                if !target.is_empty() && target.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    if let Some(p) = params.iter_mut().find(|p| p.name == target) {
                        for b in bounds {
                            if !p.bounds.contains(&b) {
                                p.bounds.push(b);
                            }
                        }
                    } else {
                        params.push(GenericParam::with_bounds(target, bounds));
                    }
                }
            }
        }
    }

    params
}
