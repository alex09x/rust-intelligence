/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Trivia handling for the overlay parser: opaque spans (comments and string/char literals)
//! and the start of a `fn` item including its leading attributes and doc comments.

use super::OverlayParser;

impl<'a> OverlayParser<'a> {
    /// Records every `//` and `/* */` comment inside `[start, end)`, a region that
    /// `skip_whitespace_and_comments` has already classified as whitespace and comments.
    pub(crate) fn record_comment_spans(&mut self, start: usize, end: usize) {
        let bytes = self.text.as_bytes();
        let mut i = start;
        while i < end {
            if bytes[i] == b'/' && i + 1 < end && bytes[i + 1] == b'/' {
                let mut j = i + 2;
                while j < end && bytes[j] != b'\n' {
                    j += 1;
                }
                self.opaque.push((i, j));
                i = j;
            } else if bytes[i] == b'/' && i + 1 < end && bytes[i + 1] == b'*' {
                let mut depth = 1usize;
                let mut j = i + 2;
                while j < end && depth > 0 {
                    if bytes[j] == b'/' && j + 1 < end && bytes[j + 1] == b'*' {
                        depth += 1;
                        j += 2;
                    } else if bytes[j] == b'*' && j + 1 < end && bytes[j + 1] == b'/' {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                self.opaque.push((i, j));
                i = j;
            } else {
                i += 1;
            }
        }
    }

    /// Records `[start, end)` as an opaque literal unless it is a lifetime (`'a`), which is a
    /// token in its own right.
    pub(crate) fn record_literal_span(&mut self, start: usize, end: usize) {
        let s = &self.text[start..end];
        let is_lifetime = s.starts_with('\'') && !s.ends_with('\'');
        if !is_lifetime && end > start {
            self.opaque.push((start, end));
        }
    }

    fn line_slice_no_eol(&self, line_idx: usize) -> &'a str {
        let start = self.line_starts[line_idx];
        let end = if line_idx + 1 < self.line_starts.len() { self.line_starts[line_idx + 1] } else { self.text.len() };
        self.text[start..end].trim_end_matches(['\r', '\n'])
    }

    /// Byte offset where the `fn` item whose name starts at `name_start` begins.
    ///
    /// Like rust-analyzer's `FN` node, the item includes the outer attributes and the doc or
    /// plain comments on the lines directly above it, up to the first blank line, inner
    /// attribute (`#![..]`), inner doc comment (`//!`, `/*!`) or line of code. When code
    /// precedes the `fn` on its own line, the item starts after the last `{`, `}` or `;`.
    pub(crate) fn find_fn_item_start(&self, name_start: usize) -> usize {
        if self.line_starts.is_empty() {
            return name_start;
        }
        let b = name_start.min(self.text.len());
        let sl = match self.line_starts.binary_search(&b) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line_start = self.line_starts[sl];
        let line_prefix = &self.text[line_start..b];

        let mut cut: Option<usize> = None;
        let mut br_depth = 0usize;
        for (idx, ch) in line_prefix.char_indices() {
            match ch {
                '[' => br_depth += 1,
                ']' => br_depth = br_depth.saturating_sub(1),
                '{' | '}' | ';' if br_depth == 0 => cut = Some(idx + ch.len_utf8()),
                _ => {}
            }
        }
        if let Some(cut_idx) = cut {
            let rem = &line_prefix[cut_idx..];
            let ws_len = rem.len() - rem.trim_start().len();
            return line_start + cut_idx + ws_len;
        }

        let sl_raw = self.line_slice_no_eol(sl);
        let mut best_byte = line_start + (sl_raw.len() - sl_raw.trim_start().len());
        let mut l = sl;
        let mut attr_bracket_depth = 0i32;
        let mut in_block_comment = false;

        while l > 0 {
            l -= 1;
            let raw = self.line_slice_no_eol(l);
            let s = raw.trim();
            let first_non_ws = self.line_starts[l] + (raw.len() - raw.trim_start().len());

            if in_block_comment {
                // Walking up through a block comment that closed on a line below.
                match s.find("/*") {
                    Some(0) if s.starts_with("/*!") => break,
                    Some(0) => {
                        in_block_comment = false;
                        best_byte = first_non_ws;
                    }
                    Some(open) => {
                        // The comment opened after code on this line and belongs to the item.
                        best_byte = first_non_ws + open;
                        break;
                    }
                    None => {}
                }
                continue;
            }
            let sc_clean = strip_trailing_line_comment(s);
            if attr_bracket_depth > 0 {
                attr_bracket_depth += bracket_delta(sc_clean);
                if attr_bracket_depth <= 0 {
                    if sc_clean.starts_with("#[") && !sc_clean.starts_with("#![") {
                        attr_bracket_depth = 0;
                        best_byte = first_non_ws;
                        continue;
                    }
                    break;
                }
                continue;
            }
            if s.is_empty() || s.starts_with("//!") || s.starts_with("/*!") || s.starts_with("#![") {
                break;
            }
            if s.starts_with("//") {
                best_byte = first_non_ws;
                continue;
            }
            if sc_clean.ends_with("*/") {
                match sc_clean.find("/*") {
                    Some(0) => best_byte = first_non_ws,
                    Some(_) => break,
                    None => in_block_comment = true,
                }
                continue;
            }
            if sc_clean.starts_with("#[") && sc_clean.ends_with(']') {
                best_byte = first_non_ws;
                continue;
            }
            if sc_clean.ends_with(']') && !sc_clean.ends_with("};") && !sc_clean.ends_with("];") {
                let delta = bracket_delta(sc_clean);
                if delta > 0 {
                    attr_bracket_depth = delta;
                    continue;
                }
            }
            break;
        }
        best_byte
    }
}

/// Closing minus opening square brackets in `s`; positive on the last line of a multi-line
/// attribute when read bottom-up.
fn bracket_delta(s: &str) -> i32 {
    s.bytes().fold(0i32, |acc, c| match c {
        b']' => acc + 1,
        b'[' => acc - 1,
        _ => acc,
    })
}

fn strip_trailing_line_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    let (mut in_str, mut esc) = (false, false);
    for i in 0..bytes.len() {
        if esc {
            esc = false;
        } else if bytes[i] == b'\\' && in_str {
            esc = true;
        } else if bytes[i] == b'"' {
            in_str = !in_str;
        } else if !in_str && bytes[i..].starts_with(b"//") {
            return s[..i].trim_end();
        }
    }
    s
}
