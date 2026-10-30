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
use super::query_runner::execute_scenario_query;
use super::session_helpers::{check_results_match, get_server_version};
use super::transport::*;
use super::types::{ExternalLspResult, ServerMetrics, SymmetricComparisonSummary, SymmetricScenario};
use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

pub fn run_server_session(server_name: &str, cmd_args: &[String], root: &Path, scenarios: &[SymmetricScenario]) -> Option<ServerMetrics> {
    if cmd_args.is_empty() {
        return None;
    }

    let version = get_server_version(cmd_args);

    let mut cmd = Command::new(&cmd_args[0]);
    if cmd_args.len() > 1 {
        cmd.args(&cmd_args[1..]);
    }
    cmd.current_dir(root);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());

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
            },
            "initializationOptions": {
                "checkOnSave": false
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
    let readiness_warmup_micros = super::warmup::wait_for_server_readiness(server_name, &mut reader, &mut stdin, root, scenarios, std::time::Duration::from_secs(120));

    // 3. Run Scenarios
    let mut query_latencies_micros = HashMap::new();
    let mut query_cold_latencies_micros = HashMap::new();
    let mut query_results = HashMap::new();

    let initial_mem = super::system::get_process_memory_details(pid);
    let initial_fds = super::system::get_process_fd_count(pid);
    let initial_threads = super::system::get_process_thread_count(pid);
    let stop_sampler = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cargo_spawned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let max_rss = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(initial_mem.rss_bytes));
    let max_anon = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(initial_mem.rss_anon_bytes));
    let max_file = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(initial_mem.rss_file_bytes));
    let max_fds = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(initial_fds));
    let max_threads = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(initial_threads));
    let initial_ridx_fds = super::system::get_process_ridx_fd_count(pid);
    let initial_ridx_maps = super::system::get_process_ridx_maps_count(pid);
    let max_ridx_fds = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(initial_ridx_fds));
    let max_ridx_maps = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(initial_ridx_maps));

    let stop_clone = std::sync::Arc::clone(&stop_sampler);
    let cargo_spawned_clone = std::sync::Arc::clone(&cargo_spawned);
    let max_rss_clone = std::sync::Arc::clone(&max_rss);
    let max_anon_clone = std::sync::Arc::clone(&max_anon);
    let max_file_clone = std::sync::Arc::clone(&max_file);
    let max_fds_clone = std::sync::Arc::clone(&max_fds);
    let max_threads_clone = std::sync::Arc::clone(&max_threads);
    let max_ridx_fds_clone = std::sync::Arc::clone(&max_ridx_fds);
    let max_ridx_maps_clone = std::sync::Arc::clone(&max_ridx_maps);
    let sampler_handle = std::thread::spawn(move || {
        while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
            let mem = super::system::get_process_memory_details(pid);
            max_rss_clone.fetch_max(mem.rss_bytes, std::sync::atomic::Ordering::Relaxed);
            max_anon_clone.fetch_max(mem.rss_anon_bytes, std::sync::atomic::Ordering::Relaxed);
            max_file_clone.fetch_max(mem.rss_file_bytes, std::sync::atomic::Ordering::Relaxed);
            let fds = super::system::get_process_fd_count(pid);
            max_fds_clone.fetch_max(fds, std::sync::atomic::Ordering::Relaxed);
            let ths = super::system::get_process_thread_count(pid);
            max_threads_clone.fetch_max(ths, std::sync::atomic::Ordering::Relaxed);
            let r_fds = super::system::get_process_ridx_fd_count(pid);
            max_ridx_fds_clone.fetch_max(r_fds, std::sync::atomic::Ordering::Relaxed);
            let r_maps = super::system::get_process_ridx_maps_count(pid);
            max_ridx_maps_clone.fetch_max(r_maps, std::sync::atomic::Ordering::Relaxed);
            if super::system::has_cargo_or_rustc_children(pid) {
                cargo_spawned_clone.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    });

    let mut req_id = 100i64;
    for sc in scenarios {
        req_id += 1;
        let resolved_loc: Option<(String, u32, u32)> = if let Some((f, l, c)) = &sc.location {
            Some((resolve_file_uri(f, Some(root)), l.saturating_sub(1), c.saturating_sub(1)))
        } else if matches!(
            sc.capability.as_str(),
            "def" | "textDocument/definition" | "refs" | "textDocument/references" | "hover" | "textDocument/hover" | "callers" | "incomingCalls"
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
            sym_resp.as_ref().and_then(|r| extract_symbol_location(r, &sc.target))
        } else {
            None
        };

        let mut mutation_mgr = super::mutation::MutationManager::new();
        if let Some(mut_spec) = &sc.mutation {
            let _ = mutation_mgr.apply(&mut stdin, mut_spec, root);
        }

        let mut iteration_latencies = Vec::with_capacity(3);
        let mut last_resp = None;

        for iter in 0..3 {
            let t_query_start = Instant::now();
            let mut resp = execute_scenario_query(&mut reader, &mut stdin, req_id, &sc.capability, &sc.target, resolved_loc.as_ref());
            let is_cancelled = resp.as_ref().and_then(|r| r.get("error")).and_then(|e| e.get("code")).and_then(|c| c.as_i64()) == Some(-32801);
            if is_cancelled {
                std::thread::sleep(std::time::Duration::from_millis(25));
                req_id += 1;
                resp = execute_scenario_query(&mut reader, &mut stdin, req_id, &sc.capability, &sc.target, resolved_loc.as_ref());
            }
            let lat = t_query_start.elapsed().as_micros();
            iteration_latencies.push(lat);
            let has_err = last_resp.as_ref().is_none_or(|r: &serde_json::Value| r.get("error").is_some());
            if iter == 0 || has_err {
                last_resp = resp;
            }
            req_id += 1;
        }

        if sc.mutation.is_some() {
            let _ = mutation_mgr.revert(&mut stdin);
        }

        let cold_latency = iteration_latencies[0];
        let warm_latency = if iteration_latencies.len() > 1 {
            let mut warm_lats = iteration_latencies[1..].to_vec();
            warm_lats.sort_unstable();
            warm_lats[warm_lats.len() / 2]
        } else {
            cold_latency
        };

        let lsp_res = last_resp.and_then(|r| parse_lsp_result(&r, warm_latency, 0)).unwrap_or_else(|| ExternalLspResult {
            found: false,
            span: "Query failed or timed out".to_string(),
            count: 0,
            latency_micros: warm_latency,
            rss_bytes: 0,
        });

        query_cold_latencies_micros.insert(sc.name.clone(), cold_latency);
        query_latencies_micros.insert(sc.name.clone(), warm_latency);
        query_results.insert(sc.name.clone(), lsp_res);
    }

    stop_sampler.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = sampler_handle.join();
    let peak_rss_bytes = max_rss.load(std::sync::atomic::Ordering::Relaxed);
    let peak_rss_anon_bytes = max_anon.load(std::sync::atomic::Ordering::Relaxed);
    let peak_rss_file_bytes = max_file.load(std::sync::atomic::Ordering::Relaxed);
    let cargo_spawned_val = cargo_spawned.load(std::sync::atomic::Ordering::Relaxed);

    // 4. Shutdown & Exit
    let shutdown_micros = super::session_helpers::shutdown_lsp_server(&mut reader, stdin, child);
    let peak_open_fds = max_fds.load(std::sync::atomic::Ordering::Relaxed);
    let peak_threads = max_threads.load(std::sync::atomic::Ordering::Relaxed);
    let peak_ridx_open_fds = max_ridx_fds.load(std::sync::atomic::Ordering::Relaxed);
    let peak_ridx_mapped_regions = max_ridx_maps.load(std::sync::atomic::Ordering::Relaxed);

    Some(ServerMetrics {
        server_name: server_name.to_string(),
        server_version: version,
        spawn_and_init_micros,
        readiness_warmup_micros,
        query_latencies_micros,
        query_cold_latencies_micros,
        query_results,
        shutdown_micros,
        peak_rss_bytes,
        peak_rss_anon_bytes,
        peak_rss_file_bytes,
        cargo_spawned: cargo_spawned_val,
        peak_open_fds,
        peak_threads,
        peak_ridx_open_fds,
        peak_ridx_mapped_regions,
    })
}

pub fn run_symmetric_comparison(
    root: &Path,
    ridx_cmd: &[String],
    ra_cmd: &[String],
    scip_path: Option<&Path>,
    scenarios: &[SymmetricScenario],
) -> Option<SymmetricComparisonSummary> {
    let ridx_metrics = run_server_session("ridx", ridx_cmd, root, scenarios)?;
    let ra_metrics = run_server_session("rust-analyzer", ra_cmd, root, scenarios)?;
    let scip_metrics = scip_path.and_then(|p| super::scip_loader::run_scip_session(p, root, scenarios));

    let mut scenario_matches = HashMap::new();
    let mut scenario_matches_line_level = HashMap::new();
    let mut scip_matches = HashMap::new();
    let mut mutation_verifications = HashMap::new();
    for sc in scenarios {
        let r_res = ridx_metrics.query_results.get(&sc.name);
        let ra_res = ra_metrics.query_results.get(&sc.name);
        let s_res = scip_metrics.as_ref().and_then(|m| m.query_results.get(&sc.name));

        if let Some(mut_spec) = &sc.mutation {
            let r_corr = super::mutation::verify_mutation(r_res, mut_spec);
            let ra_corr = super::mutation::verify_mutation(ra_res, mut_spec);
            let r_lat = ridx_metrics.query_latencies_micros.get(&sc.name).copied().unwrap_or(0);
            let ra_lat = ra_metrics.query_latencies_micros.get(&sc.name).copied().unwrap_or(0);
            mutation_verifications.insert(
                sc.name.clone(),
                super::types::MutationVerification {
                    kind: mut_spec.kind.clone(),
                    ridx_correct: r_corr,
                    ra_correct: ra_corr,
                    ridx_latency_micros: r_lat,
                    ra_latency_micros: ra_lat,
                },
            );
        }

        scenario_matches.insert(sc.name.clone(), check_results_match(r_res, ra_res, &sc.capability));
        scenario_matches_line_level.insert(sc.name.clone(), super::session_helpers::check_results_match_line_level(r_res, ra_res, &sc.capability));
        if let Some(scip_r) = s_res {
            scip_matches.insert(sc.name.clone(), check_results_match(Some(scip_r), ra_res, &sc.capability));
        }
    }

    Some(SymmetricComparisonSummary {
        workspace_root: root.display().to_string(),
        ridx: ridx_metrics,
        rust_analyzer: ra_metrics,
        rust_analyzer_scip: scip_metrics,
        scenario_matches,
        scenario_matches_line_level,
        scip_matches,
        mutation_verifications,
    })
}
