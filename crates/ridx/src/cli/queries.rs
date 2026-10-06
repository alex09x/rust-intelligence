/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Query command execution: def, refs, type_at, hover, symbols, compare.

use super::args::Commands;
use super::resolve::*;
use colored::*;
use std::time::Instant;

pub fn execute_query_command(command: super::args::Commands) {
    match command {
        Commands::Def { arg1, arg2, json } => {
            let (ws, target) = resolve_index_and_target(arg1, arg2);
            let start = Instant::now();

            let res = if let Some((file, line, col)) = parse_location(&target) {
                ws.find_definition_at(file, line, col)
            } else {
                ws.find_definition_by_path(&target)
            };
            let elapsed = start.elapsed();

            if json {
                let out = match res {
                    Some(def) => serde_json::json!({
                        "found": true,
                        "name": def.name,
                        "path": def.path,
                        "kind": format!("{:?}", def.kind),
                        "signature": def.signature,
                        "file": def.resolved_path().display().to_string(),
                        "start_line": def.start_line,
                        "start_col": def.start_col,
                        "end_line": def.end_line,
                        "end_col": def.end_col,
                        "latency_us": elapsed.as_micros(),
                    }),
                    None => serde_json::json!({
                        "found": false,
                        "target": target,
                        "latency_us": elapsed.as_micros(),
                    }),
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            match res {
                Some(def) => {
                    println!(
                        "{} {} at {}:{}:{} (kind: {:?}) [{:.1} µs]",
                        "Definition:".green().bold(),
                        def.name.bold(),
                        def.resolved_path().display(),
                        def.start_line,
                        def.start_col,
                        def.kind,
                        elapsed.as_micros()
                    );
                    println!("  Full Path: {}", def.path.cyan());
                    if let Some(sig) = def.signature {
                        println!("  Signature: {}", sig.yellow());
                    }
                }
                None => {
                    println!(
                        "{} No definition found for {} [{:.1} µs]",
                        "NotFound:".yellow().bold(),
                        target,
                        elapsed.as_micros()
                    );
                }
            }
        }

        Commands::Refs { arg1, arg2, json } => {
            super::refs_query::cmd_refs(arg1, arg2, json);
        }
        Commands::TypeAt { arg1, arg2, json } => {
            let (ws, location) = resolve_index_and_target(arg1, arg2);
            let Some((file, line, col)) = parse_location(&location) else {
                eprintln!(
                    "{}: Location must be formatted as 'file_path:line:col'",
                    "Error".red().bold()
                );
                #[cfg(not(test))]
                std::process::exit(1);
                #[cfg(test)]
                return;
            };

            let start = Instant::now();
            let ty = ws.type_at(file, line, col);
            let elapsed = start.elapsed();

            if json {
                let out = serde_json::json!({
                    "location": location,
                    "inferred_type": ty,
                    "latency_us": elapsed.as_micros(),
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            match ty {
                Some(t) => {
                    println!(
                        "{} Inferred type at {}:{}:{} [{:.1} µs]",
                        "Type:".green().bold(),
                        file,
                        line,
                        col,
                        elapsed.as_micros()
                    );
                    println!("  {}", t.cyan().bold());
                }
                None => {
                    println!(
                        "{} No type information found at {}:{}:{} [{:.1} µs]",
                        "NotFound:".yellow().bold(),
                        file,
                        line,
                        col,
                        elapsed.as_micros()
                    );
                }
            }
        }

        Commands::Hover { arg1, arg2, json } => {
            let (ws, location) = resolve_index_and_target(arg1, arg2);
            let Some((file, line, col)) = parse_location(&location) else {
                eprintln!(
                    "{}: Location must be formatted as 'file_path:line:col'",
                    "Error".red().bold()
                );
                #[cfg(not(test))]
                std::process::exit(1);
                #[cfg(test)]
                return;
            };

            let start = Instant::now();
            let hover = ws.hover_at(file, line, col);
            let elapsed = start.elapsed();

            if json {
                let out = match hover {
                    Some(h) => serde_json::json!({
                        "found": true,
                        "location": location,
                        "definition": h.definition.map(|d| serde_json::json!({
                            "name": d.name,
                            "path": d.path,
                            "kind": format!("{:?}", d.kind),
                            "signature": d.signature,
                            "file": d.resolved_path().display().to_string(),
                            "start_line": d.start_line,
                            "start_col": d.start_col,
                            "end_line": d.end_line,
                            "end_col": d.end_col,
                        })),
                        "inferred_type": h.inferred_type,
                        "latency_us": elapsed.as_micros(),
                    }),
                    None => serde_json::json!({
                        "found": false,
                        "location": location,
                        "latency_us": elapsed.as_micros(),
                    }),
                };
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            match hover {
                Some(h) => {
                    println!(
                        "{} Hover at {} [{:.1} µs]",
                        "Hover:".green().bold(),
                        location.cyan(),
                        elapsed.as_micros()
                    );
                    if let Some(ref d) = h.definition {
                        println!(
                            "  Definition : {} ({:?}) at {}:{}:{}",
                            d.name.bold(),
                            d.kind,
                            d.resolved_path().display(),
                            d.start_line,
                            d.start_col
                        );
                        println!("  Full Path  : {}", d.path.cyan());
                        if let Some(sig) = d.signature {
                            println!("  Signature  : {}", sig.yellow());
                        }
                    }
                    if let Some(ty) = h.inferred_type {
                        println!("  Inferred Ty: {}", ty.bold());
                    }
                }
                None => {
                    println!(
                        "{} No hover information at {} [{:.1} µs]",
                        "NotFound:".yellow().bold(),
                        location,
                        elapsed.as_micros()
                    );
                }
            }
        }

        Commands::Symbols { arg1, arg2, json } => {
            let (ws, query) = resolve_index_and_target(arg1, arg2);
            let start = Instant::now();
            let matches = ws.search_symbols(&query);
            let elapsed = start.elapsed();

            if json {
                let out = serde_json::json!({
                    "query": query,
                    "count": matches.len(),
                    "latency_us": elapsed.as_micros(),
                    "symbols": matches.iter().map(|s| serde_json::json!({
                        "crate": s.crate_name,
                        "name": s.name,
                        "path": s.path,
                        "kind": format!("{:?}", s.kind),
                        "file": s.resolved_path().display().to_string(),
                        "start_line": s.span.start_line(),
                        "start_col": s.span.start_col(),
                        "signature": s.signature,
                    })).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }

            println!(
                "{} {} symbol(s) matching '{}' across workspace [{:.1} µs]:",
                "Found:".green().bold(),
                matches.len(),
                query.cyan(),
                elapsed.as_micros()
            );

            for sym in matches.iter().take(50) {
                let span = sym.span;
                println!(
                    "  [{}] [{:?}] {} — {}:{}:{}",
                    sym.crate_name.dimmed(),
                    sym.kind,
                    sym.path.cyan(),
                    sym.resolved_path().display(),
                    span.start_line(),
                    span.start_col()
                );
                if let Some(sig) = sym.signature {
                    println!("       sig: {}", sig.yellow());
                }
            }
        }

        Commands::Compare {
            arg1,
            arg2,
            symbol,
            scenarios,
            ridx_cmd,
            ra_cmd,
            capability,
            json,
        } => {
            super::compare_cmd::cmd_compare(super::compare_cmd::CompareArgs {
                arg1,
                arg2,
                symbol,
                scenarios_path: scenarios,
                ridx_cmd_str: ridx_cmd,
                ra_cmd_str: ra_cmd,
                capability,
                json,
            });
        }

        _ => {}
    }
}
