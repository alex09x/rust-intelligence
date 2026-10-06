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
pub use super::transport::*;
use serde_json::Value;
use std::io::BufReader;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

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
    let _init_resp = read_lsp_response_with_writer(&mut reader, &mut stdin, 1)?;

    // 2. initialized notification
    let init_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    send_lsp_msg(&mut stdin, &init_notif).ok()?;

    // 2b. Warmup / readiness check
    let warmup_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "workspace/symbol",
        "params": { "query": target }
    });
    send_lsp_msg(&mut stdin, &warmup_req).ok()?;
    let warmup_resp = read_lsp_response_with_writer(&mut reader, &mut stdin, 99)?;
    if warmup_resp.get("error").is_some() {
        return None;
    }

    // 3. Resolve target location
    let resolved_loc: Option<(String, u32, u32)> = if let Some((f, l, c)) = loc_opt {
        Some((
            resolve_file_uri(f, root),
            l.saturating_sub(1),
            c.saturating_sub(1),
        ))
    } else if matches!(
        capability,
        "def"
            | "textDocument/definition"
            | "refs"
            | "textDocument/references"
            | "hover"
            | "textDocument/hover"
            | "callers"
            | "incomingCalls"
    ) {
        let sym_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "workspace/symbol",
            "params": { "query": target }
        });
        send_lsp_msg(&mut stdin, &sym_req).ok()?;
        let sym_resp = read_lsp_response_with_writer(&mut reader, &mut stdin, 10)?;
        extract_symbol_location(&sym_resp, target)
    } else {
        None
    };

    // 4. Execute requested capability
    let (resp, latency) = match capability {
        "callers" | "incomingCalls" => {
            if let Some((uri, line, col)) = &resolved_loc {
                let t_start = Instant::now();
                let prep_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 20,
                    "method": "textDocument/prepareCallHierarchy",
                    "params": {
                        "textDocument": { "uri": uri },
                        "position": { "line": line, "character": col }
                    }
                });
                send_lsp_msg(&mut stdin, &prep_req).ok()?;
                let prep_resp = read_lsp_response_with_writer(&mut reader, &mut stdin, 20)?;
                let final_resp =
                    if let Some(items) = prep_resp.get("result").and_then(|r| r.as_array()) {
                        if let Some(first_item) = items.first() {
                            let inc_req = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 21,
                                "method": "callHierarchy/incomingCalls",
                                "params": { "item": first_item }
                            });
                            send_lsp_msg(&mut stdin, &inc_req).ok()?;
                            read_lsp_response_with_writer(&mut reader, &mut stdin, 21)?
                        } else {
                            prep_resp
                        }
                    } else {
                        prep_resp
                    };
                (final_resp, t_start.elapsed().as_micros())
            } else {
                (serde_json::json!({ "result": null }), 0)
            }
        }
        "refs" | "textDocument/references" => {
            if let Some((uri, line, col)) = &resolved_loc {
                let t_start = Instant::now();
                let ref_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 20,
                    "method": "textDocument/references",
                    "params": {
                        "textDocument": { "uri": uri },
                        "position": { "line": line, "character": col },
                        "context": { "includeDeclaration": true }
                    }
                });
                send_lsp_msg(&mut stdin, &ref_req).ok()?;
                let r = read_lsp_response_with_writer(&mut reader, &mut stdin, 20)?;
                (r, t_start.elapsed().as_micros())
            } else {
                (serde_json::json!({ "result": null }), 0)
            }
        }
        "def" | "textDocument/definition" => {
            if let Some((uri, line, col)) = &resolved_loc {
                let t_start = Instant::now();
                let def_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 20,
                    "method": "textDocument/definition",
                    "params": {
                        "textDocument": { "uri": uri },
                        "position": { "line": line, "character": col }
                    }
                });
                send_lsp_msg(&mut stdin, &def_req).ok()?;
                let r = read_lsp_response_with_writer(&mut reader, &mut stdin, 20)?;
                (r, t_start.elapsed().as_micros())
            } else {
                (serde_json::json!({ "result": null }), 0)
            }
        }
        "hover" | "textDocument/hover" => {
            if let Some((uri, line, col)) = &resolved_loc {
                let t_start = Instant::now();
                let hover_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 20,
                    "method": "textDocument/hover",
                    "params": {
                        "textDocument": { "uri": uri },
                        "position": { "line": line, "character": col }
                    }
                });
                send_lsp_msg(&mut stdin, &hover_req).ok()?;
                let r = read_lsp_response_with_writer(&mut reader, &mut stdin, 20)?;
                (r, t_start.elapsed().as_micros())
            } else {
                (serde_json::json!({ "result": null }), 0)
            }
        }
        _ => {
            let t_start = Instant::now();
            let sym_req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 20,
                "method": "workspace/symbol",
                "params": { "query": target }
            });
            send_lsp_msg(&mut stdin, &sym_req).ok()?;
            let r = read_lsp_response_with_writer(&mut reader, &mut stdin, 20)?;
            (r, t_start.elapsed().as_micros())
        }
    };

    let rss = get_process_rss_bytes(pid).unwrap_or(0);

    // 5. shutdown & exit
    let shutdown_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 30,
        "method": "shutdown",
        "params": null
    });
    let _ = send_lsp_msg(&mut stdin, &shutdown_req);
    let _ = read_lsp_response_with_writer(&mut reader, &mut stdin, 30);
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

    parse_lsp_result(&resp, latency, rss)
}

pub fn extract_symbol_location(resp: &Value, target: &str) -> Option<(String, u32, u32)> {
    let arr = resp.get("result")?.as_array()?;
    let target_name = target.rsplit("::").next().unwrap_or(target);
    let target_container = target.strip_suffix(&format!("::{target_name}"));

    let exact_match = arr.iter().find(|item| {
        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name == target {
            return true;
        }
        if name == target_name {
            if let Some(tc) = target_container {
                let container = item
                    .get("containerName")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                return container == tc || container.ends_with(tc);
            }
            return true;
        }
        false
    });

    let item = exact_match?;
    let loc = item.get("location").unwrap_or(item);
    let uri = loc.get("uri")?.as_str()?.to_string();
    let range = loc.get("range")?;
    let line = range.get("start")?.get("line")?.as_u64()? as u32;
    let col = range.get("start")?.get("character")?.as_u64()? as u32;
    Some((uri, line, col))
}

fn extract_target_span(item: &Value) -> String {
    let uri = item
        .get("uri")
        .or_else(|| item.get("targetUri"))
        .or_else(|| item.get("location").and_then(|l| l.get("uri")))
        .and_then(|u| u.as_str())
        .unwrap_or("");
    let range = item
        .get("range")
        .or_else(|| item.get("targetRange"))
        .or_else(|| item.get("location").and_then(|l| l.get("range")));
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
    format!(
        "{}:{}:{}",
        uri.trim_start_matches("file://"),
        line + 1,
        col + 1
    )
}

fn parse_lsp_result(resp: &Value, latency: u128, rss: u64) -> Option<ExternalLspResult> {
    let val = resp.get("result")?;
    if val.is_null() {
        return Some(ExternalLspResult {
            found: false,
            span: "Not found".to_string(),
            count: 0,
            latency_micros: latency,
            rss_bytes: rss,
        });
    }

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
            let item = first.get("from").unwrap_or(first);
            let span = extract_target_span(item);
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
            let span = extract_target_span(val);
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
