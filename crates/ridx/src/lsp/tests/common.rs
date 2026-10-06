/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Common helpers for LSP stream testing.

use serde_json::Value;

pub(crate) fn make_lsp_msg(val: &Value) -> Vec<u8> {
    let s = val.to_string();
    format!("Content-Length: {}\r\n\r\n{}", s.len(), s).into_bytes()
}

pub(crate) fn parse_lsp_responses(bytes: &[u8]) -> Vec<Value> {
    let mut results = Vec::new();
    let mut cur = 0;
    while cur < bytes.len() {
        let slice = &bytes[cur..];
        let Some(header_end) = slice.windows(4).position(|w| w == b"\r\n\r\n") else {
            break;
        };
        let header_str = String::from_utf8_lossy(&slice[..header_end]);
        let mut content_len = 0;
        for line in header_str.lines() {
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                content_len = rest.trim().parse::<usize>().unwrap_or(0);
            }
        }
        let body_start = cur + header_end + 4;
        let body_end = body_start + content_len;
        if body_end > bytes.len() {
            break;
        }
        if let Ok(val) = serde_json::from_slice::<Value>(&bytes[body_start..body_end]) {
            results.push(val);
        }
        cur = body_end;
    }
    results
}
