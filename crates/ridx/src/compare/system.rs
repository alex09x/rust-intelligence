/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! System resource tracking, process memory inspection, and report formatting.

use super::types::ComparisonResult;
use colored::*;
use std::process::Command;

pub fn get_process_rss_bytes_fallback_ps(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;

    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout);
        s.trim().parse::<u64>().ok().map(|kb| kb * 1024)
    } else {
        None
    }
}

pub fn get_process_rss_bytes(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string(format!("/proc/{}/statm", pid)) {
            if let Some(pages) = statm
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<u64>().ok())
            {
                return Some(pages * 4096);
            }
        }
    }

    get_process_rss_bytes_fallback_ps(pid)
}

pub fn get_current_process_rss_bytes() -> u64 {
    get_process_rss_bytes(std::process::id()).unwrap_or(0)
}

pub fn get_rust_analyzer_rss_bytes() -> u64 {
    if let Ok(output) = Command::new("pgrep").args(["-f", "rust-analyzer"]).output() {
        let pids = String::from_utf8_lossy(&output.stdout);
        let max_ra = pids
            .lines()
            .filter_map(|p| p.trim().parse::<u32>().ok())
            .filter_map(get_process_rss_bytes)
            .max()
            .unwrap_or(0);
        if max_ra > 0 {
            return max_ra;
        }
    }
    if let Ok(output) = Command::new("pgrep")
        .args(["-f", "prod-code-server"])
        .output()
    {
        let pids = String::from_utf8_lossy(&output.stdout);
        return pids
            .lines()
            .filter_map(|p| p.trim().parse::<u32>().ok())
            .filter_map(get_process_rss_bytes)
            .max()
            .unwrap_or(0);
    }
    0
}

pub fn print_comparison(cmp: &ComparisonResult) {
    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════════════".dimmed()
    );
    println!(
        "  {} {} [{:?}]",
        "Comparison Target:".bold(),
        cmp.symbol.cyan(),
        cmp.capability
    );
    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════════════".dimmed()
    );

    println!(
        "{:<20} | {:<24} | {:<24}",
        "Metric", "RIDX (Zero-Copy Mmap)", "prod-code (rust-analyzer)"
    );
    println!("{:-<20}-+-{:-<24}-+-{:-<24}", "", "", "");

    let ridx_status = if cmp.ridx_found {
        format!("FOUND ({})", cmp.ridx_count).green()
    } else {
        "NOT FOUND".red()
    };
    let pc_status = if cmp.prodcode_found {
        format!("FOUND ({})", cmp.prodcode_count).green()
    } else {
        "NOT FOUND".yellow()
    };
    println!("{:<20} | {:<24} | {:<24}", "Status", ridx_status, pc_status);

    println!(
        "{:<20} | {:<24} | {:<24}",
        "Target Location/Span", cmp.ridx_span, cmp.prodcode_span
    );

    let speedup = if cmp.ridx_latency_micros > 0 {
        format!("{:.0}x faster", cmp.speedup_ratio())
    } else {
        "instant".to_string()
    };

    println!(
        "{:<20} | {:<24} | {:<24}",
        "Query Latency",
        format!("{} µs ({})", cmp.ridx_latency_micros, speedup)
            .green()
            .bold(),
        format!(
            "{} µs ({:.1} ms)",
            cmp.prodcode_latency_micros,
            (cmp.prodcode_latency_micros as f64) / 1000.0
        )
        .dimmed()
    );

    let ridx_rss_str = format!(
        "{:.2} MB (transient CLI)",
        (cmp.ridx_rss_bytes as f64) / 1_048_576.0
    );
    let ra_rss_str = if cmp.prodcode_rss_bytes > 0 {
        format!(
            "{:.2} MB (persistent daemon)",
            (cmp.prodcode_rss_bytes as f64) / 1_048_576.0
        )
    } else {
        "unknown (N/A)".to_string()
    };

    println!(
        "{:<20} | {:<24} | {:<24}",
        "Daemon Memory RSS",
        ridx_rss_str.green().bold(),
        ra_rss_str.yellow()
    );

    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════════════".dimmed()
    );
}

pub fn print_comparison_summary(results: &[ComparisonResult]) {
    println!(
        "{}",
        "┌──────────────────────────────────────────────────────────────────────────────────────────────┐".cyan()
    );
    println!(
        "│ {:<20} │ {:<6} │ {:<8} │ {:<14} │ {:<14} │ {:<12} │",
        "Target", "Cap", "Parity", "RIDX Latency", "RA Latency", "Speedup"
    );
    println!(
        "{}",
        "├──────────────────────┼────────┼──────────┼────────────────┼────────────────┼──────────────┤".cyan()
    );

    for r in results {
        let parity_str = if r.ridx_found && r.prodcode_found {
            "MATCH".green()
        } else if r.ridx_found && !r.prodcode_found {
            "RIDX+".cyan()
        } else if !r.ridx_found && r.prodcode_found {
            "RA+".yellow()
        } else {
            "NONE".dimmed()
        };

        let speedup_str = if r.speedup_ratio().is_infinite() {
            "∞".to_string()
        } else {
            format!("{:.1}x", r.speedup_ratio())
        };

        let target_truncated = if r.symbol.len() > 20 {
            format!("{}…", &r.symbol[..19])
        } else {
            r.symbol.clone()
        };

        println!(
            "│ {:<20} │ {:<6?} │ {:<8} │ {:<14} │ {:<14} │ {:<12} │",
            target_truncated,
            r.capability,
            parity_str,
            format!("{} µs", r.ridx_latency_micros).green(),
            format!("{:.1} ms", (r.prodcode_latency_micros as f64) / 1000.0).dimmed(),
            speedup_str.bold()
        );
    }
    println!(
        "{}",
        "└──────────────────────────────────────────────────────────────────────────────────────────────┘".cyan()
    );
}
