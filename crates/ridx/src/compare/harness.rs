/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Direct LSP comparison harness for pinned rust-analyzer and RIDX.

use super::system::get_process_rss_bytes;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

pub struct ExternalLspResult {
    pub found: bool,
    pub span: String,
    pub count: usize,
    pub latency_micros: u128,
    pub rss_bytes: u64,
}

pub fn find_pinned_rust_analyzer() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RUST_ANALYZER_BIN") {
        let p = PathBuf::from(path);
        if p.exists() {
            if let Ok(out) = Command::new(&p).arg("--version").output() {
                if out.status.success() {
                    let ver = String::from_utf8_lossy(&out.stdout);
                    if ver.contains("rust-analyzer") {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

pub fn query_pinned_rust_analyzer(
    root: Option<&Path>,
    capability: &str,
    target: &str,
    loc_opt: Option<(&str, u32, u32)>,
) -> ExternalLspResult {
    let t_start = Instant::now();

    // 1. Direct pinned rust-analyzer execution if configured
    if std::env::var("RUST_ANALYZER_BIN").is_ok() {
        if let Some(ra_bin) = find_pinned_rust_analyzer() {
            if let Some(res) = execute_pinned_ra_lsp(&ra_bin, root, capability, target, loc_opt) {
                return res;
            }
        }
        return ExternalLspResult {
            found: false,
            span: "Error: invalid or failed RUST_ANALYZER_BIN".to_string(),
            count: 0,
            latency_micros: t_start.elapsed().as_micros(),
            rss_bytes: 0,
        };
    }

    // 2. Fallback to prod-code CLI wrapper
    let mut cmd = Command::new("prod-code");
    if let Some(r) = root {
        cmd.current_dir(r);
    }
    let prodcode_output = if let Some((file, line, col)) = loc_opt {
        cmd.args([capability, file, &line.to_string(), &col.to_string()])
            .output()
    } else {
        cmd.args([capability, "--symbol", target]).output()
    };
    let prodcode_latency = t_start.elapsed().as_micros();
    let prodcode_rss_bytes = super::system::get_rust_analyzer_rss_bytes();

    let (prodcode_found, prodcode_span, prodcode_count) = match prodcode_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() || text.contains("No definition") || text.contains("No references") {
                (false, "Not found".to_string(), 0)
            } else {
                let lines_count = text.lines().count();
                (
                    true,
                    text.lines().next().unwrap_or(&text).to_string(),
                    lines_count,
                )
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            (false, format!("Error ({err})"), 0)
        }
        Err(e) => (
            false,
            format!("pinned rust-analyzer / prod-code unavailable ({e})"),
            0,
        ),
    };

    ExternalLspResult {
        found: prodcode_found,
        span: prodcode_span,
        count: prodcode_count,
        latency_micros: prodcode_latency,
        rss_bytes: prodcode_rss_bytes,
    }
}

fn execute_pinned_ra_lsp(
    ra_bin: &Path,
    root: Option<&Path>,
    capability: &str,
    target: &str,
    loc_opt: Option<(&str, u32, u32)>,
) -> Option<ExternalLspResult> {
    let t_start = Instant::now();
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
        .map(|r| format!("file://{}", r.display()))
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
    let _init_resp = read_lsp_msg(&mut reader)?;

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
                        "textDocument": { "uri": if f.starts_with("file://") { f.to_string() } else { format!("file://{f}") } },
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
                        "textDocument": { "uri": if f.starts_with("file://") { f.to_string() } else { format!("file://{f}") } },
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
        _ => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "workspace/symbol",
            "params": { "query": target }
        }),
    };

    send_lsp_msg(&mut stdin, &query_req).ok()?;
    let resp = read_lsp_msg(&mut reader)?;

    let rss = get_process_rss_bytes(pid).unwrap_or(0);

    // 4. shutdown & exit
    let shutdown_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "shutdown",
        "params": null
    });
    let _ = send_lsp_msg(&mut stdin, &shutdown_req);
    let _ = read_lsp_msg(&mut reader);
    let exit_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    });
    let _ = send_lsp_msg(&mut stdin, &exit_notif);
    let _ = child.kill();
    let _ = child.wait();

    let latency = t_start.elapsed().as_micros();

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
        let uri = obj
            .get("uri")
            .or_else(|| obj.get("targetUri"))
            .and_then(|u| u.as_str())
            .unwrap_or("");
        let range = obj.get("range").or_else(|| obj.get("targetRange"));
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

fn send_lsp_msg<W: Write>(w: &mut W, val: &Value) -> std::io::Result<()> {
    let payload = serde_json::to_vec(val)?;
    write!(w, "Content-Length: {}\r\n\r\n", payload.len())?;
    w.write_all(&payload)?;
    w.flush()
}

fn read_lsp_msg<R: BufRead>(r: &mut R) -> Option<Value> {
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
