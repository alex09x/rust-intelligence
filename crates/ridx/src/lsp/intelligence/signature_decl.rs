/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Whether the `(` or `{` signature help found after a name opens a declaration rather than a call or a struct literal:
//! an item's own parameter list or body (`fn total(&self, …)`, `struct Cart {`, `impl<T> Trait for path::S<T> {`,
//! `fn f() -> S {`) or an enum variant's fields. rust-analyzer shows no signature there.

/// Whether the name starting at `name_start` in `chars` (the text before the cursor) is declared by the delimiter after it.
pub(crate) fn opens_declaration(chars: &[char], name_start: usize) -> bool {
    header_declares(&item_header(chars, name_start), false) || in_enum_variant_list(chars, name_start)
}

/// The text before `end` back to the start of its item or statement: a `;`, `{` or `}`, or a `(`, `[`, `<` or `=` not
/// inside brackets closed before `end` (`->` and `=>` are no brackets).
fn item_header(chars: &[char], end: usize) -> String {
    let mut depth = 0usize;
    let mut i = end;
    while i > 0 {
        let c = chars[i - 1];
        let arrow = i >= 2 && matches!(chars[i - 2], '-' | '=');
        match c {
            ';' | '{' | '}' => break,
            ')' | ']' => depth += 1,
            '>' if !arrow => depth += 1,
            '(' | '[' | '<' | '=' if depth == 0 => break,
            '(' | '[' | '<' => depth -= 1,
            _ => {}
        }
        i -= 1;
    }
    chars[i..end].iter().collect()
}

/// Whether `header` declares an item: `impl`, or `fn`, `struct`, `enum`, `union`, `trait` or `mod` followed by the
/// item's name (`fn(i32)` is a type, `set.union(b)` a call). With `enum_only`, only `enum NAME`.
fn header_declares(header: &str, enum_only: bool) -> bool {
    let h: Vec<char> = header.chars().collect();
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut i = 0;
    while i < h.len() {
        if !is_ident(h[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < h.len() && is_ident(h[i]) {
            i += 1;
        }
        // A raw identifier (`r#impl`) or a field or method (`x.union`) is no keyword.
        if start > 0 && matches!(h[start - 1], '#' | '.' | '\'') {
            continue;
        }
        let word: String = h[start..i].iter().collect();
        let mut j = i;
        while j < h.len() && h[j].is_whitespace() {
            j += 1;
        }
        // The header ends where the declared name starts.
        let named = j == h.len() || h[j].is_alphabetic() || h[j] == '_';
        let declares = match word.as_str() {
            "enum" => named,
            _ if enum_only => false,
            "impl" => true,
            "fn" | "struct" | "union" | "trait" | "mod" => named,
            _ => false,
        };
        if declares {
            return true;
        }
    }
    false
}

/// Whether the name is a variant in an enum's body: right after its `{`, a `,` or an attribute, directly inside
/// `enum NAME … {` (a discriminant `A = make(` follows `=`).
fn in_enum_variant_list(chars: &[char], name_start: usize) -> bool {
    let mut i = name_start;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i == 0 || !matches!(chars[i - 1], '{' | ',' | ']') {
        return false;
    }
    let mut depth = 0usize;
    while i > 0 {
        match chars[i - 1] {
            ')' | ']' | '}' => depth += 1,
            '(' | '[' if depth == 0 => return false,
            '{' if depth == 0 => return header_declares(&item_header(chars, i - 1), true),
            '(' | '[' | '{' => depth -= 1,
            _ => {}
        }
        i -= 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::opens_declaration;

    /// `before` is the text up to the name the delimiter follows.
    fn decl(before: &str) -> bool {
        let chars: Vec<char> = before.chars().collect();
        opens_declaration(&chars, chars.len())
    }

    /// Review of 24fb28b8: only the word right before the name was checked, so generic, trait and path-qualified
    /// `impl` blocks, enum variants, raw identifiers and a function body after `-> S` still showed a signature.
    #[test]
    fn test_declarations_are_told_from_calls() {
        for d in [
            "pub fn ",
            "pub(crate) async unsafe fn ",
            "fn r#",
            "fn total<T: Fn() -> i32>(&self) -> ",
            "struct ",
            "pub struct S<T>(T);\nunion ",
            "trait ",
            "mod ",
            "impl ",
            "impl<T> ",
            "impl<'a> Trait for ",
            "impl<T> Trait<Item = u8> for path::",
            "impl<T> Iterator for S<T> where T: ",
            "fn f() -> ",
            "enum E { ",
            "pub enum E<T> where T: Clone {\n    A(u8),\n    #[default]\n    ",
            "enum E {\n    A { x: u8 },\n    ",
        ] {
            assert!(decl(d), "a declaration: {d:?}");
        }
        for c in [
            "fn t() { ",
            "fn t() { let x = ",
            "fn t() { foo(1, ",
            "fn t() { x.union(b).",
            "fn t() { set.",
            "fn t() { foo(x as fn(i32), ",
            "fn f(x: [u8; ",
            "fn t() { let s = S { a: ",
            "fn t() { match x { A => ",
            "enum E { A = ",
            "enum E { A(Vec<u8>), B = f(1, ",
            "fn t() { let f: fn(i32) -> u8 = ",
            "fn t() { for p in ",
        ] {
            assert!(!decl(c), "a call or literal: {c:?}");
        }
    }
}
