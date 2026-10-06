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

pub mod manager;
pub mod mapping;
pub mod parser;
pub mod position;
pub mod query;

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
    pub(crate) line_starts: Vec<usize>,
    pub(crate) symbols: Vec<OverlaySymbol>,
    pub(crate) scopes: Vec<Scope>,
}

impl DocumentOverlay {
    /// Creates and parses a new DocumentOverlay with explicit provenance tracking.
    pub fn new_with_provenance(
        uri: impl Into<String>,
        file_path: impl Into<String>,
        version: i32,
        text: String,
        is_compiled: bool,
    ) -> Self {
        let uri = uri.into();
        let file_path = file_path.into();
        let content_hash = normalized_source_hash(text.as_bytes());
        let disk_text = std::fs::read_to_string(&file_path).ok();
        let compiled_text = if is_compiled {
            Some(text.clone())
        } else {
            None
        };
        let mut overlay = Self {
            uri,
            file_path,
            version,
            content_hash,
            text,
            compiled_text,
            disk_text,
            line_starts: Vec::new(),
            symbols: Vec::new(),
            scopes: Vec::new(),
        };
        overlay.reparse();
        overlay
    }

    /// Creates and parses a new DocumentOverlay from buffer text assuming compiled baseline.
    #[allow(dead_code)]
    pub fn new(
        uri: impl Into<String>,
        file_path: impl Into<String>,
        version: i32,
        text: String,
    ) -> Self {
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
        self.compiled_text = Some(text);
    }

    /// Rebuilds line boundary byte offsets.
    pub(crate) fn rebuild_line_starts(&mut self) {
        self.line_starts = mapping::compute_line_starts(&self.text);
    }

    /// Applies incremental or full LSP content changes in sequence and reparses the overlay.
    pub fn apply_content_changes(&mut self, version: i32, changes: &[serde_json::Value]) {
        self.version = version;
        for change in changes {
            if let Some(range) = change.get("range") {
                let Some(text_to_insert) = change.get("text").and_then(|t| t.as_str()) else {
                    continue;
                };
                let start_line = range
                    .get("start")
                    .and_then(|p| p.get("line"))
                    .and_then(|l| l.as_u64())
                    .unwrap_or(0) as u32;
                let start_char = range
                    .get("start")
                    .and_then(|p| p.get("character"))
                    .and_then(|c| c.as_u64())
                    .unwrap_or(0) as u32;
                let end_line = range
                    .get("end")
                    .and_then(|p| p.get("line"))
                    .and_then(|l| l.as_u64())
                    .unwrap_or(0) as u32;
                let end_char = range
                    .get("end")
                    .and_then(|p| p.get("character"))
                    .and_then(|c| c.as_u64())
                    .unwrap_or(0) as u32;

                let start_byte = self
                    .position_to_byte_offset(start_line, start_char)
                    .min(self.text.len());
                let end_byte = self
                    .position_to_byte_offset(end_line, end_char)
                    .min(self.text.len())
                    .max(start_byte);

                let mut new_text = String::with_capacity(
                    start_byte + text_to_insert.len() + (self.text.len() - end_byte),
                );
                new_text.push_str(&self.text[..start_byte]);
                new_text.push_str(text_to_insert);
                new_text.push_str(&self.text[end_byte..]);
                self.text = new_text;
                self.rebuild_line_starts();
            } else if let Some(full_text) = change.get("text").and_then(|t| t.as_str()) {
                self.text = full_text.to_string();
                self.rebuild_line_starts();
            }
        }
        self.content_hash = normalized_source_hash(self.text.as_bytes());
        self.reparse();
    }

    /// Re-indexes line boundaries, lexical scopes, and declarations.
    fn reparse(&mut self) {
        // 1. Build line index
        self.rebuild_line_starts();

        // 2. Parse lexical scopes and symbols using error-tolerant token scanner
        let mut parser = OverlayParser::new(&self.text, &self.line_starts);
        parser.parse();
        self.scopes = parser.scopes;
        self.symbols = parser.symbols;
    }
}

#[cfg(test)]
pub mod tests;
