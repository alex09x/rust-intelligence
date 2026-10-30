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

/// Whether the name that starts at the 0-based character `col` of an impl header stands where the
/// implemented trait or the self type stands: not in a generic argument or a bound
/// (`Wrapper<Tr>`, `impl<T: Tr>`) and not in the `where` clause. An item named there is used by
/// the impl, not implemented by it.
pub(super) fn is_head_position(line: &str, col: usize) -> bool {
    let before: String = line.chars().take(col).collect();
    if before.contains(" where ") || before.trim_start().starts_with("where ") {
        return false;
    }
    let (mut depth, mut prev) = (0i32, ' ');
    for c in before.chars() {
        match c {
            '<' => depth += 1,
            '>' if prev != '-' => depth -= 1,
            _ => {}
        }
        prev = c;
    }
    depth == 0
}

#[cfg(test)]
mod tests {
    use super::{is_head_position, is_impl_header};

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
        for line in ["fn take(x: impl Tr) {}", "impl_tr!(S);", "implement(Tr);", "// impl Tr for S", "let f: Box<dyn Tr> = make();"] {
            assert!(!is_impl_header(line), "{line}");
        }
    }

    #[test]
    fn a_name_in_an_argument_a_bound_or_a_where_clause_is_not_at_the_head() {
        let at = |line: &str, name: &str| is_head_position(line, line.find(name).unwrap());
        assert!(at("impl Tr for S {}", "Tr"));
        assert!(at("impl<T: Send> Tr for S<T> {", "Tr"));
        assert!(at("unsafe impl Other for Tr {}", "Tr"));
        assert!(!at("impl Wrapper<Tr> for S {}", "Tr"));
        assert!(!at("impl<T: Tr> Other for T {}", "Tr"));
        assert!(!at("impl S where S: Tr {}", "Tr"));
        assert!(!at("impl Other for Vec<Tr> {", "Tr"));
        assert!(at("impl Fn() -> u8 for Tr {}", "Tr"));
    }
}
