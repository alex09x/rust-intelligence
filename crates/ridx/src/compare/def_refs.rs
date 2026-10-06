/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Definition and References comparison handlers.

use super::harness::query_pinned_rust_analyzer;
use super::system::*;
use super::types::{CapabilityKind, ComparisonResult};
use crate::cli::resolve::parse_location;
use ridx_format::WorkspaceIndex;
use std::time::Instant;

pub fn compare_definition(workspace: &WorkspaceIndex, target: &str) -> ComparisonResult {
    let loc_opt = parse_location(target);

    // 1. Query local RIDX memory-mapped index
    let t_ridx = Instant::now();
    let ridx_def = if let Some((file, line, col)) = loc_opt {
        workspace.find_definition_at(file, line, col)
    } else {
        workspace.find_definition_by_path(target)
    };
    let ridx_latency = t_ridx.elapsed().as_micros();
    let ridx_rss_bytes = get_current_process_rss_bytes();

    let (ridx_found, ridx_span) = match ridx_def {
        Some(def) => (
            true,
            format!(
                "{}:{}:{}",
                def.resolved_path().display(),
                def.start_line,
                def.start_col
            ),
        ),
        None => (false, "Not found".to_string()),
    };

    // 2. Query pinned rust-analyzer
    let ext = query_pinned_rust_analyzer(workspace.workspace_root(), "def", target, loc_opt);

    ComparisonResult {
        symbol: target.to_string(),
        capability: CapabilityKind::Def,
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

pub fn compare_references(workspace: &WorkspaceIndex, target: &str) -> ComparisonResult {
    let loc_opt = parse_location(target);

    // 1. Query local RIDX memory-mapped index
    let t_ridx = Instant::now();
    let refs = if let Some((file, line, col)) = loc_opt {
        workspace
            .find_references_at(file, line, col)
            .map(|lr| lr.references)
            .unwrap_or_default()
    } else {
        workspace.find_references(target)
    };
    let ridx_latency = t_ridx.elapsed().as_micros();
    let ridx_rss_bytes = get_current_process_rss_bytes();
    let ridx_count = refs.len();
    let ridx_found = ridx_count > 0;
    let ridx_span = if ridx_found {
        format!(
            "{} refs (first: {}:{}:{})",
            ridx_count,
            refs[0].resolved_path().display(),
            refs[0].span.start_line(),
            refs[0].span.start_col()
        )
    } else {
        "0 refs".to_string()
    };

    // 2. Query pinned rust-analyzer
    let ext = query_pinned_rust_analyzer(workspace.workspace_root(), "refs", target, loc_opt);

    ComparisonResult {
        symbol: target.to_string(),
        capability: CapabilityKind::Refs,
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
