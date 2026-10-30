/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Per-session `(path, mtime, len)` cache for closed-file `textDocument/documentSymbol` responses.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

const MAX_DOCUMENT_SYMBOL_CACHE_ENTRIES: usize = 256;

/// Cached `DocumentSymbol[]` response keyed by `(mtime, len)` metadata from `std::fs::metadata`.
///
/// # Cache Key Invariants and Trade-Offs
/// - Checking `(mtime, len)` requires a single `stat` syscall without reading or hashing the file
///   contents on repeated `textDocument/documentSymbol` requests for closed files.
/// - Open editor buffers always take precedence via [`crate::overlay::OverlayManager::get`] and
///   never consult this cache.
/// - Limitation: an external in-place disk rewrite of a closed file that preserves the exact byte
///   length (`len`) and completes within the filesystem's `mtime` resolution tick (sub-nanosecond
///   on modern APFS/ext4/XFS, coarser on legacy or network filesystems) can collide on `(mtime, len)`.
#[derive(Clone)]
struct CachedDocumentSymbols {
    mtime: Option<SystemTime>,
    len: u64,
    symbols: Value,
    #[cfg(test)]
    hits: usize,
}

/// Session-scoped cache of parsed `DocumentSymbol[]` JSON payloads for closed files on disk.
///
/// Owned by [`crate::overlay::OverlayManager`] so each LSP server session and test harness
/// maintains isolated cache state without cross-session contention or shared process globals.
/// Note: Payload byte size is not bounded and there is no LRU ordering; the map is cleared
/// when inserting a new path at `MAX_DOCUMENT_SYMBOL_CACHE_ENTRIES` (256 entries).
#[derive(Default)]
pub struct DocumentSymbolDiskCache {
    entries: Mutex<HashMap<String, CachedDocumentSymbols>>,
}

impl DocumentSymbolDiskCache {
    pub fn new() -> Self {
        Self { entries: Mutex::new(HashMap::new()) }
    }

    pub(crate) fn get(&self, file: &str, mtime: Option<SystemTime>, len: u64) -> Option<Value> {
        let mut cache = self.entries.lock().ok()?;
        let entry = cache.get_mut(file)?;
        if entry.mtime == mtime && entry.len == len {
            #[cfg(test)]
            {
                entry.hits += 1;
            }
            Some(entry.symbols.clone())
        } else {
            None
        }
    }

    pub(crate) fn insert(&self, file: String, mtime: Option<SystemTime>, len: u64, symbols: Value) {
        if let Ok(mut cache) = self.entries.lock() {
            if cache.len() >= MAX_DOCUMENT_SYMBOL_CACHE_ENTRIES && !cache.contains_key(&file) {
                cache.clear();
            }
            cache.insert(
                file,
                CachedDocumentSymbols {
                    mtime,
                    len,
                    symbols,
                    #[cfg(test)]
                    hits: 0,
                },
            );
        }
    }

    pub(crate) fn remove(&self, file: &str) {
        if let Ok(mut cache) = self.entries.lock() {
            cache.remove(file);
        }
    }

    #[cfg(test)]
    pub(crate) fn hits_for(&self, file_path: &str) -> Option<usize> {
        self.entries.lock().ok().and_then(|cache| cache.get(file_path).map(|entry| entry.hits))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.lock().map(|cache| cache.len()).unwrap_or(0)
    }
}
