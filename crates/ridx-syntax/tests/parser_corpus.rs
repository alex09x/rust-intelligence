/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Parses every `.rs` file of the bench corpora (`RIDX_BENCH_REPOS`) and of this workspace: the tree must cover
//! every token, and the share of files parsed without an `Error` node is reported with the most common error sites.

use ridx_syntax::{parse, SyntaxKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            // `test_data` holds rust-analyzer's parser fixtures, many invalid on purpose.
            if name != "target" && name != ".git" && name != "test_data" {
                rs_files(&p, out);
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

/// Parses `files`; returns (files, files with errors, error count, bytes, seconds) and records error contexts.
fn parse_all(files: &[PathBuf], contexts: &mut HashMap<String, (usize, String)>) -> (usize, usize, usize, usize, f64) {
    let (mut n, mut bad, mut errs, mut bytes, mut secs) = (0, 0, 0, 0, 0.0);
    for f in files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        let t0 = Instant::now();
        let tree = parse(&src);
        secs += t0.elapsed().as_secs_f64();
        n += 1;
        bytes += src.len();
        let root = tree.root();
        assert_eq!(tree.node(root).end as usize, tree.tokens.len(), "{}", f.display());
        let mut e = 0;
        for id in tree.errors() {
            e += 1;
            let parent = tree.parent(id).map_or(SyntaxKind::SourceFile, |p| tree.kind(p));
            let at = tree.range(id).start;
            let tok = tree.tokens.iter().find(|t| t.start as usize >= at && !t.is_trivia()).map_or("<eof>".to_string(), |t| t.text(&src).to_string());
            let key = format!("{parent:?} @ {tok}");
            let line = src[..at].matches('\n').count() + 1;
            let entry = contexts.entry(key).or_insert((0, format!("{}:{line}", f.display())));
            entry.0 += 1;
        }
        if e > 0 {
            bad += 1;
            errs += e;
        }
    }
    (n, bad, errs, bytes, secs)
}

#[test]
fn corpora_parse_without_errors() {
    let mut roots: Vec<(String, PathBuf)> = vec![("workspace".into(), PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))];
    if let Ok(base) = std::env::var("RIDX_BENCH_REPOS") {
        for c in ["serde", "ripgrep", "tokio", "rust-analyzer", "bevy", "clap", "regex"] {
            roots.push((c.into(), PathBuf::from(&base).join(c)));
        }
    }
    let mut contexts = HashMap::new();
    let (mut tn, mut tbad) = (0, 0);
    for (label, root) in &roots {
        let mut files = Vec::new();
        rs_files(root, &mut files);
        let (n, bad, errs, bytes, secs) = parse_all(&files, &mut contexts);
        tn += n;
        tbad += bad;
        let mib = bytes as f64 / (1024.0 * 1024.0);
        eprintln!("[parse:{label:<13}] files={n:<5} with_errors={bad:<4} errors={errs:<6} {mib:.1} MiB in {:.1} ms ({:.0} MiB/s)", secs * 1e3, mib / secs.max(1e-9));
    }
    let mut top: Vec<_> = contexts.into_iter().collect();
    top.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    for (k, (c, ex)) in top.iter().take(40) {
        eprintln!("  {c:>6}  {k:<40} e.g. {ex}");
    }
    let clean = 100.0 * (tn - tbad) as f64 / tn.max(1) as f64;
    eprintln!("[parse:TOTAL] files={tn} clean={:.2}%", clean);
}
