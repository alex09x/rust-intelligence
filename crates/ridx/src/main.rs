use clap::{Parser, Subcommand};
use colored::*;
use ridx_format::IndexReader;
use std::fs;
use std::path::PathBuf;
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
    /// Inspect binary header, record counts, and memory layout of a .ridx index file
    Inspect {
        /// Path to .ridx index file
        index_file: PathBuf,
    },

    /// Go to definition for a symbol path or cursor location (file:line:col)
    Def {
        /// Path to .ridx index file
        index_file: PathBuf,
        /// Target symbol path (e.g. 'crate::foo') or location (e.g. 'src/main.rs:10:15')
        target: String,
    },

    /// Find all references to a given symbol path
    Refs {
        /// Path to .ridx index file
        index_file: PathBuf,
        /// Target canonical path (e.g. 'test_crate::math::calculate')
        symbol: String,
    },

    /// Show inferred expression type at a cursor location (file:line:col)
    TypeAt {
        /// Path to .ridx index file
        index_file: PathBuf,
        /// Location formatted as 'file_path:line:col' (1-based)
        location: String,
    },

    /// Search symbol definitions matching a query string
    Symbols {
        /// Path to .ridx index file
        index_file: PathBuf,
        /// Query string (substring match on symbol path or name)
        query: String,
    },

    /// Compare accuracy and latency against prod-code (rust-analyzer)
    Compare {
        /// Path to .ridx index file
        index_file: PathBuf,
        /// Symbol path to look up and benchmark
        #[arg(long, short)]
        symbol: String,
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

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Inspect { index_file } => {
            let reader = match IndexReader::open(&index_file) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}: Failed to open index {:?}: {e}", "Error".red().bold(), index_file);
                    std::process::exit(1);
                }
            };
            let h = reader.header();
            let file_size = fs::metadata(&index_file).map(|m| m.len()).unwrap_or(0);

            println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
            println!("  {} {}", "RIDX Binary Index:".bold(), index_file.display().to_string().cyan());
            println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
            println!("{:<28}: {}", "Crate Name", reader.crate_name().bold());
            println!("{:<28}: {}", "Crate Version", reader.crate_version());
            println!("{:<28}: {}", "Format Version", h.version);
            println!("{:<28}: {} bytes ({:.2} KB)", "Total Index Size", file_size, file_size as f64 / 1024.0);
            println!("{:<28}: {} bytes ({:.2} KB)", "String Pool Size", h.string_pool_len, h.string_pool_len as f64 / 1024.0);
            println!("{:<28}: {}", "Indexed Files", h.files_count);
            println!("{:<28}: {}", "Definitions Count", h.defs_count.to_string().green());
            println!("{:<28}: {}", "References Count", h.refs_count.to_string().green());
            println!("{:<28}: {}", "Inferred Types Count", h.types_count.to_string().green());
            println!("{:<28}: {}", "Zero-Copy Lookup Ready", "YES (Mmap)".green().bold());
            println!("{:<28}: {}", "Daemon Memory Required", "0 MB (pure disk index)".green().bold());
            println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
        }

        Commands::Def { index_file, target } => {
            let reader = match IndexReader::open(&index_file) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}: Failed to open index {:?}: {e}", "Error".red().bold(), index_file);
                    std::process::exit(1);
                }
            };

            let start = Instant::now();
            if let Some((file, line, col)) = parse_location(&target) {
                let res = reader.find_definition_at(file, line, col);
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
                        println!("{}: No definition found at {} [{:.1} µs]", "Not found".yellow(), target, elapsed.as_micros());
                    }
                }
            } else {
                let def = reader.find_definition_by_path(&target);
                let elapsed = start.elapsed();

                match def {
                    Some(d) => {
                        let name = reader.get_str(d.name_offset);
                        let path = reader.get_str(d.path_offset);
                        let file = reader.get_file(d.span.file_id);
                        let sig = if d.sig_offset != 0 {
                            Some(reader.get_str(d.sig_offset))
                        } else {
                            None
                        };

                        println!(
                            "{} {} at {}:{}:{} [{:.1} µs]",
                            "Definition:".green().bold(),
                            name.bold(),
                            file,
                            d.span.start_line,
                            d.span.start_col,
                            elapsed.as_micros()
                        );
                        println!("  Full Path: {}", path.cyan());
                        if let Some(s) = sig {
                            println!("  Signature: {}", s.yellow());
                        }
                    }
                    None => {
                        println!("{}: Symbol '{}' not found [{:.1} µs]", "Not found".yellow(), target, elapsed.as_micros());
                    }
                }
            }
        }

        Commands::Refs { index_file, symbol } => {
            let reader = match IndexReader::open(&index_file) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}: Failed to open index {:?}: {e}", "Error".red().bold(), index_file);
                    std::process::exit(1);
                }
            };

            let start = Instant::now();
            let refs = reader.find_references(&symbol);
            let elapsed = start.elapsed();

            println!(
                "{} Found {} reference(s) for '{}' [{:.1} µs]:",
                "References:".green().bold(),
                refs.len(),
                symbol.cyan(),
                elapsed.as_micros()
            );

            for (i, r) in refs.iter().enumerate() {
                let file = reader.get_file(r.span.file_id);
                println!(
                    "  [{}] {}:{}:{}-{}:{} (kind: {:?})",
                    i + 1,
                    file,
                    r.span.start_line,
                    r.span.start_col,
                    r.span.end_line,
                    r.span.end_col,
                    ridx_format::DefKind::from(r.kind)
                );
            }
        }

        Commands::TypeAt { index_file, location } => {
            let reader = match IndexReader::open(&index_file) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}: Failed to open index {:?}: {e}", "Error".red().bold(), index_file);
                    std::process::exit(1);
                }
            };

            let Some((file, line, col)) = parse_location(&location) else {
                eprintln!("{}: Location must be formatted as 'file:line:col'", "Error".red().bold());
                std::process::exit(1);
            };

            let start = Instant::now();
            let ty = reader.type_at(file, line, col);
            let elapsed = start.elapsed();

            match ty {
                Some(t) => {
                    println!(
                        "{} {} at {} [{:.1} µs]",
                        "Type:".green().bold(),
                        t.cyan().bold(),
                        location,
                        elapsed.as_micros()
                    );
                }
                None => {
                    println!("{}: No type inferred at {} [{:.1} µs]", "Not found".yellow(), location, elapsed.as_micros());
                }
            }
        }

        Commands::Symbols { index_file, query } => {
            let reader = match IndexReader::open(&index_file) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}: Failed to open index {:?}: {e}", "Error".red().bold(), index_file);
                    std::process::exit(1);
                }
            };

            let start = Instant::now();
            let matches = reader.search_symbols(&query);
            let elapsed = start.elapsed();

            println!(
                "{} Found {} symbol(s) matching '{}' [{:.1} µs]:",
                "Symbols:".green().bold(),
                matches.len(),
                query.cyan(),
                elapsed.as_micros()
            );

            for def in matches.iter().take(50) {
                let path = reader.get_str(def.path_offset);
                let file = reader.get_file(def.span.file_id);
                println!(
                    "  {} ({:?}) - {}:{}:{}",
                    path.cyan(),
                    def.kind,
                    file,
                    def.span.start_line,
                    def.span.start_col
                );
            }
        }

        Commands::Compare { index_file, symbol } => {
            let reader = match IndexReader::open(&index_file) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}: Failed to open index {:?}: {e}", "Error".red().bold(), index_file);
                    std::process::exit(1);
                }
            };

            let result = compare::compare_definition(&reader, &symbol);
            compare::print_comparison(&result);
        }
    }
}
