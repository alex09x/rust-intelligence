/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Symbol resolution, definition lookup, and identifier inspection for document overlays.

use super::mapping::{
    strip_leading_whitespace_and_comments, strip_trailing_whitespace_and_comments,
};
use super::parser::{is_rust_ident_continue, is_rust_ident_start};
use super::{DocumentOverlay, OverlayDefinition, OverlaySymbol};
use ridx_format::DefKind;

impl DocumentOverlay {
    /// Finds a symbol definition at a cursor location within the active buffer.
    pub fn find_definition_at(&self, line: u32, col: u32) -> Option<OverlayDefinition> {
        // 1. Direct hit on an existing overlay definition span
        for sym in &self.symbols {
            if line >= sym.start_line
                && line <= sym.end_line
                && col >= sym.start_col
                && col <= sym.end_col
            {
                return Some(OverlayDefinition {
                    name: sym.name.clone(),
                    kind: sym.kind,
                    signature: sym.signature.clone(),
                    file: self.file_path.clone(),
                    start_line: sym.start_line,
                    start_col: sym.start_col,
                    end_line: sym.end_line,
                    end_col: sym.end_col,
                });
            }
        }

        // 2. Identifier at cursor location -> resolve to visible binding in lexical scope
        let (ident, start_b, end_b) = self.get_ident_and_span_at(line, col)?;
        let byte_pos = self.line_char_col_to_byte(line, col);

        let line_start = self.line_starts[line.saturating_sub(1) as usize];
        let ident_start = line_start + start_b;
        let ident_end = line_start + end_b;

        // If the token is part of a qualified path (e.g. `foo::bar` or `foo.bar` or `bar::baz`),
        // across newlines or comments, the single-file lexical overlay lacks path/type semantics:
        // abstain (return None) so the compiled index can resolve it accurately.
        let prefix = strip_trailing_whitespace_and_comments(&self.text[..ident_start]);
        if prefix.ends_with("::") || prefix.ends_with('.') {
            return None;
        }
        let suffix = strip_leading_whitespace_and_comments(&self.text[ident_end..]);
        if suffix.starts_with("::") {
            return None;
        }

        // Find innermost enclosing scope
        let mut best_scope = 0;
        let mut min_scope_len = usize::MAX;
        for s in &self.scopes {
            if byte_pos >= s.start_byte && byte_pos <= s.end_byte {
                let scope_len = s.end_byte.saturating_sub(s.start_byte);
                if scope_len < min_scope_len {
                    min_scope_len = scope_len;
                    best_scope = s.id;
                }
            }
        }

        // Walk up scope chain to find nearest visible matching symbol
        let mut cur = Some(best_scope);
        while let Some(sid) = cur {
            let mut best_match: Option<&OverlaySymbol> = None;
            for sym in &self.symbols {
                if sym.scope_id == sid && sym.name == ident {
                    let is_visible = match sym.kind {
                        DefKind::Fn
                        | DefKind::Struct
                        | DefKind::Enum
                        | DefKind::Trait
                        | DefKind::TyAlias
                        | DefKind::Const
                        | DefKind::Mod => true,
                        _ => sym.scope_start_byte <= byte_pos,
                    };
                    if is_visible {
                        if let Some(prev) = best_match {
                            if sym.scope_start_byte > prev.scope_start_byte {
                                best_match = Some(sym);
                            }
                        } else {
                            best_match = Some(sym);
                        }
                    }
                }
            }
            if let Some(sym) = best_match {
                return Some(OverlayDefinition {
                    name: sym.name.clone(),
                    kind: sym.kind,
                    signature: sym.signature.clone(),
                    file: self.file_path.clone(),
                    start_line: sym.start_line,
                    start_col: sym.start_col,
                    end_line: sym.end_line,
                    end_col: sym.end_col,
                });
            }
            cur = self.scopes.get(sid).and_then(|s| s.parent);
        }

        None
    }

    /// Returns the identifier at a given 1-based (line, char_col) in the buffer.
    #[allow(dead_code)]
    pub fn get_ident_at(&self, line: u32, col: u32) -> Option<String> {
        self.get_ident_and_span_at(line, col).map(|(s, _, _)| s)
    }

    /// Returns the identifier and its byte start and end offset within the line.
    pub fn get_ident_and_span_at(&self, line: u32, col: u32) -> Option<(String, usize, usize)> {
        let byte_pos = self.line_char_col_to_byte(line, col);
        let line_idx = line.saturating_sub(1) as usize;
        if line_idx >= self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[line_idx];
        let line_end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.text.len()
        };
        let line_text = &self.text[line_start..line_end];
        let col_byte = byte_pos.saturating_sub(line_start).min(line_text.len());

        let char_indices: Vec<(usize, char)> = line_text.char_indices().collect();
        if char_indices.is_empty() {
            return None;
        }

        let mut target_idx = None;
        for (i, &(idx, c)) in char_indices.iter().enumerate() {
            if idx <= col_byte && col_byte < idx + c.len_utf8() {
                target_idx = Some(i);
                break;
            }
        }
        if target_idx.is_none() && col_byte >= line_text.len() {
            target_idx = Some(char_indices.len().saturating_sub(1));
        }

        let idx = target_idx?;
        let c = char_indices[idx].1;
        if !is_rust_ident_continue(c) {
            if idx > 0 && is_rust_ident_continue(char_indices[idx - 1].1) {
                let mut start_i = idx - 1;
                while start_i > 0 && is_rust_ident_continue(char_indices[start_i - 1].1) {
                    start_i -= 1;
                }
                if is_rust_ident_start(char_indices[start_i].1) {
                    let start_b = char_indices[start_i].0;
                    let end_b = char_indices[idx].0;
                    return Some((line_text[start_b..end_b].to_string(), start_b, end_b));
                }
            }
            return None;
        }

        let mut start_i = idx;
        while start_i > 0 && is_rust_ident_continue(char_indices[start_i - 1].1) {
            start_i -= 1;
        }
        let mut end_i = idx + 1;
        while end_i < char_indices.len() && is_rust_ident_continue(char_indices[end_i].1) {
            end_i += 1;
        }

        if is_rust_ident_start(char_indices[start_i].1) {
            let start_b = char_indices[start_i].0;
            let end_b = if end_i < char_indices.len() {
                char_indices[end_i].0
            } else {
                line_text.len()
            };
            Some((line_text[start_b..end_b].to_string(), start_b, end_b))
        } else {
            None
        }
    }

    /// Returns all top-level and item symbols for LSP documentSymbol.
    pub fn document_symbols(&self) -> &[OverlaySymbol] {
        &self.symbols
    }

    /// Finds all references (including declaration) that resolve to the same local definition as (line, col).
    pub fn find_local_binding_references(&self, line: u32, col: u32) -> Vec<(u32, u32, u32, u32)> {
        let Some(target_def) = self.find_definition_at(line, col) else {
            return Vec::new();
        };

        // Only local bindings (let bindings, parameters) are handled by local fast path
        if target_def.kind != ridx_format::DefKind::Local {
            return Vec::new();
        }

        let mut results = Vec::new();
        let target_name = &target_def.name;

        // Iterate through all real code identifier tokens across the document
        for (line_1based, char_col) in find_document_matching_identifiers(&self.text, target_name) {
            if let Some(resolved) = self.find_definition_at(line_1based, char_col) {
                if resolved.start_line == target_def.start_line
                    && resolved.start_col == target_def.start_col
                    && resolved.file == target_def.file
                {
                    let end_col = char_col + target_name.chars().count() as u32;
                    let span = (line_1based, char_col, line_1based, end_col);
                    if !results.contains(&span) {
                        results.push(span);
                    }
                }
            }
        }
        results
    }
}

/// Scans the full document text in lexer order and returns (line_1based, char_col_1based)
/// for all identifier occurrences matching `target_name` that are real code tokens
/// (ignoring comments, block comments with nesting, string literals, raw string literals, and char literals).
pub fn find_document_matching_identifiers(text: &str, target_name: &str) -> Vec<(u32, u32)> {
    let mut matches = Vec::new();
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    let mut line = 1u32;
    let mut line_start_byte = 0;

    while i < n {
        while i < n && !text.is_char_boundary(i) {
            i += 1;
        }
        if i >= n {
            break;
        }
        let b = bytes[i];

        if b == b'\n' {
            line += 1;
            i += 1;
            line_start_byte = i;
            continue;
        }

        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1;
            while i + 1 < n && depth > 0 {
                if bytes[i] == b'\n' {
                    line += 1;
                    line_start_byte = i + 1;
                }
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth > 0 {
                i = n;
            }
            continue;
        }

        if b == b'r' && i + 1 < n && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            let mut h_idx = i + 1;
            let mut hashes = 0;
            while h_idx < n && bytes[h_idx] == b'#' {
                hashes += 1;
                h_idx += 1;
            }
            if h_idx < n && bytes[h_idx] == b'"' {
                i = h_idx + 1;
                while i < n {
                    if bytes[i] == b'\n' {
                        line += 1;
                        line_start_byte = i + 1;
                    }
                    if bytes[i] == b'"'
                        && i + hashes < n
                        && (0..hashes).all(|k| bytes[i + 1 + k] == b'#')
                    {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }

        if b == b'"' {
            i += 1;
            let mut closed = false;
            while i < n {
                if bytes[i] == b'\n' {
                    line += 1;
                    line_start_byte = i + 1;
                }
                if bytes[i] == b'\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                i = n;
            }
            continue;
        }

        if b == b'\'' {
            let mut j = i + 1;
            if j < n && bytes[j] == b'\\' {
                j += 2;
                while j < n && bytes[j] != b'\'' && bytes[j] != b'\n' {
                    j += 1;
                }
                if j < n && bytes[j] == b'\'' {
                    i = j + 1;
                    continue;
                }
            } else if j + 1 < n && bytes[j + 1] == b'\'' {
                i = j + 2;
                continue;
            }
        }

        let ch = text[i..].chars().next().unwrap();
        if is_rust_ident_start(ch) {
            let start_byte = i;
            let ident_line = line;
            let line_slice_before = &text[line_start_byte..start_byte];
            let char_col = line_slice_before.chars().count() as u32 + 1;

            let mut ident_len = 0;
            for c in text[start_byte..].chars() {
                if is_rust_ident_continue(c) {
                    ident_len += c.len_utf8();
                } else {
                    break;
                }
            }

            let ident = &text[start_byte..start_byte + ident_len];
            if ident == target_name {
                matches.push((ident_line, char_col));
            }
            i += ident_len;
            continue;
        }

        i += ch.len_utf8();
    }

    matches
}
