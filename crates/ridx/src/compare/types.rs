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
    pub fn speedup_ratio(&self) -> f64 {
        if self.ridx_latency_micros > 0 {
            self.prodcode_latency_micros as f64 / self.ridx_latency_micros as f64
        } else {
            f64::INFINITY
        }
    }

    pub fn is_match(&self) -> bool {
        if self.ridx_found != self.prodcode_found {
            return false;
        }
        if !self.ridx_found {
            return true;
        }
        if self.prodcode_span.starts_with("Error")
            || self.prodcode_span.contains("unavailable")
            || self.ridx_span.starts_with("Error")
        {
            return false;
        }

        let r_norm = normalize_target_span(&self.ridx_span);
        let p_norm = normalize_target_span(&self.prodcode_span);

        match self.capability {
            CapabilityKind::Def => r_norm == p_norm,
            CapabilityKind::Refs | CapabilityKind::Callers => {
                (self.ridx_count == self.prodcode_count && self.ridx_count > 0)
                    || (r_norm == p_norm && !r_norm.is_empty())
            }
            _ => {
                r_norm == p_norm || (self.ridx_count > 0 && self.ridx_count == self.prodcode_count)
            }
        }
    }
}

fn normalize_target_span(span: &str) -> String {
    let s = span.trim().trim_start_matches("file://");
    if let Some((path_part, rest)) = s.split_once(':') {
        let file_name = std::path::Path::new(path_part)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(path_part);
        format!("{file_name}:{rest}")
    } else {
        s.to_string()
    }
}
