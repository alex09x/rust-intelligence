/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Type definitions for the LSP capability comparison engine.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityKind {
    Def,
    Refs,
    Hover,
    Symbols,
    Callers,
    All,
}

impl CapabilityKind {
    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "def" | "definition" => Some(Self::Def),
            "refs" | "references" => Some(Self::Refs),
            "hover" | "type" | "type_at" => Some(Self::Hover),
            "symbols" | "syms" | "sym" => Some(Self::Symbols),
            "callers" | "calls" => Some(Self::Callers),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalLspResult {
    pub found: bool,
    pub span: String,
    pub count: usize,
    pub latency_micros: u128,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioMutation {
    pub file: String,
    pub insert_lines: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymmetricScenario {
    pub name: String,
    pub capability: String,
    pub target: String,
    pub location: Option<(String, u32, u32)>,
    #[serde(default)]
    pub mutation: Option<ScenarioMutation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMetrics {
    pub server_name: String,
    pub server_version: String,
    pub spawn_and_init_micros: u128,
    pub readiness_warmup_micros: u128,
    pub query_latencies_micros: HashMap<String, u128>,
    #[serde(default)]
    pub query_cold_latencies_micros: HashMap<String, u128>,
    pub query_results: HashMap<String, ExternalLspResult>,
    pub shutdown_micros: u128,
    pub peak_rss_bytes: u64,
    #[serde(default)]
    pub peak_rss_anon_bytes: u64,
    #[serde(default)]
    pub peak_rss_file_bytes: u64,
    #[serde(default)]
    pub cargo_spawned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymmetricComparisonSummary {
    pub workspace_root: String,
    pub ridx: ServerMetrics,
    pub rust_analyzer: ServerMetrics,
    pub scenario_matches: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub symbol: String,
    pub capability: CapabilityKind,
    pub ridx_found: bool,
    pub ridx_span: String,
    pub ridx_count: usize,
    pub ridx_latency_micros: u128,
    pub ridx_rss_bytes: u64,
    pub prodcode_found: bool,
    pub prodcode_span: String,
    pub prodcode_count: usize,
    pub prodcode_latency_micros: u128,
    pub prodcode_rss_bytes: u64,
}

impl ComparisonResult {
    pub fn new(capability: CapabilityKind, symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            capability,
            ridx_found: false,
            ridx_span: String::new(),
            ridx_count: 0,
            ridx_latency_micros: 0,
            ridx_rss_bytes: 0,
            prodcode_found: false,
            prodcode_span: String::new(),
            prodcode_count: 0,
            prodcode_latency_micros: 0,
            prodcode_rss_bytes: 0,
        }
    }

    pub fn speedup_ratio(&self) -> f64 {
        if self.ridx_latency_micros > 0 {
            self.prodcode_latency_micros as f64 / self.ridx_latency_micros as f64
        } else {
            f64::INFINITY
        }
    }

    pub fn is_match(&self) -> bool {
        let p_lower = self.prodcode_span.to_lowercase();
        let r_lower = self.ridx_span.to_lowercase();
        if p_lower.starts_with("error")
            || p_lower.contains("unavailable")
            || p_lower.contains("failed")
            || r_lower.starts_with("error")
            || r_lower.contains("unavailable")
            || r_lower.contains("failed")
        {
            return false;
        }

        if self.ridx_found != self.prodcode_found {
            return false;
        }
        if !self.ridx_found {
            return true;
        }

        let r_norm = normalize_target_span(&self.ridx_span);
        let p_norm = normalize_target_span(&self.prodcode_span);

        r_norm == p_norm
    }
}

fn normalize_target_span(span: &str) -> String {
    let s = span.trim().trim_start_matches("file://");
    if let Some((path_part, rest)) = s.split_once(':') {
        let p = std::path::Path::new(path_part);
        let norm_path = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        format!("{}:{rest}", norm_path.display())
    } else {
        s.to_string()
    }
}
