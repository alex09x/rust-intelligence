/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Pretty printing and summary tables for comparison benchmark results.

use super::types::ComparisonResult;
use colored::Colorize;

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

        let speedup_str = if r.ridx_latency_micros > 0 {
            format!("{:.1}x", r.speedup_ratio())
        } else {
            "∞".to_string()
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
