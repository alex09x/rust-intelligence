/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Implementation of the 'inspect' subcommand.

use colored::*;
use ridx_format::WorkspaceIndex;

pub fn cmd_inspect(ws: &WorkspaceIndex, json: bool) {
    let stats = ws.stats();

    if json {
        let out = serde_json::json!({
            "total_crates": stats.total_crates,
            "total_bytes": stats.total_bytes,
            "total_files": stats.total_files,
            "total_defs": stats.total_defs,
            "total_refs": stats.total_refs,
            "total_types": stats.total_types,
            "active_variant": stats.active_variant,
            "crates": stats.crate_summaries.iter().map(|c| serde_json::json!({
                "name": c.name,
                "version": c.version,
                "file_size": c.file_size,
                "defs_count": c.defs_count,
                "refs_count": c.refs_count,
                "types_count": c.types_count,
                "files_count": c.files_count,
                "path": c.path.display().to_string(),
            })).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return;
    }

    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    println!("  {} {} crate(s) loaded", "RIDX Workspace Index:".bold(), stats.total_crates.to_string().cyan());
    if let Some(ref v) = stats.active_variant {
        println!("  Active Variant        : {}", v.cyan().bold());
    }
    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    println!("{:<28}: {}", "Total Crates", stats.total_crates.to_string().bold());
    println!("{:<28}: {} bytes ({:.2} KB / {:.2} MB)", "Total Index Size", stats.total_bytes, stats.total_bytes as f64 / 1024.0, stats.total_bytes as f64 / (1024.0 * 1024.0));
    println!("{:<28}: {}", "Indexed Files", stats.total_files);
    println!("{:<28}: {}", "Definitions Count", stats.total_defs.to_string().green().bold());
    println!("{:<28}: {}", "References Count", stats.total_refs.to_string().green().bold());
    println!("{:<28}: {}", "Inferred Types Count", stats.total_types.to_string().green().bold());
    println!("{:<28}: {}", "Zero-Copy Lookup Ready", "YES (Mmap)".green().bold());
    println!("{:<28}: {}", "Daemon Memory Required", "0 MB (pure disk index)".green().bold());
    println!("{}", "───────────────────────────────────────────────────────────────────────".dimmed());
    println!("  Crate Breakdown:");
    for c in &stats.crate_summaries {
        println!(
            "  • {:<24} v{:<8} ({:>4} defs, {:>5} refs, {:>5} types, {:>6.1} KB) -> {}",
            c.name.cyan(),
            c.version,
            c.defs_count,
            c.refs_count,
            c.types_count,
            c.file_size as f64 / 1024.0,
            c.path.display()
        );
    }
    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
}
