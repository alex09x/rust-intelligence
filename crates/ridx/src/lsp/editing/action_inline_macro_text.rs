/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Text helpers of `Inline macro`: the expansion's operators rejoined, and the macro arguments read as expressions.

use super::action_parse::mask_comments_and_strings;
use super::action_parse_items::contains_word;

/// The expansion with the spaces the transcription put between the characters of one operator removed: a `tt`
/// sequence is written token by token, so `tokio::net` comes out `tokio : : net`. Only pairs that are one token in
/// Rust whatever surrounds them are joined (`::`, `->`, `=>`, `..`, `==`, `!=`, `<=`, `>=`, `&&`, `||`, `+=` ..);
/// `< <` and `> >` may be two generic brackets and stay. Strings and comments are left as they are.
pub(crate) fn rejoin_punctuation(exp: &str) -> String {
    const PAIRS: &[(u8, u8)] = &[
        (b':', b':'),
        (b'-', b'>'),
        (b'=', b'>'),
        (b'.', b'.'),
        (b'=', b'='),
        (b'!', b'='),
        (b'<', b'='),
        (b'>', b'='),
        (b'&', b'&'),
        (b'|', b'|'),
        (b'+', b'='),
        (b'-', b'='),
        (b'*', b'='),
        (b'/', b'='),
        (b'%', b'='),
        (b'^', b'='),
    ];
    // Only spaces go, so the masking of the original text stays valid for every position.
    let masked = mask_comments_and_strings(exp);
    let b = masked.as_bytes();
    let mut out = String::with_capacity(exp.len());
    let mut last = 0;
    for i in 1..b.len().saturating_sub(1) {
        // `a : : b`: the space between the two halves of one operator (a `:` already joined is not joined again).
        // A `:` next to a `::` (`field : ::krate::T`) is a field's colon before a path: it stays apart.
        let colon_by_path = b[i - 1] == b':' && ((i >= 2 && b[i - 2] == b':') || b.get(i + 2) == Some(&b':'));
        if b[i] == b' ' && PAIRS.contains(&(b[i - 1], b[i + 1])) && !colon_by_path {
            out.push_str(&exp[last..i]);
            last = i + 1;
        }
    }
    out.push_str(&exp[last..]);
    out
}

/// The top-level pieces of macro arguments, split at `,`, `;` and `=>`.
pub(crate) fn top_level_pieces(m: &str) -> Vec<&str> {
    let b = m.as_bytes();
    let (mut depth, mut lo, mut out) = (0i32, 0usize, Vec::new());
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' | b';' if depth == 0 => {
                out.push(&m[lo..i]);
                lo = i + 1;
            }
            b'>' if depth == 0 && i > 0 && b[i - 1] == b'=' => {
                out.push(&m[lo..i - 1]);
                lo = i + 1;
            }
            _ => {}
        }
    }
    out.push(&m[lo..]);
    out.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// Whether an expression needs no parentheses wherever it is spliced: no leading unary operator, no binary operator,
/// cast or range outside brackets.
pub(crate) fn is_atomic(expr: &str) -> bool {
    if expr.starts_with(['-', '!', '*', '&']) {
        return false;
    }
    let b = expr.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'|' | b'=' | b'<' | b'>' if depth == 0 => {
                // A path's `::` and a generic argument list's `<`/`>` after `::` are no operators.
                if (b[i] == b'<' && expr[..i].ends_with("::")) || (b[i] == b'>' && expr[..i].contains("::<")) || (b[i] == b'-' && b.get(i + 1) == Some(&b'>')) {
                    continue;
                }
                return false;
            }
            b'&' if depth == 0 && b.get(i + 1) == Some(&b'&') => return false,
            b'.' if depth == 0 && b.get(i + 1) == Some(&b'.') => return false,
            _ => {}
        }
    }
    !contains_word(expr, "as")
}

#[cfg(test)]
mod tests {
    use super::{is_atomic, rejoin_punctuation, top_level_pieces};

    #[test]
    fn operators_split_by_the_transcription_are_joined_again() {
        assert_eq!(rejoin_punctuation("use tokio : : net : : TcpStream ; fn f ( ) - > u8 { 0 . . 1 }"), "use tokio :: net :: TcpStream ; fn f ( ) -> u8 { 0 .. 1 }");
        assert_eq!(rejoin_punctuation("Vec < Vec < u8 > > ; x = = y ; s = \": :\""), "Vec < Vec < u8 > > ; x == y ; s = \": :\"");
        assert_eq!(rejoin_punctuation("E { position : ::expect_test :: P }"), "E { position : ::expect_test :: P }");
    }

    #[test]
    fn macro_arguments_are_read_as_expressions() {
        assert_eq!(top_level_pieces("a + b, f(x, y); c => d"), ["a + b", "f(x, y)", "c", "d"]);
        assert!(is_atomic("self.f(a + b)") && is_atomic("Vec::<u8>::new()") && !is_atomic("a + b") && !is_atomic("-x") && !is_atomic("x as u8"));
    }
}
