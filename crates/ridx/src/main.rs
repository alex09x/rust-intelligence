/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use clap::{Parser, Subcommand};
use colored::*;
use ridx_format::WorkspaceIndex;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

mod compare;
mod indexer;
mod lsp;
mod overlay;

use indexer::*;

#[derive(Parser)]
#[command(name = "ridx")]
#[command(about = "Rust Intelligence: Ultra-fast, zero-memory code intelligence CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index a Cargo crate or workspace using ridx-driver and cargo check
    Index {
        /// Path to Cargo workspace or crate directory (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output directory for .ridx index files (default: target/ridx)
        #[arg(long, short)]
        out_dir: Option<PathBuf>,

        /// Optional package filter (e.g. -p my_crate)
        #[arg(long, short)]
        package: Option<String>,

        /// Run cargo check in release mode
        #[arg(long)]
        release: bool,

        /// Space or comma separated list of features to activate
        #[arg(long)]
        features: Option<String>,

        /// Do not activate the `default` feature
        #[arg(long)]
        no_default_features: bool,

        /// Activate all available features
        #[arg(long)]
        all_features: bool,

        /// Build for the target triple
        #[arg(long)]
        target: Option<String>,
    },

    /// Inspect binary header, record counts, and memory layout of a .ridx index file or workspace
    Inspect {
        /// Path to .ridx index file or directory (default: auto-discover target/ridx)
        index_path: Option<PathBuf>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Go to definition for a symbol path or cursor location (file:line:col)
    Def {
        /// First argument: index file/dir, or target symbol/location if index is omitted
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Find all references to a given symbol path or cursor location (file:line:col)
    Refs {
        /// First argument: index file/dir, or target symbol/location if index is omitted
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show inferred expression type at a cursor location (file:line:col)
    TypeAt {
        /// First argument: index file/dir, or location if index is omitted
        arg1: String,
        /// Second argument: location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show combined hover information (signature + inferred type) at cursor location
    Hover {
        /// First argument: index file/dir, or location if index is omitted
        arg1: String,
        /// Second argument: location if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search symbol definitions matching a query string across the workspace
    Symbols {
        /// First argument: index file/dir, or query if index is omitted
        arg1: String,
        /// Second argument: query if first argument was index path
        arg2: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Compare accuracy and latency against prod-code (rust-analyzer)
    Compare {
        /// First argument: index file/dir, or target symbol/location
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,
        /// Optional explicit symbol flag
        #[arg(long, short)]
        symbol: Option<String>,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run as a Language Server Protocol (LSP) server over stdio
    Lsp {
        /// Path to .ridx index file or workspace directory (default: auto-discover)
        #[arg(long, short)]
        index_path: Option<PathBuf>,
    },
}

fn parse_location(loc: &str) -> Option<(&str, u32, u32)> {
    let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
    if parts.len() == 3 {
        let col: u32 = parts[0].parse().ok()?;
        let line: u32 = parts[1].parse().ok()?;
        let file = parts[2];
        Some((file, line, col))
    } else {
        None
    }
}

fn find_enclosing_workspace(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(dir) = cur {
        if dir.join("Cargo.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

fn find_manifest_for_path(p: &Path) -> Option<(PathBuf, ridx_format::VariantManifest)> {
    let mut candidate_dirs = Vec::new();
    if p.is_dir() {
        candidate_dirs.push(p.to_path_buf());
        candidate_dirs.push(p.join("target/ridx"));
        let mut cur = p.parent();
        while let Some(parent) = cur {
            candidate_dirs.push(parent.to_path_buf());
            cur = parent.parent();
        }
    } else if let Some(parent) = p.parent() {
        candidate_dirs.push(parent.to_path_buf());
        let mut cur = parent.parent();
        while let Some(grandparent) = cur {
            candidate_dirs.push(grandparent.to_path_buf());
            cur = grandparent.parent();
        }
    }

    // 1. Check if a shard-specific variant_manifest.json exists in candidate dirs (innermost first)
    for dir in &candidate_dirs {
        let v_manifest_file = dir.join("variant_manifest.json");
        if v_manifest_file.is_file() {
            if let Ok(content) = fs::read(&v_manifest_file) {
                if let Ok(manifest) =
                    serde_json::from_slice::<ridx_format::VariantManifest>(&content)
                {
                    return Some((dir.clone(), manifest));
                }
            }
        }
    }

    // 2. Check root ridx_manifest.json
    for dir in candidate_dirs {
        let manifest_file = dir.join("ridx_manifest.json");
        if manifest_file.is_file() {
            if let Ok(content) = fs::read(&manifest_file) {
                if let Ok(manifest) =
                    serde_json::from_slice::<ridx_format::VariantManifest>(&content)
                {
                    // If p specifies a variant directory or shard under variants/<id>, verify that
                    // it matches manifest.active_variant_id. If p targets an inactive variant shard
                    // whose recipe is not the root active manifest, return None rather than substituting
                    // an unrelated variant's recipe.
                    let p_str = p.to_string_lossy();
                    if let Some(pos) = p_str.find("variants/") {
                        let sub = &p_str[pos + "variants/".len()..];
                        let var_id = sub.split('/').next().unwrap_or("");
                        if !var_id.is_empty()
                            && !manifest.active_variant_id.is_empty()
                            && var_id != manifest.active_variant_id
                        {
                            return None;
                        }
                    }
                    return Some((dir, manifest));
                }
            }
        }
    }
    None
}

fn try_resolve_workspace(path_opt: Option<PathBuf>) -> Result<WorkspaceIndex, String> {
    match path_opt {
        Some(p) => match WorkspaceIndex::open(&p) {
            Ok(ws) => Ok(ws),
            Err(e) => {
                if ridx_format::is_version_mismatch(&e) || e.kind() == std::io::ErrorKind::NotFound
                {
                    eprintln!(
                        "{} Index at {:?} is missing or incompatible with format v{}. Automatically reindexing...",
                        "Notice:".yellow().bold(), p, ridx_format::CURRENT_FORMAT_VERSION
                    );
                    let (
                        work_dir,
                        out_dir_opt,
                        package,
                        release,
                        features,
                        no_default_features,
                        all_features,
                        target,
                    ) = if let Some((manifest_dir, manifest)) = find_manifest_for_path(&p) {
                        let ws_dir = if !manifest.workspace_root.is_empty()
                            && Path::new(&manifest.workspace_root).exists()
                        {
                            PathBuf::from(manifest.workspace_root)
                        } else if let Some(ws) = find_enclosing_workspace(&p) {
                            ws
                        } else {
                            return Err(format!(
                                "Cannot find Cargo workspace for index at {:?}.",
                                p
                            ));
                        };
                        let base_dir = if let Some(ref b) = manifest.base_out_dir {
                            PathBuf::from(b)
                        } else if manifest_dir
                            .parent()
                            .and_then(|pr| pr.file_name())
                            .and_then(|s| s.to_str())
                            == Some("variants")
                        {
                            manifest_dir
                                .parent()
                                .unwrap()
                                .parent()
                                .unwrap()
                                .to_path_buf()
                        } else if manifest_dir.file_name().and_then(|s| s.to_str())
                            == Some("variants")
                        {
                            manifest_dir.parent().unwrap().to_path_buf()
                        } else {
                            manifest_dir
                        };
                        (
                            ws_dir,
                            Some(base_dir),
                            manifest.package_filter,
                            manifest.profile == "release",
                            manifest.features,
                            manifest.no_default_features,
                            manifest.all_features,
                            manifest.target,
                        )
                    } else {
                        let p_str = p.to_string_lossy();
                        if p_str.contains("variants/") {
                            return Err(format!(
                                "Cannot automatically reindex inactive variant shard at {:?}: no matching variant manifest found.\nRun 'ridx index' with the appropriate build configuration for this variant.",
                                p
                            ));
                        }
                        if let Some(ws) = find_enclosing_workspace(&p) {
                            (ws, None, None, false, None, false, false, None)
                        } else {
                            return Err(format!(
                                "Cannot determine Cargo workspace for {:?}. No Cargo.toml found in path or parent directories.\nSpecify the path to a Cargo workspace or run 'ridx index' first.",
                                p
                            ));
                        }
                    };

                    let published = run_indexing(IndexOptions {
                        path: work_dir,
                        out_dir_opt,
                        package,
                        release,
                        features,
                        no_default_features,
                        all_features,
                        target,
                    });

                    // Reopen the rebuilt variant or shard rather than failing on an obsolete artifact path:
                    let mut cand_paths = Vec::new();
                    if p.extension().and_then(|s| s.to_str()) == Some("ridx") {
                        if let Some(file_name) = p.file_name() {
                            cand_paths.push(published.variant_out_dir.join(file_name));
                        }
                    }
                    cand_paths.push(published.variant_out_dir.clone());
                    cand_paths.push(published.base_out_dir.clone());
                    cand_paths.push(p.clone());

                    for cand in cand_paths {
                        if let Ok(ws) = WorkspaceIndex::open(&cand) {
                            return Ok(ws);
                        }
                    }

                    return Err(format!(
                        "Failed to open reindexed workspace after variant publish {:?}.",
                        published.variant_out_dir
                    ));
                }
                Err(format!("Failed to open index {:?}: {e}", p))
            }
        },
        None => match WorkspaceIndex::auto_discover() {
            Ok(ws) => Ok(ws),
            Err(e) => {
                let cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let (
                    work_dir,
                    out_dir_opt,
                    package,
                    release,
                    features,
                    no_default_features,
                    all_features,
                    target,
                ) = if let Some((manifest_dir, manifest)) = find_manifest_for_path(&cur) {
                    let ws_dir = if !manifest.workspace_root.is_empty()
                        && Path::new(&manifest.workspace_root).exists()
                    {
                        PathBuf::from(manifest.workspace_root)
                    } else if let Some(ws) = find_enclosing_workspace(&cur) {
                        ws
                    } else {
                        return Err(format!(
                            "Cannot find Cargo workspace for manifest at {:?}.",
                            manifest_dir
                        ));
                    };
                    (
                        ws_dir,
                        Some(manifest_dir),
                        manifest.package_filter,
                        manifest.profile == "release",
                        manifest.features,
                        manifest.no_default_features,
                        manifest.all_features,
                        manifest.target,
                    )
                } else if let Some(ws) = find_enclosing_workspace(&cur) {
                    (ws, None, None, false, None, false, false, None)
                } else {
                    return Err(format!(
                        "{e}\nSpecify the path to a .ridx file or directory, or run from a workspace with target/ridx/."
                    ));
                };

                eprintln!(
                    "{} No valid RIDX index found for current workspace (expected format v{}). Automatically indexing...",
                    "Notice:".yellow().bold(), ridx_format::CURRENT_FORMAT_VERSION
                );
                run_indexing(IndexOptions {
                    path: work_dir,
                    out_dir_opt,
                    package,
                    release,
                    features,
                    no_default_features,
                    all_features,
                    target,
                });
                match WorkspaceIndex::auto_discover() {
                    Ok(ws) => Ok(ws),
                    Err(e2) => Err(format!("Failed to auto-discover reindexed workspace: {e2}")),
                }
            }
        },
    }
}

fn resolve_workspace(path_opt: Option<PathBuf>) -> WorkspaceIndex {
    match try_resolve_workspace(path_opt) {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("{}: {e}", "Error".red().bold());
            #[cfg(not(test))]
            std::process::exit(1);
            #[cfg(test)]
            panic!("{e}");
        }
    }
}

fn try_resolve_index_and_target(
    arg1: String,
    arg2: Option<String>,
) -> Result<(WorkspaceIndex, String), String> {
    match arg2 {
        Some(target) => Ok((try_resolve_workspace(Some(PathBuf::from(arg1)))?, target)),
        None => Ok((try_resolve_workspace(None)?, arg1)),
    }
}

fn resolve_index_and_target(arg1: String, arg2: Option<String>) -> (WorkspaceIndex, String) {
    try_resolve_index_and_target(arg1, arg2).unwrap_or_else(|e| {
        eprintln!("{}: {e}", "Error".red().bold());
        #[cfg(not(test))]
        std::process::exit(1);
        #[cfg(test)]
        panic!("{e}");
    })
}

fn run_cli(command: Commands) {
    match command {
        Commands::Index {
            path,
            out_dir,
            package,
            release,
            features,
            no_default_features,
            all_features,
            target,
        } => {
            run_indexing(IndexOptions {
                path,
                out_dir_opt: out_dir,
                package,
                release,
                features,
                no_default_features,
                all_features,
                target,
            });
        }

        Commands::Inspect { index_path, json } => {
            let ws = resolve_workspace(index_path);
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

            println!(
                "{}",
                "═══════════════════════════════════════════════════════════════════════".dimmed()
            );
            println!(
                "  {} {} crate(s) loaded",
                "RIDX Workspace Index:".bold(),
                stats.total_crates.to_string().cyan()
            );
            if let Some(ref v) = stats.active_variant {
                println!("  Active Variant        : {}", v.cyan().bold());
            }
            println!(
                "{}",
                "═══════════════════════════════════════════════════════════════════════".dimmed()
            );
            println!(
                "{:<28}: {}",
                "Total Crates",
                stats.total_crates.to_string().bold()
            );
            println!(
                "{:<28}: {} bytes ({:.2} KB / {:.2} MB)",
                "Total Index Size",
                stats.total_bytes,
                stats.total_bytes as f64 / 1024.0,
                stats.total_bytes as f64 / (1024.0 * 1024.0)
            );
            println!("{:<28}: {}", "Indexed Files", stats.total_files);
            println!(
                "{:<28}: {}",
                "Definitions Count",
                stats.total_defs.to_string().green().bold()
            );
            println!(
                "{:<28}: {}",
                "References Count",
                stats.total_refs.to_string().green().bold()
            );
            println!(
                "{:<28}: {}",
                "Inferred Types Count",
                stats.total_types.to_string().green().bold()
            );
            println!(
                "{:<28}: {}",
                "Zero-Copy Lookup Ready",
                "YES (Mmap)".green().bold()
            );
            println!(
                "{:<28}: {}",
                "Daemon Memory Required",
                "0 MB (pure disk index)".green().bold()
            );
            println!(
                "{}",
                "───────────────────────────────────────────────────────────────────────".dimmed()
            );
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
            println!(
                "{}",
                "═══════════════════════════════════════════════════════════════════════".dimmed()
            );
        }

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
            json,
        } => {
            let (ws, target) = match symbol {
                Some(s) => {
                    let ws = resolve_workspace(Some(PathBuf::from(arg1)));
                    (ws, s)
                }
                None => resolve_index_and_target(arg1, arg2),
            };

            let cmp = compare::compare_definition(&ws, &target);
            if json {
                let speedup = if cmp.ridx_latency_micros > 0 {
                    format!(
                        "{:.1}x",
                        (cmp.prodcode_latency_micros as f64) / (cmp.ridx_latency_micros as f64)
                    )
                } else {
                    "inf".to_string()
                };
                let out = serde_json::json!({
                    "target": cmp.symbol,
                    "ridx": {
                        "found": cmp.ridx_found,
                        "span": cmp.ridx_span,
                        "latency_us": cmp.ridx_latency_micros,
                    },
                    "prodcode": {
                        "found": cmp.prodcode_found,
                        "span": cmp.prodcode_span,
                        "latency_us": cmp.prodcode_latency_micros,
                    },
                    "speedup": speedup,
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return;
            }
            compare::print_comparison(&cmp);
        }

        Commands::Lsp { index_path } => {
            if let Err(e) = lsp::run_lsp_server(index_path) {
                eprintln!(
                    "{}: LSP server exited with error: {e}",
                    "Error".red().bold()
                );
                std::process::exit(1);
            }
        }
    }
}

#[cfg(not(test))]
fn main() {
    let cli = Cli::parse();
    run_cli(cli.command);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference, RawType};

    struct TestDir(PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    static CLI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn new_test_dir(prefix: &str) -> TestDir {
        static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TestDir(path)
    }

    #[test]
    fn test_parse_location_cases() {
        assert_eq!(
            parse_location("src/lib.rs:10:5"),
            Some(("src/lib.rs", 10, 5))
        );
        assert_eq!(parse_location("src/lib.rs"), None);
        assert_eq!(parse_location("src/lib.rs:not_num:5"), None);
        assert_eq!(parse_location("src/lib.rs:10:not_num"), None);
        assert_eq!(parse_location(""), None);
    }

    #[test]
    fn test_find_enclosing_workspace_cases() {
        let test_dir = new_test_dir("ridx_ws_find");
        let cargo = test_dir.0.join("Cargo.toml");
        fs::write(&cargo, "[workspace]\n").unwrap();

        let sub_dir = test_dir.0.join("crates/sub");
        fs::create_dir_all(&sub_dir).unwrap();

        assert_eq!(find_enclosing_workspace(&sub_dir), Some(test_dir.0.clone()));
        assert_eq!(find_enclosing_workspace(&cargo), Some(test_dir.0.clone()));

        let empty_dir = new_test_dir("ridx_no_ws");
        assert_eq!(find_enclosing_workspace(&empty_dir.0), None);
    }

    #[test]
    fn test_find_manifest_for_path_variants() {
        let test_dir = new_test_dir("ridx_manifest_find");
        let manifest_file = test_dir.0.join("ridx_manifest.json");

        let manifest_data = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "active_variant_id": "var1",
            "variant_dir": "variants/var1",
            "workspace_root": test_dir.0.display().to_string(),
        });
        fs::write(
            &manifest_file,
            serde_json::to_string(&manifest_data).unwrap(),
        )
        .unwrap();

        let found = find_manifest_for_path(&test_dir.0);
        assert!(found.is_some());
        let (dir, man) = found.unwrap();
        assert_eq!(dir, test_dir.0);
        assert_eq!(man.active_variant_id, "var1");

        // Target an inactive variant path
        let inactive_path = test_dir.0.join("variants/var2/test.ridx");
        let found_inactive = find_manifest_for_path(&inactive_path);
        assert!(found_inactive.is_none());

        // Target a directory with variant_manifest.json directly
        let v_dir = test_dir.0.join("v_dir");
        fs::create_dir_all(&v_dir).unwrap();
        let v_manifest_file = v_dir.join("variant_manifest.json");
        fs::write(
            &v_manifest_file,
            serde_json::to_string(&manifest_data).unwrap(),
        )
        .unwrap();
        let found_v = find_manifest_for_path(&v_dir);
        assert!(found_v.is_some());

        // Path with grandparents where candidate_dirs pushes grandparents (lines 200-207)
        let deep_file = test_dir.0.join("deep/nested/sub/file.ridx");
        let _ = find_manifest_for_path(&deep_file);

        // Invalid JSON variant_manifest.json (line 219)
        let inv_v_dir = test_dir.0.join("inv_v_dir");
        fs::create_dir_all(&inv_v_dir).unwrap();
        fs::write(inv_v_dir.join("variant_manifest.json"), b"invalid json").unwrap();
        let _ = find_manifest_for_path(&inv_v_dir);

        // Invalid JSON ridx_manifest.json (line 248)
        let inv_r_dir = test_dir.0.join("inv_r_dir");
        fs::create_dir_all(&inv_r_dir).unwrap();
        fs::write(inv_r_dir.join("ridx_manifest.json"), b"invalid json").unwrap();
        let _ = find_manifest_for_path(&inv_r_dir);
    }

    fn create_indexed_workspace() -> (TestDir, String) {
        let test_dir = new_test_dir("ridx_cli_ws");
        let index_file = test_dir.0.join("test_crate.ridx");

        let mut writer = IndexWriter::new("test_crate", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            name: "test_fn".to_string(),
            path: "test_crate::test_fn".to_string(),
            signature: Some("fn()".to_string()),
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 12,
            end_col: 2,
        });

        writer.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Const,
            visibility: 0,
            is_macro_expansion: false,
            name: "NO_SIG".to_string(),
            path: "test_crate::NO_SIG".to_string(),
            signature: None,
            file: "src/lib.rs".to_string(),
            start_line: 15,
            start_col: 1,
            end_line: 15,
            end_col: 10,
        });

        writer.add_reference(RawReference {
            file: "src/main.rs".to_string(),
            start_line: 20,
            start_col: 5,
            end_line: 20,
            end_col: 12,
            target_def_id: Some(1),
            target_path: "test_crate::test_fn".to_string(),
            target_unit: None,
            target_file: Some("src/lib.rs".to_string()),
            target_line: 10,
            target_col: 1,
            kind: DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });

        writer.add_type(RawType {
            file: "src/lib.rs".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 10,
            end_col: 8,
            type_name: "fn()".to_string(),
        });

        let mut file = fs::File::create(&index_file).unwrap();
        writer.write_to(&mut file).unwrap();

        let dir_str = test_dir.0.display().to_string();
        (test_dir, dir_str)
    }

    #[test]
    fn test_cli_inspect_and_queries() {
        let (test_dir, dir_str) = create_indexed_workspace();
        let path = PathBuf::from(&dir_str);

        // Inspect (text and json)
        run_cli(Commands::Inspect {
            index_path: Some(path.clone()),
            json: false,
        });
        run_cli(Commands::Inspect {
            index_path: Some(path.clone()),
            json: true,
        });

        // Def by symbol (text and json)
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            json: false,
        });
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            json: true,
        });

        // Def by location (text and json)
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("src/main.rs:20:5".to_string()),
            json: false,
        });
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("src/main.rs:20:5".to_string()),
            json: true,
        });

        // Def not found
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("missing::symbol".to_string()),
            json: false,
        });
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("missing::symbol".to_string()),
            json: true,
        });

        // Refs by symbol (text and json)
        run_cli(Commands::Refs {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            json: false,
        });
        run_cli(Commands::Refs {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            json: true,
        });

        // Refs by location (text and json)
        run_cli(Commands::Refs {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: false,
        });
        run_cli(Commands::Refs {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: true,
        });

        // TypeAt (text and json)
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: false,
        });
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: true,
        });
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:99:99".to_string()),
            json: false,
        });

        // Hover (text and json)
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: false,
        });
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: true,
        });
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:15:1".to_string()),
            json: false,
        });
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("invalid_loc_format".to_string()),
            json: false,
        });

        // Symbols (text and json)
        run_cli(Commands::Symbols {
            arg1: dir_str.clone(),
            arg2: Some("test_fn".to_string()),
            json: false,
        });
        run_cli(Commands::Symbols {
            arg1: dir_str.clone(),
            arg2: Some("test_fn".to_string()),
            json: true,
        });
        run_cli(Commands::Symbols {
            arg1: dir_str.clone(),
            arg2: Some("NO_SIG".to_string()),
            json: false,
        });

        // Compare (text and json)
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            symbol: None,
            json: false,
        });
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            symbol: None,
            json: true,
        });
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: None,
            symbol: Some("test_crate::test_fn".to_string()),
            json: false,
        });
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: None,
            symbol: Some("test_crate::test_fn".to_string()),
            json: true,
        });

        // Def by path (text and json)
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            json: false,
        });
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            json: true,
        });

        // Def by location (text and json)
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: false,
        });
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: true,
        });

        // Inspect with directory path (text and json)
        run_cli(Commands::Inspect {
            index_path: Some(test_dir.0.clone()),
            json: false,
        });
        run_cli(Commands::Inspect {
            index_path: Some(test_dir.0.clone()),
            json: true,
        });

        let resolved = resolve_workspace(Some(test_dir.0.join("test_crate.ridx")));
        assert_eq!(resolved.stats().total_defs, 2);
        let (resolved_ws, target) =
            resolve_index_and_target(dir_str.clone(), Some("my_target".to_string()));
        assert_eq!(target, "my_target");
        assert_eq!(resolved_ws.stats().total_defs, 2);

        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("invalid_loc".to_string()),
            json: false,
        });
        run_cli(Commands::Lsp { index_path: None });
        let _ = std::panic::catch_unwind(|| {
            resolve_index_and_target(
                "/nonexistent/bogus.ridx".to_string(),
                Some("target".to_string()),
            );
        });
    }

    #[test]
    fn test_parse_location_and_workspace_helpers() {
        assert_eq!(
            parse_location("src/lib.rs:10:5"),
            Some(("src/lib.rs", 10, 5))
        );
        assert_eq!(parse_location("invalid"), None);
        assert_eq!(parse_location("src/lib.rs:bad:5"), None);
        assert_eq!(parse_location("src/lib.rs:10:bad"), None);

        let cur = std::env::current_dir().unwrap();
        let enc = find_enclosing_workspace(&cur);
        assert!(enc.is_some());

        let fake_path = Path::new("/nonexistent_directory/test");
        assert!(find_manifest_for_path(fake_path).is_none());
    }

    #[test]
    fn test_cli_index_command() {
        let (test_dir, _) = create_indexed_workspace();
        let cargo_toml = test_dir.0.join("Cargo.toml");
        fs::write(
            &cargo_toml,
            "[package]\nname = \"cli_sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let src_dir = test_dir.0.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("lib.rs"), "pub fn sample() {}\n").unwrap();

        if find_driver_binary().is_some() {
            run_cli(Commands::Index {
                path: test_dir.0.clone(),
                out_dir: None,
                package: None,
                release: false,
                features: None,
                no_default_features: false,
                all_features: false,
                target: None,
            });

            // Test auto-reindexing on missing shard in valid workspace
            let missing_shard = test_dir.0.join("target/ridx/missing_shard.ridx");
            let ws = resolve_workspace(Some(missing_shard));
            assert!(ws.crate_count() > 0);

            // Test resolve_workspace for missing shard without variant manifest (lines 326-327, 353-355)
            let no_man_ws = new_test_dir("no_man_ws");
            let cargo_toml = no_man_ws.0.join("Cargo.toml");
            fs::write(
                &cargo_toml,
                "[package]\nname = \"no_man\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .unwrap();
            let src_dir = no_man_ws.0.join("src");
            fs::create_dir_all(&src_dir).unwrap();
            fs::write(src_dir.join("lib.rs"), "pub fn f() {}\n").unwrap();
            let missing_in_no_man = no_man_ws.0.join("target/ridx/no_man.ridx");
            let ws_no_man = resolve_workspace(Some(missing_in_no_man));
            assert!(ws_no_man.crate_count() > 0);

            // Test inactive variant path in find_manifest_for_path
            let inactive_path = test_dir
                .0
                .join("target/ridx/variants/inactive_var/shard.ridx");
            assert!(find_manifest_for_path(&inactive_path).is_none());
        }
    }

    #[test]
    fn test_cli_single_arg_auto_discover() {
        let _lock = CLI_TEST_LOCK.lock().unwrap();
        let (test_dir, dir_str) = create_indexed_workspace();

        // Write variant manifest so Inspect prints active variant
        let manifest_data = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "extractor_version": ridx_format::EXTRACTOR_VERSION,
            "active_variant_id": "test_var_1",
            "variant_dir": "variants/test_var_1",
            "workspace_root": dir_str,
            "dependency_shards": []
        });
        let _ = fs::write(
            test_dir.0.join("variant_manifest.json"),
            serde_json::to_string(&manifest_data).unwrap(),
        );

        std::env::set_var("RIDX_INDEX_DIR", &dir_str);

        // 1. Inspect with None (triggers auto_discover and active variant print)
        run_cli(Commands::Inspect {
            index_path: None,
            json: false,
        });
        run_cli(Commands::Inspect {
            index_path: None,
            json: true,
        });

        // 2. Def with arg2 = None (target as arg1)
        run_cli(Commands::Def {
            arg1: "test_crate::test_fn".to_string(),
            arg2: None,
            json: false,
        });
        run_cli(Commands::Def {
            arg1: "test_crate::test_fn".to_string(),
            arg2: None,
            json: true,
        });
        run_cli(Commands::Def {
            arg1: "src/lib.rs:10:1".to_string(),
            arg2: None,
            json: false,
        });
        run_cli(Commands::Def {
            arg1: "src/lib.rs:10:1".to_string(),
            arg2: None,
            json: true,
        });

        // 3. Refs with arg2 = None (target as arg1)
        run_cli(Commands::Refs {
            arg1: "test_crate::test_fn".to_string(),
            arg2: None,
            json: false,
        });
        run_cli(Commands::Refs {
            arg1: "test_crate::test_fn".to_string(),
            arg2: None,
            json: true,
        });
        run_cli(Commands::Refs {
            arg1: "src/main.rs:20:5".to_string(),
            arg2: None,
            json: false,
        });
        run_cli(Commands::Refs {
            arg1: "src/main.rs:20:5".to_string(),
            arg2: None,
            json: true,
        });

        // 4. TypeAt with arg2 = None
        run_cli(Commands::TypeAt {
            arg1: "src/lib.rs:10:1".to_string(),
            arg2: None,
            json: false,
        });
        run_cli(Commands::TypeAt {
            arg1: "src/lib.rs:10:1".to_string(),
            arg2: None,
            json: true,
        });

        // 5. Hover with arg2 = None
        run_cli(Commands::Hover {
            arg1: "src/lib.rs:10:1".to_string(),
            arg2: None,
            json: false,
        });
        run_cli(Commands::Hover {
            arg1: "src/lib.rs:10:1".to_string(),
            arg2: None,
            json: true,
        });
        // Hover on definition without signature (line 912)
        run_cli(Commands::Hover {
            arg1: "src/lib.rs:15:1".to_string(),
            arg2: None,
            json: false,
        });

        // 6. Symbols with arg2 = None
        run_cli(Commands::Symbols {
            arg1: "test_fn".to_string(),
            arg2: None,
            json: false,
        });
        run_cli(Commands::Symbols {
            arg1: "test_fn".to_string(),
            arg2: None,
            json: true,
        });

        // 7. Compare with arg2 = None
        run_cli(Commands::Compare {
            arg1: "test_crate::test_fn".to_string(),
            arg2: None,
            symbol: None,
            json: false,
        });
        run_cli(Commands::Compare {
            arg1: "test_crate::test_fn".to_string(),
            arg2: None,
            symbol: None,
            json: true,
        });

        std::env::remove_var("RIDX_INDEX_DIR");
    }

    #[test]
    fn test_cli_not_found_branches_and_helpers() {
        let _lock = CLI_TEST_LOCK.lock().unwrap();
        let (test_dir, _) = create_indexed_workspace();
        let dir_str = test_dir.0.display().to_string();

        // 1. Def not found (text & json)
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("nonexistent_symbol".to_string()),
            json: false,
        });
        run_cli(Commands::Def {
            arg1: dir_str.clone(),
            arg2: Some("nonexistent_symbol".to_string()),
            json: true,
        });

        // 2. Refs not found (text & json)
        run_cli(Commands::Refs {
            arg1: dir_str.clone(),
            arg2: Some("nonexistent_symbol".to_string()),
            json: false,
        });
        run_cli(Commands::Refs {
            arg1: dir_str.clone(),
            arg2: Some("nonexistent_symbol".to_string()),
            json: true,
        });

        // 3. Hover not found
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:999:999".to_string()),
            json: false,
        });
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:999:999".to_string()),
            json: true,
        });

        // 4. Symbols not found
        run_cli(Commands::Symbols {
            arg1: dir_str.clone(),
            arg2: Some("nonexistent_symbol_query".to_string()),
            json: false,
        });
        run_cli(Commands::Symbols {
            arg1: dir_str.clone(),
            arg2: Some("nonexistent_symbol_query".to_string()),
            json: true,
        });

        // 5. Compare not found
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: Some("nonexistent_symbol".to_string()),
            symbol: None,
            json: false,
        });
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: Some("nonexistent_symbol".to_string()),
            symbol: None,
            json: true,
        });
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            symbol: None,
            json: true,
        });
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            symbol: None,
            json: false,
        });

        // 6. Inspect with direct path
        run_cli(Commands::Inspect {
            index_path: Some(test_dir.0.join("test_crate.ridx")),
            json: false,
        });
        run_cli(Commands::Inspect {
            index_path: Some(test_dir.0.join("test_crate.ridx")),
            json: true,
        });

        // 7. TypeAt not found & found
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:999:999".to_string()),
            json: false,
        });
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:999:999".to_string()),
            json: true,
        });
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: false,
        });
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: true,
        });

        // 8. Hover found
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: false,
        });
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:10:1".to_string()),
            json: true,
        });

        // 9. Refs location not found (text & json)
        run_cli(Commands::Refs {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:999:999".to_string()),
            json: false,
        });
        run_cli(Commands::Refs {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:999:999".to_string()),
            json: true,
        });

        // 10. Location parser & workspace helpers
        assert_eq!(
            parse_location("src/lib.rs:10:5"),
            Some(("src/lib.rs", 10, 5))
        );
        assert_eq!(parse_location("invalid"), None);
        assert_eq!(parse_location("src:not_a_num:5"), None);
        assert_eq!(parse_location("src/lib.rs:10:not_a_num"), None);

        assert!(find_enclosing_workspace(Path::new("/")).is_none());
        assert!(find_manifest_for_path(Path::new("/nonexistent/path/for/test")).is_none());

        // Manifest with nonexistent workspace_root outside cargo workspace (lines 282-285)
        let outside_dir = new_test_dir("outside_no_cargo");
        let vm_outside = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "extractor_version": ridx_format::EXTRACTOR_VERSION,
            "active_variant_id": "v1",
            "variant_dir": "variants/v1",
            "workspace_root": "/nonexistent/ws_root/12345",
            "shards": []
        });
        fs::write(
            outside_dir.0.join("variant_manifest.json"),
            serde_json::to_string(&vm_outside).unwrap(),
        )
        .unwrap();
        let res_no_cargo_ws = try_resolve_workspace(Some(outside_dir.0.join("shard.ridx")));
        assert!(res_no_cargo_ws.is_err());
        assert!(res_no_cargo_ws
            .unwrap_err()
            .contains("Cannot find Cargo workspace for index"));

        // 11. try_resolve_workspace error cases and auto-discover
        let inactive_variant = PathBuf::from("/tmp/test/variants/old_variant/shard.ridx");
        let res_inactive = try_resolve_workspace(Some(inactive_variant));
        assert!(res_inactive
            .unwrap_err()
            .contains("Cannot automatically reindex inactive variant shard"));

        let no_ws = PathBuf::from("/tmp/nonexistent_without_cargo_toml/shard.ridx");
        let res_no_ws = try_resolve_workspace(Some(no_ws));
        assert!(res_no_ws.is_err());

        std::env::set_var("RIDX_INDEX_DIR", &test_dir.0);
        let res_auto = try_resolve_workspace(None);
        assert!(res_auto.is_ok());
        std::env::remove_var("RIDX_INDEX_DIR");
    }

    #[test]
    fn test_manifest_discovery_and_auto_reindexing() {
        let _lock = CLI_TEST_LOCK.lock().unwrap();
        std::env::remove_var("RIDX_INDEX_DIR");
        let (test_dir, dir_str) = create_indexed_workspace();
        let deep_file = test_dir.0.join("a/b/c/nested_file.rs");
        assert!(find_manifest_for_path(&deep_file).is_none());

        let sub_dir = test_dir.0.join("sub_package");
        fs::create_dir_all(&sub_dir).unwrap();

        // 1. Shard-specific variant_manifest.json in candidate directory (lines 212-221)
        let v_manifest = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "extractor_version": ridx_format::EXTRACTOR_VERSION,
            "active_variant_id": "v_shard",
            "variant_dir": "variants/v_shard",
            "workspace_root": test_dir.0.display().to_string(),
            "shards": []
        });
        fs::write(
            sub_dir.join("variant_manifest.json"),
            serde_json::to_string(&v_manifest).unwrap(),
        )
        .unwrap();

        let found_vm = find_manifest_for_path(&sub_dir.join("shard.ridx"));
        assert!(found_vm.is_some());
        assert_eq!(found_vm.unwrap().1.active_variant_id, "v_shard");

        // 2. Active variant match in ridx_manifest.json (lines 235-248)
        let root_manifest = test_dir.0.join("ridx_manifest.json");
        let r_manifest = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "extractor_version": ridx_format::EXTRACTOR_VERSION,
            "active_variant_id": "active_var",
            "variant_dir": "variants/active_var",
            "workspace_root": test_dir.0.display().to_string(),
            "shards": []
        });
        fs::write(&root_manifest, serde_json::to_string(&r_manifest).unwrap()).unwrap();

        let active_shard = test_dir.0.join("variants/active_var/shard.ridx");
        let found_rm = find_manifest_for_path(&active_shard);
        assert!(found_rm.is_some());

        let inactive_shard = test_dir.0.join("variants/inactive_var/shard.ridx");
        assert!(find_manifest_for_path(&inactive_shard).is_none());
        let _ = fs::remove_file(&root_manifest);

        // 3. CLI commands execution: TypeAt, Hover, Compare
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:1:1".to_string()),
            json: false,
        });
        run_cli(Commands::TypeAt {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:1:1".to_string()),
            json: true,
        });

        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:1:1".to_string()),
            json: false,
        });
        run_cli(Commands::Hover {
            arg1: dir_str.clone(),
            arg2: Some("src/lib.rs:1:1".to_string()),
            json: true,
        });

        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            symbol: None,
            json: false,
        });
        run_cli(Commands::Compare {
            arg1: dir_str.clone(),
            arg2: Some("test_crate::test_fn".to_string()),
            symbol: None,
            json: true,
        });

        // 4. try_resolve_workspace variant_manifest error cases (lines 281-286, 289-305)
        let test_bad_ws = new_test_dir("bad_ws");
        let bad_manifest = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "extractor_version": ridx_format::EXTRACTOR_VERSION,
            "active_variant_id": "bad",
            "variant_dir": "variants/bad",
            "workspace_root": "/nonexistent/workspace/root",
            "shards": []
        });
        fs::write(
            test_bad_ws.0.join("variant_manifest.json"),
            serde_json::to_string(&bad_manifest).unwrap(),
        )
        .unwrap();
        let res_bad = try_resolve_workspace(Some(test_bad_ws.0.join("shard.ridx")));
        assert!(res_bad
            .unwrap_err()
            .contains("Cannot find Cargo workspace for index"));

        // try_resolve_workspace where manifest.workspace_root is empty and find_enclosing_workspace succeeds (line 280)
        let test_empty_root_ws = new_test_dir("empty_root_ws");
        fs::write(
            test_empty_root_ws.0.join("Cargo.toml"),
            "[package]\nname = \"empty_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        let empty_src = test_empty_root_ws.0.join("src");
        fs::create_dir_all(&empty_src).unwrap();
        fs::write(empty_src.join("lib.rs"), "pub fn empty_fn() {}\n").unwrap();
        let target_ridx = test_empty_root_ws.0.join("target/ridx");
        fs::create_dir_all(&target_ridx).unwrap();
        let empty_root_manifest = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "extractor_version": ridx_format::EXTRACTOR_VERSION,
            "active_variant_id": "v_empty",
            "variant_dir": "variants/v_empty",
            "workspace_root": "",
            "shards": []
        });
        fs::write(
            target_ridx.join("variant_manifest.json"),
            serde_json::to_string(&empty_root_manifest).unwrap(),
        )
        .unwrap();
        let _ = try_resolve_workspace(Some(target_ridx.join("shard.ridx")));

        // Base out dir resolution from parent variants dir (lines 289-305)
        let var_dir = test_bad_ws.0.join("target/ridx/variants/v1");
        fs::create_dir_all(&var_dir).unwrap();
        let v1_manifest = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "extractor_version": ridx_format::EXTRACTOR_VERSION,
            "active_variant_id": "v1",
            "variant_dir": "variants/v1",
            "workspace_root": "/nonexistent/workspace/root",
            "shards": []
        });
        fs::write(
            var_dir.join("variant_manifest.json"),
            serde_json::to_string(&v1_manifest).unwrap(),
        )
        .unwrap();
        let res_v1 = try_resolve_workspace(Some(var_dir.join("shard.ridx")));
        assert!(res_v1.is_err());

        let variants_dir = test_bad_ws.0.join("target2/ridx/variants");
        fs::create_dir_all(&variants_dir).unwrap();
        fs::write(
            variants_dir.join("variant_manifest.json"),
            serde_json::to_string(&v1_manifest).unwrap(),
        )
        .unwrap();
        let res_v2 = try_resolve_workspace(Some(variants_dir.join("shard.ridx")));
        assert!(res_v2.is_err());

        // 5. try_resolve_workspace(None) auto-indexing trigger (lines 386-445)
        let test_auto = new_test_dir("auto_idx");
        let empty_dir = test_auto.0.join("empty");
        fs::create_dir_all(&empty_dir).unwrap();
        let orig_cur = std::env::current_dir().unwrap();
        std::env::remove_var("RIDX_INDEX_DIR");
        std::env::set_current_dir(&empty_dir).unwrap();
        let res_empty = try_resolve_workspace(None);
        assert!(res_empty.is_err());
        std::env::set_current_dir(&orig_cur).unwrap();

        if find_driver_binary().is_some() {
            let ws_auto = test_auto.0.join("ws");
            let src_auto = ws_auto.join("src");
            fs::create_dir_all(&src_auto).unwrap();
            fs::write(
                ws_auto.join("Cargo.toml"),
                "[package]\nname = \"auto_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
            )
            .unwrap();
            fs::write(src_auto.join("lib.rs"), "pub fn auto_fn() {}\n").unwrap();

            std::env::set_var("RIDX_NO_DEPS", "1");
            std::env::set_current_dir(&ws_auto).unwrap();
            let res_auto_build = try_resolve_workspace(None);
            assert!(res_auto_build.is_ok());

            // Now test find_manifest_for_path returning valid manifest with workspace_root (lines 398-419)
            // In ws_auto, target/ridx/ridx_manifest.json exists now from auto_indexing
            let out_dir = ws_auto.join("target/ridx");
            let mut v_manifest = serde_json::json!({
                "version": "2.0",
                "format_version": 2,
                "extractor_version": "1",
                "active_variant_id": "test_var",
                "variant_dir": "variants/test_var",
                "base_out_dir": out_dir.display().to_string(),
                "workspace_root": ws_auto.display().to_string(),
                "manifest_path": ws_auto.join("Cargo.toml").display().to_string(),
                "profile": "release",
                "target": null,
                "features": null,
                "no_default_features": false,
                "all_features": false,
                "package_filter": null,
                "timestamp_epoch_secs": 0,
                "total_crates": 1,
                "total_defs": 0,
                "total_refs": 0,
                "total_types": 0,
                "dependency_shards": [],
                "crates": []
            });
            fs::write(
                out_dir.join("ridx_manifest.json"),
                serde_json::to_string(&v_manifest).unwrap(),
            )
            .unwrap();

            // Point current_dir to out_dir where ridx_manifest.json has workspace_root
            std::env::set_current_dir(&out_dir).unwrap();
            let res_from_manifest = try_resolve_workspace(None);
            assert!(res_from_manifest.is_ok());

            // Test manifest with non-existent workspace_root and no enclosing workspace (lines 405-408)
            v_manifest["workspace_root"] =
                serde_json::Value::String("/nonexistent/ws/root".to_string());
            let isolated_manifest_dir = test_auto.0.join("isolated_manifest");
            fs::create_dir_all(&isolated_manifest_dir).unwrap();
            fs::write(
                isolated_manifest_dir.join("ridx_manifest.json"),
                serde_json::to_string(&v_manifest).unwrap(),
            )
            .unwrap();
            std::env::set_current_dir(&isolated_manifest_dir).unwrap();
            let res_isolated = try_resolve_workspace(None);
            assert!(res_isolated
                .unwrap_err()
                .contains("Cannot find Cargo workspace"));

            // Test manifest with empty workspace_root where enclosing workspace exists (line 403)
            let cur_empty_ws = test_auto.0.join("cur_empty_ws");
            fs::create_dir_all(&cur_empty_ws).unwrap();
            let cur_src = cur_empty_ws.join("src");
            fs::create_dir_all(&cur_src).unwrap();
            fs::write(cur_src.join("lib.rs"), "pub fn cur_fn() {}\n").unwrap();
            fs::write(
                cur_empty_ws.join("Cargo.toml"),
                "[package]\nname = \"cur_empty\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
            )
            .unwrap();
            let cur_target_ridx = cur_empty_ws.join("target/ridx");
            fs::create_dir_all(&cur_target_ridx).unwrap();
            v_manifest["workspace_root"] = serde_json::Value::String("".to_string());
            fs::write(
                cur_target_ridx.join("ridx_manifest.json"),
                serde_json::to_string(&v_manifest).unwrap(),
            )
            .unwrap();
            std::env::set_current_dir(&cur_empty_ws).unwrap();
            let _ = try_resolve_workspace(None);

            std::env::set_current_dir(&orig_cur).unwrap();
            std::env::remove_var("RIDX_NO_DEPS");
        }

        // Test corrupted variant_manifest.json and ridx_manifest.json (lines 218-219, 247-248)
        let corrupt_dir = new_test_dir("corrupt_manifests");
        fs::write(corrupt_dir.0.join("variant_manifest.json"), b"invalid json").unwrap();
        fs::write(corrupt_dir.0.join("ridx_manifest.json"), b"invalid json").unwrap();
        assert!(find_manifest_for_path(&corrupt_dir.0).is_none());

        // Test corrupted index file returning io::ErrorKind::InvalidData (line 381)
        let corrupt_idx_file = corrupt_dir.0.join("bad.ridx");
        fs::write(&corrupt_idx_file, b"not a ridx file").unwrap();
        let res_corrupt = try_resolve_workspace(Some(corrupt_idx_file));
        assert!(res_corrupt.is_err());
        assert!(res_corrupt.unwrap_err().contains("Failed to open index"));

        // Test manifest_dir inside variants/var_id with base_out_dir None (lines 295-300)
        let var_base = new_test_dir("var_base_test");
        fs::write(
            var_base.0.join("Cargo.toml"),
            "[package]\nname = \"var_base\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        let var_src = var_base.0.join("src");
        fs::create_dir_all(&var_src).unwrap();
        fs::write(var_src.join("lib.rs"), "pub fn v() {}\n").unwrap();

        let variants_dir = var_base.0.join("variants");
        let dev_dir = variants_dir.join("dev");
        fs::create_dir_all(&dev_dir).unwrap();
        let no_base_mf = serde_json::json!({
            "version": "2.0",
            "format_version": ridx_format::CURRENT_FORMAT_VERSION,
            "extractor_version": ridx_format::EXTRACTOR_VERSION,
            "active_variant_id": "dev",
            "variant_dir": "variants/dev",
            "base_out_dir": null,
            "workspace_root": var_base.0.display().to_string(),
            "manifest_path": "",
            "profile": "dev",
            "timestamp_epoch_secs": 0,
            "total_crates": 0,
            "total_defs": 0,
            "total_refs": 0,
            "total_types": 0,
            "dependency_shards": [],
            "crates": []
        });
        fs::write(
            dev_dir.join("ridx_manifest.json"),
            serde_json::to_string(&no_base_mf).unwrap(),
        )
        .unwrap();
        let (m_dir, m_obj) = find_manifest_for_path(&dev_dir).unwrap();
        assert_eq!(m_dir, dev_dir);
        assert_eq!(m_obj.base_out_dir, None);
        let _ = try_resolve_workspace(Some(dev_dir));

        // Also test with manifest_dir ending in variants (line 304)
        fs::write(
            variants_dir.join("ridx_manifest.json"),
            serde_json::to_string(&no_base_mf).unwrap(),
        )
        .unwrap();
        let (v_dir, _) = find_manifest_for_path(&variants_dir).unwrap();
        assert_eq!(v_dir, variants_dir);
        let _ = try_resolve_workspace(Some(variants_dir));

        // Test try_resolve_index_and_target (lines 461-465)
        let _ = try_resolve_index_and_target(
            var_base.0.display().to_string(),
            Some("sample_target".to_string()),
        );
        let orig_c = std::env::current_dir().unwrap();
        std::env::set_current_dir(&var_base.0).unwrap();
        let _ = try_resolve_index_and_target("sample_target".to_string(), None);
        std::env::set_current_dir(&orig_c).unwrap();
    }

    #[test]
    fn test_resolve_workspace_and_index_target_error_paths() {
        let _ = std::panic::catch_unwind(|| {
            resolve_workspace(Some(PathBuf::from("/nonexistent/ridx/path/12345")));
        });
        let _ = std::panic::catch_unwind(|| {
            resolve_index_and_target("/nonexistent/ridx/path/12345".to_string(), None);
        });
    }
}
