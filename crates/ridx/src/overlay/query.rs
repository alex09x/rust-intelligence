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

        let mut results = Vec::new();
        let target_name = &target_def.name;

        // Iterate through all lines of the buffer to find matching identifier tokens
        for (line_idx, line_str) in self.text.lines().enumerate() {
            let line_1based = (line_idx + 1) as u32;
            let mut search_idx = 0;
            while let Some(pos) = line_str[search_idx..].find(target_name.as_str()) {
                let byte_offset = search_idx + pos;
                let char_col = line_str[..byte_offset].chars().count() as u32 + 1;
                search_idx = byte_offset + target_name.len();

                // Check that this is a full identifier match (not a substring of longer_identifier)
                if let Some((ident, _, _)) = self.get_ident_and_span_at(line_1based, char_col) {
                    if &ident == target_name {
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
                }
            }
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overlay_query_edge_cases() {
        let code = "fn compute() {\n    let val = 42;\n    val + 1\n}\n";
        let overlay = DocumentOverlay::new("file:///test.rs", "/test.rs", 1, code.to_string());

        assert_eq!(overlay.get_ident_at(2, 9), Some("val".to_string()));
        assert_eq!(overlay.get_ident_at(99, 1), None);
        assert_eq!(overlay.get_ident_at(1, 1), Some("fn".to_string()));

        // Out of bounds definition
        assert!(overlay.find_definition_at(999, 1).is_none());

        // Local binding references for non-existent symbol
        assert!(overlay.find_local_binding_references(999, 1).is_empty());

        // Document symbols accessor
        assert!(!overlay.document_symbols().is_empty());

        // Token with path suffix abstains from overlay resolution
        let path_code = "fn run() { std::cmp::max(1, 2); }\n";
        let path_ov = DocumentOverlay::new("file:///p.rs", "/p.rs", 1, path_code.to_string());
        assert!(path_ov.find_definition_at(1, 13).is_none());

        // Shadowed local binding references (skips non-matching definitions)
        let shadow_code = "fn outer() {\n    let val = 1;\n    {\n        let val = 2;\n        val;\n    }\n    val;\n}\n";
        let shadow_ov = DocumentOverlay::new("file:///s.rs", "/s.rs", 1, shadow_code.to_string());
        let outer_refs = shadow_ov.find_local_binding_references(2, 9);
        assert!(!outer_refs.is_empty());
        let inner_refs = shadow_ov.find_local_binding_references(4, 13);
        assert!(!inner_refs.is_empty());
        assert_ne!(outer_refs, inner_refs);

        // Ident at exact end of line and past end of line (lines 173-174, 180, 183)
        let eol_code = "let  my_var";
        let eol_ov = DocumentOverlay::new("file:///e.rs", "/e.rs", 1, eol_code.to_string());
        assert_eq!(eol_ov.get_ident_at(1, 10), Some("my_var".to_string()));
        assert_eq!(eol_ov.get_ident_at(1, 12), Some("my_var".to_string()));
        assert_eq!(eol_ov.get_ident_at(1, 5), None); // space not preceded by ident

        // Line out of bounds (line 49)
        assert!(eol_ov.find_definition_at(999, 1).is_none());

        // Find local binding references with different symbol mismatch in overlay (lines 235-237)
        let diff_code = "fn alpha() {}\nfn beta() { let alpha = 1; }\n";
        let diff_ov = DocumentOverlay::new("file:///d.rs", "/d.rs", 1, diff_code.to_string());
        let refs = diff_ov.find_local_binding_references(1, 4);
        assert!(!refs.is_empty());

        let test_shadow = "let target = 1;\n{\n    let target = 2;\n    target;\n}\ntarget;\n";
        let ov_shadow = DocumentOverlay::new("file:///sh.rs", "/sh.rs", 1, test_shadow.to_string());
        let def1_refs = ov_shadow.find_local_binding_references(1, 5);
        assert_eq!(def1_refs.len(), 2);

        // Empty line in get_ident_at (line 145)
        let empty_ov = DocumentOverlay::new("file:///empty.rs", "/empty.rs", 1, "\n\n".to_string());
        assert_eq!(empty_ov.get_ident_at(1, 1), None);

        // Digit prefix in get_ident_at (line 171, 194)
        let num_ov =
            DocumentOverlay::new("file:///num.rs", "/num.rs", 1, "123_abc.456".to_string());
        assert_eq!(num_ov.get_ident_at(1, 2), None);
        assert_eq!(num_ov.get_ident_at(1, 8), None);
    }
}
