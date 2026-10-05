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

fn resolve_workspace(path_opt: Option<PathBuf>) -> WorkspaceIndex {
    match path_opt {
        Some(p) => match WorkspaceIndex::open(&p) {
            Ok(ws) => ws,
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
                            eprintln!(
                                "{}: Cannot find Cargo workspace for index at {:?}.",
                                "Error".red().bold(),
                                p
                            );
                            std::process::exit(1);
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
                            eprintln!(
                                    "{}: Cannot automatically reindex inactive variant shard at {:?}: no matching variant manifest found.",
                                    "Error".red().bold(), p
                                );
                            eprintln!("Run 'ridx index' with the appropriate build configuration for this variant.");
                            std::process::exit(1);
                        }
                        if let Some(ws) = find_enclosing_workspace(&p) {
                            (ws, None, None, false, None, false, false, None)
                        } else {
                            eprintln!("{}: Cannot determine Cargo workspace for {:?}. No Cargo.toml found in path or parent directories.", "Error".red().bold(), p);
                            eprintln!(
                                "Specify the path to a Cargo workspace or run 'ridx index' first."
                            );
                            std::process::exit(1);
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
                    let cand_paths = if p.is_file()
                        || p.extension().and_then(|s| s.to_str()) == Some("ridx")
                    {
                        let mut v = Vec::new();
                        if let Some(file_name) = p.file_name() {
                            let shard_in_new_variant = published.variant_out_dir.join(file_name);
                            if shard_in_new_variant.exists() {
                                v.push(shard_in_new_variant);
                            }
                        }
                        v.push(published.variant_out_dir.clone());
                        v.push(published.base_out_dir.clone());
                        v.push(p.clone());
                        v
                    } else {
                        vec![
                            published.variant_out_dir.clone(),
                            published.base_out_dir.clone(),
                            p.clone(),
                        ]
                    };

                    for cand in cand_paths {
                        if let Ok(ws) = WorkspaceIndex::open(&cand) {
                            return ws;
                        }
                    }

                    eprintln!(
                        "{}: Failed to open reindexed workspace after variant publish {:?}.",
                        "Error".red().bold(),
                        published.variant_out_dir
                    );
                    std::process::exit(1);
                }
                eprintln!(
                    "{}: Failed to open index {:?}: {e}",
                    "Error".red().bold(),
                    p
                );
                std::process::exit(1);
            }
        },
        None => match WorkspaceIndex::auto_discover() {
            Ok(ws) => ws,
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
                        eprintln!(
                            "{}: Cannot find Cargo workspace for manifest at {:?}.",
                            "Error".red().bold(),
                            manifest_dir
                        );
                        std::process::exit(1);
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
                    eprintln!("{}: {e}", "Error".red().bold());
                    eprintln!("Specify the path to a .ridx file or directory, or run from a workspace with target/ridx/.");
                    std::process::exit(1);
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
                    Ok(ws) => ws,
                    Err(e2) => {
                        eprintln!(
                            "{}: Failed to auto-discover reindexed workspace: {e2}",
                            "Error".red().bold()
                        );
                        std::process::exit(1);
                    }
                }
            }
        },
    }
}

fn resolve_index_and_target(arg1: String, arg2: Option<String>) -> (WorkspaceIndex, String) {
    match arg2 {
        Some(target) => (resolve_workspace(Some(PathBuf::from(arg1))), target),
        None => (resolve_workspace(None), arg1),
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
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
                std::process::exit(1);
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
                std::process::exit(1);
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
