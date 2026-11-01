/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether a reference span records a derive's generated call rather than a name.

/// Whether a reference span that starts on no identifier covers other identifiers: the span of a
/// type expression (`&'static str`, `(A, B)`) or of an attribute (`#[serde(..)]`, possibly over
/// several lines) a derive wrote its call over, not the position of a name. Such a span is no
/// occurrence of the renamed symbol. `span` is `(start line, start col, end line, end col)`,
/// 1-based; `path` is the file, `None` when the text lives in an editor buffer.
pub fn span_covers_other_identifier(path: Option<&str>, span: (u32, u32, u32, u32)) -> bool {
    let (start_line, start_col, end_line, end_col) = span;
    let Some(path) = path else { return false };
    if end_line < start_line {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let first = start_line.saturating_sub(1) as usize;
    let last = end_line.saturating_sub(1) as usize;
    let from = start_col.saturating_sub(1) as usize;
    let to = end_col.saturating_sub(1) as usize;
    text.lines().enumerate().skip(first).take(last - first + 1).any(|(n, line)| {
        let skip = if n == first { from } else { 0 };
        let take = if n == last { to.saturating_sub(skip) } else { usize::MAX };
        line.chars().skip(skip).take(take).any(|c| c.is_alphanumeric() || matches!(c, '_' | '(' | ')' | '[' | ']' | '&' | '#'))
    })
}
