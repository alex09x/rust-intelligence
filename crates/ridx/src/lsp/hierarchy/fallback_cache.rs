/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! In-memory bounded cache for parsed impl lines during type hierarchy reference fallback.
//!
//! Cached entries and memoized query results persist for the lifetime of the current
//! ImplTable until the index generation increments on save or re-indexing.

use super::scan::{get_impl_line_from_file, parse_impl_line};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

/// Maximum number of cached line entries in fallback cache (~1.5 MB memory cap).
const MAX_FALLBACK_CACHE_ENTRIES: usize = 32768;

pub type CachedLineNames = (Option<Box<str>>, Option<Box<str>>);
pub type FallbackLinesMap = HashMap<(u32, u32), CachedLineNames>;

#[derive(Default)]
struct FallbackInner {
    file_ids: HashMap<Box<str>, u32>,
    lines: FallbackLinesMap,
    results: HashMap<(Box<str>, bool), Vec<Value>>,
}

pub struct FallbackCache {
    inner: RwLock<FallbackInner>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl Default for FallbackCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FallbackCache {
    pub fn new() -> Self {
        Self { inner: RwLock::new(FallbackInner::default()), hits: AtomicUsize::new(0), misses: AtomicUsize::new(0) }
    }

    pub fn with_prepopulated(file_ids: HashMap<Box<str>, u32>, lines: FallbackLinesMap) -> Self {
        Self { inner: RwLock::new(FallbackInner { file_ids, lines, results: HashMap::new() }), hits: AtomicUsize::new(0), misses: AtomicUsize::new(0) }
    }

    pub fn hit_stats(&self) -> (usize, usize) {
        (self.hits.load(Ordering::Relaxed), self.misses.load(Ordering::Relaxed))
    }

    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_or_parse(&self, file_path: &str, line_1based: u32, fallback: (Option<String>, Option<String>)) -> (Option<String>, Option<String>) {
        if let Ok(guard) = self.inner.read() {
            if let Some(&fid) = guard.file_ids.get(file_path) {
                if let Some((tr, tgt)) = guard.lines.get(&(fid, line_1based)) {
                    self.hits.fetch_add(1, Ordering::Relaxed);
                    return (tr.as_deref().map(Into::into), tgt.as_deref().map(Into::into));
                }
            }
        }

        self.misses.fetch_add(1, Ordering::Relaxed);

        let parsed = get_impl_line_from_file(Path::new(file_path), line_1based).map(|l| parse_impl_line(&l)).unwrap_or(fallback);

        if let Ok(mut guard) = self.inner.write() {
            if guard.lines.len() < MAX_FALLBACK_CACHE_ENTRIES {
                let fid = if let Some(&id) = guard.file_ids.get(file_path) {
                    id
                } else {
                    let id = guard.file_ids.len() as u32;
                    guard.file_ids.insert(file_path.into(), id);
                    id
                };

                let tr_box = parsed.0.as_deref().map(Into::into);
                let tgt_box = parsed.1.as_deref().map(Into::into);
                guard.lines.insert((fid, line_1based), (tr_box, tgt_box));
            }
        }

        parsed
    }

    pub fn get_fallback_results(&self, name: &str, is_supertype: bool) -> Option<Vec<Value>> {
        if let Ok(guard) = self.inner.read() {
            if let Some(res) = guard.results.get(&(name.into(), is_supertype)) {
                return Some(res.clone());
            }
        }
        None
    }

    pub fn insert_fallback_results(&self, name: &str, is_supertype: bool, results: Vec<Value>) {
        if let Ok(mut guard) = self.inner.write() {
            if guard.results.len() < 4096 {
                guard.results.insert((name.into(), is_supertype), results);
            }
        }
    }
}

impl Drop for FallbackCache {
    fn drop(&mut self) {
        if std::env::var_os("RIDX_LOG_FALLBACK_STATS").is_some() {
            eprintln!("[fallback_cache] hits={} misses={}", self.hits.load(Ordering::Relaxed), self.misses.load(Ordering::Relaxed));
        }
    }
}
