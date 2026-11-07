/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use ridx_syntax::{lex, token_at};
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

const SEVEN_CORPORA: &[&str] = &["serde", "ripgrep", "tokio", "rust-analyzer", "bevy", "clap", "regex"];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == ".git" || name == "target" {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

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

    // Spot-check binary search at beginning, middle, and end of the file.
    assert_eq!(token_at(&tokens, 0), Some(0));
    let mid = (src.len() / 2) as u32;
    let mid_idx = token_at(&tokens, mid).expect("mid byte must map to token");
    assert!(tokens[mid_idx].start <= mid && mid < tokens[mid_idx].end);
    assert_eq!(token_at(&tokens, (src.len() - 1) as u32), Some(tokens.len() - 1));
    assert_eq!(token_at(&tokens, src.len() as u32), None);

    tokens.len()
}

#[test]
fn workspace_self_roundtrip() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ws_root = manifest_dir.parent().unwrap().parent().unwrap();
    let mut files = Vec::new();
    collect_rs_files(ws_root, &mut files);
    assert!(!files.is_empty(), "expected .rs files in workspace");

    let mut total_bytes = 0usize;
    let mut total_tokens = 0usize;
    for file in &files {
        let src = fs::read_to_string(file).unwrap_or_else(|e| panic!("failed to read {}: {e}", file.display()));
        total_bytes += src.len();
        total_tokens += verify_file_roundtrip(file, &src);
    }
    eprintln!("[ridx-syntax] workspace round-trip: {} files, {} bytes, {} tokens", files.len(), total_bytes, total_tokens);
}

#[test]
fn seven_corpora_roundtrip_and_bevy_throughput() {
    let Ok(bench_root) = std::env::var("RIDX_BENCH_REPOS") else {
        return;
    };
    let bench_root = PathBuf::from(bench_root);

    let mut grand_files = 0usize;
    let mut grand_non_utf8 = 0usize;
    let mut grand_bytes = 0usize;
    let mut grand_tokens = 0usize;
    let mut bevy_sources: Vec<String> = Vec::new();

    for &corpus in SEVEN_CORPORA {
        let dir = bench_root.join(corpus);
        assert!(dir.is_dir(), "missing corpus directory: {}", dir.display());
        let mut files = Vec::new();
        collect_rs_files(&dir, &mut files);
        assert!(!files.is_empty(), "no .rs files found in {}", dir.display());

        let mut c_utf8_files = 0usize;
        let mut c_non_utf8 = 0usize;
        let mut c_bytes = 0usize;
        let mut c_tokens = 0usize;

        for path in &files {
            let raw = fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
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
    }

    eprintln!("[corpora:TOTAL] utf8_files={grand_files} non_utf8={grand_non_utf8} bytes={grand_bytes} tokens={grand_tokens}");

    // Benchmark pure in-memory lexing throughput on bevy across 5 iterations after 1 warmup pass.
    let bevy_bytes: usize = bevy_sources.iter().map(|s| s.len()).sum();
    for src in &bevy_sources {
        black_box(lex(black_box(src.as_str())));
    }
    let iters = 5usize;
    let t0 = Instant::now();
    let mut tok_sum = 0usize;
    for _ in 0..iters {
        for src in &bevy_sources {
            let toks = lex(black_box(src.as_str()));
            tok_sum += toks.len();
            black_box(toks);
        }
    }
    let elapsed = t0.elapsed();
    let total_mb = (bevy_bytes as f64 * iters as f64) / (1024.0 * 1024.0);
    let total_decimal_mb = (bevy_bytes as f64 * iters as f64) / 1_000_000.0;
    let secs = elapsed.as_secs_f64();
    eprintln!(
        "[bench:bevy] files={} size={:.2} MiB ({:.2} MB) iters={} elapsed={:.2} ms ({:.2} ms/iter) throughput={:.1} MiB/s ({:.1} MB/s) tokens_per_iter={}",
        bevy_sources.len(),
        bevy_bytes as f64 / (1024.0 * 1024.0),
        bevy_bytes as f64 / 1_000_000.0,
        iters,
        secs * 1000.0,
        (secs * 1000.0) / iters as f64,
        total_mb / secs,
        total_decimal_mb / secs,
        tok_sum / iters,
    );
}
