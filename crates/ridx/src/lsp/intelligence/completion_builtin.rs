/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Where a type is typed (`let x: u`, `-> i`, `as u`, `Vec<u`): the language's own types first. The built-in types
//! are no item of any index, so a crate that did not build, or has no index yet, still gets them; and the index's
//! matches, capped, no longer crowd them out. Only where nothing but a type can follow (a binding's annotation, a
//! return type, a cast) are functions, macros and other values left out: a struct literal's field (`S { a: x }`), a
//! const generic argument (`Buf<u8, CAP>`) or a comparison (`f(a<b, c)`) reads the same as a type up to the cursor.

use ridx_format::DefKind;
use serde_json::{json, Value};

/// The built-in types rust-analyzer completes on stable.
const BUILTIN_TYPES: [&str; 17] = ["bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64"];

/// LSP `CompletionItemKind.Struct`, rust-analyzer's kind for a built-in type.
const STRUCT_KIND: u32 = 22;

/// What the name being typed is, as far as the text before it tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TypeContext {
    /// No sign of a type.
    No,
    /// Maybe a type, maybe a value: the built-in types lead, nothing is left out.
    Possible,
    /// Only a type can follow: values are left out too.
    Certain,
}

/// The context of the name typed after `before`: the document up to it (the name's own characters removed), comments
/// and strings blanked. Only the statement the name is in is read, across lines.
pub(super) fn type_context(before: &str) -> TypeContext {
    let statement = before.rfind([';', '{', '}']).map_or(before, |i| &before[i + 1..]);
    let mut t = statement.trim_end();
    // `&`, `&mut`, `&'a`, `*const`, `*mut`, `dyn`, `impl` lead into a type and are part of it.
    loop {
        let stripped =
            ["&", "mut", "*const", "*mut", "dyn", "impl"].iter().find_map(|p| t.strip_suffix(p).filter(|rest| p.starts_with(['&', '*']) || !rest.ends_with(is_ident_char)));
        match stripped.or_else(|| lifetime_suffix(t)) {
            Some(rest) if rest.len() < t.len() => t = rest.trim_end(),
            _ => break,
        }
    }
    if t.ends_with("->") {
        return TypeContext::Certain;
    }
    if let Some(rest) = t.strip_suffix(':') {
        return if rest.ends_with(':') {
            TypeContext::No
        } else if annotates_a_binding(rest) {
            TypeContext::Certain
        } else {
            TypeContext::Possible
        };
    }
    if let Some(rest) = t.strip_suffix("as") {
        if rest.ends_with(|c: char| c.is_whitespace() || c == ')') && !rest.trim().is_empty() {
            return TypeContext::Certain;
        }
    }
    // `Vec<` and `::<` (not `a < b`): a type, or a const argument.
    if t.strip_suffix('<').is_some_and(|rest| rest.ends_with(|c: char| is_ident_char(c) || c == ':')) {
        return TypeContext::Possible;
    }
    // The first element of a tuple or an array, or the next element of a list still open: as the list itself
    // (`let p: (u`, `let p: (u8, u`, `let b: [u`), not a call's or an index's argument (`f(u`, `xs[u`). The next
    // argument of a generic list may be a const too, or a value after a comparison without spaces (`f(a<b, c`).
    let open = if t.ends_with(['(', '[']) {
        Some(t.len() - 1)
    } else if t.ends_with(',') {
        innermost_open(t)
    } else {
        None
    };
    match open.map(|at| (at, t.as_bytes()[at])) {
        Some((_, b'<')) => TypeContext::Possible,
        // A function pointer's or an `Fn` trait's parameters are types (`fn(u8, u`, `impl Fn(u`); a function's own
        // (`fn name(a`) are not, and `fn name` ends with its name.
        Some((at, b'(')) if ["fn", "Fn", "FnMut", "FnOnce"].iter().any(|w| last_word(&t[..at], w).is_some_and(|i| t[..at][i + w.len()..].trim().is_empty())) => {
            TypeContext::Certain
        }
        Some((at, _)) => type_context(&t[..at]),
        None => TypeContext::No,
    }
}

/// The offset of the innermost `(`, `[` or generic `<` (after a name) still open at the end of `t`.
fn innermost_open(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'[' => open.push(i),
            b'<' if t[..i].ends_with(|c: char| is_ident_char(c) || c == ':') => open.push(i),
            b')' | b']' => {
                // Closes its bracket, and any `<` left open inside it (a comparison).
                while let Some(o) = open.pop() {
                    if b[o] != b'<' {
                        break;
                    }
                }
            }
            b'>' if (i == 0 || !matches!(b[i - 1], b'-' | b'=')) && open.last().is_some_and(|&o| b[o] == b'<') => {
                open.pop();
            }
            _ => {}
        }
    }
    open.pop()
}

/// Whether the `:` after `rest` (the statement up to it) annotates a binding or an item, so that a type follows:
/// `let x:`, `|a, b:`, a function's parameter, `const N:`, `static S:`. Not a field of a struct or of its literal.
fn annotates_a_binding(rest: &str) -> bool {
    let words: Vec<&str> = rest.split(|c: char| !is_ident_char(c)).filter(|w| !w.is_empty()).collect();
    if words.len() >= 2 && matches!(words[words.len() - 2], "const" | "static") {
        return true;
    }
    if let Some(at) = last_word(rest, "let") {
        if !rest[at..].contains('=') {
            return true;
        }
    }
    if let Some(bar) = rest.rfind('|') {
        if rest[bar + 1..].chars().all(|c| is_ident_char(c) || c.is_whitespace() || matches!(c, ',' | '&')) {
            return true;
        }
    }
    // The parameter list of a function: its `(` still open, after `fn name`.
    open_paren(rest).is_some_and(|open| last_word(&rest[..open], "fn").is_some())
}

/// The offset of the last `word` in `text` standing as a whole word.
fn last_word(text: &str, word: &str) -> Option<usize> {
    text.rmatch_indices(word).map(|(i, _)| i).find(|&i| {
        let before_ok = text[..i].chars().next_back().is_none_or(|c| !is_ident_char(c));
        let after_ok = text[i + word.len()..].chars().next().is_none_or(|c| !is_ident_char(c));
        before_ok && after_ok
    })
}

/// The offset of the last `(` of `text` not closed in it.
fn open_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in text.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => return Some(i),
            '(' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The text before a trailing lifetime (`&'a`), if `t` ends with one.
fn lifetime_suffix(t: &str) -> Option<&str> {
    let name_start = t.trim_end_matches(is_ident_char);
    name_start.strip_suffix('\'').filter(|rest| rest.len() < t.len() - 1 && rest.ends_with('&'))
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether a name of `kind` is a value, never a type: left out where only a type can follow.
pub(super) fn is_value_kind(kind: DefKind) -> bool {
    matches!(
        kind,
        DefKind::Fn
            | DefKind::AssocFn
            | DefKind::Const
            | DefKind::AssocConst
            | DefKind::Static
            | DefKind::StructCtor
            | DefKind::VariantCtor
            | DefKind::Macro
            | DefKind::Local
            | DefKind::Field
            | DefKind::ConstParam
    )
}

/// The built-in types starting with `prefix`, ordered before every other name, by width (`u8` before `u128`).
pub(super) fn builtin_type_items(prefix: &str) -> Vec<Value> {
    BUILTIN_TYPES
        .iter()
        .enumerate()
        .filter(|(_, name)| name.starts_with(prefix))
        .map(|(i, name)| json!({ "label": name, "kind": STRUCT_KIND, "sortText": format!("!{i:02}") }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_contexts() {
        use TypeContext::*;
        let cases = [
            // Only a type can follow.
            ("    let x: ", Certain),
            ("    let (a, mut b): ", Certain),
            ("fn f(a: ", Certain),
            ("pub fn f<T: Clone>(a: T, b: ", Certain),
            ("fn f() -> ", Certain),
            ("    let n = x as ", Certain),
            ("    xs.iter().map(|a, b: ", Certain),
            ("const MAX: ", Certain),
            ("fn f(s: &", Certain),
            ("fn f(s: &mut ", Certain),
            ("fn f(s: &'a ", Certain),
            ("fn f(p: *const ", Certain),
            ("fn f(x: impl ", Certain),
            // A type, or a value read the same way up to the cursor.
            ("    let v: Vec<", Possible),
            ("    let m: HashMap<String, ", Possible),
            ("    let r: Result<\n        ", Possible),
            ("    let b: Box<dyn ", Possible),
            ("    x.parse::<", Possible),
            ("struct S {\n    a: ", Possible),
            ("    let r = Record { field: ", Possible),
            ("    let b: Buffer<u8, ", Possible),
            ("    consume(a<b, ", Possible),
            // Inside a tuple or an array: as the list itself (review of c697ee37).
            ("    let p: (", Certain),
            ("    let p: (u8, ", Certain),
            ("    let b: [", Certain),
            ("fn f() -> (bool, ", Certain),
            ("    let v: Vec<(", Possible),
            ("fn f(a: u8, ", No),
            ("    consume(", No),
            ("    xs[", No),
            ("    let t = (1, ", No),
            // Function pointers, `Fn` traits, identifiers past ASCII (review of adb611a7).
            ("    let cb: fn(", Certain),
            ("    let cb: fn(u8, ", Certain),
            ("fn f(g: impl Fn(", Certain),
            ("fn f(g: &dyn FnMut(u8, ", Certain),
            ("    callback(", No),
            ("fn run(", No),
            ("    let v: 容器<u8, ", Possible),
            // A value.
            ("    let x = ", No),
            ("    foo(", No),
            ("    foo(a, ", No),
            ("    if a < ", No),
            ("    std::", No),
            ("    x.", No),
            ("    let mutable = ", No),
            ("    let x = &", No),
            ("    match x { A => ", No),
            ("    let x: u8 = 1;\n    ", No),
        ];
        for (before, expected) in cases {
            assert_eq!(type_context(before), expected, "{before:?}");
        }
    }

    #[test]
    fn test_builtin_type_items_by_prefix() {
        let labels: Vec<String> = builtin_type_items("u").iter().map(|i| i["label"].as_str().unwrap().to_string()).collect();
        assert_eq!(labels, ["u8", "u16", "u32", "u64", "u128", "usize"]);
        assert_eq!(builtin_type_items("").len(), BUILTIN_TYPES.len());
        assert!(builtin_type_items("St").is_empty(), "case-sensitive, as rustc is");
        assert_eq!(builtin_type_items("bo")[0]["sortText"], "!00");
    }
}
