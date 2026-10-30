/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Hover, Workspace Symbols, and Callers comparison handlers.

use super::harness::query_pinned_rust_analyzer;
use super::system::*;
use super::types::{CapabilityKind, ComparisonResult};
use crate::cli::resolve::parse_location;
use ridx_format::WorkspaceIndex;
use std::time::Instant;

pub fn compare_hover(workspace: &WorkspaceIndex, target: &str) -> ComparisonResult {
    let loc_opt = parse_location(target);

    // 1. Query local RIDX memory-mapped index
    let t_ridx = Instant::now();
    let (ridx_found, ridx_span) = if let Some((file, line, col)) = loc_opt {
        if let Some(hover) = workspace.hover_at(file, line, col) {
            let sig = hover.definition.as_ref().and_then(|d| d.signature).unwrap_or("");
            let ty = hover.inferred_type.unwrap_or_default();
            (true, format!("{sig} | type: {ty}"))
        } else {
            (false, "No hover".to_string())
        }
    } else if let Some(def) = workspace.find_definition_by_path(target) {
        let sig = def.signature.unwrap_or_default();
        (true, format!("{sig} (kind: {:?})", def.kind))
    } else {
        (false, "Symbol not found".to_string())
    };
    let ridx_latency = t_ridx.elapsed().as_micros();
    let ridx_rss_bytes = get_current_process_rss_bytes();

    // 2. Query pinned rust-analyzer
    let ext = query_pinned_rust_analyzer(workspace.workspace_root(), "hover", target, loc_opt);

    ComparisonResult {
        symbol: target.to_string(),
        capability: CapabilityKind::Hover,
        ridx_found,
        ridx_span,
        ridx_count: if ridx_found { 1 } else { 0 },
        ridx_latency_micros: ridx_latency,
        ridx_rss_bytes,
        prodcode_found: ext.found,
        prodcode_span: ext.span,
        prodcode_count: ext.count,
        prodcode_latency_micros: ext.latency_micros,
        prodcode_rss_bytes: ext.rss_bytes,
    }
}

pub fn compare_symbols(workspace: &WorkspaceIndex, query: &str) -> ComparisonResult {
    // 1. Query local RIDX memory-mapped index
    let t_ridx = Instant::now();
    let syms = workspace.search_symbols(query);
    let ridx_latency = t_ridx.elapsed().as_micros();
    let ridx_rss_bytes = get_current_process_rss_bytes();
    let ridx_count = syms.len();
    let ridx_found = ridx_count > 0;
    let ridx_span = if ridx_found { format!("{} symbols matching '{}'", ridx_count, query) } else { "0 symbols".to_string() };

    // 2. Query pinned rust-analyzer
    let ext = query_pinned_rust_analyzer(workspace.workspace_root(), "symbols", query, None);

    ComparisonResult {
        symbol: query.to_string(),
        capability: CapabilityKind::Symbols,
        ridx_found,
        ridx_span,
        ridx_count,
        ridx_latency_micros: ridx_latency,
        ridx_rss_bytes,
        prodcode_found: ext.found,
        prodcode_span: ext.span,
        prodcode_count: ext.count,
        prodcode_latency_micros: ext.latency_micros,
        prodcode_rss_bytes: ext.rss_bytes,
    }
}

pub fn compare_callers(workspace: &WorkspaceIndex, target: &str) -> ComparisonResult {
    let loc_opt = parse_location(target);

    // 1. Query local RIDX memory-mapped index
    let t_ridx = Instant::now();
    let refs = if let Some((file, line, col)) = loc_opt {
        workspace.find_references_at(file, line, col).map(|lr| lr.references).unwrap_or_default()
    } else {
        workspace.find_references(target)
    };
    let callers_count = refs.len();
    let ridx_latency = t_ridx.elapsed().as_micros();
    let ridx_rss_bytes = get_current_process_rss_bytes();
    let ridx_found = callers_count > 0;
    let ridx_span = format!("{} call sites", callers_count);

    // 2. Query pinned rust-analyzer
    let ext = query_pinned_rust_analyzer(workspace.workspace_root(), "callers", target, loc_opt);

    ComparisonResult {
        symbol: target.to_string(),
        capability: CapabilityKind::Callers,
        ridx_found,
        ridx_span,
        ridx_count: callers_count,
        ridx_latency_micros: ridx_latency,
        ridx_rss_bytes,
        prodcode_found: ext.found,
        prodcode_span: ext.span,
        prodcode_count: ext.count,
        prodcode_latency_micros: ext.latency_micros,
        prodcode_rss_bytes: ext.rss_bytes,
    }
}
