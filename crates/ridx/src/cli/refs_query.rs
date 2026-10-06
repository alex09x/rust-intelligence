/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Implementation of the 'refs' subcommand.

use super::resolve::*;
use colored::*;
use std::time::Instant;

pub fn cmd_refs(arg1: String, arg2: Option<String>, json: bool) {
    let (ws, target) = resolve_index_and_target(arg1, arg2);
    let start = Instant::now();

    if let Some((file, line, col)) = parse_location(&target) {
        let loc_refs = ws.find_references_at(file, line, col);
        let elapsed = start.elapsed();

        if json {
            let out = match loc_refs {
                Some(lr) => serde_json::json!({
                    "target_symbol": lr.target_symbol,
                    "target_kind": format!("{:?}", lr.target_kind),
                    "target_path": lr.target_path,
                    "target_file": lr.target_resolved_path().display().to_string(),
                    "target_line": lr.target_line,
                    "target_col": lr.target_col,
                    "references_count": lr.references.len(),
                    "latency_us": elapsed.as_micros(),
                    "references": lr.references.iter().map(|r| serde_json::json!({
                        "crate": r.crate_name,
                        "file": r.resolved_path().display().to_string(),
                        "start_line": r.span.start_line(),
                        "start_col": r.span.start_col(),
                        "end_line": r.span.end_line(),
                        "end_col": r.span.end_col(),
                        "kind": format!("{:?}", r.kind),
                        "is_write": r.is_write,
                    })).collect::<Vec<_>>()
                }),
                None => serde_json::json!({
                    "target_symbol": null,
                    "references_count": 0,
                    "latency_us": elapsed.as_micros(),
                    "references": [],
                }),
            };
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
            return;
        }

        match loc_refs {
            Some(lr) => {
                println!(
                    "{} Symbol {} ({:?}) at {}:{}:{} — {} reference(s) [{:.1} µs]:",
                    "Found:".green().bold(),
                    lr.target_symbol.cyan().bold(),
                    lr.target_kind,
                    lr.target_resolved_path().display(),
                    lr.target_line,
                    lr.target_col,
                    lr.references.len(),
                    elapsed.as_micros()
                );

                for (i, r) in lr.references.iter().enumerate() {
                    let span = r.span;
                    println!(
                        "  [{}] ({}) {}:{}:{}-{}:{} (kind: {:?})",
                        i + 1,
                        r.crate_name.cyan(),
                        r.resolved_path().display(),
                        span.start_line(),
                        span.start_col(),
                        span.end_line(),
                        span.end_col(),
                        r.kind
                    );
                }
            }
            None => {
                println!(
                    "{} No references found at {} [{:.1} µs]",
                    "NotFound:".yellow().bold(),
                    target,
                    elapsed.as_micros()
                );
            }
        }
    } else {
        let refs = ws.find_references(&target);
        let elapsed = start.elapsed();

        if json {
            let out = serde_json::json!({
                "target_symbol": target,
                "references_count": refs.len(),
                "latency_us": elapsed.as_micros(),
                "references": refs.iter().map(|r| serde_json::json!({
                    "crate": r.crate_name,
                    "file": r.resolved_path().display().to_string(),
                    "start_line": r.span.start_line(),
                    "start_col": r.span.start_col(),
                    "end_line": r.span.end_line(),
                    "end_col": r.span.end_col(),
                    "kind": format!("{:?}", r.kind),
                    "is_write": r.is_write,
                })).collect::<Vec<_>>()
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
            return;
        }

        println!(
            "{} {} reference(s) to {} across workspace [{:.1} µs]:",
            "Found:".green().bold(),
            refs.len(),
            target.cyan(),
            elapsed.as_micros()
        );

        for (i, r) in refs.iter().enumerate() {
            let span = r.span;
            println!(
                "  [{}] ({}) {}:{}:{}-{}:{} (kind: {:?})",
                i + 1,
                r.crate_name.cyan(),
                r.resolved_path().display(),
                span.start_line(),
                span.start_col(),
                span.end_line(),
                span.end_col(),
                r.kind
            );
        }
    }
}
