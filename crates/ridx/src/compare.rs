/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Comparison module: benchmarks and compares query results between RIDX and rust-analyzer.

use colored::*;
use ridx_format::WorkspaceIndex;
use std::process::Command;
use std::time::Instant;

pub struct ComparisonResult {
    pub symbol: String,
    pub ridx_found: bool,
    pub ridx_span: String,
    pub ridx_latency_micros: u128,
    pub ridx_rss_bytes: u64,
    pub prodcode_found: bool,
    pub prodcode_span: String,
    pub prodcode_latency_micros: u128,
    pub prodcode_rss_bytes: u64,
}

pub fn get_process_rss_bytes_fallback_ps(pid: u32) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;

    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout);
        s.trim().parse::<u64>().ok().map(|kb| kb * 1024)
    } else {
        None
    }
}

pub fn get_process_rss_bytes(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string(format!("/proc/{}/statm", pid)) {
            if let Some(pages) = statm
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<u64>().ok())
            {
                return Some(pages * 4096);
            }
        }
    }

    get_process_rss_bytes_fallback_ps(pid)
}

pub fn get_current_process_rss_bytes() -> u64 {
    get_process_rss_bytes(std::process::id()).unwrap_or(0)
}

pub fn compare_definition(workspace: &WorkspaceIndex, target: &str) -> ComparisonResult {
    let is_loc = target.contains(':') && target.split(':').count() >= 3;

    // 1. Query local RIDX memory-mapped index
    let t_ridx = Instant::now();
    let ridx_def = if is_loc {
        let parts: Vec<&str> = target.rsplitn(3, ':').collect();
        if let (Some(col), Some(line)) =
            (parts[0].parse::<u32>().ok(), parts[1].parse::<u32>().ok())
        {
            workspace.find_definition_at(parts[2], line, col)
        } else {
            workspace.find_definition_by_path(target)
        }
    } else {
        workspace.find_definition_by_path(target)
    };
    let ridx_latency = t_ridx.elapsed().as_micros();
    let ridx_rss_bytes = get_current_process_rss_bytes();

    let (ridx_found, ridx_span) = match ridx_def {
        Some(def) => (
            true,
            format!(
                "{}:{}:{}",
                def.resolved_path().display(),
                def.start_line,
                def.start_col
            ),
        ),
        None => (false, "Not found".to_string()),
    };

    // 2. Query rust-analyzer / prod-code CLI if available
    let t_prodcode = Instant::now();
    let prodcode_output = if is_loc {
        let parts: Vec<&str> = target.rsplitn(3, ':').collect();
        Command::new("prod-code")
            .args(["def", parts[2], parts[1], parts[0]])
            .output()
    } else {
        Command::new("prod-code")
            .args(["def", "--symbol", target])
            .output()
    };
    let prodcode_latency = t_prodcode.elapsed().as_micros();

    let prodcode_rss_bytes =
        if let Ok(output) = Command::new("pgrep").args(["-f", "rust-analyzer"]).output() {
            let pids = String::from_utf8_lossy(&output.stdout);
            pids.lines()
                .filter_map(|p| p.trim().parse::<u32>().ok())
                .filter_map(get_process_rss_bytes)
                .max()
                .unwrap_or(0)
        } else {
            0
        };

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
        symbol: target.to_string(),
        ridx_found,
        ridx_span,
        ridx_latency_micros: ridx_latency,
        ridx_rss_bytes,
        prodcode_found,
        prodcode_span,
        prodcode_latency_micros: prodcode_latency,
        prodcode_rss_bytes,
    }
}

pub fn print_comparison(cmp: &ComparisonResult) {
    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════════════".dimmed()
    );
    println!("  {} {}", "Comparison Target:".bold(), cmp.symbol.cyan());
    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════════════".dimmed()
    );

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
        format!("{} µs ({})", cmp.ridx_latency_micros, speedup)
            .green()
            .bold(),
        format!(
            "{} µs ({} ms)",
            cmp.prodcode_latency_micros,
            cmp.prodcode_latency_micros / 1000
        )
        .dimmed()
    );

    let ridx_rss_str = if cmp.ridx_rss_bytes > 0 {
        format!(
            "{:.2} MB (transient CLI)",
            (cmp.ridx_rss_bytes as f64) / 1_048_576.0
        )
    } else {
        "0.00 MB (transient CLI)".to_string()
    };
    let ra_rss_str = if cmp.prodcode_rss_bytes > 0 {
        format!(
            "{:.2} MB (persistent daemon)",
            (cmp.prodcode_rss_bytes as f64) / 1_048_576.0
        )
    } else {
        "~0.00 MB (remote server)".to_string()
    };

    println!(
        "{:<20} | {:<22} | {:<22}",
        "Daemon Memory RSS",
        ridx_rss_str.green().bold(),
        ra_rss_str.yellow()
    );

    println!(
        "{}",
        "═══════════════════════════════════════════════════════════════════════".dimmed()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ridx_format::{DefKind, IndexWriter, RawDefinition};
    use std::fs;
    use std::path::PathBuf;

    struct TestDir(PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_compare_definition_and_print() {
        let temp_dir = std::env::temp_dir().join(format!("ridx_test_cmp_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let _guard = TestDir(temp_dir.clone());

        let ridx_path = temp_dir.join("test.ridx");
        let mut writer = IndexWriter::new("test_crate", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Fn,
            visibility: 0,
            is_macro_expansion: false,
            file: "src/lib.rs".to_string(),
            name: "test_fn".to_string(),
            start_line: 10,
            start_col: 1,
            end_line: 12,
            end_col: 2,
            path: "test_crate::test_fn".to_string(),
            signature: Some("fn()".to_string()),
        });
        let mut f = fs::File::create(&ridx_path).unwrap();
        writer.write_to(&mut f).unwrap();

        let ws = WorkspaceIndex::open(&ridx_path).unwrap();
        let res = compare_definition(&ws, "test_crate::test_fn");
        assert!(res.ridx_found);
        print_comparison(&res);

        let res_missing = compare_definition(&ws, "nonexistent::symbol");
        assert!(!res_missing.ridx_found);
        print_comparison(&res_missing);

        let res_loc = compare_definition(&ws, "src/lib.rs:10:1");
        assert!(res_loc.ridx_found);

        let cmp_custom = ComparisonResult {
            symbol: "test_crate::test_fn".to_string(),
            ridx_found: true,
            ridx_span: "src/lib.rs:10:1".to_string(),
            ridx_latency_micros: 0,
            ridx_rss_bytes: 0,
            prodcode_found: true,
            prodcode_span: "src/lib.rs:10:1".to_string(),
            prodcode_latency_micros: 100,
            prodcode_rss_bytes: 0,
        };
        print_comparison(&cmp_custom);

        let cmp_not_found = ComparisonResult {
            symbol: "test_crate::test_fn".to_string(),
            ridx_found: false,
            ridx_span: "Not found".to_string(),
            ridx_latency_micros: 10,
            ridx_rss_bytes: 1024 * 1024,
            prodcode_found: false,
            prodcode_span: "Not found".to_string(),
            prodcode_latency_micros: 200,
            prodcode_rss_bytes: 2048 * 1024,
        };
        print_comparison(&cmp_not_found);

        assert!(get_process_rss_bytes(std::process::id()).is_some());
        let _ = get_process_rss_bytes_fallback_ps(std::process::id());
        let _ = get_process_rss_bytes(99999999);

        // Mock prod-code execution (lines 121-126)
        let mock_bin = temp_dir.join("bin");
        let _ = fs::create_dir_all(&mock_bin);
        let mock_script = mock_bin.join("prod-code");
        let _ = fs::write(&mock_script, b"#!/bin/sh\necho 'src/lib.rs:10:1'\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&mock_script, fs::Permissions::from_mode(0o755));
        }
        let orig_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", mock_bin.display(), orig_path));
        let res_mock = compare_definition(&ws, "test_crate::test_fn");
        assert!(res_mock.prodcode_found);

        let _ = fs::write(&mock_script, b"#!/bin/sh\necho 'No definition'\n");
        let res_mock_none = compare_definition(&ws, "test_crate::test_fn");
        assert!(!res_mock_none.prodcode_found);

        let _ = fs::write(
            &mock_script,
            b"#!/bin/sh\necho 'internal failure' >&2\nexit 1\n",
        );
        let res_mock_err = compare_definition(&ws, "test_crate::test_fn");
        assert!(!res_mock_err.prodcode_found);
        assert!(!res_mock_err.prodcode_span.is_empty());

        std::env::set_var("PATH", "/nonexistent_bin_dir_123");
        let res_no_bin = compare_definition(&ws, "test_crate::test_fn");
        assert!(!res_no_bin.prodcode_found);
        assert!(!res_no_bin.prodcode_span.is_empty());

        std::env::set_var("PATH", orig_path);
    }
}
