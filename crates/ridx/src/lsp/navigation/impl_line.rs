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

/// The byte range of the self type of the `impl` whose header holds byte `start` of the file `tree` was parsed from:
/// its last type before the `where` clause and the items (`Wrapper<T>` of `impl<T> Tr for Wrapper<T>`, `S` of
/// `impl S`). It is where rust-analyzer places an implementation (prod-code, m=20226: ridx gave the `impl`). The
/// syntax tree tells it, never the characters of the line: a comment, `r#where` or `for <'a>` is no keyword there
/// (review of 465e02ad). `None` when no impl header holds `start`.
pub(super) fn self_type_range(tree: &ridx_syntax::SyntaxTree<'_>, start: usize) -> Option<std::ops::Range<usize>> {
    use ridx_syntax::SyntaxKind as K;
    let node = tree.covering(start);
    let imp = std::iter::once(node).chain(tree.ancestors(node)).find(|&n| tree.kind(n) == K::Impl)?;
    if tree.child(imp, K::ItemList).is_some_and(|items| start >= tree.range(items).start) {
        return None;
    }
    let ty = tree.children(imp).filter(|&c| !matches!(tree.kind(c), K::GenericParamList | K::WhereClause | K::ItemList | K::Attr | K::Visibility)).last()?;
    Some(tree.range(ty))
}

#[cfg(test)]
mod tests {
    use super::{is_head_position, is_impl_header, self_type_range};

    /// prod-code, m=20226: rust-analyzer places an implementation at the impl's self type.
    #[test]
    fn the_self_type_of_an_impl_header_is_found() {
        let ty = |src: &str| {
            let tree = ridx_syntax::parse(src);
            self_type_range(&tree, src.find("impl").unwrap()).map(|r| src[r].to_string())
        };
        assert_eq!(ty("impl Shape for Circle {}").as_deref(), Some("Circle"));
        assert_eq!(ty("impl<T: Clone> Tr<T> for Wrapper<T> where T: Copy {}").as_deref(), Some("Wrapper<T>"));
        assert_eq!(ty("impl Config {}").as_deref(), Some("Config"));
        assert_eq!(ty("impl<T> Vec<T> {}").as_deref(), Some("Vec<T>"));
        assert_eq!(ty("unsafe impl Send for S {}").as_deref(), Some("S"));
        assert_eq!(ty("impl Fn() -> u8 for F {}").as_deref(), Some("F"));
        assert_eq!(ty("impl<T> From<T> for Box<dyn Fn(T) -> T> {}").as_deref(), Some("Box<dyn Fn(T) -> T>"));
        assert_eq!(ty("impl<'a> Tr for &'a S {}").as_deref(), Some("&'a S"));
        assert_eq!(ty("fn f(x: impl Tr) {}"), None);
        // Review of 465e02ad: a comment, a raw identifier and a binder with a space are no keywords.
        assert_eq!(ty("impl Tr for /* where { */ S {}").as_deref(), Some("S"));
        assert_eq!(ty("impl Tr for r#where {}").as_deref(), Some("r#where"));
        assert_eq!(ty("impl Tr for Box<dyn for <'a> Fn(&'a u8)> {}").as_deref(), Some("Box<dyn for <'a> Fn(&'a u8)>"));
        assert_eq!(ty("impl Tr\n    for S\n{}").as_deref(), Some("S"), "a header on several lines");
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
