/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP folding range calculation for functions, blocks, and imports.

use crate::lsp::protocol::uri_to_path;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

fn opens_char_literal(chars: &[char], idx: usize) -> bool {
    match chars.get(idx + 1).copied() {
        Some('\\') => chars.get(idx + 2).is_some_and(|c| !matches!(c, '\n' | '\r')),
        Some(c) if !matches!(c, '\'' | '\n' | '\r') => chars.get(idx + 2) == Some(&'\''),
        _ => false,
    }
}

fn collect_folding_ranges(content: &str) -> Vec<Value> {
    let mut ranges = Vec::new();
    let mut delim_stack: Vec<(char, u32, u32)> = Vec::new();
    let mut in_block_comment = false;

    for (line_idx, line) in content.lines().enumerate() {
        let line_u32 = line_idx as u32;
        let mut in_string = false;
        let mut in_char = false;
        let mut escaped = false;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let ch = chars[i];
            let next = chars.get(i + 1).copied();

            if in_block_comment {
                if ch == '*' && next == Some('/') {
                    in_block_comment = false;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }

            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                i += 1;
                continue;
            }

            if in_char {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '\'' {
                    in_char = false;
                }
                i += 1;
                continue;
            }

            if ch == '/' && next == Some('/') {
                break;
            }
            if ch == '/' && next == Some('*') {
                in_block_comment = true;
                i += 2;
                continue;
            }

            if ch == '"' {
                in_string = true;
            } else if ch == '\'' && opens_char_literal(&chars, i) {
                in_char = true;
            } else if matches!(ch, '{' | '(' | '[') {
                delim_stack.push((ch, line_u32, i as u32));
            } else if matches!(ch, '}' | ')' | ']') {
                let expected = match ch {
                    '}' => '{',
                    ')' => '(',
                    _ => '[',
                };
                if let Some(pos) = delim_stack.iter().rposition(|(open, _, _)| *open == expected) {
                    let (_, start_line, start_col) = delim_stack.remove(pos);
                    delim_stack.truncate(pos);
                    if line_u32 > start_line {
                        ranges.push(json!({
                            "startLine": start_line,
                            "startCharacter": start_col,
                            "endLine": line_u32,
                            "endCharacter": i as u32,
                            "kind": "region"
                        }));
                    }
                }
            }
            i += 1;
        }
    }

    ranges
}

/// Handles `textDocument/foldingRange` requests.
pub fn handle_folding_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    if let Some(overlay) = overlay_mgr.get(&file) {
        let mut ranges = Vec::new();
        for s in &overlay.scopes {
            let (sl, sc) = overlay.byte_offset_to_position(s.start_byte);
            let (el, ec) = overlay.byte_offset_to_position(s.end_byte);
            if el > sl {
                ranges.push(json!({
                    "startLine": sl,
                    "startCharacter": sc,
                    "endLine": el,
                    "endCharacter": ec,
                    "kind": "region"
                }));
            }
        }
        return json!(ranges);
    }

    if let Ok(content) = std::fs::read_to_string(&file) {
        return json!(collect_folding_ranges(&content));
    }

    json!([])
}
