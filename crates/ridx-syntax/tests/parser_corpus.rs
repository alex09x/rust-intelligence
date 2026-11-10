/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Parses `.rs` files under an explicitly configured corpus root (`RIDX_CORPUS_ROOT` or `RIDX_BENCH_REPOS`)
//! within strict file-count and wall-clock limits. Skips immediately when no root env var is set.

mod common;

use common::{collect_bounded_rs_files, corpus_roots_from_env, validate_explicit_root, CorpusBudget};
use ridx_syntax::{parse, SyntaxKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn parse_all(files: &[PathBuf], deadline: Instant, contexts: &mut HashMap<String, (usize, String)>) -> (usize, usize, usize, usize, f64, bool) {
    let (mut n, mut bad, mut errs, mut bytes, mut secs) = (0, 0, 0, 0, 0.0);
    let mut timed_out = false;
    for f in files {
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
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
    (n, bad, errs, bytes, secs, timed_out)
}

#[test]
fn corpora_parse_without_errors() {
    let roots = corpus_roots_from_env();
    if roots.is_empty() {
        eprintln!("[parse:skip] set RIDX_CORPUS_ROOT or RIDX_BENCH_REPOS to run corpus parse test");
        return;
    }
    let mut budget = CorpusBudget::from_env();
    let mut contexts = HashMap::new();
    let (mut tn, mut tbad) = (0, 0);
    for (label, root) in &roots {
        if budget.exhausted() {
            break;
        }
        let mut files = Vec::new();
        collect_bounded_rs_files(root, true, &mut budget, &mut files);
        let (n, bad, errs, bytes, secs, timed_out) = parse_all(&files, budget.deadline, &mut contexts);
        if timed_out {
            budget.truncated = true;
        }
        tn += n;
        tbad += bad;
        let mib = bytes as f64 / (1024.0 * 1024.0);
        eprintln!("[parse:{label:<13}] files={n:<5} with_errors={bad:<4} errors={errs:<6} {mib:.1} MiB in {:.1} ms ({:.0} MiB/s)", secs * 1e3, mib / secs.max(1e-9));
        if budget.truncated {
            break;
        }
    }
    let mut top: Vec<_> = contexts.into_iter().collect();
    top.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    for (k, (c, ex)) in top.iter().take(40) {
        eprintln!("  {c:>6}  {k:<40} e.g. {ex}");
    }
    let clean = 100.0 * (tn - tbad) as f64 / tn.max(1) as f64;
    eprintln!("[parse:TOTAL] files={tn} clean={clean:.2}% truncated={}", budget.truncated);
}

#[test]
fn corpus_guard_skips_symlinks_and_enforces_file_and_time_caps() {
    assert!(validate_explicit_root(Path::new("")).is_none());
    assert!(validate_explicit_root(Path::new("/")).is_none());
    assert!(validate_explicit_root(Path::new("/usr")).is_none());
    assert!(validate_explicit_root(Path::new("/tmp")).is_none());
    if let Some(home) = std::env::var_os("HOME") {
        assert!(validate_explicit_root(Path::new(&home)).is_none());
    }

    let tmp = std::env::temp_dir().join(format!("ridx_corpus_guard_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("sub")).expect("create temp corpus dir");
    for i in 0..5 {
        std::fs::write(tmp.join("sub").join(format!("f{i}.rs")), format!("pub fn f{i}() -> usize {{ {i} }}\n")).expect("write rs file");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&tmp, tmp.join("sub").join("self_loop")).expect("create loop symlink");
    }

    let canon = validate_explicit_root(&tmp).expect("explicit temp sub-root must be valid");
    let mut cap_budget = CorpusBudget::with_limits(3, Duration::from_secs(5));
    let mut capped_files = Vec::new();
    collect_bounded_rs_files(&canon, true, &mut cap_budget, &mut capped_files);
    assert_eq!(capped_files.len(), 3);
    assert!(cap_budget.truncated);

    let mut full_budget = CorpusBudget::with_limits(50, Duration::from_secs(5));
    let mut all_files = Vec::new();
    collect_bounded_rs_files(&canon, true, &mut full_budget, &mut all_files);
    assert_eq!(all_files.len(), 5);
    assert!(!full_budget.truncated);

    let mut expired_budget = CorpusBudget::with_limits(50, Duration::ZERO);
    let mut expired_files = Vec::new();
    collect_bounded_rs_files(&canon, true, &mut expired_budget, &mut expired_files);
    assert!(expired_files.is_empty());
    assert!(expired_budget.truncated);

    let _ = std::fs::remove_dir_all(&tmp);
}
