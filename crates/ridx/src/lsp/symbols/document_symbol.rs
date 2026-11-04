/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `textDocument/documentSymbol` handler with on-disk fallback and `(path, mtime, len)` cache.

use super::document_symbol_tree::document_symbols;
use crate::lsp::protocol::{uri_to_path, PositionConverter};
use crate::lsp::symbols::disk_cache::DocumentSymbolDiskCache;
use crate::overlay::OverlayManager;
use serde_json::{json, Value};

/// Parses `DocumentSymbol[]` from disk for a closed file, using `cache` keyed by `(mtime, len)`
/// to avoid re-reading and re-parsing unchanged files across repeated requests.
///
/// If `std::fs::metadata` or `std::fs::read_to_string` fails (or the path is not a regular file),
/// any stale entry for `file` is evicted from `cache`.
fn document_symbols_from_disk(file: &str, cache: &DocumentSymbolDiskCache) -> Value {
    let Ok(meta) = std::fs::metadata(file) else {
        cache.remove(file);
        return json!([]);
    };
    if !meta.is_file() {
        cache.remove(file);
        return json!([]);
    }

    let mtime = meta.modified().ok();
    let len = meta.len();

    if let Some(cached) = cache.get(file, mtime, len) {
        return cached;
    }

    let Ok(text) = std::fs::read_to_string(file) else {
        cache.remove(file);
        return json!([]);
    };

    let symbols = document_symbols(&text);

    cache.insert(file.to_owned(), mtime, len, symbols.clone());

    symbols
}

/// Handles `textDocument/documentSymbol` from the active `OverlayManager` buffer when open,
/// or falls back to parsing the on-disk file with a `(path, mtime, len)` cache when closed.
pub fn handle_document_symbol(msg: &Value, overlay_mgr: &OverlayManager, _conv: &mut PositionConverter) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    if uri.is_empty() {
        return json!([]);
    }
    let file = uri_to_path(uri);
    if file.is_empty() {
        return json!([]);
    }

    if let Some(overlay) = overlay_mgr.get(&file) {
        return document_symbols(&overlay.text);
    }

    document_symbols_from_disk(&file, overlay_mgr.doc_symbol_cache())
}

#[cfg(test)]
pub(crate) fn document_symbol_cache_hits_for(overlay_mgr: &OverlayManager, file_path: &str) -> Option<usize> {
    overlay_mgr.doc_symbol_cache().hits_for(file_path)
}

#[cfg(test)]
pub(crate) fn document_symbol_cache_len(overlay_mgr: &OverlayManager) -> usize {
    overlay_mgr.doc_symbol_cache().len()
}
