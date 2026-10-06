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
