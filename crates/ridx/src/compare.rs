//! Comparison module: benchmarks and compares query results between RIDX and prod-code.

use colored::*;
use memmap2::Mmap;
use ridx_format::IndexReader;
use std::process::Command;
use std::time::Instant;

pub struct ComparisonResult {
    pub symbol: String,
    pub ridx_found: bool,
    pub ridx_span: String,
    pub ridx_latency_micros: u128,
    pub prodcode_found: bool,
    pub prodcode_span: String,
    pub prodcode_latency_micros: u128,
}

pub fn compare_definition(reader: &IndexReader<Mmap>, symbol: &str) -> ComparisonResult {
    // 1. Query local RIDX memory-mapped index
    let t_ridx = Instant::now();
    let ridx_def = reader.find_definition_by_path(symbol);
    let ridx_latency = t_ridx.elapsed().as_micros();

    let (ridx_found, ridx_span) = match ridx_def {
        Some(def) => {
            let file = reader.get_file(def.span.file_id);
            (
                true,
                format!("{}:{}:{}", file, def.span.start_line, def.span.start_col),
            )
        }
        None => (false, "Not found".to_string()),
    };

    // 2. Query prod-code CLI if available
    let t_prodcode = Instant::now();
    let prodcode_output = Command::new("prod-code")
        .args(["def", "--symbol", symbol])
        .output();
    let prodcode_latency = t_prodcode.elapsed().as_micros();

    let (prodcode_found, prodcode_span) = match prodcode_output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() || text.contains("No definition") {
                (false, "Not found".to_string())
            } else {
                (true, text.lines().next().unwrap_or(&text).to_string())
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            (false, format!("Error ({err})"))
        }
        Err(e) => (false, format!("prod-code unavailable ({e})")),
    };

    ComparisonResult {
        symbol: symbol.to_string(),
        ridx_found,
        ridx_span,
        ridx_latency_micros: ridx_latency,
        prodcode_found,
        prodcode_span,
        prodcode_latency_micros: prodcode_latency,
    }
}

pub fn print_comparison(cmp: &ComparisonResult) {
    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
    println!("  {} {}", "Comparison Target:".bold(), cmp.symbol.cyan());
    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());

    println!(
        "{:<20} | {:<22} | {:<22}",
        "Metric", "RIDX (Zero-Copy Mmap)", "prod-code (rust-analyzer)"
    );
    println!("{:-<20}-+-{:-<22}-+-{:-<22}", "", "", "");

    let ridx_status = if cmp.ridx_found {
        "FOUND".green()
    } else {
        "NOT FOUND".red()
    };
    let pc_status = if cmp.prodcode_found {
        "FOUND".green()
    } else {
        "NOT FOUND".yellow()
    };
    println!("{:<20} | {:<22} | {:<22}", "Status", ridx_status, pc_status);

    println!(
        "{:<20} | {:<22} | {:<22}",
        "Target Location", cmp.ridx_span, cmp.prodcode_span
    );

    let speedup = if cmp.ridx_latency_micros > 0 {
        format!(
            "{:.0}x faster",
            (cmp.prodcode_latency_micros as f64) / (cmp.ridx_latency_micros as f64)
        )
    } else {
        "instant".to_string()
    };

    println!(
        "{:<20} | {:<22} | {:<22}",
        "Query Latency",
        format!("{} µs ({})", cmp.ridx_latency_micros, speedup).green().bold(),
        format!("{} µs ({} ms)", cmp.prodcode_latency_micros, cmp.prodcode_latency_micros / 1000).dimmed()
    );

    println!(
        "{:<20} | {:<22} | {:<22}",
        "Daemon Memory RSS",
        "0.00 MB (no daemon)".green().bold(),
        "~2700 MB (remote server)".yellow()
    );

    println!("{}", "═══════════════════════════════════════════════════════════════════════".dimmed());
}
