use clap::{Parser, Subcommand};
use colored::*;
use ridx_format::WorkspaceIndex;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

mod compare;

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
    },

    /// Inspect binary header, record counts, and memory layout of a .ridx index file or workspace
    Inspect {
        /// Path to .ridx index file or directory (default: auto-discover target/ridx)
        index_path: Option<PathBuf>,
    },

    /// Go to definition for a symbol path or cursor location (file:line:col)
    Def {
        /// First argument: index file/dir, or target symbol/location if index is omitted
        arg1: String,
        /// Second argument: target symbol/location if first argument was index path
        arg2: Option<String>,
    },

    /// Find all references to a given symbol path across the workspace
    Refs {
        /// First argument: index file/dir, or target symbol if index is omitted
        arg1: String,
        /// Second argument: target symbol if first argument was index path
        arg2: Option<String>,
    },

    /// Show inferred expression type at a cursor location (file:line:col)
    TypeAt {
        /// First argument: index file/dir, or location if index is omitted
        arg1: String,
        /// Second argument: location if first argument was index path
        arg2: Option<String>,
    },

    /// Search symbol definitions matching a query string across the workspace
    Symbols {
        /// First argument: index file/dir, or query if index is omitted
        arg1: String,
        /// Second argument: query if first argument was index path
        arg2: Option<String>,
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

fn resolve_workspace(path_opt: Option<PathBuf>) -> WorkspaceIndex {
    match path_opt {
        Some(p) => match WorkspaceIndex::open(&p) {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("{}: Failed to open index {:?}: {e}", "Error".red().bold(), p);
                std::process::exit(1);
            }
        },
        None => match WorkspaceIndex::auto_discover() {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("{}: {e}", "Error".red().bold());
                eprintln!("Specify the path to a .ridx file or directory, or run from a workspace with target/ridx/.");
                std::process::exit(1);
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

fn find_driver_binary() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("ridx-driver");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for candidate in [
        PathBuf::from("target/release/ridx-driver"),
        PathBuf::from("target/debug/ridx-driver"),
        PathBuf::from("../target/release/ridx-driver"),
        PathBuf::from("../target/debug/ridx-driver"),
    ] {
        if candidate.is_file() {
            return candidate.canonicalize().ok();
        }
    }
    if let Ok(out) = Command::new("which").arg("ridx-driver").output() {
        if out.status.success() {
            let path_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }
    None
}

fn run_indexing(
    path: PathBuf,
    out_dir_opt: Option<PathBuf>,
    package: Option<String>,
    release: bool,
) {
    let start = Instant::now();
    let out_dir = out_dir_opt.unwrap_or_else(|| path.join("target/ridx"));
    if let Err(e) = fs::create_dir_all(&out_dir) {
        eprintln!("{}: failed to create output directory {:?}: {e}", "Error".red().bold(), out_dir);
        std::process::exit(1);
    }

    let driver_bin = match find_driver_binary() {
        Some(b) => b,
        None => {
            eprintln!("{}: could not find ridx-driver binary.", "Error".red().bold());
            eprintln!("Please build ridx-driver first (e.g. cargo build -p ridx-driver) or place it in PATH.");
            std::process::exit(1);
        }
    };

    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    println!("  {} {}", "RIDX Workspace Indexer:".bold(), path.display().to_string().cyan());
    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    println!("  Driver Binary  : {}", driver_bin.display().to_string().green());
    println!("  Index Output   : {}", out_dir.display().to_string().green());
    println!("  Action         : cargo check with RUSTC_WORKSPACE_WRAPPER...");

    let mut cmd = Command::new("cargo");
    cmd.arg("check");
    let manifest = path.join("Cargo.toml");
    if manifest.exists() {
        cmd.arg("--manifest-path").arg(&manifest);
    }
    if let Some(ref p) = package {
        cmd.arg("-p").arg(p);
    } else {
        cmd.arg("--workspace");
    }
    if release {
        cmd.arg("--release");
    }

    cmd.env("RUSTC_WORKSPACE_WRAPPER", &driver_bin);
    cmd.env("RIDX_OUT_DIR", &out_dir);
    cmd.env("RUSTC_BOOTSTRAP", "1");
    cmd.env("RUSTC_WRAPPER", "");

    // Ensure librustc_driver shared library can be located by dynamic linker
    if let Ok(out) = Command::new("rustc").args(["--print", "sysroot"]).output() {
        if out.status.success() {
            let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let lib_dir = PathBuf::from(&sysroot).join("lib");
            let mut ld_path = lib_dir.to_string_lossy().to_string();
            if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
                ld_path = format!("{}:{}", ld_path, existing);
            }
            cmd.env("LD_LIBRARY_PATH", &ld_path);
            cmd.env("DYLD_FALLBACK_LIBRARY_PATH", &ld_path);
        }
    }

    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: failed to execute cargo check: {e}", "Error".red().bold());
            std::process::exit(1);
        }
    };

    let elapsed = start.elapsed();

    if !status.success() {
        eprintln!("{}: cargo check exited with status {status}", "Error".red().bold());
        std::process::exit(1);
    }

    println!("\n{} Indexed workspace in {:.2} s", "Success:".green().bold(), elapsed.as_secs_f64());

    if let Ok(ws) = WorkspaceIndex::open(&out_dir) {
        let stats = ws.stats();
        println!(
            "Indexed {} crate(s), {} total definitions, {} references, {} types ({:.2} KB total index size)",
            stats.total_crates.to_string().bold(),
            stats.total_defs.to_string().green(),
            stats.total_refs.to_string().green(),
            stats.total_types.to_string().green(),
            stats.total_bytes as f64 / 1024.0
        );
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
        } => {
            run_indexing(path, out_dir, package, release);
        }

        Commands::Inspect { index_path } => {
            let ws = resolve_workspace(index_path);
            let stats = ws.stats();

            println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
            println!("  {} {} crate(s) loaded", "RIDX Workspace Index:".bold(), stats.total_crates.to_string().cyan());
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

        Commands::Def { arg1, arg2 } => {
            let (ws, target) = resolve_index_and_target(arg1, arg2);
            let start = Instant::now();

            if let Some((file, line, col)) = parse_location(&target) {
                let res = ws.find_definition_at(file, line, col);
                let elapsed = start.elapsed();

                match res {
                    Some(def) => {
                        println!(
                            "{} {} at {}:{}:{} (kind: {:?}) [{:.1} µs]",
                            "Definition:".green().bold(),
                            def.name.bold(),
                            def.file,
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
                            "{} No definition found at {} [{:.1} µs]",
                            "NotFound:".yellow().bold(),
                            target,
                            elapsed.as_micros()
                        );
                    }
                }
            } else {
                let res = ws.find_definition_by_path(&target);
                let elapsed = start.elapsed();

                match res {
                    Some(d) => {
                        println!(
                            "{} {} at {}:{}:{} (kind: {:?}) [{:.1} µs]",
                            "Definition:".green().bold(),
                            d.name.bold(),
                            d.file,
                            d.start_line,
                            d.start_col,
                            d.kind,
                            elapsed.as_micros()
                        );
                        println!("  Full Path: {}", d.path.cyan());
                        if let Some(sig) = d.signature {
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
        }

        Commands::Refs { arg1, arg2 } => {
            let (ws, symbol) = resolve_index_and_target(arg1, arg2);
            let start = Instant::now();
            let refs = ws.find_references(&symbol);
            let elapsed = start.elapsed();

            println!(
                "{} {} reference(s) to {} across workspace [{:.1} µs]:",
                "Found:".green().bold(),
                refs.len(),
                symbol.cyan(),
                elapsed.as_micros()
            );

            for (i, r) in refs.iter().enumerate() {
                let span = r.span;
                println!(
                    "  [{}] ({}) {}:{}:{}-{}:{} (kind: {:?})",
                    i + 1,
                    r.crate_name.cyan(),
                    r.file,
                    span.start_line(),
                    span.start_col(),
                    span.end_line(),
                    span.end_col(),
                    r.kind
                );
            }
        }

        Commands::TypeAt { arg1, arg2 } => {
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

        Commands::Symbols { arg1, arg2 } => {
            let (ws, query) = resolve_index_and_target(arg1, arg2);
            let start = Instant::now();
            let matches = ws.search_symbols(&query);
            let elapsed = start.elapsed();

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
                    sym.file,
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
        } => {
            let (ws, target) = match symbol {
                Some(s) => {
                    let ws = resolve_workspace(Some(PathBuf::from(arg1)));
                    (ws, s)
                }
                None => resolve_index_and_target(arg1, arg2),
            };

            let cmp = compare::compare_definition(&ws, &target);
            compare::print_comparison(&cmp);
        }
    }
}
