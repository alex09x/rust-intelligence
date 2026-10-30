/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Framed JSON-RPC 2.0 transport for LSP peer communication.

use serde_json::Value;
use std::io::{BufRead, Write};
use std::path::Path;

pub fn resolve_file_uri(f: &str, root: Option<&Path>) -> String {
    if f.starts_with("file://") {
        return f.to_string();
    }
    let p = Path::new(f);
    let abs_path = if p.is_relative() {
        if let Some(r) = root {
            r.join(p)
        } else {
            std::env::current_dir().unwrap_or_default().join(p)
        }
    } else {
        p.to_path_buf()
    };
    format!("file://{}", abs_path.canonicalize().unwrap_or(abs_path).display())
}

pub fn send_lsp_msg<W: Write>(w: &mut W, val: &Value) -> std::io::Result<()> {
    let payload = serde_json::to_vec(val)?;
    write!(w, "Content-Length: {}\r\n\r\n", payload.len())?;
    w.write_all(&payload)?;
    w.flush()
}

pub fn read_lsp_msg<R: BufRead>(r: &mut R) -> Option<Value> {
    let mut content_len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_len = rest.trim().parse().ok();
        }
    }
    let len = content_len?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

pub fn read_lsp_response<R: BufRead>(r: &mut R, expected_id: i64) -> Option<Value> {
    let mut sink = std::io::sink();
    read_lsp_response_with_writer(r, &mut sink, expected_id)
}

pub fn read_lsp_response_with_writer<R: BufRead, W: Write>(r: &mut R, w: &mut W, expected_id: i64) -> Option<Value> {
    for _ in 0..50_000 {
        let msg = read_lsp_msg(r)?;
        if msg.get("method").is_some() {
            if let Some(srv_id) = msg.get("id") {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": srv_id,
                    "result": null
                });
                let _ = send_lsp_msg(w, &resp);
            }
            continue;
        }
        if let Some(id) = msg.get("id").and_then(|i| i.as_i64()) {
            if id == expected_id {
                return Some(msg);
            }
        }
    }
    None
}

pub fn extract_target_span(item: &Value) -> String {
    let uri = item.get("uri").or_else(|| item.get("targetUri")).or_else(|| item.get("location").and_then(|l| l.get("uri"))).and_then(|u| u.as_str()).unwrap_or("");
    let range = item.get("range").or_else(|| item.get("targetSelectionRange")).or_else(|| item.get("targetRange")).or_else(|| item.get("location").and_then(|l| l.get("range")));
    let line = range.and_then(|r| r.get("start")).and_then(|s| s.get("line")).and_then(|l| l.as_u64()).unwrap_or(0);
    let col = range.and_then(|r| r.get("start")).and_then(|s| s.get("character")).and_then(|c| c.as_u64()).unwrap_or(0);
    format!("{}:{}:{}", uri.trim_start_matches("file://"), line + 1, col + 1)
}

pub fn parse_lsp_result(resp: &Value, latency: u128, rss: u64) -> Option<super::types::ExternalLspResult> {
    if let Some(err) = resp.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("LSP error");
        return Some(super::types::ExternalLspResult { found: false, span: format!("Error ({msg})"), count: 0, latency_micros: latency, rss_bytes: rss });
    }

    let val = resp.get("result")?;
    if val.is_null() {
        return Some(super::types::ExternalLspResult { found: false, span: "Not found".to_string(), count: 0, latency_micros: latency, rss_bytes: rss });
    }

    if let Some(arr) = val.as_array() {
        if arr.is_empty() {
            Some(super::types::ExternalLspResult { found: false, span: "Not found".to_string(), count: 0, latency_micros: latency, rss_bytes: rss })
        } else {
            let first = &arr[0];
            let item = first.get("from").unwrap_or(first);
            let span = extract_target_span(item);
            Some(super::types::ExternalLspResult { found: true, span, count: arr.len(), latency_micros: latency, rss_bytes: rss })
        }
    } else if let Some(obj) = val.as_object() {
        if let Some(contents) = obj.get("contents") {
            let text = if let Some(v) = contents.get("value").and_then(|v| v.as_str()) {
                v.to_string()
            } else if let Some(s) = contents.as_str() {
                s.to_string()
            } else {
                contents.to_string()
            };
            Some(super::types::ExternalLspResult { found: !text.is_empty(), span: text, count: 1, latency_micros: latency, rss_bytes: rss })
        } else {
            let span = extract_target_span(val);
            Some(super::types::ExternalLspResult { found: true, span, count: 1, latency_micros: latency, rss_bytes: rss })
        }
    } else {
        Some(super::types::ExternalLspResult { found: false, span: "Not found".to_string(), count: 0, latency_micros: latency, rss_bytes: rss })
    }
}
