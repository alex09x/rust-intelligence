/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Item parsing methods for OverlayParser: functions, parameters, let-bindings, and named items.

use super::{is_ident, skip_string_or_char_literal, OverlayParser, OverlaySymbol, Scope};
use ridx_format::DefKind;

impl<'a> OverlayParser<'a> {
    pub(crate) fn parse_fn_item(&mut self, pos: &mut usize, current_scope: usize) {
        let Some((name, start, end)) = self.parse_ident(pos) else {
            return;
        };
        let (s_line, s_col) = self.byte_to_char_line_col(start);
        let (e_line, e_col) = self.byte_to_char_line_col(end);

        // Find signature up to `{` or `;` at depth 0
        let sig_start = start;
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        let mut depth = 0usize;
        while *pos < len {
            let b = bytes[*pos];
            if b == b'-' && *pos + 1 < len && bytes[*pos + 1] == b'>' {
                *pos += 2;
                continue;
            } else if b == b'(' || b == b'[' || b == b'<' {
                depth += 1;
            } else if (b == b')' || b == b']' || b == b'>') && depth > 0 {
                depth -= 1;
            } else if depth == 0 && (b == b'{' || b == b';') {
                break;
            }
            *pos += 1;
        }
        let sig_end = (*pos).min(len);
        let raw_sig = self.text[sig_start..sig_end].trim();
        let signature = Some(format!("fn {}", raw_sig));

        // Add function symbol to enclosing outer scope
        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind: DefKind::Fn,
            signature,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
            decl_start_byte: start,
            decl_end_byte: end,
            scope_start_byte: 0,
        });

        // Allocate a dedicated scope for this function's parameters and body
        let sel_start_byte = self.find_fn_item_start(start);
        let fn_scope_id = self.scopes.len();
        self.scopes.push(Scope { id: fn_scope_id, parent: Some(current_scope), start_byte: start, end_byte: len, sel_start_byte, body_start_byte: None, pattern: false });

        // Parse parameters inside `(...)` and assign them to fn_scope_id
        if let Some(open_paren) = self.text[start..sig_end].find('(') {
            if let Some(close_paren) = self.text[start + open_paren..sig_end].find(')') {
                let params_text = self.text[start + open_paren + 1..start + open_paren + close_paren].to_string();
                self.extract_params(&params_text, start + open_paren + 1, fn_scope_id);
            }
        }

        // If followed by `{`, register pending_fn_scope so the body block reuses this scope
        if *pos < len && bytes[*pos] == b'{' {
            self.pending_fn_scope = Some(fn_scope_id);
        } else if *pos < len && bytes[*pos] == b';' {
            // Trait fn declaration without body
            self.scopes[fn_scope_id].end_byte = *pos + 1;
        }
    }

    pub(crate) fn extract_params(&mut self, params_text: &str, base_byte: usize, scope_id: usize) {
        for part in params_text.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty() || trimmed == "self" || trimmed == "&self" || trimmed == "&mut self" {
                continue;
            }
            if let Some((p_name, _)) = trimmed.split_once(':') {
                let mut p_name = p_name.trim();
                if let Some(after) = p_name.strip_prefix("mut") {
                    if after.chars().next().is_some_and(|c| c.is_whitespace()) {
                        p_name = after.trim();
                    }
                }
                if !p_name.is_empty() && is_ident(p_name) {
                    let offset = part.find(p_name).unwrap();
                    let abs_offset = base_byte + (part.as_ptr() as usize - params_text.as_ptr() as usize) + offset;
                    let (s_line, s_col) = self.byte_to_char_line_col(abs_offset);
                    let (e_line, e_col) = self.byte_to_char_line_col(abs_offset + p_name.len());
                    self.symbols.push(OverlaySymbol {
                        name: p_name.to_string(),
                        kind: DefKind::Local,
                        signature: Some(trimmed.to_string()),
                        start_line: s_line,
                        start_col: s_col,
                        end_line: e_line,
                        end_col: e_col,
                        scope_id,
                        decl_start_byte: abs_offset,
                        decl_end_byte: abs_offset + p_name.len(),
                        scope_start_byte: base_byte,
                    });
                }
            }
        }
    }

    pub(crate) fn parse_named_item(&mut self, pos: &mut usize, kind: DefKind, current_scope: usize) {
        let Some((name, start, end)) = self.parse_ident(pos) else {
            return;
        };
        let (s_line, s_col) = self.byte_to_char_line_col(start);
        let (e_line, e_col) = self.byte_to_char_line_col(end);

        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind,
            signature: None,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
            decl_start_byte: start,
            decl_end_byte: end,
            scope_start_byte: 0,
        });
    }

    pub(crate) fn parse_let_binding(&mut self, pos: &mut usize, current_scope: usize) {
        let decl_start = *pos;
        self.skip_whitespace_and_comments(pos);
        // Optional `mut` keyword (byte-safe UTF-8 check)
        let remainder = &self.text[*pos..];
        if let Some(after_mut) = remainder.strip_prefix("mut") {
            if after_mut.chars().next().is_some_and(|c| c.is_whitespace()) {
                *pos += 3;
                self.skip_whitespace_and_comments(pos);
            }
        }

        // A destructuring pattern (`let (a, b) = …`, `let Some(x) = … else { … }`) binds every name in it.
        if let Some((names, pattern_end)) = self.let_pattern(*pos) {
            let scope_start = self.let_statement_end(pattern_end);
            self.push_locals(names, current_scope, scope_start);
            return;
        }
        let Some((name, start, end)) = self.parse_ident(pos) else {
            return;
        };
        let (s_line, s_col) = self.byte_to_char_line_col(start);
        let (e_line, e_col) = self.byte_to_char_line_col(end);
        let scope_start = self.let_statement_end(*pos);

        self.symbols.push(OverlaySymbol {
            name: name.to_string(),
            kind: DefKind::Local,
            signature: None,
            start_line: s_line,
            start_col: s_col,
            end_line: e_line,
            end_col: e_col,
            scope_id: current_scope,
            decl_start_byte: decl_start,
            decl_end_byte: end,
            scope_start_byte: scope_start,
        });
    }

    /// The byte after the `let` statement whose pattern ends at `from`: its `;`, or the `}` of the enclosing block.
    /// In Rust, a let binding's scope starts AFTER its entire initializer statement.
    fn let_statement_end(&self, from: usize) -> usize {
        let bytes = self.text.as_bytes();
        let len = bytes.len();
        let mut cur = from;
        let mut brace_depth: usize = 0;
        let mut paren_depth: usize = 0;
        let mut bracket_depth: usize = 0;

        while cur < len {
            let prev = cur;
            self.skip_whitespace_and_comments(&mut cur);
            if cur != prev {
                continue;
            }
            let b = bytes[cur];
            // Strings, chars, and lifetimes
            if skip_string_or_char_literal(self.text, &mut cur) {
                continue;
            }
            // Delimiters
            if b == b'{' {
                brace_depth += 1;
                cur += 1;
                continue;
            }
            if b == b'}' {
                if brace_depth > 0 {
                    brace_depth -= 1;
                    cur += 1;
                    continue;
                } else {
                    // Closing brace of enclosing block: ends the let statement
                    break;
                }
            }
            if b == b'(' {
                paren_depth += 1;
                cur += 1;
                continue;
            }
            if b == b')' {
                paren_depth = paren_depth.saturating_sub(1);
                cur += 1;
                continue;
            }
            if b == b'[' {
                bracket_depth += 1;
                cur += 1;
                continue;
            }
            if b == b']' {
                bracket_depth = bracket_depth.saturating_sub(1);
                cur += 1;
                continue;
            }
            if b == b';' {
                if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 {
                    cur += 1;
                    break;
                }
                cur += 1;
                continue;
            }
            cur += 1;
        }

        cur
    }
}
