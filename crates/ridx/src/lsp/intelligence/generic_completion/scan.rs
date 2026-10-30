/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lexical brace scanning to collect all in-scope generic parameters for completion.

use super::parse::parse_generic_params_from_header;
use super::types::GenericParam;
use std::collections::HashMap;

/// Scans source text forward to collect all generic parameters in scope for the cursor.
pub fn find_generic_params_in_scope(text: &str, line_1based: u32, line_slice: &str) -> Vec<GenericParam> {
    let mut current_line = 1;
    let mut line_start = 0;
    for (idx, &b) in text.as_bytes().iter().enumerate() {
        if current_line == line_1based {
            break;
        }
        if b == b'\n' {
            current_line += 1;
            line_start = idx + 1;
        }
    }
    let cursor_byte = (line_start + line_slice.len()).min(text.len());
    let bytes = &text.as_bytes()[..cursor_byte];

    let mut stack: Vec<Vec<GenericParam>> = Vec::new();
    let mut last_stmt_end = 0;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            } else if bytes[i + 1] == b'*' {
                i += 2;
                let mut depth = 1;
                while i + 1 < bytes.len() && depth > 0 {
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
                continue;
            }
        }

        if bytes[i] == b'"' {
            let mut is_raw = false;
            let mut num_hashes = 0;
            if i > 0 && bytes[i - 1] == b'r' {
                is_raw = true;
            } else if i > 1 {
                let mut h = i - 1;
                while h > 0 && bytes[h] == b'#' {
                    h -= 1;
                }
                if bytes[h] == b'r' {
                    is_raw = true;
                    num_hashes = (i - 1) - h;
                }
            }

            i += 1;
            if is_raw {
                while i < bytes.len() {
                    if bytes[i] == b'"' {
                        let mut match_hashes = 0;
                        while i + 1 + match_hashes < bytes.len() && bytes[i + 1 + match_hashes] == b'#' && match_hashes < num_hashes {
                            match_hashes += 1;
                        }
                        if match_hashes == num_hashes {
                            i += 1 + num_hashes;
                            break;
                        }
                    }
                    i += 1;
                }
            } else {
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            continue;
        }

        let b = bytes[i];
        if b == b'{' {
            let header = &text[last_stmt_end..i];
            let params = parse_generic_params_from_header(header);
            stack.push(params);
            last_stmt_end = i + 1;
        } else if b == b'}' {
            stack.pop();
            last_stmt_end = i + 1;
        } else if b == b';' {
            last_stmt_end = i + 1;
        }
        i += 1;
    }

    // Include parameters from the unclosed declaration header directly preceding the cursor
    let current_unclosed = &text[last_stmt_end..cursor_byte];
    let current_params = parse_generic_params_from_header(current_unclosed);

    let mut merged: HashMap<String, Vec<String>> = HashMap::new();
    for scope_params in stack {
        for p in scope_params {
            merged.entry(p.name).or_default().extend(p.bounds);
        }
    }
    for p in current_params {
        merged.entry(p.name).or_default().extend(p.bounds);
    }

    merged
        .into_iter()
        .map(|(name, mut bounds)| {
            bounds.dedup();
            GenericParam { name, bounds }
        })
        .collect()
}
