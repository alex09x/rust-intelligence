/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The indentation a formatted range takes in the file around it.

/// The indentation of a line after `before` when the innermost delimiter still open there is a `{`: that brace's line
/// indentation and one `unit` more (none more for a line that closes it). `None` inside parentheses or brackets, whose
/// lines rustfmt aligns otherwise.
pub(super) fn block_indent(before: &[&str], closes: bool, unit: &str) -> Option<String> {
    let text = before.join("\n");
    let masked = super::action_parse::mask_comments_and_strings(&text);
    let mut open: Vec<(u8, usize)> = Vec::new();
    for (i, b) in masked.bytes().enumerate() {
        match b {
            b'{' | b'(' | b'[' => open.push((b, i)),
            b'}' | b')' | b']' => {
                open.pop();
            }
            _ => {}
        }
    }
    let (b, at) = open.last().copied().unwrap_or((b'{', usize::MAX));
    if b != b'{' {
        return None;
    }
    if at == usize::MAX {
        return Some(String::new());
    }
    let line = &text[text[..at].rfind('\n').map_or(0, |i| i + 1)..];
    let base = &line[..line.len() - line.trim_start().len()];
    Some(if closes { base.to_string() } else { format!("{base}{unit}") })
}
