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

use crate::lsp::protocol::{lsp_symbol_kind, uri_to_path, PositionConverter};
use crate::overlay::{DocumentOverlay, OverlayManager};
use ridx_format::DefKind;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

const MAX_DOCUMENT_SYMBOL_CACHE_ENTRIES: usize = 256;

#[derive(Clone)]
struct CachedDocumentSymbols {
    mtime: Option<SystemTime>,
    len: u64,
    symbols: Value,
    #[cfg(test)]
    hits: usize,
}

static DOC_SYMBOL_CACHE: LazyLock<Mutex<HashMap<String, CachedDocumentSymbols>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Formats parsed overlay symbols into the LSP `DocumentSymbol[]` JSON array.
fn format_overlay_symbols(overlay: &DocumentOverlay) -> Value {
    let mut results = Vec::new();
    for s in &overlay.symbols {
        if s.kind == DefKind::TyParam {
            continue;
        }
        let start_char = overlay.char_col_to_utf16_col(s.start_line, s.start_col);
        let end_char = overlay.char_col_to_utf16_col(s.end_line, s.end_col);
        let range = json!({
            "start": { "line": s.start_line.saturating_sub(1), "character": start_char },
            "end": { "line": s.end_line.saturating_sub(1), "character": end_char }
        });
        results.push(json!({
            "name": s.name,
            "kind": lsp_symbol_kind(s.kind),
            "range": range,
            "selectionRange": range,
            "detail": s.signature
        }));
    }
    json!(results)
}

fn evict_cached_symbols(file: &str) {
    if let Ok(mut cache) = DOC_SYMBOL_CACHE.lock() {
        cache.remove(file);
    }
}

fn document_symbols_from_disk(file: &str) -> Value {
    let Ok(meta) = std::fs::metadata(file) else {
        evict_cached_symbols(file);
        return json!([]);
    };
    if !meta.is_file() {
        evict_cached_symbols(file);
        return json!([]);
    }

    let mtime = meta.modified().ok();
    let len = meta.len();

    if let Ok(mut cache) = DOC_SYMBOL_CACHE.lock() {
        if let Some(entry) = cache.get_mut(file) {
            if entry.mtime == mtime && entry.len == len {
                #[cfg(test)]
                {
                    entry.hits += 1;
                }
                return entry.symbols.clone();
            }
        }
    }

    let Ok(text) = std::fs::read_to_string(file) else {
        evict_cached_symbols(file);
        return json!([]);
    };

    let overlay = DocumentOverlay::parse_detached(text);
    let symbols = format_overlay_symbols(&overlay);

    if let Ok(mut cache) = DOC_SYMBOL_CACHE.lock() {
        if cache.len() >= MAX_DOCUMENT_SYMBOL_CACHE_ENTRIES && !cache.contains_key(file) {
            cache.clear();
        }
        cache.insert(
            file.to_owned(),
            CachedDocumentSymbols {
                mtime,
                len,
                symbols: symbols.clone(),
                #[cfg(test)]
                hits: 0,
            },
        );
    }

    symbols
}

/// Handles `textDocument/documentSymbol` from the active `OverlayManager` buffer when open,
/// or falls back to parsing the on-disk file with a `(path, mtime, len)` cache when closed.
pub fn handle_document_symbol(
    msg: &Value,
    overlay_mgr: &OverlayManager,
    _conv: &mut PositionConverter,
) -> Value {
    let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap_or("");
    if uri.is_empty() {
        return json!([]);
    }
    let file = uri_to_path(uri);
    if file.is_empty() {
        return json!([]);
    }

    if let Some(overlay) = overlay_mgr.get(&file) {
        return format_overlay_symbols(overlay);
    }

    document_symbols_from_disk(&file)
}

#[cfg(test)]
pub(crate) fn document_symbol_cache_hits_for(file_path: &str) -> Option<usize> {
    DOC_SYMBOL_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(file_path).map(|entry| entry.hits))
}
