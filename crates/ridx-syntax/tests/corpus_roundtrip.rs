/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Lossless lexer round-trip over explicitly configured corpus roots (`RIDX_CORPUS_ROOT` or `RIDX_BENCH_REPOS`)
//! within strict file-count and wall-clock limits. Skips immediately when no root env var is set.

mod common;

use common::{collect_bounded_rs_files, validate_explicit_root, CorpusBudget, SEVEN_CORPORA};
use ridx_syntax::{lex, token_at};
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn verify_file_roundtrip(path: &Path, src: &str) -> usize {
    let tokens = lex(src);
    if src.is_empty() {
        assert!(tokens.is_empty(), "{}: expected empty tokens", path.display());
        return 0;
    }
    assert_eq!(tokens[0].start, 0, "{}: start != 0", path.display());
    let mut rebuilt = String::with_capacity(src.len());
    let mut prev_end = 0u32;
    for (idx, tok) in tokens.iter().enumerate() {
        assert_eq!(tok.start, prev_end, "{}: gap at token {idx}", path.display());
        assert!(tok.start < tok.end, "{}: zero-length token {idx}", path.display());
        if idx + 1 < tokens.len() {
            let expected_joint = tok.kind.is_punct() && tokens[idx + 1].kind.is_punct();
            assert_eq!(tok.is_joint(), expected_joint, "{}: JOINT_NEXT mismatch at token {idx}", path.display());
        } else {
            assert!(!tok.is_joint(), "{}: last token has JOINT_NEXT", path.display());
        }
        rebuilt.push_str(tok.text(src));
        prev_end = tok.end;
    }
    assert_eq!(prev_end as usize, src.len(), "{}: end != len", path.display());
    assert_eq!(rebuilt, src, "{}: round-trip text mismatch", path.display());

    assert_eq!(token_at(&tokens, 0), Some(0));
    let mid = (src.len() / 2) as u32;
    let mid_idx = token_at(&tokens, mid).expect("mid byte must map to token");
    assert!(tokens[mid_idx].start <= mid && mid < tokens[mid_idx].end);
    assert_eq!(token_at(&tokens, (src.len() - 1) as u32), Some(tokens.len() - 1));
    assert_eq!(token_at(&tokens, src.len() as u32), None);

    tokens.len()
}

#[test]
fn explicit_corpus_root_roundtrip() {
    let Ok(raw_root) = std::env::var("RIDX_CORPUS_ROOT") else {
        eprintln!("[roundtrip:skip] set RIDX_CORPUS_ROOT to run corpus round-trip test");
        return;
    };
    let Some(canon_root) = validate_explicit_root(Path::new(raw_root.trim())) else {
        eprintln!("[roundtrip:skip] invalid or forbidden RIDX_CORPUS_ROOT");
        return;
    };
    let mut budget = CorpusBudget::from_env();
    let mut files = Vec::new();
    collect_bounded_rs_files(&canon_root, false, &mut budget, &mut files);

    let mut checked = 0usize;
    let mut total_bytes = 0usize;
    let mut total_tokens = 0usize;
    for file in &files {
        if Instant::now() >= budget.deadline {
            budget.truncated = true;
            break;
        }
        let Ok(src) = fs::read_to_string(file) else { continue };
        checked += 1;
        total_bytes += src.len();
        total_tokens += verify_file_roundtrip(file, &src);
    }
    eprintln!("[ridx-syntax] round-trip: {checked} files, {total_bytes} bytes, {total_tokens} tokens, truncated={}", budget.truncated);
}

#[test]
fn seven_corpora_roundtrip_and_bevy_throughput() {
    let Ok(bench_root) = std::env::var("RIDX_BENCH_REPOS") else {
        return;
    };
    let bench_root = PathBuf::from(bench_root.trim());
    if bench_root.as_os_str().is_empty() {
        return;
    }

    let mut budget = CorpusBudget::from_env();
    let (mut grand_files, mut grand_non_utf8, mut grand_bytes, mut grand_tokens) = (0usize, 0usize, 0usize, 0usize);
    let mut bevy_sources: Vec<String> = Vec::new();

    for &corpus in SEVEN_CORPORA {
        if budget.exhausted() {
            break;
        }
        let Some(dir) = validate_explicit_root(&bench_root.join(corpus)) else {
            continue;
        };
        let mut files = Vec::new();
        collect_bounded_rs_files(&dir, false, &mut budget, &mut files);
        let (mut c_utf8_files, mut c_non_utf8, mut c_bytes, mut c_tokens) = (0usize, 0usize, 0usize, 0usize);

        for path in &files {
            if Instant::now() >= budget.deadline {
                budget.truncated = true;
                break;
            }
            let Ok(raw) = fs::read(path) else { continue };
            match String::from_utf8(raw) {
                Ok(src) => {
                    c_utf8_files += 1;
                    c_bytes += src.len();
                    c_tokens += verify_file_roundtrip(path, &src);
                    if corpus == "bevy" {
                        bevy_sources.push(src);
                    }
                }
                Err(_) => {
                    c_non_utf8 += 1;
                }
            }
        }
        eprintln!("[corpus:{corpus:<14}] utf8_files={c_utf8_files:<6} non_utf8={c_non_utf8:<2} bytes={c_bytes:<10} tokens={c_tokens}");
        grand_files += c_utf8_files;
        grand_non_utf8 += c_non_utf8;
        grand_bytes += c_bytes;
        grand_tokens += c_tokens;
        if budget.truncated {
            break;
        }
    }

    eprintln!("[corpora:TOTAL] utf8_files={grand_files} non_utf8={grand_non_utf8} bytes={grand_bytes} tokens={grand_tokens} truncated={}", budget.truncated);
    if bevy_sources.is_empty() || Instant::now() >= budget.deadline {
        return;
    }

    let bevy_bytes: usize = bevy_sources.iter().map(|s| s.len()).sum();
    for src in &bevy_sources {
        black_box(lex(black_box(src.as_str())));
    }
    let mut iters = 0usize;
    let t0 = Instant::now();
    let mut tok_sum = 0usize;
    while iters < 5 && Instant::now() < budget.deadline {
        for src in &bevy_sources {
            let toks = lex(black_box(src.as_str()));
            tok_sum += toks.len();
            black_box(toks);
        }
        iters += 1;
    }
    if iters > 0 {
        let secs = t0.elapsed().as_secs_f64().max(1e-9);
        let total_mb = (bevy_bytes as f64 * iters as f64) / (1024.0 * 1024.0);
        eprintln!(
            "[bench:bevy] files={} iters={iters} elapsed={:.2} ms throughput={:.1} MiB/s tokens_per_iter={}",
            bevy_sources.len(),
            secs * 1000.0,
            total_mb / secs,
            tok_sum / iters
        );
    }
}
