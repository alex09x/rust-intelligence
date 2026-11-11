/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Macro arguments in the Rust code of doc examples. rust-analyzer does not expand macros there, so the names inside a
//! call's delimiters (`assert!(result.is_err())`) stay plain `generic` tokens while the code around them is typed.

use super::tokens::{RawToken, TOKEN_TYPE_GENERIC};
use super::tokens_scan::byte_to_utf16_col;

/// Open delimiters across the lines of one code block; `true` marks one that opens a macro call's arguments.
#[derive(Default)]
pub(super) struct MacroArgs {
    stack: Vec<bool>,
}

impl MacroArgs {
    fn inside(&self) -> bool {
        self.stack.contains(&true)
    }

    /// Scans the code of `line` from byte `from` and retypes the identifier `tokens` of this line that lie inside a
    /// macro call's arguments.
    pub(super) fn retype_line(&mut self, line: &str, from: usize, tokens: &mut [RawToken]) {
        let b = line.as_bytes();
        let mut inside_at: Vec<(usize, usize)> = Vec::new();
        let mut i = from;
        while i < b.len() {
            let c = b[i];
            if c == b'"' {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
                continue;
            }
            if c == b'\'' {
                // A char literal (`'x'`, `'\n'`) is skipped whole; a lifetime's quote alone.
                i += match (b.get(i + 1), b.get(i + 2)) {
                    (Some(b'\\'), _) => line[i + 2..].find('\'').map_or(1, |e| e + 3),
                    (Some(_), Some(b'\'')) => 3,
                    _ => 1,
                };
                continue;
            }
            if c == b'/' && b.get(i + 1) == Some(&b'/') {
                break;
            }
            if c.is_ascii_alphabetic() || c == b'_' {
                let s = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                if self.inside() {
                    inside_at.push((s, i));
                }
                let mut j = i;
                if b.get(j) == Some(&b'!') {
                    j += 1;
                    while b.get(j) == Some(&b' ') {
                        j += 1;
                    }
                    if matches!(b.get(j), Some(b'(' | b'[' | b'{')) {
                        self.stack.push(true);
                        i = j + 1;
                    }
                }
                continue;
            }
            match c {
                b'(' | b'[' | b'{' => self.stack.push(false),
                b')' | b']' | b'}' => {
                    self.stack.pop();
                }
                _ => {}
            }
            i += 1;
        }
        if inside_at.is_empty() {
            return;
        }
        let utf16: Vec<(u32, u32)> = inside_at.iter().map(|&(s, e)| (byte_to_utf16_col(line, s), byte_to_utf16_col(line, e))).collect();
        for t in tokens.iter_mut() {
            if utf16.contains(&(t.col, t.col + t.length)) {
                t.token_type = TOKEN_TYPE_GENERIC;
                t.token_modifiers = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_inside_macro_arguments_are_generic_across_lines() {
        let tok = |col, length| RawToken { line: 0, col, length, token_type: 8, token_modifiers: 0 };
        let mut args = MacroArgs::default();
        let l1 = "/// assert_eq!(result.kind(),";
        let mut t1 = [tok(4, 9), tok(15, 6), tok(22, 4)];
        args.retype_line(l1, 4, &mut t1);
        assert_eq!(t1.map(|t| t.token_type), [8, TOKEN_TYPE_GENERIC, TOKEN_TYPE_GENERIC]);
        let l2 = "///     Kind::Value); let x = y;";
        let mut t2 = [tok(8, 4), tok(14, 5), tok(26, 1), tok(30, 1)];
        args.retype_line(l2, 4, &mut t2);
        assert_eq!(t2.map(|t| t.token_type), [TOKEN_TYPE_GENERIC, TOKEN_TYPE_GENERIC, 8, 8]);
    }
}
