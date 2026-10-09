/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Data structures and cursor mappings for pre-parsed file lexical scopes.

use super::super::generic_completion::{parse_generic_params_from_header, GenericParam};
use super::super::self_completion::EnclosingScope;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

/// Represents a lexical block delimited by `{` and `}`.
#[derive(Debug, Clone)]
pub struct ScopeBlock {
    pub start_byte: usize,
    pub end_byte: usize,
    pub enclosing: EnclosingScope,
    pub generic_params: Vec<GenericParam>,
}

/// Pre-parsed scope data for a source file.
#[derive(Debug)]
pub struct FileScopeData {
    pub text: Arc<str>,
    pub line_starts: Vec<usize>,
    pub blocks: Vec<ScopeBlock>,
}

impl FileScopeData {
    /// Maps a 1-based line number and column slice to a byte offset within the file text.
    pub fn cursor_byte(&self, line_1based: u32, line_slice: &str) -> usize {
        let line_idx = line_1based.saturating_sub(1) as usize;
        let line_start = self.line_starts.get(line_idx).copied().unwrap_or(0);
        (line_start + line_slice.len()).min(self.text.len())
    }

    /// Finds the innermost enclosing `Impl` or `Trait` scope containing `cursor_byte`.
    pub fn find_enclosing_scope(&self, cursor_byte: usize) -> Option<EnclosingScope> {
        self.blocks
            .iter()
            .filter(|b| {
                b.start_byte <= cursor_byte
                    && cursor_byte <= b.end_byte
                    && !matches!(b.enclosing, EnclosingScope::Other)
            })
            .max_by_key(|b| b.start_byte)
            .map(|b| b.enclosing.clone())
    }

    /// Collects all generic parameters in scope for `cursor_byte`.
    pub fn find_generic_params_in_scope(&self, cursor_byte: usize) -> Vec<GenericParam> {
        let mut merged: HashMap<String, Vec<String>> = HashMap::new();

        // 1. Collect generic parameters from all blocks enclosing cursor
        for b in &self.blocks {
            if b.start_byte <= cursor_byte && cursor_byte <= b.end_byte {
                for p in &b.generic_params {
                    merged
                        .entry(p.name.clone())
                        .or_default()
                        .extend(p.bounds.clone());
                }
            }
        }

        // 2. Collect from unclosed declaration header directly preceding cursor
        let text_bytes = self.text.as_bytes();
        let scan_limit = cursor_byte.min(text_bytes.len());
        let mut last_stmt_end = 0;
        for i in (0..scan_limit).rev() {
            if text_bytes[i] == b';' || text_bytes[i] == b'{' || text_bytes[i] == b'}' {
                last_stmt_end = i + 1;
                break;
            }
        }
        let unclosed = &self.text[last_stmt_end..scan_limit];
        let unclosed_params = parse_generic_params_from_header(unclosed);
        for p in unclosed_params {
            merged.entry(p.name).or_default().extend(p.bounds);
        }

        merged
            .into_iter()
            .map(|(name, mut bounds)| {
                bounds.dedup();
                GenericParam { name, bounds }
            })
            .collect()
    }
}

/// Cache key identifying disk or overlay source state.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum FileCacheKey {
    Disk {
        path: String,
        mtime: SystemTime,
        len: u64,
    },
    Overlay {
        path: String,
        full_hash: u64,
    },
}
