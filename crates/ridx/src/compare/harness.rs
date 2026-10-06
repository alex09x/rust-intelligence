/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Direct LSP comparison harness for pinned rust-analyzer and RIDX.

use super::client::execute_pinned_ra_lsp;
pub use super::types::ExternalLspResult;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub fn find_pinned_rust_analyzer() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RUST_ANALYZER_BIN") {
        let p = PathBuf::from(path);
        if p.exists() {
            if let Ok(out) = Command::new(&p).arg("--version").output() {
                if out.status.success() {
                    let ver = String::from_utf8_lossy(&out.stdout);
                    if ver.contains("rust-analyzer") {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

pub fn query_pinned_rust_analyzer(
    root: Option<&Path>,
    capability: &str,
    target: &str,
    loc_opt: Option<(&str, u32, u32)>,
) -> ExternalLspResult {
    let t_start = Instant::now();

    // 1. Direct pinned rust-analyzer execution if configured
    if std::env::var("RUST_ANALYZER_BIN").is_ok() {
        if let Some(ra_bin) = find_pinned_rust_analyzer() {
            if let Some(res) = execute_pinned_ra_lsp(&ra_bin, root, capability, target, loc_opt) {
                return res;
            }
        }
        return ExternalLspResult {
            found: false,
            span: "Error: invalid or failed RUST_ANALYZER_BIN".to_string(),
            count: 0,
            latency_micros: t_start.elapsed().as_micros(),
            rss_bytes: 0,
        };
    }

    // 2. Fallback to prod-code CLI wrapper
    let mut cmd = Command::new("prod-code");
    if let Some(r) = root {
        cmd.current_dir(r);
    }
    let prodcode_output = if let Some((file, line, col)) = loc_opt {
        cmd.args([capability, file, &line.to_string(), &col.to_string()])
            .output()
    } else {
        cmd.args([capability, "--symbol", target]).output()
    };
    let prodcode_latency = t_start.elapsed().as_micros();
    let prodcode_rss_bytes = super::system::get_rust_analyzer_rss_bytes();

    let (prodcode_found, prodcode_span, prodcode_count) = match prodcode_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() || text.contains("No definition") || text.contains("No references") {
                (false, "Not found".to_string(), 0)
            } else {
                let lines_count = text.lines().count();
                (
                    true,
                    text.lines().next().unwrap_or(&text).to_string(),
                    lines_count,
                )
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            (false, format!("Error ({err})"), 0)
        }
        Err(e) => (
            false,
            format!("pinned rust-analyzer / prod-code unavailable ({e})"),
            0,
        ),
    };

    ExternalLspResult {
        found: prodcode_found,
        span: prodcode_span,
        count: prodcode_count,
        latency_micros: prodcode_latency,
        rss_bytes: prodcode_rss_bytes,
    }
}
