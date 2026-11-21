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

/// The 0-based character range of the self type of the impl header that starts at character `col` of `line`: after
/// `for` (`impl Tr for S<T>`: `S<T>`), else after the impl's generics (`impl<T> S<T>`), up to `where`, `{` or the end
/// of the line. It is where rust-analyzer places an implementation (prod-code, m=20226: ridx gave the `impl`). `None`
/// when the header does not start there or its self type is not on that line.
pub(super) fn self_type_range(line: &str, col: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let rest: String = chars.get(col..)?.iter().collect();
    if !is_impl_header(&rest) {
        return None;
    }
    // The character just after the `impl` keyword.
    let at = rest.find("impl")? + 4;
    let mut i = col + rest[..at].chars().count();
    let skip_ws = |mut i: usize| {
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        i
    };
    i = skip_ws(i);
    if chars.get(i) == Some(&'<') {
        let mut depth = 0i32;
        while let Some(&c) = chars.get(i) {
            match c {
                '<' => depth += 1,
                '>' if i > 0 && chars[i - 1] != '-' => depth -= 1,
                _ => {}
            }
            i += 1;
            if depth == 0 {
                break;
            }
        }
        i = skip_ws(i);
    }
    let word_at = |j: usize, w: &str| {
        let wc: Vec<char> = w.chars().collect();
        chars.get(j..j + wc.len()) == Some(&wc[..])
            && (j == 0 || !chars[j - 1].is_alphanumeric() && chars[j - 1] != '_')
            && chars.get(j + wc.len()).is_none_or(|c| !c.is_alphanumeric() && *c != '_')
    };
    // The head's end (`where`, `{`) and a `for` in it, both at depth 0.
    let (mut depth, mut start, mut end) = (0i32, i, chars.len());
    let mut j = i;
    while j < chars.len() {
        match chars[j] {
            '<' | '(' | '[' => depth += 1,
            '>' if chars[j - 1] != '-' => depth -= 1,
            ')' | ']' => depth -= 1,
            '{' if depth == 0 => {
                end = j;
                break;
            }
            _ if depth == 0 && word_at(j, "where") => {
                end = j;
                break;
            }
            _ if depth == 0 && word_at(j, "for") && chars.get(j + 3) != Some(&'<') => start = skip_ws(j + 3),
            _ => {}
        }
        j += 1;
    }
    while end > start && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    (end > start).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::{is_head_position, is_impl_header, self_type_range};

    /// prod-code, m=20226: rust-analyzer places an implementation at the impl's self type.
    #[test]
    fn the_self_type_of_an_impl_header_is_found() {
        let ty = |line: &str| self_type_range(line, 0).map(|(s, e)| line.chars().skip(s).take(e - s).collect::<String>());
        assert_eq!(ty("impl Shape for Circle {").as_deref(), Some("Circle"));
        assert_eq!(ty("impl<T: Clone> Tr<T> for Wrapper<T> where T: Copy {").as_deref(), Some("Wrapper<T>"));
        assert_eq!(ty("impl Config {").as_deref(), Some("Config"));
        assert_eq!(ty("impl<T> Vec<T> {").as_deref(), Some("Vec<T>"));
        assert_eq!(ty("unsafe impl Send for S {}").as_deref(), Some("S"));
        assert_eq!(ty("impl Fn() -> u8 for F {}").as_deref(), Some("F"));
        assert_eq!(ty("impl<T> From<T> for Box<dyn Fn(T) -> T>").as_deref(), Some("Box<dyn Fn(T) -> T>"));
        assert_eq!(ty("impl<'a> Tr for &'a S {").as_deref(), Some("&'a S"));
        assert_eq!(ty("fn f(x: impl Tr) {}"), None);
        let line = "    impl Shape for Circle {";
        assert_eq!(self_type_range(line, 4), Some((19, 25)));
    }

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
