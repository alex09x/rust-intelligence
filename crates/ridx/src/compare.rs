/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Comparison module: benchmarks and compares query results between RIDX and prod-code.

use colored::*;
use ridx_format::WorkspaceIndex;
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

    // 2. Query prod-code CLI if available
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
        prodcode_found,
        prodcode_span,
        prodcode_latency_micros: prodcode_latency,
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

    println!(
        "{:<20} | {:<22} | {:<22}",
        "Daemon Memory RSS",
        "0.00 MB (no daemon)".green().bold(),
        "~2700 MB (remote server)".yellow()
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
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn create_test_workspace() -> (TestDir, WorkspaceIndex) {
        static CNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = CNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("ridx_cmp_test_{}_{}", std::process::id(), id));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();

        let index_file = path.join("test_crate.ridx");

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

        let mut file = std::fs::File::create(&index_file).unwrap();
        writer.write_to(&mut file).unwrap();

        let ws = WorkspaceIndex::open(&path).unwrap();
        (TestDir(path), ws)
    }

    #[test]
    fn test_compare_definition_symbol_path_found_and_not_found() {
        let (_dir, ws) = create_test_workspace();

        let cmp = compare_definition(&ws, "test_crate::test_fn");
        assert_eq!(cmp.symbol, "test_crate::test_fn");
        assert!(cmp.ridx_found);
        assert!(cmp.ridx_span.contains("src/lib.rs:10:1"));

        print_comparison(&cmp);

        let cmp_not_found = compare_definition(&ws, "test_crate::missing_fn");
        assert_eq!(cmp_not_found.symbol, "test_crate::missing_fn");
        assert!(!cmp_not_found.ridx_found);
        assert_eq!(cmp_not_found.ridx_span, "Not found");

        print_comparison(&cmp_not_found);
    }

    #[test]
    fn test_compare_definition_location() {
        let (_dir, ws) = create_test_workspace();

        // Valid location format file:line:col
        let cmp = compare_definition(&ws, "src/lib.rs:10:1");
        assert_eq!(cmp.symbol, "src/lib.rs:10:1");

        // Invalid numbers in location format falls back to path
        let cmp_invalid = compare_definition(&ws, "src/lib.rs:not_a_line:not_a_col");
        assert_eq!(cmp_invalid.symbol, "src/lib.rs:not_a_line:not_a_col");

        // Edge case: latency is 0 and prodcode_found is true (line 124)
        let zero_lat = ComparisonResult {
            symbol: "test".to_string(),
            ridx_found: true,
            ridx_span: "file.rs:1:1".to_string(),
            ridx_latency_micros: 0,
            prodcode_found: true,
            prodcode_span: "src/lib.rs:1:1".to_string(),
            prodcode_latency_micros: 100,
        };
        print_comparison(&zero_lat);

        // Test prodcode_output success with definition and with "No definition" (lines 76-80)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mock_dir =
                std::env::temp_dir().join(format!("mock_prodcode_{}", std::process::id()));
            let _ = fs::create_dir_all(&mock_dir);
            let script_path = mock_dir.join("prod-code");
            fs::write(
                &script_path,
                b"#!/bin/sh\ncase \"$*\" in *nodef*) echo 'No definition' ;; *err_target*) echo 'simulated error' >&2; exit 1 ;; *) echo 'src/lib.rs:10:1' ;; esac\n",
            )
            .unwrap();
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();

            let orig_path = std::env::var("PATH").ok();
            let new_path = format!(
                "{}:{}",
                mock_dir.display(),
                orig_path.as_deref().unwrap_or("")
            );
            std::env::set_var("PATH", &new_path);

            let cmp_mock = compare_definition(&ws, "test_crate::test_fn");
            assert!(cmp_mock.prodcode_found);
            assert_eq!(cmp_mock.prodcode_span, "src/lib.rs:10:1");

            // Location target (lines 62-66)
            let cmp_loc = compare_definition(&ws, "src/lib.rs:10:1");
            assert!(cmp_loc.prodcode_found);

            let cmp_nodef = compare_definition(&ws, "nodef");
            assert!(!cmp_nodef.prodcode_found);

            // Error output from prodcode command (lines 83-86)
            let cmp_err = compare_definition(&ws, "err_target");
            assert!(!cmp_err.prodcode_found);
            assert!(cmp_err.prodcode_span.starts_with("Error"));

            let _ = orig_path.map(|p| std::env::set_var("PATH", p));

            let _ = fs::remove_dir_all(&mock_dir);
        }
    }
}
