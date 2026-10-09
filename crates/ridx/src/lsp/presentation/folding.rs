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

/// Handles `textDocument/foldingRange` requests.
pub fn handle_folding_range(msg: &Value, overlay_mgr: &OverlayManager) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    let file = uri_to_path(uri);

    let mut ranges = Vec::new();

    // 1. Overlay-based folding ranges (live buffers)
    if let Some(overlay) = overlay_mgr.get(&file) {
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

    // 2. Disk-file fallback folding ranges via brace matching (ignoring strings and comments)
    if let Ok(content) = std::fs::read_to_string(&file) {
        let mut brace_stack = Vec::new();
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
                } else if ch == '\'' {
                    in_char = true;
                } else if ch == '{' {
                    brace_stack.push((line_u32, i as u32));
                } else if ch == '}' {
                    if let Some((start_line, start_col)) = brace_stack.pop() {
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
    }

    json!(ranges)
}
