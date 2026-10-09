/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Source header parsing and lexical brace scanning for enclosing scopes.

use super::types::{clean_trait_name, clean_type_name, find_keyword, EnclosingScope};

/// Parses a declaration header slice preceding `{` into an `EnclosingScope`.
pub fn parse_header(header: &str) -> EnclosingScope {
    if let Some(impl_pos) = find_keyword(header, "impl") {
        let mut text = header[impl_pos + 4..].trim_start();
        if let Some(where_pos) = find_keyword(text, "where") {
            text = text[..where_pos].trim_end();
        }
        if text.starts_with('<') {
            let mut depth: usize = 0;
            let mut close_pos = None;
            for (idx, c) in text.char_indices() {
                if c == '<' {
                    depth += 1;
                } else if c == '>' {
                    depth -= 1;
                    if depth == 0 {
                        close_pos = Some(idx);
                        break;
                    }
                }
            }
            if let Some(cp) = close_pos {
                text = text[cp + 1..].trim_start();
            }
        }

        if let Some(for_pos) = find_keyword(text, "for") {
            let trait_raw = &text[..for_pos];
            let type_raw = &text[for_pos + 3..];
            let trait_name = clean_trait_name(trait_raw);
            let type_name = clean_type_name(type_raw);
            if !type_name.is_empty() {
                return EnclosingScope::Impl {
                    trait_name: if trait_name.is_empty() {
                        None
                    } else {
                        Some(trait_name)
                    },
                    type_name,
                };
            }
        } else {
            let type_name = clean_type_name(text);
            if !type_name.is_empty() {
                return EnclosingScope::Impl {
                    trait_name: None,
                    type_name,
                };
            }
        }
    } else if let Some(trait_pos) = find_keyword(header, "trait") {
        let mut text = header[trait_pos + 5..].trim_start();
        if let Some(where_pos) = find_keyword(text, "where") {
            text = text[..where_pos].trim_end();
        }
        let (raw_name, super_slice) = if let Some(colon_pos) = text.find(':') {
            (text[..colon_pos].trim(), Some(&text[colon_pos + 1..]))
        } else {
            (text.trim(), None)
        };
        let trait_name = clean_trait_name(raw_name);
        let mut supertraits = Vec::new();
        if let Some(supers) = super_slice {
            for part in supers.split('+') {
                let st = clean_trait_name(part);
                if !st.is_empty() {
                    supertraits.push(st);
                }
            }
        }
        if !trait_name.is_empty() {
            return EnclosingScope::Trait {
                trait_name,
                supertraits,
            };
        }
    }

    EnclosingScope::Other
}

/// Scans source text forward to determine the enclosing `impl` or `trait` for the cursor position.
pub fn find_enclosing_scope(text: &str, line_1based: u32, line_slice: &str) -> Option<EnclosingScope> {
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

    let mut stack = Vec::new();
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
                        while i + 1 + match_hashes < bytes.len()
                            && bytes[i + 1 + match_hashes] == b'#'
                            && match_hashes < num_hashes
                        {
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
            let scope = parse_header(header);
            stack.push(scope);
            last_stmt_end = i + 1;
        } else if b == b'}' {
            stack.pop();
            last_stmt_end = i + 1;
        } else if b == b';' {
            last_stmt_end = i + 1;
        }
        i += 1;
    }

    stack
        .into_iter()
        .rev()
        .find(|s| !matches!(s, EnclosingScope::Other))
}
