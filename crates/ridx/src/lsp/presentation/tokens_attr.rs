/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Tokens inside attributes as rust-analyzer types them: the paths a `#[derive(…)]` lists are derive macros, and
//! everything else inside `#[…]` / `#![…]` (`cfg(not(…))`, lint names, `derive` itself) carries only lexical
//! highlighting, so no symbol token is emitted for it.

use super::tokens::{RawToken, TOKEN_TYPE_MACRO};
use super::tokens_scan::byte_to_utf16_col;

/// A `(line, UTF-16 column)` range, end exclusive.
type Span = ((u32, u32), (u32, u32));

struct AttrSpans {
    attrs: Vec<Span>,
    derive_args: Vec<Span>,
}

fn attr_spans(text: &str) -> AttrSpans {
    let m = crate::lsp::editing::action_parse::mask_comments_and_strings(text);
    let b = m.as_bytes();
    let mut line_starts = vec![0usize];
    line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let pos = |off: usize| {
        let l = line_starts.partition_point(|&s| s <= off) - 1;
        let line_end = line_starts.get(l + 1).map_or(text.len(), |&s| s - 1);
        (l as u32, byte_to_utf16_col(&text[line_starts[l]..line_end], off - line_starts[l]))
    };
    let close_of = |open: usize| {
        let mut depth = 0i32;
        for (j, &c) in b.iter().enumerate().skip(open) {
            match c {
                b'[' | b'(' | b'{' => depth += 1,
                b']' | b')' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        None
    };
    let mut out = AttrSpans { attrs: Vec::new(), derive_args: Vec::new() };
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'#' {
            i += 1;
            continue;
        }
        let mut open = i + 1;
        if b.get(open) == Some(&b'!') {
            open += 1;
        }
        if b.get(open) != Some(&b'[') {
            i += 1;
            continue;
        }
        let Some(close) = close_of(open) else { break };
        out.attrs.push((pos(i), pos(close + 1)));
        // `derive(…)` at the top of the attribute or nested (`cfg_attr(feature = "x", derive(…))`).
        for (at, _) in m[open + 1..close].match_indices("derive") {
            let kw = open + 1 + at;
            let mut paren = kw + "derive".len();
            if kw > 0 && (b[kw - 1].is_ascii_alphanumeric() || b[kw - 1] == b'_') {
                continue;
            }
            while paren < close && b[paren].is_ascii_whitespace() {
                paren += 1;
            }
            if b[paren] == b'(' {
                if let Some(end) = close_of(paren) {
                    out.derive_args.push((pos(paren + 1), pos(end)));
                }
            }
        }
        i = close + 1;
    }
    out
}

/// Whether the token lies in one of `spans` (sorted by start, not overlapping).
fn inside(t: &RawToken, spans: &[Span]) -> bool {
    let (s, e) = ((t.line, t.col), (t.line, t.col + t.length));
    let i = spans.partition_point(|&(start, _)| start <= s);
    i > 0 && e <= spans[i - 1].1
}

/// Retypes derive arguments as macros and drops every other token inside an attribute.
pub(super) fn apply_attribute_rules(text: &str, tokens: &mut Vec<RawToken>) {
    if !text.contains('#') {
        return;
    }
    let spans = attr_spans(text);
    if spans.attrs.is_empty() {
        return;
    }
    tokens.retain_mut(|t| {
        if !inside(t, &spans.attrs) {
            return true;
        }
        if inside(t, &spans.derive_args) {
            t.token_type = TOKEN_TYPE_MACRO;
            t.token_modifiers = 0;
            return true;
        }
        false
    });
}
