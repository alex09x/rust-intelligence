/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

/// The hover at the first `name` in `src` after `after`, with layouts recorded at `notes` (1-based column, text).
fn hover_at(src: &str, after: &str, name: &str, ty: &str, notes: &[(u32, &'static str)]) -> Option<Value> {
    let from = src.find(after).unwrap_or(0) + after.len();
    let at = from + src[from..].find(name).unwrap();
    let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = src[..line_start].matches('\n').count() as u32;
    let col = src[line_start..at].chars().count() as u32 + 1;
    let notes = notes.to_vec();
    binding_hover(src, line, col, ty, &move |c| notes.iter().find(|(n, _)| *n == c).map(|(_, t)| t.to_string()))
}

fn text(h: Option<Value>) -> Option<String> {
    h.map(|v| v["contents"]["value"].as_str().unwrap_or_default().to_string())
}

/// prod-code, m=20120: the parameter `on` of `fn gap(a: Guard, on: bool)` showed `Type: bool`.
#[test]
fn test_a_parameter_shows_its_name_type_layout_and_range() {
    let src = "fn gap(a: Guard, on: bool) {}";
    let h = hover_at(src, "", "on", "bool", &[(18, "size = 1, align = 0x1, no Drop")]).unwrap();
    assert_eq!(h["contents"]["value"], "```rust\non: bool\n```\n\n---\n\nsize = 1, align = 0x1, no Drop");
    assert_eq!(h["range"], json!({ "start": { "line": 0, "character": 17 }, "end": { "line": 0, "character": 19 } }));
    let a = hover_at(src, "", "a:", "app::Guard", &[(8, "size = 24 (0x18), align = 0x8, needs Drop")]);
    assert_eq!(text(a).as_deref(), Some("```rust\na: Guard\n```\n\n---\n\nsize = 24 (0x18), align = 0x8, needs Drop"));
}

/// Review of 2e1fcfb1: the layout is the one at the pattern's own start, at its `mut` however far, never a neighbour's.
#[test]
fn test_the_layout_is_the_one_where_the_pattern_starts() {
    let note = "size = 1, align = 0x1, no Drop";
    for src in ["fn f(mut n: u8) {}", "fn f(mut  n: u8) {}"] {
        assert_eq!(text(hover_at(src, "", "n:", "u8", &[(6, note)])).as_deref(), Some(format!("```rust\nmut n: u8\n```\n\n---\n\n{note}").as_str()), "{src}");
    }
    let tuple = "fn f<T>((a,  b): (u8, T)) {}";
    assert_eq!(text(hover_at(tuple, "", "b)", "T", &[(10, note)])).as_deref(), Some("```rust\nb: T\n```"), "a's layout is not b's");
    let src = "fn f() {\n    let mut v: Vec<String> = Vec::new();\n    let c = Circle::new(2.0);\n}\n";
    let v = hover_at(src, "", "v:", "std::vec::Vec<std::string::String>", &[(9, "size = 24 (0x18), align = 0x8, needs Drop")]);
    assert_eq!(text(v).as_deref(), Some("```rust\nlet mut v: Vec<String>\n```\n\n---\n\nsize = 24 (0x18), align = 0x8, needs Drop"));
    assert_eq!(text(hover_at(src, "let ", "c ", "shapes::Circle", &[])).as_deref(), Some("```rust\nlet c: Circle\n```"));
}

/// Review of 2e1fcfb1: `r#flag` lost its `r#`, after a character outside the BMP.
#[test]
fn test_a_raw_identifier_keeps_its_prefix_in_the_name_and_the_range() {
    let src = "/*🙂*/ fn f(r#flag: bool) {}";
    let h = hover_at(src, "", "r#flag", "bool", &[(12, "size = 1, align = 0x1, no Drop")]).unwrap();
    assert_eq!(h["contents"]["value"], "```rust\nr#flag: bool\n```\n\n---\n\nsize = 1, align = 0x1, no Drop");
    assert_eq!(h["range"]["start"]["character"], 12);
    assert_eq!(h["range"]["end"]["character"], 18);
}

#[test]
fn test_bindings_are_what_the_syntax_tree_says() {
    let src = "fn g(x: Option<u32>, p: Point, items: Vec<u32>) {\n    f(42);\n    match x { Some(v) => v, n if n > 0 => n, _ => 0 };\n    let add = |a, b| a + b;\n    let Point { x, y } = p;\n    for item in items {}\n    total.count();\n    std::mem::take(x);\n}\nfn h(n /* units */: u8) {}\n";
    let at = |after: &str, name: &str| text(hover_at(src, after, name, "u32", &[]));
    assert_eq!(at("f(", "42"), None, "a literal");
    assert_eq!(at("Some(", "v"), Some("```rust\nv: u32\n```".into()));
    assert_eq!(at("v, ", "n"), Some("```rust\nn: u32\n```".into()), "a binding with a guard");
    assert_eq!(at("|a, ", "b"), Some("```rust\nb: u32\n```".into()));
    assert_eq!(at("Point { x, ", "y"), Some("```rust\ny: u32\n```".into()));
    assert_eq!(at("for ", "item"), Some("```rust\nitem: u32\n```".into()));
    assert_eq!(at("fn h(", "n"), Some("```rust\nn: u32\n```".into()), "a comment before the colon");
    assert_eq!(at("total.", "count"), None, "a method");
    assert_eq!(at("    ", "total"), None, "an expression");
    assert_eq!(at("take(", "x"), None, "an argument");
}
