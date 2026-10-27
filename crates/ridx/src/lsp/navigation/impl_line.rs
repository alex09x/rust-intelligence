/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether a source line holds the header of an `impl` block.

/// Strips the whole word `word` from the start of `s`.
fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(word)?;
    match rest.chars().next() {
        Some(c) if c.is_alphanumeric() || c == '_' => None,
        _ => Some(rest.trim_start()),
    }
}

/// Whether `line` starts an `impl` block: optional attributes, `default`, `unsafe`, then the
/// keyword `impl` as a whole word. `fn f(x: impl Trait)`, `impl_trait!(S)` and `implement(..)`
/// are no impl headers.
pub(super) fn is_impl_header(line: &str) -> bool {
    let mut rest = line.trim();
    while rest.starts_with("#[") {
        match rest.find(']') {
            Some(end) => rest = rest[end + 1..].trim_start(),
            None => return false,
        }
    }
    for word in ["default", "unsafe"] {
        if let Some(after) = strip_word(rest, word) {
            rest = after;
        }
    }
    strip_word(rest, "impl").is_some()
}

#[cfg(test)]
mod tests {
    use super::is_impl_header;

    #[test]
    fn impl_headers_are_found() {
        for line in [
            "impl Tr for S {}",
            "    impl<T> Tr for Vec<T> {",
            "unsafe impl Send for S {}",
            "#[cfg(test)] impl Tr for S {}",
            "default unsafe impl<T> Tr for T {}",
            "impl<T: Clone> Tr for T where T: Copy {",
        ] {
            assert!(is_impl_header(line), "{line}");
        }
    }

    #[test]
    fn an_impl_keyword_elsewhere_or_a_longer_word_is_no_header() {
        for line in [
            "fn take(x: impl Tr) {}",
            "impl_tr!(S);",
            "implement(Tr);",
            "// impl Tr for S",
            "let f: Box<dyn Tr> = make();",
        ] {
            assert!(!is_impl_header(line), "{line}");
        }
    }
}
