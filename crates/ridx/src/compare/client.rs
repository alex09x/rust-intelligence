/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Framed JSON-RPC 2.0 client for communicating with rust-analyzer / LSP processes.

use super::harness::ExternalLspResult;
use super::system::get_process_rss_bytes;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

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
    format!(
        "file://{}",
        abs_path.canonicalize().unwrap_or(abs_path).display()
    )
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
    for _ in 0..100 {
        let msg = read_lsp_msg(r)?;
        if let Some(id) = msg.get("id").and_then(|i| i.as_i64()) {
            if id == expected_id {
                return Some(msg);
            }
        }
    }
    None
}

pub fn execute_pinned_ra_lsp(
    ra_bin: &Path,
    root: Option<&Path>,
    capability: &str,
    target: &str,
    loc_opt: Option<(&str, u32, u32)>,
) -> Option<ExternalLspResult> {
    let mut cmd = Command::new(ra_bin);
    if let Some(r) = root {
        cmd.current_dir(r);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().ok()?;
    let pid = child.id();
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout);

    let root_uri = root
        .map(|r| {
            let canon = r.canonicalize().unwrap_or_else(|_| r.to_path_buf());
            format!("file://{}", canon.display())
        })
        .unwrap_or_else(|| "file:///".to_string());

    // 1. initialize
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {}
        }
    });
    send_lsp_msg(&mut stdin, &init_req).ok()?;
    let _init_resp = read_lsp_response(&mut reader, 1)?;

    // 2. initialized
    let init_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    send_lsp_msg(&mut stdin, &init_notif).ok()?;

    // 3. query capability
    let query_req = match capability {
        "def" | "textDocument/definition" => {
            if let Some((f, l, c)) = loc_opt {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "textDocument/definition",
                    "params": {
                        "textDocument": { "uri": resolve_file_uri(f, root) },
                        "position": { "line": l.saturating_sub(1), "character": c.saturating_sub(1) }
                    }
                })
            } else {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "workspace/symbol",
                    "params": { "query": target }
                })
            }
        }
        "refs" | "textDocument/references" => {
            if let Some((f, l, c)) = loc_opt {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "textDocument/references",
                    "params": {
                        "textDocument": { "uri": resolve_file_uri(f, root) },
                        "position": { "line": l.saturating_sub(1), "character": c.saturating_sub(1) },
                        "context": { "includeDeclaration": true }
                    }
                })
            } else {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "workspace/symbol",
                    "params": { "query": target }
                })
            }
        }
        "hover" | "textDocument/hover" => {
            if let Some((f, l, c)) = loc_opt {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "textDocument/hover",
                    "params": {
                        "textDocument": { "uri": resolve_file_uri(f, root) },
                        "position": { "line": l.saturating_sub(1), "character": c.saturating_sub(1) }
                    }
                })
            } else {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "workspace/symbol",
                    "params": { "query": target }
                })
            }
        }
        "callers" | "incomingCalls" => {
            if let Some((f, l, c)) = loc_opt {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "textDocument/prepareCallHierarchy",
                    "params": {
                        "textDocument": { "uri": resolve_file_uri(f, root) },
                        "position": { "line": l.saturating_sub(1), "character": c.saturating_sub(1) }
                    }
                })
            } else {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "workspace/symbol",
                    "params": { "query": target }
                })
            }
        }
        _ => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "workspace/symbol",
            "params": { "query": target }
        }),
    };

    let t_query = Instant::now();
    send_lsp_msg(&mut stdin, &query_req).ok()?;
    let resp = read_lsp_response(&mut reader, 2)?;
    let latency = t_query.elapsed().as_micros();

    let rss = get_process_rss_bytes(pid).unwrap_or(0);

    // 4. shutdown & exit
    let shutdown_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "shutdown",
        "params": null
    });
    let _ = send_lsp_msg(&mut stdin, &shutdown_req);
    let _ = read_lsp_response(&mut reader, 3);
    let exit_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    });
    let _ = send_lsp_msg(&mut stdin, &exit_notif);
    let _ = child.kill();
    let _ = child.wait();

    if let Some(err) = resp.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("JSON-RPC error");
        return Some(ExternalLspResult {
            found: false,
            span: format!("Error ({msg})"),
            count: 0,
            latency_micros: latency,
            rss_bytes: rss,
        });
    }

    let result = resp.get("result");
    if result.is_none() || result == Some(&Value::Null) {
        return Some(ExternalLspResult {
            found: false,
            span: "Not found".to_string(),
            count: 0,
            latency_micros: latency,
            rss_bytes: rss,
        });
    }

    let val = result.unwrap();
    if let Some(arr) = val.as_array() {
        if arr.is_empty() {
            Some(ExternalLspResult {
                found: false,
                span: "Not found".to_string(),
                count: 0,
                latency_micros: latency,
                rss_bytes: rss,
            })
        } else {
            let first = &arr[0];
            let uri = first
                .get("uri")
                .or_else(|| first.get("targetUri"))
                .or_else(|| first.get("location").and_then(|l| l.get("uri")))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            let range = first
                .get("range")
                .or_else(|| first.get("targetRange"))
                .or_else(|| first.get("location").and_then(|l| l.get("range")));
            let line = range
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("line"))
                .and_then(|l| l.as_u64())
                .unwrap_or(0);
            let col = range
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("character"))
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            let span = format!(
                "{}:{}:{}",
                uri.trim_start_matches("file://"),
                line + 1,
                col + 1
            );
            Some(ExternalLspResult {
                found: true,
                span,
                count: arr.len(),
                latency_micros: latency,
                rss_bytes: rss,
            })
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
            Some(ExternalLspResult {
                found: !text.is_empty(),
                span: text,
                count: 1,
                latency_micros: latency,
                rss_bytes: rss,
            })
        } else {
            let uri = obj
                .get("uri")
                .or_else(|| obj.get("targetUri"))
                .or_else(|| obj.get("location").and_then(|l| l.get("uri")))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            let range = obj
                .get("range")
                .or_else(|| obj.get("targetRange"))
                .or_else(|| obj.get("location").and_then(|l| l.get("range")));
            let line = range
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("line"))
                .and_then(|l| l.as_u64())
                .unwrap_or(0);
            let col = range
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("character"))
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            let span = format!(
                "{}:{}:{}",
                uri.trim_start_matches("file://"),
                line + 1,
                col + 1
            );
            Some(ExternalLspResult {
                found: true,
                span,
                count: 1,
                latency_micros: latency,
                rss_bytes: rss,
            })
        }
    } else {
        Some(ExternalLspResult {
            found: false,
            span: "Not found".to_string(),
            count: 0,
            latency_micros: latency,
            rss_bytes: rss,
        })
    }
}
