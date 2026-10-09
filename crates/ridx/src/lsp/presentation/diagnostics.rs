/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Syntax diagnostic publishing and delimiter balancing inspection.

use super::super::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

pub use publish_syntax_diagnostics as handle_diagnostic;

pub fn publish_syntax_diagnostics(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let doc_text = if let Some(overlay) = overlay_mgr.get(&file) {
        overlay.text.clone()
    } else if let Ok(t) = std::fs::read_to_string(&file) {
        t
    } else {
        return json!({ "kind": "full", "items": [] });
    };

    handle_document_diagnostic_text(&doc_text)
}

pub fn handle_document_diagnostic_text(doc_text: &str) -> Value {
    let mut items = Vec::new();
    let mut delimiter_stack: Vec<(char, u32, u32)> = Vec::new();

    let bytes = doc_text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut line_0based = 0u32;
    let mut line_start_byte = 0;

    while i < n {
        while i < n && !doc_text.is_char_boundary(i) {
            i += 1;
        }
        if i >= n {
            break;
        }
        let b = bytes[i];

        if b == b'\n' {
            line_0based += 1;
            i += 1;
            line_start_byte = i;
            continue;
        }

        // Line comment: // ...
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment: /* ... */ with nesting
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1;
            while i + 1 < n && depth > 0 {
                if bytes[i] == b'\n' {
                    line_0based += 1;
                    line_start_byte = i + 1;
                }
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth > 0 {
                i = n;
            }
            continue;
        }

        // Raw string: r#"..."# or r"..."
        if b == b'r' && i + 1 < n && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            let mut h_idx = i + 1;
            let mut hashes = 0;
            while h_idx < n && bytes[h_idx] == b'#' {
                hashes += 1;
                h_idx += 1;
            }
            if h_idx < n && bytes[h_idx] == b'"' {
                i = h_idx + 1;
                let start_l = line_0based;
                let start_col = doc_text[line_start_byte..h_idx].encode_utf16().count() as u32;
                let mut closed = false;
                while i < n {
                    if bytes[i] == b'\n' {
                        line_0based += 1;
                        line_start_byte = i + 1;
                    }
                    if bytes[i] == b'"'
                        && i + hashes < n
                        && (0..hashes).all(|k| bytes[i + 1 + k] == b'#')
                    {
                        i += 1 + hashes;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    items.push(json!({
                        "range": {
                            "start": { "line": start_l, "character": start_col },
                            "end": { "line": start_l, "character": start_col + 1 }
                        },
                        "severity": 1,
                        "source": "ridx",
                        "message": "unclosed raw string literal"
                    }));
                }
                continue;
            }
        }

        // String literal: "..."
        if b == b'"' {
            let start_l = line_0based;
            let start_col = doc_text[line_start_byte..i].encode_utf16().count() as u32;
            i += 1;
            let mut closed = false;
            while i < n {
                if bytes[i] == b'\n' {
                    line_0based += 1;
                    line_start_byte = i + 1;
                }
                if bytes[i] == b'\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                items.push(json!({
                    "range": {
                        "start": { "line": start_l, "character": start_col },
                        "end": { "line": start_l, "character": start_col + 1 }
                    },
                    "severity": 1,
                    "source": "ridx",
                    "message": "unclosed string literal"
                }));
            }
            continue;
        }

        // Char literal: 'c' or '\n' or '\''
        if b == b'\'' {
            let mut j = i + 1;
            if j < n && bytes[j] == b'\\' {
                j += 2;
                while j < n && bytes[j] != b'\'' && bytes[j] != b'\n' {
                    j += 1;
                }
                if j < n && bytes[j] == b'\'' {
                    i = j + 1;
                    continue;
                }
            } else {
                let ch_str = &doc_text[j..];
                if let Some(c) = ch_str.chars().next() {
                    let next_pos = j + c.len_utf8();
                    if next_pos < n && bytes[next_pos] == b'\'' {
                        i = next_pos + 1;
                        continue;
                    }
                }
            }
        }

        // Code character
        if b == b'l'
            && bytes[i..].starts_with(b"let ")
            && (i == line_start_byte
                || bytes[i - 1].is_ascii_whitespace()
                || bytes[i - 1] == b';'
                || bytes[i - 1] == b'{')
        {
            if let Some(diag) =
                check_let_type_mismatch(bytes, i, doc_text, line_0based, line_start_byte)
            {
                items.push(diag);
            }
        }

        let ch = doc_text[i..].chars().next().unwrap();
        let utf16_col = doc_text[line_start_byte..i].encode_utf16().count() as u32;

        match ch {
            '(' | '[' | '{' => {
                delimiter_stack.push((ch, line_0based, utf16_col));
            }
            ')' => match delimiter_stack.pop() {
                Some(('(', _, _)) => {}
                Some((expected, el, _)) => {
                    items.push(json!({
                        "range": {
                            "start": { "line": line_0based, "character": utf16_col },
                            "end": { "line": line_0based, "character": utf16_col + 1 }
                        },
                        "severity": 1,
                        "source": "ridx",
                        "message": format!("mismatched closing delimiter ')', expected matching for '{}' at line {}", expected, el + 1)
                    }));
                }
                None => {
                    items.push(json!({
                        "range": {
                            "start": { "line": line_0based, "character": utf16_col },
                            "end": { "line": line_0based, "character": utf16_col + 1 }
                        },
                        "severity": 1,
                        "source": "ridx",
                        "message": "unmatched closing delimiter ')'"
                    }));
                }
            },
            ']' => match delimiter_stack.pop() {
                Some(('[', _, _)) => {}
                _ => {
                    items.push(json!({
                        "range": {
                            "start": { "line": line_0based, "character": utf16_col },
                            "end": { "line": line_0based, "character": utf16_col + 1 }
                        },
                        "severity": 1,
                        "source": "ridx",
                        "message": "unmatched closing delimiter ']'"
                    }));
                }
            },
            '}' => match delimiter_stack.pop() {
                Some(('{', _, _)) => {}
                _ => {
                    items.push(json!({
                        "range": {
                            "start": { "line": line_0based, "character": utf16_col },
                            "end": { "line": line_0based, "character": utf16_col + 1 }
                        },
                        "severity": 1,
                        "source": "ridx",
                        "message": "unmatched closing delimiter '}'"
                    }));
                }
            },
            _ => {}
        }

        i += ch.len_utf8();
    }

    while let Some((c, l, col)) = delimiter_stack.pop() {
        items.push(json!({
            "range": {
                "start": { "line": l, "character": col },
                "end": { "line": l, "character": col + 1 }
            },
            "severity": 1,
            "source": "ridx",
            "message": format!("unclosed delimiter '{}'", c)
        }));
    }

    json!({
        "kind": "full",
        "items": items
    })
}

fn check_let_type_mismatch(
    bytes: &[u8],
    i: usize,
    doc_text: &str,
    line_0based: u32,
    line_start_byte: usize,
) -> Option<Value> {
    let stmt_bytes = &bytes[i..];
    let semi_pos = stmt_bytes.iter().position(|&x| x == b';')?;
    let stmt_str = std::str::from_utf8(&stmt_bytes[..semi_pos]).ok()?;
    let rest = stmt_str.strip_prefix("let ")?;
    let (lhs, rhs_full) = rest.split_once('=')?;
    let (_var, ty_part) = lhs.split_once(':')?;
    let expected_ty = ty_part.trim();
    let rhs = rhs_full.trim();
    let found_ty = if rhs.starts_with('"') && rhs.ends_with('"') {
        Some("&str")
    } else if rhs == "true" || rhs == "false" {
        Some("bool")
    } else if !rhs.is_empty()
        && (rhs.chars().all(|c| c.is_ascii_digit() || c == '_')
            || (rhs.starts_with('-') && rhs[1..].chars().all(|c| c.is_ascii_digit() || c == '_')))
    {
        Some("{integer}")
    } else {
        None
    };

    let is_mismatch = matches!(
        (expected_ty, found_ty),
        (
            "u8" | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize",
            Some("&str" | "bool"),
        ) | ("&str" | "String", Some("{integer}" | "bool"))
            | ("bool", Some("&str" | "{integer}"))
    );

    if is_mismatch {
        if let Some(found) = found_ty {
            let rhs_offset = stmt_str.rfind(rhs).unwrap_or(0);
            let rhs_start = i + rhs_offset;
            let _rhs_end = rhs_start + rhs.len();
            let sc = doc_text[line_start_byte..rhs_start].encode_utf16().count() as u32;
            let ec = sc + rhs.encode_utf16().count() as u32;
            return Some(json!({
                "range": {
                    "start": { "line": line_0based, "character": sc },
                    "end": { "line": line_0based, "character": ec }
                },
                "severity": 1,
                "code": "E0308",
                "source": "ridx",
                "message": format!("mismatched types: expected `{expected_ty}`, found `{found}`")
            }));
        }
    }
    None
}

/// Reconciles heuristic in-memory diagnostics with authoritative flycheck diagnostics.
///
/// If an authoritative `rustc` diagnostic covers the same line with the same error code,
/// the heuristic `ridx` entry is suppressed to prevent duplicated error highlights in the editor.
pub fn reconcile_diagnostics(heuristic: &[Value], authoritative: &[Value]) -> Vec<Value> {
    let mut result = authoritative.to_vec();
    for h in heuristic {
        let h_line = h["range"]["start"]["line"].as_u64();
        let h_code = h.get("code").and_then(|c| c.as_str());

        let is_shadowed = authoritative.iter().any(|a| {
            let a_line = a["range"]["start"]["line"].as_u64();
            let a_code = a.get("code").and_then(|c| c.as_str());
            a_line == h_line && a_code == h_code
        });

        if !is_shadowed {
            result.push(h.clone());
        }
    }
    result
}
