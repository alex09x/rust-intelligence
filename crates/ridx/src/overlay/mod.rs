/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Fast in-memory document overlay for live buffer indexing (Variant B / As-You-Type).
//!
//! Maintains an error-tolerant, pure-Rust AST and lexical scope index for open,
//! unsaved editor buffers. Resolves local variables, function parameters, and newly
//! declared items within milliseconds, even when the buffer contains incomplete syntax
//! or compilation errors.

pub mod imports;
pub mod line_map;
pub mod manager;
pub mod mapping;
pub mod parser;
pub mod position;
pub mod query;
pub mod scan;

#[allow(unused_imports)]
pub use imports::*;
#[allow(unused_imports)]
pub use line_map::*;
#[allow(unused_imports)]
pub use manager::*;
#[allow(unused_imports)]
pub use mapping::*;
#[allow(unused_imports)]
pub use parser::*;
#[allow(unused_imports)]
pub use position::*;
#[allow(unused_imports)]
pub use query::*;
#[allow(unused_imports)]
pub use scan::*;

use parser::{OverlayParser, Scope};
use ridx_format::{normalized_source_hash, DefKind};

/// Representation of a symbol in the live document overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlaySymbol {
    pub name: String,
    pub kind: DefKind,
    pub signature: Option<String>,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub scope_id: usize,
    pub decl_start_byte: usize,
    pub decl_end_byte: usize,
    pub scope_start_byte: usize,
}

impl OverlaySymbol {
    pub fn to_definition(&self, file: &str) -> OverlayDefinition {
        OverlayDefinition {
            name: self.name.clone(),
            kind: self.kind,
            signature: self.signature.clone(),
            file: file.to_string(),
            start_line: self.start_line,
            start_col: self.start_col,
            end_line: self.end_line,
            end_col: self.end_col,
        }
    }
}

/// Representation of a resolved definition within the live buffer overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayDefinition {
    pub name: String,
    pub kind: DefKind,
    pub signature: Option<String>,
    pub file: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// In-memory overlay for a single active document buffer.
pub struct DocumentOverlay {
    #[allow(dead_code)]
    pub uri: String,
    pub file_path: String,
    pub version: i32,
    pub content_hash: u64,
    pub text: String,
    pub compiled_text: Option<String>,
    pub disk_text: Option<String>,
    /// The index recorded no source hash for this file (a std or dependency source): its text on disk is what was
    /// indexed. A workspace file whose hash does not match is stale instead, and has no baseline.
    pub unhashed_source: bool,
    pub line_starts: Vec<usize>,
    pub symbols: Vec<OverlaySymbol>,
    pub(crate) scopes: Vec<Scope>,
    /// Comment and string/char literal byte spans, in text order.
    pub(crate) opaque: Vec<(usize, usize)>,
    /// Incremental dirty-function tracker and local type inference cache (`Checkbox 4.1`).
    pub dirty_tracker: ridx_syntax::ty_infer::DirtyFileTracker,
}

/// Resolves the compiled baseline text by checking whether the live buffer hash or on-disk file hash matches `expected_hash`.
pub(crate) fn resolve_compiled_text(content_hash: u64, text: &str, disk_text: Option<&str>, expected_hash: Option<u64>) -> Option<String> {
    let expected = expected_hash?;
    if content_hash == expected {
        Some(text.to_owned())
    } else {
        let disk = disk_text?;
        (normalized_source_hash(disk.as_bytes()) == expected).then(|| disk.to_owned())
    }
}

impl DocumentOverlay {
    /// Parses source text into a detached overlay without disk I/O or baseline cloning.
    pub fn parse_detached(text: String) -> Self {
        let mut overlay = Self {
            uri: String::new(),
            file_path: String::new(),
            version: 0,
            content_hash: 0,
            text,
            compiled_text: None,
            disk_text: None,
            unhashed_source: false,
            line_starts: Vec::new(),
            symbols: Vec::new(),
            scopes: Vec::new(),
            opaque: Vec::new(),
            dirty_tracker: ridx_syntax::ty_infer::DirtyFileTracker::new(),
        };
        overlay.reparse();
        overlay
    }

    /// Creates and parses a new DocumentOverlay with explicit provenance tracking.
    pub fn new_with_provenance(uri: impl Into<String>, file_path: impl Into<String>, version: i32, text: String, is_compiled: bool) -> Self {
        let uri = uri.into();
        let file_path = file_path.into();
        let content_hash = normalized_source_hash(text.as_bytes());
        let disk_text = std::fs::read_to_string(&file_path).ok();
        let compiled_text = if is_compiled { Some(text.clone()) } else { None };
        let mut dirty_tracker = ridx_syntax::ty_infer::DirtyFileTracker::new();
        if let Some(ref compiled) = compiled_text {
            dirty_tracker.set_baseline(compiled);
        }
        let mut overlay = Self {
            uri,
            file_path,
            version,
            content_hash,
            text,
            compiled_text,
            disk_text,
            unhashed_source: false,
            line_starts: Vec::new(),
            symbols: Vec::new(),
            scopes: Vec::new(),
            opaque: Vec::new(),
            dirty_tracker,
        };
        overlay.reparse();
        overlay
    }

    /// Creates and parses a new DocumentOverlay resolving `compiled_text` from buffer or disk against `expected_hash`.
    pub fn new_with_source_hash(uri: impl Into<String>, file_path: impl Into<String>, version: i32, text: String, expected_hash: Option<u64>) -> Self {
        let uri = uri.into();
        let file_path = file_path.into();
        let content_hash = normalized_source_hash(text.as_bytes());
        let disk_text = std::fs::read_to_string(&file_path).ok();
        let compiled_text = resolve_compiled_text(content_hash, &text, disk_text.as_deref(), expected_hash);
        let mut dirty_tracker = ridx_syntax::ty_infer::DirtyFileTracker::new();
        if let Some(ref compiled) = compiled_text {
            dirty_tracker.set_baseline(compiled);
        }
        let mut overlay = Self {
            uri,
            file_path,
            version,
            content_hash,
            text,
            compiled_text,
            disk_text,
            unhashed_source: expected_hash.is_none(),
            line_starts: Vec::new(),
            symbols: Vec::new(),
            scopes: Vec::new(),
            opaque: Vec::new(),
            dirty_tracker,
        };
        overlay.reparse();
        overlay
    }

    /// Creates and parses a new DocumentOverlay from buffer text assuming compiled baseline.
    #[allow(dead_code)]
    pub fn new(uri: impl Into<String>, file_path: impl Into<String>, version: i32, text: String) -> Self {
        Self::new_with_provenance(uri, file_path, version, text, true)
    }

    /// Updates buffer text, version, and reparses symbols.
    pub fn update(&mut self, version: i32, text: String) {
        self.version = version;
        self.content_hash = normalized_source_hash(text.as_bytes());
        self.text = text;
        self.reparse();
    }

    /// Whether this buffer has unsaved or unindexed edits relative to compiled index.
    #[allow(dead_code)]
    pub fn is_dirty(&self) -> bool {
        self.compiled_text.as_deref() != Some(&self.text)
    }

    /// Whether this buffer matches the content currently saved on disk.
    #[allow(dead_code)]
    pub fn is_synced_with_disk(&self) -> bool {
        self.disk_text.as_deref() == Some(&self.text)
    }

    /// Advances the compiled index baseline when a newly built index is published.
    pub fn advance_compiled_baseline(&mut self, text: String) {
        self.dirty_tracker.set_baseline(&text);
        self.compiled_text = Some(text);
        if self.is_dirty() {
            self.dirty_tracker.on_did_change(&self.text);
        }
    }

    /// Returns telemetry from the most recent `didChange` dirty-function tracking pass (`Checkbox 4.1`).
    #[inline]
    pub fn dirty_fn_stats(&self) -> ridx_syntax::ty_infer::DirtyFnUpdateStats {
        self.dirty_tracker.last_stats()
    }

    /// Returns the cached inference result if `byte_offset` falls inside a dirty function body.
    #[inline]
    pub fn dirty_fn_at_offset(&self, byte_offset: u32) -> Option<&ridx_syntax::ty_infer::CachedDirtyFn> {
        self.dirty_tracker.dirty_fn_at_offset(byte_offset)
    }

    /// Returns the cached inference result if `(line_1based, col_1based)` falls inside a dirty function body.
    pub fn dirty_fn_at_position(&self, line: u32, col: u32) -> Option<&ridx_syntax::ty_infer::CachedDirtyFn> {
        let byte_off = self.position_to_byte_offset(line.saturating_sub(1), col.saturating_sub(1)) as u32;
        self.dirty_tracker.dirty_fn_at_offset(byte_off)
    }

    /// Rebuilds line boundary byte offsets.
    pub(crate) fn rebuild_line_starts(&mut self) {
        self.line_starts = mapping::compute_line_starts(&self.text);
    }

    /// Applies incremental or full LSP content changes in sequence and reparses the overlay.
    pub fn apply_content_changes(&mut self, version: i32, changes: &[serde_json::Value]) {
        if version > 0 {
            self.version = version;
        } else {
            self.version += 1;
        }
        for change in changes {
            if let Some(range) = change.get("range") {
                let Some(text_to_insert) = change.get("newText").or_else(|| change.get("text")).and_then(|t| t.as_str()) else {
                    continue;
                };
                let start_line = range.get("start").and_then(|p| p.get("line")).and_then(|l| l.as_u64()).unwrap_or(0) as u32;
                let start_char = range.get("start").and_then(|p| p.get("character")).and_then(|c| c.as_u64()).unwrap_or(0) as u32;
                let end_line = range.get("end").and_then(|p| p.get("line")).and_then(|l| l.as_u64()).unwrap_or(0) as u32;
                let end_char = range.get("end").and_then(|p| p.get("character")).and_then(|c| c.as_u64()).unwrap_or(0) as u32;

                let start_byte = self.position_to_byte_offset(start_line, start_char).min(self.text.len());
                let end_byte = self.position_to_byte_offset(end_line, end_char).min(self.text.len()).max(start_byte);

                let mut new_text = String::with_capacity(start_byte + text_to_insert.len() + (self.text.len() - end_byte));
                new_text.push_str(&self.text[..start_byte]);
                new_text.push_str(text_to_insert);
                new_text.push_str(&self.text[end_byte..]);
                self.text = new_text;
                self.rebuild_line_starts();
            } else if let Some(full_text) = change.get("newText").or_else(|| change.get("text")).and_then(|t| t.as_str()) {
                self.text = full_text.to_string();
                self.rebuild_line_starts();
            }
        }
        self.content_hash = normalized_source_hash(self.text.as_bytes());
        self.reparse();
    }

    /// Re-indexes line boundaries, lexical scopes, declarations, and dirty function bodies.
    fn reparse(&mut self) {
        // 1. Build line index
        self.rebuild_line_starts();

        // 2. Parse lexical scopes and symbols using error-tolerant token scanner
        let mut parser = OverlayParser::new(&self.text, &self.line_starts);
        parser.parse();
        self.scopes = parser.scopes;
        self.symbols = parser.symbols;
        self.opaque = parser.opaque;

        // 3. Incrementally reparse CST and infer only modified function bodies (Checkbox 4.1)
        if self.dirty_tracker.has_baseline() {
            if self.is_dirty() || !self.dirty_tracker.dirty_fns().is_empty() {
                self.dirty_tracker.on_did_change(&self.text);
            }
        } else if (!self.uri.is_empty() || !self.file_path.is_empty()) && self.is_dirty() {
            self.dirty_tracker.on_did_change(&self.text);
        }
    }
}

#[cfg(test)]
pub mod tests;
