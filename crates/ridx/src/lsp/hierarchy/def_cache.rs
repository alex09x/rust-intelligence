/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Definition caching and formatting helpers for type hierarchy navigation.

use crate::lsp::protocol::{lsp_symbol_kind, path_to_uri};
use ridx_format::{DefKind, WorkspaceIndex, WorkspaceSymbol};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

#[derive(Clone, Debug)]
pub struct CachedDef {
    pub uri: String,
    pub range: Value,
    pub kind: u32,
}

impl CachedDef {
    pub fn to_json(&self, name: &str) -> Value {
        json!({
            "name": name,
            "kind": self.kind,
            "uri": self.uri,
            "range": self.range,
            "selectionRange": self.range,
        })
    }
}

pub fn fallback_json(name: &str, kind: u32, file_path: &Path, impl_line: u32) -> Value {
    let uri = path_to_uri(file_path.to_str().unwrap_or(""));
    let line = impl_line.saturating_sub(1);
    let r = json!({ "start": { "line": line, "character": 0 }, "end": { "line": line, "character": 0 } });
    json!({ "name": name, "kind": kind, "uri": uri, "range": r, "selectionRange": r })
}

pub fn make_cached_def(sym: &WorkspaceSymbol, kind: u32) -> CachedDef {
    let p = sym.resolved_path();
    let s = p.to_str().unwrap_or(sym.file);
    let range = json!({
        "start": { "line": sym.span.start_line().saturating_sub(1), "character": sym.span.start_col().saturating_sub(1) },
        "end": { "line": sym.span.end_line().saturating_sub(1), "character": sym.span.end_col().saturating_sub(1) }
    });
    CachedDef { uri: path_to_uri(s), range, kind }
}

pub fn find_first_trait_def(ws: &WorkspaceIndex, trait_base: &str) -> Option<CachedDef> {
    ws.find_first_symbol(trait_base, |d| d.name == trait_base && d.kind == DefKind::Trait).map(|t| make_cached_def(&t, 11))
}

pub fn find_first_target_def(ws: &WorkspaceIndex, target_base: &str) -> Option<CachedDef> {
    ws.find_first_symbol(target_base, |d| d.name == target_base && matches!(d.kind, DefKind::Struct | DefKind::Enum)).map(|t| make_cached_def(&t, lsp_symbol_kind(t.kind)))
}

pub fn resolve_def<F>(cache: &RwLock<HashMap<Box<str>, Option<CachedDef>>>, name: &str, kind: u32, file: &Path, line: u32, find_fn: F) -> Value
where
    F: FnOnce() -> Option<CachedDef>,
{
    if let Ok(guard) = cache.read() {
        if let Some(cached) = guard.get(name) {
            return match cached {
                Some(cd) => cd.to_json(name),
                None => fallback_json(name, kind, file, line),
            };
        }
    }
    let opt_def = find_fn();
    let result = match &opt_def {
        Some(cd) => cd.to_json(name),
        None => fallback_json(name, kind, file, line),
    };
    if let Ok(mut guard) = cache.write() {
        guard.insert(name.into(), opt_def);
    }
    result
}
