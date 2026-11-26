/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Where a type is typed (`let x: u`, `-> i`, `as u`, `Vec<u`): the language's own types first, and no function,
//! macro or value among the names. The built-in types are no item of any index, so a crate that did not build, or
//! has no index yet, still gets them; and the index's matches, capped, no longer crowd them out.

use ridx_format::DefKind;
use serde_json::{json, Value};

/// The built-in types rust-analyzer completes on stable.
const BUILTIN_TYPES: [&str; 17] = ["bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64"];

/// LSP `CompletionItemKind.Struct`, rust-analyzer's kind for a built-in type.
const STRUCT_KIND: u32 = 22;

/// Whether the name being typed after `before` (the line up to it, the name's own characters removed) is a type.
pub(super) fn in_type_position(before: &str) -> bool {
    let mut t = before.trim_end();
    // `&`, `&mut`, `&'a`, `*const`, `*mut`, `dyn`, `impl` lead into a type and are part of it.
    loop {
        let stripped =
            ["&", "mut", "*const", "*mut", "dyn", "impl"].iter().find_map(|p| t.strip_suffix(p).filter(|rest| p.starts_with(['&', '*']) || !rest.ends_with(is_ident_char)));
        let stripped = stripped.or_else(|| lifetime_suffix(t));
        match stripped {
            Some(rest) if rest.len() < t.len() => t = rest.trim_end(),
            _ => break,
        }
    }
    if t.ends_with("->") {
        return true;
    }
    if let Some(rest) = t.strip_suffix(':') {
        return !rest.ends_with(':');
    }
    if let Some(rest) = t.strip_suffix("as") {
        return rest.ends_with(|c: char| c.is_whitespace() || c == ')');
    }
    if let Some(rest) = t.strip_suffix('<') {
        // `Vec<` and `::<`, not a comparison `a < b`.
        return rest.ends_with(|c: char| is_ident_char(c) || c == ':');
    }
    if t.ends_with(',') {
        return open_angle_brackets(t) > 0;
    }
    false
}

/// The text before a trailing lifetime (`&'a`), if `t` ends with one.
fn lifetime_suffix(t: &str) -> Option<&str> {
    let name_start = t.trim_end_matches(is_ident_char);
    name_start.strip_suffix('\'').filter(|rest| rest.len() < t.len() - 1 && rest.ends_with('&'))
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// How many generic argument lists are open at the end of `t`: a `<` after a name, closed by a `>` that ends no
/// `->` or `=>`.
fn open_angle_brackets(t: &str) -> i32 {
    let b = t.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'<' if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b':') => depth += 1,
            b'>' if i == 0 || !matches!(b[i - 1], b'-' | b'=') => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    depth
}

/// Whether a name of `kind` is a value, never a type: left out where a type is typed.
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
    fn test_type_positions() {
        for before in [
            "    let x: ",
            "fn f(a: ",
            "fn f() -> ",
            "    let n = x as ",
            "    let v: Vec<",
            "    let m: HashMap<String, ",
            "fn f(s: &",
            "fn f(s: &mut ",
            "fn f(s: &'a ",
            "fn f(p: *const ",
            "fn f(x: impl ",
            "    let b: Box<dyn ",
            "    x.parse::<",
            "struct S { a: ",
        ] {
            assert!(in_type_position(before), "{before:?}");
        }
        for before in ["    let x = ", "    foo(", "    foo(a, ", "    if a < ", "    std::", "    x.", "    let mutable = ", "    let x = &", "    match x { A => "] {
            assert!(!in_type_position(before), "{before:?}");
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
