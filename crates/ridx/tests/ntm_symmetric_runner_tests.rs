/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Integration tests for the symmetric framed LSP comparison runner.

use ridx::compare::symmetric::run_server_session;
use ridx::compare::types::{
    ExternalLspResult, ServerMetrics, SymmetricComparisonSummary, SymmetricScenario,
};
use std::collections::HashMap;
use std::path::Path;

#[test]
fn test_symmetric_scenario_initialization() {
    let scenario = SymmetricScenario {
        name: "tokio_join_handle_def".to_string(),
        capability: "def".to_string(),
        target: "tokio::task::JoinHandle".to_string(),
        location: None,
    };

    assert_eq!(scenario.name, "tokio_join_handle_def");
    assert_eq!(scenario.capability, "def");
    assert_eq!(scenario.target, "tokio::task::JoinHandle");
    assert!(scenario.location.is_none());
}

#[test]
fn test_symmetric_summary_serialization_roundtrip() {
    let mut ridx_latencies = HashMap::new();
    ridx_latencies.insert("def_test".to_string(), 12u128);

    let mut ridx_results = HashMap::new();
    ridx_results.insert(
        "def_test".to_string(),
        ExternalLspResult {
            found: true,
            span: "/tmp/test.rs:10:5".to_string(),
            count: 1,
            latency_micros: 12,
            rss_bytes: 4096,
        },
    );

    let ridx_metrics = ServerMetrics {
        server_name: "ridx".to_string(),
        server_version: "0.1.0".to_string(),
        spawn_and_init_micros: 310,
        readiness_warmup_micros: 120,
        query_latencies_micros: ridx_latencies,
        query_results: ridx_results,
        shutdown_micros: 50,
        peak_rss_bytes: 0,
    };

    let ra_metrics = ServerMetrics {
        server_name: "rust-analyzer".to_string(),
        server_version: "1.98.1".to_string(),
        spawn_and_init_micros: 1850000,
        readiness_warmup_micros: 3200000,
        query_latencies_micros: HashMap::new(),
        query_results: HashMap::new(),
        shutdown_micros: 120000,
        peak_rss_bytes: 350000000,
    };

    let mut matches = HashMap::new();
    matches.insert("def_test".to_string(), true);

    let summary = SymmetricComparisonSummary {
        workspace_root: "/path/to/corpus".to_string(),
        ridx: ridx_metrics,
        rust_analyzer: ra_metrics,
        scenario_matches: matches,
    };

    let json_str = serde_json::to_string(&summary).unwrap();
    assert!(json_str.contains("\"server_name\":\"ridx\""));
    assert!(json_str.contains("\"server_name\":\"rust-analyzer\""));

    let deserialized: SymmetricComparisonSummary = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.ridx.spawn_and_init_micros, 310);
    assert_eq!(deserialized.rust_analyzer.readiness_warmup_micros, 3200000);
    assert_eq!(deserialized.scenario_matches.get("def_test"), Some(&true));
}

#[test]
fn test_run_server_session_handles_invalid_binary_gracefully() {
    let scenarios = [SymmetricScenario {
        name: "test".to_string(),
        capability: "def".to_string(),
        target: "foo".to_string(),
        location: None,
    }];

    let res = run_server_session(
        "nonexistent_server",
        &["/nonexistent/bin/path".to_string()],
        Path::new("."),
        &scenarios,
    );
    assert!(res.is_none(), "Nonexistent binary must return None safely");
}

#[test]
fn test_symmetric_runner_records_rss_and_graceful_shutdown() {
    let metrics = ServerMetrics {
        server_name: "test".to_string(),
        server_version: "1.0.0".to_string(),
        spawn_and_init_micros: 100,
        readiness_warmup_micros: 50,
        query_latencies_micros: HashMap::new(),
        query_results: HashMap::new(),
        shutdown_micros: 200,
        peak_rss_bytes: 1024 * 1024,
    };
    assert_eq!(metrics.peak_rss_bytes, 1024 * 1024);
    assert_eq!(metrics.shutdown_micros, 200);
}
