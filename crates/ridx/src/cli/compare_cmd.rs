/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Implementation of the 'compare' subcommand.

use super::resolve::*;
use crate::compare::types::SymmetricScenario;
use crate::compare::{self, run_symmetric_comparison, CapabilityKind};
use colored::*;
use std::path::{Path, PathBuf};

pub struct CompareArgs {
    pub arg1: String,
    pub arg2: Option<String>,
    pub symbol: Option<String>,
    pub scenarios_path: Option<PathBuf>,
    pub ridx_cmd_str: Option<String>,
    pub ra_cmd_str: String,
    pub capability: String,
    pub json: bool,
    pub scip_path: Option<PathBuf>,
}

pub fn cmd_compare(args: CompareArgs) {
    let CompareArgs { arg1, arg2, symbol, scenarios_path, ridx_cmd_str, ra_cmd_str, capability, json, scip_path } = args;
    if let Some(scenarios_file) = scenarios_path {
        let Ok(data) = std::fs::read_to_string(&scenarios_file) else {
            eprintln!("{}: Failed to read scenarios file: {}", "Error".red().bold(), scenarios_file.display());
            return;
        };
        let scenarios: Vec<SymmetricScenario> = match serde_json::from_str(&data) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: Failed to parse scenarios JSON: {e}", "Error".red().bold());
                return;
            }
        };

        let root = Path::new(&arg1);

        let ridx_cmd: Vec<String> = if let Some(cmd) = ridx_cmd_str {
            cmd.split_whitespace().map(|s| s.to_string()).collect()
        } else {
            let current_exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "ridx".to_string());
            vec![current_exe, "lsp".to_string()]
        };

        let ra_cmd: Vec<String> = ra_cmd_str.split_whitespace().map(|s| s.to_string()).collect();

        let summary = run_symmetric_comparison(root, &ridx_cmd, &ra_cmd, scip_path.as_deref(), &scenarios);
        match summary {
            Some(sum) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&sum).unwrap());
                } else {
                    println!("Symmetric comparison completed on {}", sum.workspace_root);
                    println!("RIDX init: {} µs, warmup: {} µs, peak RSS: {} bytes", sum.ridx.spawn_and_init_micros, sum.ridx.readiness_warmup_micros, sum.ridx.peak_rss_bytes);
                    println!(
                        "RA init: {} µs, warmup: {} µs, peak RSS: {} bytes",
                        sum.rust_analyzer.spawn_and_init_micros, sum.rust_analyzer.readiness_warmup_micros, sum.rust_analyzer.peak_rss_bytes
                    );
                    let matches_count = sum.scenario_matches.values().filter(|&&m| m).count();
                    println!("Scenario matches: {}/{}", matches_count, sum.scenario_matches.len());
                }
            }
            None => {
                eprintln!("{}: Symmetric comparison failed to run", "Error".red().bold());
            }
        }
        return;
    }

    // Default single-symbol comparison
    let a1 = arg1;
    let (ws, target) = match symbol {
        Some(s) => {
            let ws = resolve_workspace(Some(PathBuf::from(a1)));
            (ws, s)
        }
        None => resolve_index_and_target(a1, arg2),
    };

    let cap = CapabilityKind::parse_str(&capability).unwrap_or(CapabilityKind::Def);
    let results = compare::compare_capability(&ws, cap, &target);

    if json {
        let out = serde_json::json!({
            "target": target,
            "capability": format!("{:?}", cap),
            "comparisons": results.iter().map(|cmp| {
                let speedup = if cmp.ridx_latency_micros > 0 {
                    format!(
                        "{:.1}x",
                        (cmp.prodcode_latency_micros as f64) / (cmp.ridx_latency_micros as f64)
                    )
                } else {
                    "inf".to_string()
                };
                serde_json::json!({
                    "capability": format!("{:?}", cmp.capability),
                    "ridx": {
                        "found": cmp.ridx_found,
                        "span": cmp.ridx_span,
                        "count": cmp.ridx_count,
                        "latency_us": cmp.ridx_latency_micros,
                        "rss_bytes": cmp.ridx_rss_bytes,
                    },
                    "prodcode": {
                        "found": cmp.prodcode_found,
                        "span": cmp.prodcode_span,
                        "count": cmp.prodcode_count,
                        "latency_us": cmp.prodcode_latency_micros,
                        "rss_bytes": cmp.prodcode_rss_bytes,
                    },
                    "speedup": speedup,
                })
            }).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return;
    }

    if results.len() == 1 {
        compare::print_comparison(&results[0]);
    } else {
        compare::print_comparison_summary(&results);
    }
}
