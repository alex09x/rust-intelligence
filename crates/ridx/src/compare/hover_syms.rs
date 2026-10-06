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

use super::system::*;
use super::types::{CapabilityKind, ComparisonResult};
use crate::cli::resolve::parse_location;
use ridx_format::WorkspaceIndex;
use std::process::Command;
use std::time::Instant;

pub fn compare_hover(workspace: &WorkspaceIndex, target: &str) -> ComparisonResult {
    let loc_opt = parse_location(target);

    // 1. Query local RIDX memory-mapped index
    let t_ridx = Instant::now();
    let (ridx_found, ridx_span) = if let Some((file, line, col)) = loc_opt {
        if let Some(hover) = workspace.hover_at(file, line, col) {
            let sig = hover
                .definition
                .as_ref()
                .and_then(|d| d.signature)
                .unwrap_or("");
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

    // 2. Query rust-analyzer / prod-code CLI
    let ws_root = workspace.workspace_root();
    let t_prodcode = Instant::now();
    let mut cmd = Command::new("prod-code");
    if let Some(root) = ws_root {
        cmd.current_dir(root);
    }
    let prodcode_output = if let Some((file, line, col)) = loc_opt {
        cmd.args(["hover", file, &line.to_string(), &col.to_string()])
            .output()
    } else {
        cmd.args(["hover", "--symbol", target]).output()
    };
    let prodcode_latency = t_prodcode.elapsed().as_micros();
    let prodcode_rss_bytes = get_rust_analyzer_rss_bytes();

    let (prodcode_found, prodcode_span) = match prodcode_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() || text.contains("No hover") {
                (false, "Not found".to_string())
            } else {
                let first_line = text
                    .lines()
                    .find(|l| !l.is_empty() && !l.starts_with("```"))
                    .unwrap_or(&text);
                (true, first_line.to_string())
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            (false, format!("Error ({err})"))
        }
        Err(e) => (false, format!("prod-code unavailable ({e})")),
    };

    ComparisonResult {
        symbol: target.to_string(),
        capability: CapabilityKind::Hover,
        ridx_found,
        ridx_span,
        ridx_count: if ridx_found { 1 } else { 0 },
        ridx_latency_micros: ridx_latency,
        ridx_rss_bytes,
        prodcode_found,
        prodcode_span,
        prodcode_count: if prodcode_found { 1 } else { 0 },
        prodcode_latency_micros: prodcode_latency,
        prodcode_rss_bytes,
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
    let ridx_span = if ridx_found {
        format!("{} symbols matching '{}'", ridx_count, query)
    } else {
        "0 symbols".to_string()
    };

    // 2. Query rust-analyzer / prod-code CLI
    let ws_root = workspace.workspace_root();
    let t_prodcode = Instant::now();
    let mut cmd = Command::new("prod-code");
    if let Some(root) = ws_root {
        cmd.current_dir(root);
    }
    let prodcode_output = cmd.args(["symbols", query]).output();
    let prodcode_latency = t_prodcode.elapsed().as_micros();
    let prodcode_rss_bytes = get_rust_analyzer_rss_bytes();

    let (prodcode_found, prodcode_count, prodcode_span) = match prodcode_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let count = text
                .lines()
                .filter(|l| l.contains('—') || l.contains('•') || l.contains('['))
                .count();
            if count > 0 {
                (true, count, format!("{} symbols found", count))
            } else {
                (false, 0, "0 symbols".to_string())
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            (false, 0, format!("Error ({err})"))
        }
        Err(e) => (false, 0, format!("prod-code unavailable ({e})")),
    };

    ComparisonResult {
        symbol: query.to_string(),
        capability: CapabilityKind::Symbols,
        ridx_found,
        ridx_span,
        ridx_count,
        ridx_latency_micros: ridx_latency,
        ridx_rss_bytes,
        prodcode_found,
        prodcode_span,
        prodcode_count,
        prodcode_latency_micros: prodcode_latency,
        prodcode_rss_bytes,
    }
}

pub fn compare_callers(workspace: &WorkspaceIndex, target: &str) -> ComparisonResult {
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
    let callers_count = refs.len();
    let ridx_latency = t_ridx.elapsed().as_micros();
    let ridx_rss_bytes = get_current_process_rss_bytes();
    let ridx_found = callers_count > 0;
    let ridx_span = format!("{} call sites", callers_count);

    // 2. Query rust-analyzer / prod-code callers
    let ws_root = workspace.workspace_root();
    let t_prodcode = Instant::now();
    let mut cmd = Command::new("prod-code");
    if let Some(root) = ws_root {
        cmd.current_dir(root);
    }
    let prodcode_output = if let Some((file, line, col)) = loc_opt {
        cmd.args(["callers", file, &line.to_string(), &col.to_string()])
            .output()
    } else {
        cmd.args(["callers", "--symbol", target]).output()
    };
    let prodcode_latency = t_prodcode.elapsed().as_micros();
    let prodcode_rss_bytes = get_rust_analyzer_rss_bytes();

    let (prodcode_found, prodcode_count, prodcode_span) = match prodcode_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let count = text
                .lines()
                .filter(|l| l.contains('•') || l.contains("file://"))
                .count();
            if count > 0 {
                (true, count, format!("{} callers found", count))
            } else {
                (false, 0, "0 callers".to_string())
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            (false, 0, format!("Error ({err})"))
        }
        Err(e) => (false, 0, format!("prod-code unavailable ({e})")),
    };

    ComparisonResult {
        symbol: target.to_string(),
        capability: CapabilityKind::Callers,
        ridx_found,
        ridx_span,
        ridx_count: callers_count,
        ridx_latency_micros: ridx_latency,
        ridx_rss_bytes,
        prodcode_found,
        prodcode_span,
        prodcode_count,
        prodcode_latency_micros: prodcode_latency,
        prodcode_rss_bytes,
    }
}
