/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fast lexer-level scanner for identifier occurrences in open document buffers.

use super::parser::{is_rust_ident_continue, is_rust_ident_start};

/// Scans the full document text in lexer order and returns (line_1based, char_col_1based)
/// for all identifier occurrences matching `target_name` that are real code tokens
/// (ignoring comments, block comments with nesting, string literals, raw string literals, and char literals).
pub fn find_document_matching_identifiers(text: &str, target_name: &str) -> Vec<(u32, u32)> {
    let mut matches = Vec::new();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    let mut line = 1u32;
    let mut line_start_byte = 0;

    while i < n {
        while i < n && !text.is_char_boundary(i) {
            i += 1;
        }
        if i >= n {
            break;
        }
        let b = bytes[i];

        if b == b'\n' {
            line += 1;
            i += 1;
            line_start_byte = i;
            continue;
        }

        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1;
            while i + 1 < n && depth > 0 {
                if bytes[i] == b'\n' {
                    line += 1;
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
            continue;
        }

        if b == b'r' && i + 1 < n && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            let mut h_idx = i + 1;
            let mut hashes = 0;
            while h_idx < n && bytes[h_idx] == b'#' {
                hashes += 1;
                h_idx += 1;
            }
            if h_idx < n && bytes[h_idx] == b'"' {
                i = h_idx + 1;
                while i < n {
                    if bytes[i] == b'\n' {
                        line += 1;
                        line_start_byte = i + 1;
                    }
                    if bytes[i] == b'"' && i + hashes < n && (0..hashes).all(|k| bytes[i + 1 + k] == b'#') {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }

        if b == b'"' {
            i += 1;
            let mut closed = false;
            while i < n {
                if bytes[i] == b'\n' {
                    line += 1;
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
                break;
            }
            continue;
        }

        if b == b'\'' {
            let mut j = i + 1;
            if j < n && bytes[j] == b'\\' {
                j += 1;
                while j < n && bytes[j] != b'\'' && bytes[j] != b'\n' {
                    j += 1;
                }
                if j < n && bytes[j] == b'\'' {
                    i = j + 1;
                    continue;
                }
            } else if j + 1 < n && bytes[j + 1] == b'\'' {
                i = j + 2;
                continue;
            }
        }

        let ch = text[i..].chars().next().unwrap();
        if is_rust_ident_start(ch) {
            let start_byte = i;
            let ident_line = line;
            let line_slice_before = &text[line_start_byte..start_byte];
            let char_col = line_slice_before.chars().count() as u32 + 1;

            // A raw identifier (`r#type`) is one name, as the parser records it.
            let raw = text[start_byte..].starts_with("r#") && text[start_byte + 2..].chars().next().is_some_and(is_rust_ident_start);
            let mut ident_len = if raw { 2 } else { 0 };
            for c in text[start_byte + ident_len..].chars() {
                if is_rust_ident_continue(c) {
                    ident_len += c.len_utf8();
                } else {
                    break;
                }
            }

            let ident = &text[start_byte..start_byte + ident_len];
            if ident == target_name {
                matches.push((ident_line, char_col));
            }
            i += ident_len;
            continue;
        }

        i += ch.len_utf8();
    }

    matches
}
