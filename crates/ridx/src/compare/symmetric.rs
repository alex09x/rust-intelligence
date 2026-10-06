/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Symmetric framed JSON-RPC comparison runner for benchmarking RIDX and rust-analyzer.

use super::client::extract_symbol_location;
use super::query_runner::{execute_scenario_query, spans_match};
use super::system::get_process_rss_bytes;
use super::transport::*;
use super::types::{
    ExternalLspResult, ServerMetrics, SymmetricComparisonSummary, SymmetricScenario,
};
use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

pub fn get_server_version(cmd: &[String]) -> String {
    let bin = if cmd.first().map(|s| s.as_str()) == Some("taskset") {
        cmd.iter()
            .skip(1)
            .find(|s| !s.starts_with('-') && Some(s.as_str()) != cmd.get(2).map(|x| x.as_str()))
    } else {
        cmd.first()
    };
    if let Some(b) = bin {
        if let Ok(out) = Command::new(b).arg("--version").output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
    }
    "unknown".to_string()
}

pub fn run_server_session(
    server_name: &str,
    cmd_args: &[String],
    root: &Path,
    scenarios: &[SymmetricScenario],
) -> Option<ServerMetrics> {
    if cmd_args.is_empty() {
        return None;
    }

    let version = get_server_version(cmd_args);

    let mut cmd = Command::new(&cmd_args[0]);
    if cmd_args.len() > 1 {
        cmd.args(&cmd_args[1..]);
    }
    cmd.current_dir(root);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().ok()?;
    let pid = child.id();
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout);

    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let root_uri = format!("file://{}", canon_root.display());

    // 1. Initialize
    let t_init_start = Instant::now();
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "rootUri": root_uri,
            "capabilities": {
                "window": { "workDoneProgress": true },
                "experimental": { "serverStatusNotification": true },
                "textDocument": {
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "definition": { "linkSupport": true },
                    "references": {},
                    "completion": {
                        "completionItem": {
                            "snippetSupport": true,
                            "labelDetailsSupport": true
                        }
                    }
                },
                "workspace": {
                    "symbol": { "symbolKind": { "valueSet": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13] } }
                }
            }
        }
    });
    send_lsp_msg(&mut stdin, &init_req).ok()?;
    let _init_resp = read_lsp_response_with_writer(&mut reader, &mut stdin, 1)?;
    let init_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    send_lsp_msg(&mut stdin, &init_notif).ok()?;
    let spawn_and_init_micros = t_init_start.elapsed().as_micros();

    // 2. Readiness / Warmup: open scenario documents and wait for server readiness
    let readiness_warmup_micros = super::warmup::wait_for_server_readiness(
        server_name,
        &mut reader,
        &mut stdin,
        root,
        scenarios,
        std::time::Duration::from_secs(120),
    );

    // 3. Run Scenarios
    let mut query_latencies_micros = HashMap::new();
    let mut query_results = HashMap::new();

    let stop_sampler = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let max_rss = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
        get_process_rss_bytes(pid).unwrap_or(0),
    ));

    let stop_clone = std::sync::Arc::clone(&stop_sampler);
    let max_clone = std::sync::Arc::clone(&max_rss);
    let sampler_handle = std::thread::spawn(move || {
        while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(rss) = get_process_rss_bytes(pid) {
                max_clone.fetch_max(rss, std::sync::atomic::Ordering::Relaxed);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });

    let mut req_id = 100i64;
    for sc in scenarios {
        req_id += 1;
        let resolved_loc: Option<(String, u32, u32)> = if let Some((f, l, c)) = &sc.location {
            Some((
                resolve_file_uri(f, Some(root)),
                l.saturating_sub(1),
                c.saturating_sub(1),
            ))
        } else if matches!(
            sc.capability.as_str(),
            "def"
                | "textDocument/definition"
                | "refs"
                | "textDocument/references"
                | "hover"
                | "textDocument/hover"
                | "callers"
                | "incomingCalls"
        ) {
            let sym_id = req_id;
            req_id += 1;
            let sym_req = serde_json::json!({
                "jsonrpc": "2.0",
                "id": sym_id,
                "method": "workspace/symbol",
                "params": { "query": sc.target }
            });
            let _ = send_lsp_msg(&mut stdin, &sym_req);
            let sym_resp = read_lsp_response_with_writer(&mut reader, &mut stdin, sym_id);
            sym_resp
                .as_ref()
                .and_then(|r| extract_symbol_location(r, &sc.target))
        } else {
            None
        };

        let t_query_start = Instant::now();
        let resp = execute_scenario_query(
            &mut reader,
            &mut stdin,
            req_id,
            &sc.capability,
            &sc.target,
            resolved_loc.as_ref(),
        );
        let query_latency = t_query_start.elapsed().as_micros();

        let lsp_res = resp
            .and_then(|r| parse_lsp_result(&r, query_latency, 0))
            .unwrap_or_else(|| ExternalLspResult {
                found: false,
                span: "Query failed or timed out".to_string(),
                count: 0,
                latency_micros: query_latency,
                rss_bytes: 0,
            });

        query_latencies_micros.insert(sc.name.clone(), query_latency);
        query_results.insert(sc.name.clone(), lsp_res);
    }

    stop_sampler.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = sampler_handle.join();
    let peak_rss_bytes = max_rss.load(std::sync::atomic::Ordering::Relaxed);

    // 4. Shutdown & Exit
    let t_shutdown_start = Instant::now();
    let shutdown_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 999,
        "method": "shutdown",
        "params": null
    });
    let _ = send_lsp_msg(&mut stdin, &shutdown_req);
    let _ = read_lsp_response_with_writer(&mut reader, &mut stdin, 999);
    let exit_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "exit",
        "params": null
    });
    let _ = send_lsp_msg(&mut stdin, &exit_notif);
    drop(stdin);

    let deadline = Instant::now() + std::time::Duration::from_millis(500);
    let mut graceful_exit = false;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            graceful_exit = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    if !graceful_exit {
        let _ = child.kill();
        let _ = child.wait();
    }
    let shutdown_micros = t_shutdown_start.elapsed().as_micros();

    Some(ServerMetrics {
        server_name: server_name.to_string(),
        server_version: version,
        spawn_and_init_micros,
        readiness_warmup_micros,
        query_latencies_micros,
        query_results,
        shutdown_micros,
        peak_rss_bytes,
    })
}

pub fn run_symmetric_comparison(
    root: &Path,
    ridx_cmd: &[String],
    ra_cmd: &[String],
    scenarios: &[SymmetricScenario],
) -> Option<SymmetricComparisonSummary> {
    let ridx_metrics = run_server_session("ridx", ridx_cmd, root, scenarios)?;
    let ra_metrics = run_server_session("rust-analyzer", ra_cmd, root, scenarios)?;

    let mut scenario_matches = HashMap::new();
    for sc in scenarios {
        let r_res = ridx_metrics.query_results.get(&sc.name);
        let ra_res = ra_metrics.query_results.get(&sc.name);
        let is_match = match (r_res, ra_res) {
            (Some(r), Some(ra)) => {
                if !r.found && !ra.found {
                    true
                } else if r.found && ra.found {
                    match sc.capability.as_str() {
                        "hover" | "textDocument/hover" => true,
                        "refs" | "textDocument/references" => r.count > 0 && ra.count > 0,
                        _ => spans_match(&r.span, &ra.span),
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        scenario_matches.insert(sc.name.clone(), is_match);
    }

    Some(SymmetricComparisonSummary {
        workspace_root: root.display().to_string(),
        ridx: ridx_metrics,
        rust_analyzer: ra_metrics,
        scenario_matches,
    })
}
