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

use super::system::*;
use super::types::{CapabilityKind, ComparisonResult};
use crate::cli::resolve::parse_location;
use ridx_format::WorkspaceIndex;
use std::process::Command;
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

    // 2. Query rust-analyzer / prod-code CLI
    let ws_root = workspace.workspace_root();
    let t_prodcode = Instant::now();
    let mut cmd = Command::new("prod-code");
    if let Some(root) = ws_root {
        cmd.current_dir(root);
    }
    let prodcode_output = if let Some((file, line, col)) = loc_opt {
        cmd.args(["def", file, &line.to_string(), &col.to_string()])
            .output()
    } else {
        cmd.args(["def", "--symbol", target]).output()
    };
    let prodcode_latency = t_prodcode.elapsed().as_micros();
    let prodcode_rss_bytes = get_rust_analyzer_rss_bytes();

    let (prodcode_found, prodcode_span) = match prodcode_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() || text.contains("No definition") {
                (false, "Not found".to_string())
            } else {
                (true, text.lines().next().unwrap_or(&text).to_string())
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
        capability: CapabilityKind::Def,
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

    // 2. Query rust-analyzer / prod-code CLI
    let ws_root = workspace.workspace_root();
    let t_prodcode = Instant::now();
    let mut cmd = Command::new("prod-code");
    if let Some(root) = ws_root {
        cmd.current_dir(root);
    }
    let prodcode_output = if let Some((file, line, col)) = loc_opt {
        cmd.args(["refs", file, &line.to_string(), &col.to_string()])
            .output()
    } else {
        cmd.args(["refs", "--symbol", target]).output()
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
                (true, count, format!("{} refs found", count))
            } else {
                (false, 0, "0 refs".to_string())
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
        capability: CapabilityKind::Refs,
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
