/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lexical token and brace parser for building `FileScopeData`.

use super::super::generic_completion::{parse_generic_params_from_header, GenericParam};
use super::super::self_completion::{parse_header, EnclosingScope};
use super::types::{FileScopeData, ScopeBlock};
use std::sync::Arc;

/// Parses the full source text into `FileScopeData`.
pub fn parse_file_scopes(text: Arc<str>) -> FileScopeData {
    let bytes = text.as_bytes();
    let mut line_starts = vec![0];
    for (idx, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            line_starts.push(idx + 1);
        }
    }

    let mut blocks = Vec::new();
    let mut stack: Vec<(usize, EnclosingScope, Vec<GenericParam>)> = Vec::new();
    let mut last_stmt_end = 0;
    let mut i = 0;

    while i < bytes.len() {
        // Skip comments
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

        // Skip strings
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
            let params = parse_generic_params_from_header(header);
            stack.push((i, scope, params));
            last_stmt_end = i + 1;
        } else if b == b'}' {
            if let Some((brace_pos, scope, params)) = stack.pop() {
                blocks.push(ScopeBlock {
                    start_byte: brace_pos,
                    end_byte: i,
                    enclosing: scope,
                    generic_params: params,
                });
            }
            last_stmt_end = i + 1;
        } else if b == b';' {
            last_stmt_end = i + 1;
        }
        i += 1;
    }

    // Flush unclosed blocks
    while let Some((brace_pos, scope, params)) = stack.pop() {
        blocks.push(ScopeBlock {
            start_byte: brace_pos,
            end_byte: text.len(),
            enclosing: scope,
            generic_params: params,
        });
    }

    FileScopeData {
        text,
        line_starts,
        blocks,
    }
}
