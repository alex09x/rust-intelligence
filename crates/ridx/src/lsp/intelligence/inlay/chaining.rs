/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Chaining hints: at the end of a line whose expression continues with `.method` on the next line, the
//! expression's type, as rust-analyzer shows it.

use super::{Hint, SourceMap, TypeSpan};

pub fn chaining_hints(src: &SourceMap<'_>, types: &[TypeSpan<'_>], lines: (u32, u32)) -> Vec<Hint> {
    let mut out = Vec::new();
    let masked_lines: Vec<&str> = src.masked.split('\n').collect();
    for l in lines.0 as usize..=(lines.1 as usize).min(masked_lines.len().saturating_sub(1)) {
        let line = masked_lines[l].trim_end_matches('\r');
        let code = line.trim_end();
        if code.trim().is_empty() || code.ends_with(['{', '(', ',', ';', '=', '[', '|', '.', '&', '+', '-', '*', '/', ':', '>', '<']) {
            continue;
        }
        let Some(next) = masked_lines[l + 1..].iter().map(|s| s.trim()).find(|s| !s.is_empty()) else { continue };
        if !next.starts_with('.') || next.starts_with("..") {
            continue;
        }
        // The outermost expression that ends where this line's code ends.
        let end_char = code.chars().count() as u32;
        let line1 = l as u32 + 1;
        // A candidate inside brackets that close later on the line (a literal inside `arg!("..")`, whose own type
        // the index does not record) is not the chained expression.
        let Some(line_start) = src.line_start(l) else { continue };
        let balanced = |t: &&TypeSpan<'_>| {
            let Some(ls) = src.line_start(t.start.0.saturating_sub(1) as usize) else { return false };
            let first_line = src.masked[ls..].split('\n').next().unwrap_or("");
            let from = ls + first_line.char_indices().nth(t.start.1.saturating_sub(1) as usize).map_or(first_line.len(), |(b, _)| b);
            let end = line_start + code.len();
            if from > end {
                return false;
            }
            let tail = &src.masked[from..end];
            tail.matches(['(', '[', '{']).count() >= tail.matches([')', ']', '}']).count()
        };
        let Some(t) = types.iter().filter(|t| t.end.0 == line1 && (t.end.1 == end_char + 1 || t.end.1 == end_char)).filter(balanced).min_by_key(|t| t.start) else { continue };
        let Some(label) = super::type_label::hint_type(t.ty) else { continue };
        let Some(start) = src.line_start(l) else { continue };
        let offset = start + code.len();
        let mut h = Hint::new(src, offset, label, Some(1));
        h.padding_left = true;
        out.push(h);
    }
    out
}
