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
        self.ridx_found == self.prodcode_found
    }
}
