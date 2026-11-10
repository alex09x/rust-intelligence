/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Shared bounded corpus discovery for `ridx-syntax` integration tests.
//! Never traverses outside an explicit environment-provided root, never follows symlinks,
//! and enforces both file-count and wall-clock budgets.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const SEVEN_CORPORA: &[&str] = &["serde", "ripgrep", "tokio", "rust-analyzer", "bevy", "clap", "regex"];

const DEFAULT_MAX_FILES: usize = 10_000;
const HARD_MAX_FILES: usize = 25_000;
const DEFAULT_TIMEOUT_SECS: u64 = 25;
const HARD_TIMEOUT_SECS: u64 = 60;
const MAX_DIR_DEPTH: usize = 24;

pub struct CorpusBudget {
    pub max_files: usize,
    pub deadline: Instant,
    pub files_seen: usize,
    pub truncated: bool,
}

impl CorpusBudget {
    pub fn from_env() -> Self {
        let max_files = std::env::var("RIDX_CORPUS_MAX_FILES").ok().and_then(|v| v.parse::<usize>().ok()).map_or(DEFAULT_MAX_FILES, |n| n.clamp(1, HARD_MAX_FILES));
        let timeout_secs = std::env::var("RIDX_CORPUS_TIMEOUT_SECS").ok().and_then(|v| v.parse::<u64>().ok()).map_or(DEFAULT_TIMEOUT_SECS, |s| s.clamp(1, HARD_TIMEOUT_SECS));
        Self::with_limits(max_files, Duration::from_secs(timeout_secs))
    }

    pub fn with_limits(max_files: usize, timeout: Duration) -> Self {
        Self { max_files, deadline: Instant::now() + timeout, files_seen: 0, truncated: false }
    }

    #[inline]
    pub fn exhausted(&mut self) -> bool {
        if self.files_seen >= self.max_files || Instant::now() >= self.deadline {
            self.truncated = true;
            true
        } else {
            false
        }
    }
}

pub fn validate_explicit_root(raw: &Path) -> Option<PathBuf> {
    if raw.as_os_str().is_empty() {
        return None;
    }
    let canon = fs::canonicalize(raw).ok()?;
    if !canon.is_dir() || canon.parent().is_none() {
        return None;
    }
    for forbidden in ["/usr", "/lib", "/lib64", "/etc", "/proc", "/sys", "/dev", "/var", "/System", "/Library"] {
        let fp = Path::new(forbidden);
        if canon == fp || canon.starts_with(fp) {
            return None;
        }
    }
    if canon == Path::new("/tmp") || canon == Path::new("/private/tmp") {
        return None;
    }
    if let Some(home) = std::env::var_os("HOME").and_then(|h| fs::canonicalize(h).ok()) {
        if canon == home {
            return None;
        }
    }
    Some(canon)
}

pub fn corpus_roots_from_env() -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();
    if let Ok(single) = std::env::var("RIDX_CORPUS_ROOT") {
        let trimmed = single.trim();
        if !trimmed.is_empty() {
            if let Some(canon) = validate_explicit_root(Path::new(trimmed)) {
                roots.push(("corpus_root".to_string(), canon));
            }
        }
    }
    if let Ok(base) = std::env::var("RIDX_BENCH_REPOS") {
        let trimmed = base.trim();
        if !trimmed.is_empty() {
            let base_path = Path::new(trimmed);
            for &c in SEVEN_CORPORA {
                if let Some(canon) = validate_explicit_root(&base_path.join(c)) {
                    roots.push((c.to_string(), canon));
                }
            }
        }
    }
    roots
}

pub fn collect_bounded_rs_files(canon_root: &Path, skip_test_data: bool, budget: &mut CorpusBudget, out: &mut Vec<PathBuf>) {
    if !canon_root.is_absolute() || canon_root.parent().is_none() || budget.exhausted() {
        return;
    }
    let mut stack: Vec<(PathBuf, usize)> = vec![(canon_root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if budget.exhausted() {
            break;
        }
        if depth > MAX_DIR_DEPTH || !dir.starts_with(canon_root) {
            continue;
        }
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for p in entries {
            if budget.exhausted() {
                return;
            }
            if !p.starts_with(canon_root) {
                continue;
            }
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if matches!(name, ".git" | "target" | "node_modules") || (skip_test_data && name == "test_data") {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push((p, depth + 1));
            } else if meta.is_file() && p.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(p);
                budget.files_seen += 1;
            }
        }
    }
}
